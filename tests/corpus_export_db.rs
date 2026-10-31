//! Postgres checks of the export snapshot loader's scope and order (issue #42 review).
//!
//! The source-span export has always scoped transcript segments by the segment row's own
//! `video_id` (which the schema allows to differ from its stream's) and ordered OCR tracks with
//! SQL (`start_seconds NULLS LAST, track_key, id` under the database collation). The snapshot
//! loader must keep both.

mod support;

use serde_json::Value;
use support::*;
use uuid::Uuid;
use youtube_corpus::corpus_export::{
    export_belief_evidence, export_corpus_core_jsonl, export_source_span, load_export_snapshot,
    CorpusExportSnapshot,
};
use youtube_corpus::{
    begin_visual_processing_run, export_source_span_batch, upsert_video_scene,
    upsert_visual_text_track, BeginVisualProcessingRunRequest, ProcessingProvenance,
    UpsertVideoSceneRequest, UpsertVisualTextTrackRequest, VisualProcessingKind, VisualTextRole,
};

async fn insert_video(pool: &sqlx::PgPool, id: Uuid, title: &str) {
    sqlx::query(
        "INSERT INTO videos (id, youtube_id, source_url, title, channel)
         VALUES ($1, $2, $3, $4, 'Channel')",
    )
    .bind(id)
    .bind(format!("yt-{id}"))
    .bind(format!("https://youtube.test/{id}"))
    .bind(title)
    .execute(pool)
    .await
    .unwrap();
}

fn spans_by_id(json: &str) -> (Vec<Value>, Vec<Value>) {
    let value: Value = serde_json::from_str(json).unwrap();
    (
        value["sources"].as_array().unwrap().clone(),
        value["spans"].as_array().unwrap().clone(),
    )
}

