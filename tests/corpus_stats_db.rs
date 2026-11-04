//! `youtube-corpus stats` against a real Postgres (#200): per-video row attribution, disk
//! attribution and SponsorBlock savings from a stored snapshot (no network).

use uuid::Uuid;
use youtube_corpus::corpus_stats::{
    collect_corpus_stats, render_markdown, CorpusStatsRequest, SponsorBlockFetch, EMBEDDINGS,
    MEDIA, SEARCH_INDEX, TRANSCRIPTS,
};
use youtube_corpus::{parse_hash_response, persist_sponsorblock_snapshot};

fn sha256_hex(value: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn stats_attribute_rows_files_and_sponsorblock_savings_per_video() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();

    let video_id = Uuid::new_v4();
    let youtube_id = format!("st{}", &video_id.simple().to_string()[..9]);
    let source_url = format!("https://www.youtube.com/watch?v={youtube_id}");
    sqlx::query(
        "INSERT INTO videos (id, youtube_id, source_url, title, duration_seconds)
         VALUES ($1, $2, $3, 'Stats fixture', 600.0)",
    )
    .bind(video_id)
    .bind(&youtube_id)
    .bind(&source_url)
    .execute(&pool)
    .await
    .unwrap();
    let stream_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status)
         VALUES ($1, $2, 'caption_auto', 'en', 'completed')",
    )
    .bind(stream_id)
    .bind(video_id)
    .execute(&pool)
    .await
    .unwrap();
    // Ten 60 s segments; segments 0 and 1 lie in the sponsor range [0, 120).
    for index in 0..10_i64 {
        sqlx::query(
            "INSERT INTO transcript_segments
             (id, stream_id, video_id, segment_index, start_seconds, end_seconds, text, language)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'en')",
        )
        .bind(Uuid::new_v4())
        .bind(stream_id)
        .bind(video_id)
        .bind(index)
        .bind(index as f64 * 60.0)
        .bind((index + 1) as f64 * 60.0)
        .bind(format!("segment number {index} of the stats fixture"))
        .execute(&pool)
        .await
        .unwrap();
    }

    let response = serde_json::json!([{
        "videoID": youtube_id,
        "hash": sha256_hex(&youtube_id),
        "segments": [
            {"category": "sponsor", "actionType": "skip", "segment": [0.0, 120.0], "UUID": "a"},
            {"category": "intro", "actionType": "skip", "segment": [60.0, 150.0], "UUID": "b"},
            {"category": "poi_highlight", "actionType": "poi", "segment": [300.0, 300.0], "UUID": "c"},
            {"category": "outro", "actionType": "skip", "segment": [570.0, 700.0], "UUID": "d"}
        ]
    }]);
    let snapshot = parse_hash_response(
        &youtube_id,
        &[
            "sponsor".to_string(),
            "intro".to_string(),
            "outro".to_string(),
            "poi_highlight".to_string(),
        ],
        &serde_json::to_vec(&response).unwrap(),
    )
    .unwrap();
    persist_sponsorblock_snapshot(&pool, video_id, &snapshot)
        .await
        .unwrap();

    let work_dir = tempfile::tempdir().unwrap();
    let item = work_dir
        .path()
        .join(format!("https-www-youtube-com-watch-v-{youtube_id}"));
    std::fs::create_dir_all(item.join("captions")).unwrap();
    std::fs::create_dir_all(item.join("media")).unwrap();
    std::fs::write(item.join("captions/x.en.vtt"), vec![b'a'; 1000]).unwrap();
    std::fs::write(item.join("media/x.m4a"), vec![b'b'; 6000]).unwrap();
    std::fs::create_dir_all(work_dir.path().join("orphan")).unwrap();
    std::fs::write(work_dir.path().join("orphan/y.json"), b"{}").unwrap();

    let report = collect_corpus_stats(
        &pool,
        &CorpusStatsRequest {
            work_dir: work_dir.path().to_path_buf(),
            shared_dirs: Vec::new(),
            sponsorblock: SponsorBlockFetch::Off,
        },
    )
    .await
    .unwrap();

    let video = report
        .videos
        .iter()
        .find(|v| v.video_id == video_id)
        .expect("fixture video in report");
    assert_eq!(video.bucket.as_deref(), Some("10–30 min"));
    assert_eq!(video.counts["transcript_segments"], 10);
    assert_eq!(video.counts["transcript_streams"], 1);
    assert_eq!(video.disk_bytes_by_kind["captions"], 1000);
    assert_eq!(video.disk_bytes_by_kind["audio"], 6000);
    assert_eq!(video.bytes_by_artifact[MEDIA], 6000);
    assert!(video.bytes_by_artifact[TRANSCRIPTS] > 1000);
    assert!(video.bytes_by_artifact[SEARCH_INDEX] > 0, "tsvector bytes");
    assert!(
        !video.bytes_by_artifact.contains_key(EMBEDDINGS),
        "no vectors stored"
    );
    assert!(report
        .disk
        .unattributed_items
        .contains(&"orphan".to_string()));

    let sponsorblock = video.sponsorblock.as_ref().expect("stored snapshot used");
    assert_eq!(sponsorblock.segments, 4);
    // [0, 150) ∪ [570, 600) — the outro is clipped to the duration, the highlight ignored.
    assert_eq!(sponsorblock.skippable_seconds, 180.0);
    assert_eq!(sponsorblock.skippable_share, Some(0.3));
    assert_eq!(sponsorblock.seconds_by_category["sponsor"], 120.0);
    assert_eq!(sponsorblock.seconds_by_category["intro"], 90.0);
    assert_eq!(sponsorblock.seconds_by_category["outro"], 30.0);
    assert!(!sponsorblock
        .seconds_by_category
        .contains_key("poi_highlight"));
    // Segments 0 and 1 are fully inside; segment 2 ([120, 180)) and segment 9 ([540, 600),
    // outro clipped to [570, 600)) are half inside (≥ 50 %).
    assert_eq!(sponsorblock.transcript_segments_skippable, 4);
    assert!(report.sponsorblock.savings.media_bytes_estimate >= 1800.0);

    let transcripts = report
        .database
        .tables
        .iter()
        .find(|t| t.table == "transcript_segments")
        .unwrap();
    assert!(transcripts.links_video && transcripts.rows >= 10);
    assert!(transcripts.indexes.iter().any(|i| i.artifact == EMBEDDINGS));
    assert!(report
        .embeddings
        .iter()
        .any(|e| e.table == "transcript_segments" && e.column == "embedding"));

    let markdown = render_markdown(&report);
    assert!(markdown.contains("## SponsorBlock savings"));
    assert!(markdown.contains("CC BY-NC-SA 4.0"));
    assert!(markdown.contains(&youtube_id));
}