async fn assert_snapshot_matches_database(pool: &sqlx::PgPool, video_id: Option<Uuid>) -> String {
    let snapshot = load_export_snapshot(pool, video_id).await.unwrap();
    let from_snapshot = export_source_span(&snapshot, "git:scope").unwrap();
    let from_database = export_source_span_batch(pool, "git:scope", video_id)
        .await
        .unwrap();
    assert_eq!(
        from_snapshot,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&from_database).unwrap()
        )
    );
    // The snapshot file path reproduces the database export byte for byte.
    let round_tripped: CorpusExportSnapshot =
        serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
    assert_eq!(
        export_source_span(&round_tripped, "git:scope").unwrap(),
        from_snapshot
    );
    parse_corpus_core_jsonl(&export_corpus_core_jsonl(&snapshot, "git:scope").unwrap())
        .expect("consumable corpus-core");
    if video_id.is_none()
        && snapshot.transcript_streams.iter().any(|stream| {
            stream.processor_revision.is_none() && stream.processor_version == "0.1.0"
        })
    {
        // Other database suites deliberately retain legacy rows. A global export
        // must refuse them rather than relabeling their historical processor.
        assert!(export_belief_evidence(&snapshot, "git:scope").is_err());
    } else {
        validate_belief_batch(&export_belief_evidence(&snapshot, "git:scope").unwrap())
            .expect("importable belief evidence");
    }
    from_snapshot
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn loader_exports_only_persisted_speaker_turn_rows_with_original_provenance() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();

    let (with_turn, without_turn, corpus_id) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    insert_video(&pool, with_turn, "Speaker-only video").await;
    insert_video(&pool, without_turn, "Unannotated video").await;
    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Speaker export test')")
        .bind(corpus_id)
        .bind(format!("speaker-export-{corpus_id}"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO corpus_video_memberships (corpus_id, video_id) VALUES ($1, $2)")
        .bind(corpus_id)
        .bind(with_turn)
        .execute(&pool)
        .await
        .unwrap();
    let turn_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO research_annotations
           (id, corpus_id, video_id, kind, start_seconds, end_seconds, label,
            source_kind, processor, processor_version)
         VALUES ($1, $2, $3, 'speaker_turn', 1.125, 2.75, 'SPEAKER_03',
                 'processor', 'audio-analysis/diarization', 'git:model-fixture')",
    )
    .bind(turn_id)
    .bind(corpus_id)
    .bind(with_turn)
    .execute(&pool)
    .await
    .unwrap();

    let snapshot = load_export_snapshot(&pool, Some(with_turn)).await.unwrap();
    assert_eq!(snapshot.videos.len(), 1, "turn-only video is included");
    assert!(snapshot.transcript_streams.is_empty());
    assert_eq!(snapshot.speaker_turns.len(), 1);
    let turn = &snapshot.speaker_turns[0];
    assert_eq!(turn.id, turn_id);
    assert_eq!(turn.speaker_label.as_deref(), Some("SPEAKER_03"));
    assert_eq!(turn.start_seconds, Some(1.125));
    assert_eq!(turn.end_seconds, Some(2.75));
    assert_eq!(turn.producer_revision.as_deref(), Some("git:model-fixture"));
    assert_eq!(turn.annotation_revision, 1);
    let batch = parse_corpus_core_jsonl(
        &export_corpus_core_jsonl(&snapshot, "git:db-speaker-test").unwrap(),
    )
    .unwrap();
    let source = batch
        .records
        .iter()
        .find_map(|record| match record {
            corpus_core::CorpusRecord::Source(source) => Some(source),
            _ => None,
        })
        .unwrap();
    let exported: Vec<Value> = serde_json::from_str(&source.metadata["speaker_turns"]).unwrap();
    assert_eq!(exported.len(), 1);
    assert_eq!(exported[0]["id"], turn_id.to_string());
    assert_eq!(exported[0]["sourceKind"], "processor");
    assert_eq!(exported[0]["producerRevision"], "git:model-fixture");

    let empty = load_export_snapshot(&pool, Some(without_turn))
        .await
        .unwrap();
    assert!(empty.speaker_turns.is_empty(), "no turn may be invented");

    // These randomly identified rows belong exclusively to this ignored database test.
    for video in [with_turn, without_turn] {
        sqlx::query("DELETE FROM videos WHERE id = $1")
            .bind(video)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM corpora WHERE id = $1")
        .bind(corpus_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn loader_keeps_segment_video_scope_and_database_track_order() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();

    let (stream_video, segment_video) = (Uuid::new_v4(), Uuid::new_v4());
    insert_video(&pool, stream_video, "Stream video").await;
    insert_video(&pool, segment_video, "Segment video").await;
    let stream_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status, processor_revision)
         VALUES ($1, $2, 'asr', 'en', 'completed', 'git:synthetic-db-processor')",
    )
    .bind(stream_id)
    .bind(stream_video)
    .execute(&pool)
    .await
    .unwrap();
    let (own_segment, foreign_segment) = (Uuid::new_v4(), Uuid::new_v4());
    for (id, index, video, text) in [
        (own_segment, 0_i64, stream_video, "Own segment."),
        (
            foreign_segment,
            1,
            segment_video,
            "Segment naming another video.",
        ),
    ] {
        sqlx::query(
            "INSERT INTO transcript_segments
               (id, stream_id, video_id, segment_index, start_seconds, end_seconds, text)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(stream_id)
        .bind(video)
        .bind(index)
        .bind(index as f64)
        .bind(index as f64 + 1.0)
        .bind(text)
        .execute(&pool)
        .await
        .unwrap();
    }

    let run_id = begin_visual_processing_run(
        &pool,
        BeginVisualProcessingRunRequest {
            video_id: stream_video,
            kind: VisualProcessingKind::Ocr,
            provenance: ProcessingProvenance {
                processor: "visual-analysis".to_string(),
                processor_version: "ocr-fixture".to_string(),
                model: "trocr".to_string(),
                model_version: "fixture".to_string(),
                input_hash: "sha256:video".to_string(),
                config_hash: "sha256:ocr-config".to_string(),
                processing_config: serde_json::json!({}),
            },
        },
    )
    .await
    .unwrap();
    // Tied starts whose key order may differ between the collation and byte order, a -0.0
    // start that SQL ties with 0.0, and an untimed track.
    for (key, start) in [
        ("B", Some(1.0)),
        ("a", Some(1.0)),
        ("z", Some(0.0)),
        ("y", Some(-0.0)),
        ("untimed", None),
    ] {
        upsert_visual_text_track(
            &pool,
            UpsertVisualTextTrackRequest {
                run_id,
                video_id: stream_video,
                track_key: key.to_string(),
                text: format!("Track {key}"),
                role: VisualTextRole::SceneText,
                language: None,
                sample_count: 1,
                start_frame: None,
                end_frame: None,
                start_seconds: start,
                end_seconds: start,
                region: None,
                metadata: serde_json::json!({}),
                observation_ids: Vec::new(),
                scene_ids: Vec::new(),
            },
        )
        .await
        .unwrap();
    }
    let database_order = sqlx::query_scalar::<_, String>(
        "SELECT track_key FROM visual_text_tracks WHERE run_id = $1
         ORDER BY start_seconds NULLS LAST, track_key, id",
    )
    .bind(run_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    // Whole corpus scope: the stream's source describes its first segment's video, and each
    // span names its own segment's video.
    let (sources, spans) = spans_by_id(&assert_snapshot_matches_database(&pool, None).await);
    let source = sources
        .iter()
        .find(|source| source["id"] == stream_id.to_string())
        .unwrap();
    assert_eq!(source["metadata"]["videoId"], stream_video.to_string());
    let span = |spans: &[Value], id: Uuid| {
        spans
            .iter()
            .find(|span| span["id"] == id.to_string())
            .cloned()
    };
    assert_eq!(
        span(&spans, foreign_segment).unwrap()["metadata"]["videoId"],
        segment_video.to_string()
    );
    let ocr_order = spans
        .iter()
        .filter(|span| span["sourceId"] == format!("ocr:{run_id}"))
        .map(|span| {
            span["text"]
                .as_str()
                .unwrap()
                .trim_start_matches("Track ")
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ocr_order, database_order,
        "OCR spans keep the database order"
    );

    // Scoped to the segment's own video: only that segment, its stream and both videos.
    let snapshot = load_export_snapshot(&pool, Some(segment_video))
        .await
        .unwrap();
    assert_eq!(snapshot.transcript_streams.len(), 1);
    assert_eq!(snapshot.transcript_streams[0].video_id, stream_video);
    assert_eq!(snapshot.transcript_streams[0].segments.len(), 1);
    assert_eq!(
        snapshot.transcript_streams[0].segments[0].video_id,
        Some(segment_video)
    );
    assert!(snapshot.visual_text_runs.is_empty());
    let (sources, spans) =
        spans_by_id(&assert_snapshot_matches_database(&pool, Some(segment_video)).await);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["metadata"]["videoId"], segment_video.to_string());
    assert_eq!(sources[0]["title"], "Segment video");
    assert_eq!(spans.len(), 1);
    assert!(span(&spans, foreign_segment).is_some());
    let belief: Value =
        serde_json::from_str(&export_belief_evidence(&snapshot, "git:scope").unwrap()).unwrap();
    assert_eq!(
        belief["evidence"][0]["source"]["scopeId"],
        format!("video:{segment_video}")
    );

    // Scoped to the stream's video: its own segment and the OCR run, not the foreign segment.
    let (_, spans) =
        spans_by_id(&assert_snapshot_matches_database(&pool, Some(stream_video)).await);
    assert!(span(&spans, own_segment).is_some());
    assert!(span(&spans, foreign_segment).is_none());

    // Stored scenes name the scenes covering each segment's span, only on the segment's video.
    let scene_run = begin_visual_processing_run(
        &pool,
        BeginVisualProcessingRunRequest {
            video_id: stream_video,
            kind: VisualProcessingKind::Scene,
            provenance: ProcessingProvenance {
                processor: "visual-analysis".to_string(),
                processor_version: "scene-fixture".to_string(),
                model: "scenedetect-core".to_string(),
                model_version: "fixture".to_string(),
                input_hash: "sha256:video".to_string(),
                config_hash: "sha256:scene-config".to_string(),
                processing_config: serde_json::json!({}),
            },
        },
    )
    .await
    .unwrap();
    let scene = upsert_video_scene(
        &pool,
        UpsertVideoSceneRequest {
            run_id: scene_run,
            video_id: stream_video,
            scene_index: 0,
            start_frame: 12,
            end_frame: 36,
            start_seconds: 0.5,
            end_seconds: 1.5,
            metadata: serde_json::json!({}),
        },
    )
    .await
    .unwrap();
    // Whole-corpus scope so scenes of every video are loaded, restricted to this test's stream:
    // other suites (db_behavior) leave legacy streams the belief export rightly refuses.
    let mut snapshot = load_export_snapshot(&pool, None).await.unwrap();
    snapshot
        .transcript_streams
        .retain(|stream| stream.id == stream_id);
    let belief: Value =
        serde_json::from_str(&export_belief_evidence(&snapshot, "git:scenes").unwrap()).unwrap();
    let reference = |segment: Uuid| {
        belief["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["source"]["recordId"] == format!("transcript-segment:{segment}"))
            .cloned()
            .unwrap()
    };
    assert_eq!(belief["schemaVersion"], 2);
    let own = reference(own_segment);
    assert_eq!(own["mediaSpan"]["end"]["pts"], 1_000_000);
    assert_eq!(
        own["sceneIds"],
        serde_json::json!([format!("scene:{}", scene.id)])
    );
    assert!(reference(foreign_segment).get("sceneIds").is_none());

    for video in [stream_video, segment_video] {
        sqlx::query("DELETE FROM videos WHERE id = $1")
            .bind(video)
            .execute(&pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn resegmentation_records_actual_processor_without_restamping_other_streams() {
    use youtube_corpus::config::YtDlpConfig;
    use youtube_corpus::reprocessing::{reprocess_video, ReprocessRequest, ReprocessStage};
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let video_id = Uuid::new_v4();
    insert_video(&pool, video_id, "Synthetic provenance regression").await;
    let directory = std::env::temp_dir().join(format!("youtube-provenance-{video_id}"));
    std::fs::create_dir_all(&directory).unwrap();
    let captions = directory.join("fixture-subs.manual.id.en.vtt");
    std::fs::write(
        &captions,
        "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nSynthetic caption.\n",
    )
    .unwrap();
    let caption_id = youtube_corpus::youtube::stable_child_id(video_id, "caption_manual:en");
    let asr_id = Uuid::new_v4();
    for (id, kind, path) in [
        (
            caption_id,
            "caption_manual",
            Some(captions.to_string_lossy().into_owned()),
        ),
        (asr_id, "asr", None),
    ] {
        sqlx::query("INSERT INTO transcript_streams (id, video_id, source_kind, language, status, source_path, processor, processor_revision, processing_config) VALUES ($1, $2, $3, 'en', 'completed', $4, 'upstream-producer', 'git:historical-upstream', '{}')")
            .bind(id).bind(video_id).bind(kind).bind(path).execute(&pool).await.unwrap();
    }
    let original_asr: (String, Option<String>, serde_json::Value, i64) = sqlx::query_as("SELECT processor, processor_revision, processing_config, processing_revision FROM transcript_streams WHERE id=$1").bind(asr_id).fetch_one(&pool).await.unwrap();
    // A report with no newly generated transcript must not assign this build's
    // provenance to an already stored upstream ASR stream.
    let report = youtube_corpus::ingest::IngestReport {
        workflow: "youtube_corpus_ingest".into(),
        run_id: Uuid::new_v4(),
        videos_seen: 1,
        videos_indexed: 0,
        segments_indexed: 0,
        items: vec![youtube_corpus::ingest::IngestItemReport {
            video_id: Some(video_id),
            source_url: "https://youtube.test/synthetic".into(),
            title: None,
            status: "no_transcript".into(),
            streams_indexed: 0,
            segments_indexed: 0,
            message: Some("No new transcript output".into()),
        }],
    };
    let provenance = youtube_corpus::provenance::IngestProvenance {
        workflow: "youtube_corpus_ingest".into(),
        source_url: "https://youtube.test/synthetic".into(),
        caption: youtube_corpus::config::CaptionConfig::default(),
        yt_dlp: YtDlpConfig::default(),
        asr_enabled: false,
        transcriber_command: None,
        transcriber_args: vec![],
        transcriber_timeout_seconds: None,
        embedding: youtube_corpus::provenance::EmbeddingProvenance {
            provider: "hashed-text-embedder".into(),
            dimensions: 128,
        },
    };
    youtube_corpus::provenance::record_ingest_report(&database_url, &report, &provenance)
        .await
        .unwrap();
    let retained_asr: (String, Option<String>, serde_json::Value, i64) = sqlx::query_as("SELECT processor, processor_revision, processing_config, processing_revision FROM transcript_streams WHERE id=$1").bind(asr_id).fetch_one(&pool).await.unwrap();
    assert_eq!(original_asr, retained_asr);
    let mut request = ReprocessRequest {
        database_url,
        video_id,
        stage: ReprocessStage::Segments,
        work_dir: directory.clone(),
        caption_languages: vec!["en".into()],
        yt_dlp: YtDlpConfig::default(),
        transcriber_command: None,
        transcriber_args: vec![],
        transcriber_timeout_seconds: None,
    };
    let report = reprocess_video(request.clone()).await.unwrap();
    assert!(report.segments_resegmented > 0);
    let snapshot = load_export_snapshot(&pool, Some(video_id)).await.unwrap();
    let caption = snapshot
        .transcript_streams
        .iter()
        .find(|stream| stream.id == caption_id)
        .unwrap();
    assert_eq!(caption.processor, "youtube-corpus");
    assert_eq!(
        caption.processor_revision.as_deref(),
        Some(youtube_corpus::provenance::PROCESSOR_SOURCE_REVISION)
    );
    validate_belief_batch(&export_belief_evidence(&snapshot, "git:exporter").unwrap()).unwrap();
    let before = serde_json::to_value(&snapshot.transcript_streams).unwrap();
    request.stage = ReprocessStage::Embeddings;
    reprocess_video(request).await.unwrap();
    let after = load_export_snapshot(&pool, Some(video_id)).await.unwrap();
    assert_eq!(
        before,
        serde_json::to_value(&after.transcript_streams).unwrap()
    );
    let after_asr: (String, Option<String>, serde_json::Value, i64) = sqlx::query_as("SELECT processor, processor_revision, processing_config, processing_revision FROM transcript_streams WHERE id=$1").bind(asr_id).fetch_one(&pool).await.unwrap();
    assert_eq!(original_asr, after_asr);
    sqlx::query("DELETE FROM videos WHERE id=$1")
        .bind(video_id)
        .execute(&pool)
        .await
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
