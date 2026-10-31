//! Citation-oriented CLI search against Postgres (#140): youtube-id, corpus and speaker-label
//! filters, citation identifiers, context windows and the per-span hash.

use text_embeddings::{HashedTextEmbedder, TextEmbeddingConfig};
use text_lexical::CorpusOptions;
use uuid::Uuid;
use youtube_corpus::citations::{
    attach_citations, media_spans, render_search_text, ContextWindow, SpanQuery,
};
use youtube_corpus::config::SearchMode;
use youtube_corpus::corpus_export::load_export_snapshot;
use youtube_corpus::ingest::vector_literal;
use youtube_corpus::search::{search_corpus, SearchReport, SearchRequest};

const CORPUS_DEBATES: Uuid = Uuid::from_u128(0xc1000000000000000000000000000001);
const CORPUS_OTHER: Uuid = Uuid::from_u128(0xc1000000000000000000000000000002);
const VIDEO_DEBATE: Uuid = Uuid::from_u128(0x11000000000000000000000000000001);
const VIDEO_LECTURE: Uuid = Uuid::from_u128(0x11000000000000000000000000000002);
const STREAM_DEBATE: Uuid = Uuid::from_u128(0x21000000000000000000000000000001);
const STREAM_LECTURE: Uuid = Uuid::from_u128(0x21000000000000000000000000000002);

fn segment_id(stream: Uuid, index: u128) -> Uuid {
    Uuid::from_u128(stream.as_u128() + ((index + 1) << 32))
}

fn request(database_url: &str, query: &str) -> SearchRequest {
    SearchRequest {
        database_url: database_url.to_string(),
        query: query.to_string(),
        top_k: 10,
        mode: SearchMode::Fts,
        source_kind: None,
        modality: None,
        video_id: None,
        language: None,
        transcript_start_min: None,
        transcript_start_max: None,
        upload_date_from: None,
        upload_date_to: None,
        duration_min: None,
        duration_max: None,
        channel_query: None,
        title_query: None,
        category_query: None,
        tag_query: None,
        metadata_query: None,
        view_count_min: None,
        view_count_max: None,
        youtube_id: None,
        corpus_id: None,
        speaker_label: None,
    }
}

async fn hits(request: SearchRequest) -> Vec<Uuid> {
    let report = search_corpus(request).await.unwrap();
    let mut ids: Vec<Uuid> = report.results.iter().map(|hit| hit.video_id).collect();
    ids.sort();
    ids
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn citation_search_filters_enriches_and_pins_spans() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    seed(&pool).await;

    // "justice" is said in both videos.
    assert_eq!(
        hits(request(&database_url, "justice")).await,
        [VIDEO_DEBATE, VIDEO_LECTURE]
    );
    let mut by_youtube = request(&database_url, "justice");
    by_youtube.youtube_id = Some("debate00001".into());
    assert_eq!(hits(by_youtube).await, [VIDEO_DEBATE]);
    let mut by_corpus = request(&database_url, "justice");
    by_corpus.corpus_id = Some(CORPUS_OTHER);
    assert_eq!(hits(by_corpus).await, [VIDEO_LECTURE]);
    let mut by_speaker = request(&database_url, "justice");
    by_speaker.speaker_label = Some("SPEAKER_01".into());
    let report = search_corpus(by_speaker.clone()).await.unwrap();
    assert_eq!(
        report
            .results
            .iter()
            .map(|hit| hit.segment_id)
            .collect::<Vec<_>>(),
        [segment_id(STREAM_DEBATE, 2)]
    );
    // The speaker turn belongs to the debates corpus only.
    by_speaker.corpus_id = Some(CORPUS_OTHER);
    assert!(hits(by_speaker).await.is_empty());
    let mut hybrid = request(&database_url, "justice");
    hybrid.mode = SearchMode::Hybrid;
    hybrid.youtube_id = Some("debate00001".into());
    hybrid.speaker_label = Some("SPEAKER_01".into());
    assert_eq!(hits(hybrid).await, [VIDEO_DEBATE]);

    // Citation identifiers and a one-segment context window.
    let mut debate = request(&database_url, "justice");
    debate.youtube_id = Some("debate00001".into());
    let mut report: SearchReport = search_corpus(debate.clone()).await.unwrap();
    attach_citations(
        &pool,
        &mut report.results,
        Some(CORPUS_DEBATES),
        Some(ContextWindow::Segments(1)),
    )
    .await
    .unwrap();
    let hit = &report.results[0];
    let citation = hit.citation.as_ref().unwrap();
    assert_eq!(citation.youtube_id.as_deref(), Some("debate00001"));
    assert_eq!(citation.channel.as_deref(), Some("Debate Channel"));
    assert_eq!(citation.upload_date.as_deref(), Some("2024-03-09"));
    assert_eq!(
        citation.transcript_source.as_deref(),
        Some("caption_manual")
    );
    assert_eq!(
        citation.timestamp_url.as_deref(),
        Some("https://www.youtube.com/watch?v=debate00001&t=20s")
    );
    assert_eq!(citation.speaker_labels, ["SPEAKER_01"]);
    assert_eq!(
        citation
            .context
            .iter()
            .map(|segment| (segment.segment_index, segment.is_hit))
            .collect::<Vec<_>>(),
        [(1, false), (2, true), (3, false)]
    );
    assert_eq!(citation.context[0].speaker_labels, ["SPEAKER_00"]);
    let text = render_search_text(&report);
    assert!(text.contains("#1 debate00001 · \"The Great Debate\" · Debate Channel · 2024-03-09"));
    assert!(text.contains("> [00:20–00:30] SPEAKER_01: justice demands an answer"));
    assert!(text.contains("  [00:10–00:20] SPEAKER_00: what does fairness require"));

    // A five-second window reaches the neighbours that come within 5 s of the hit.
    let mut report = search_corpus(debate).await.unwrap();
    attach_citations(
        &pool,
        &mut report.results,
        None,
        Some(ContextWindow::Seconds(5.0)),
    )
    .await
    .unwrap();
    let context = &report.results[0].citation.as_ref().unwrap().context;
    assert_eq!(
        context
            .iter()
            .map(|segment| segment.segment_index)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );

    // The per-video span equals the whole-corpus export's span.
    let query = SpanQuery {
        youtube_id: "debate00001".into(),
        start_seconds: 12.0,
        end_seconds: 25.0,
        stream_id: None,
        transcript_source: Some("caption_manual".into()),
        language: Some("en".into()),
    };
    let one = load_export_snapshot(&pool, Some(VIDEO_DEBATE))
        .await
        .unwrap();
    let all = load_export_snapshot(&pool, None).await.unwrap();
    let pinned = media_spans(&one, "git:test", &query).unwrap();
    assert_eq!(pinned, media_spans(&all, "git:test", &query).unwrap());
    let span = &pinned.spans[0];
    assert_eq!(span.segment_count, 2);
    assert_eq!(span.speaker_labels, ["SPEAKER_00", "SPEAKER_01"]);
}

async fn seed(pool: &sqlx::PgPool) {
    sqlx::query("TRUNCATE transcript_segments, transcript_streams, videos CASCADE")
        .execute(pool)
        .await
        .unwrap();
    for (id, slug) in [
        (CORPUS_DEBATES, "citation-debates"),
        (CORPUS_OTHER, "citation-other"),
    ] {
        sqlx::query(
            "INSERT INTO corpora (id, slug, name) VALUES ($1, $2, $2) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(slug)
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO videos (id, youtube_id, source_url, title, upload_date, channel)
         VALUES ($1, 'debate00001', 'https://www.youtube.com/watch?v=debate00001',
                 'The Great Debate', '20240309', 'Debate Channel'),
                ($2, 'lecture0001', 'https://www.youtube.com/watch?v=lecture0001',
                 'A Lecture', '20230101', 'Lecture Channel')",
    )
    .bind(VIDEO_DEBATE)
    .bind(VIDEO_LECTURE)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO corpus_video_memberships (corpus_id, video_id)
         VALUES ($1, $3), ($2, $4)",
    )
    .bind(CORPUS_DEBATES)
    .bind(CORPUS_OTHER)
    .bind(VIDEO_DEBATE)
    .bind(VIDEO_LECTURE)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status)
         VALUES ($1, $3, 'caption_manual', 'en', 'completed'),
                ($2, $4, 'asr', 'en', 'completed')",
    )
    .bind(STREAM_DEBATE)
    .bind(STREAM_LECTURE)
    .bind(VIDEO_DEBATE)
    .bind(VIDEO_LECTURE)
    .execute(pool)
    .await
    .unwrap();
    let debate = [
        "welcome to the debate",
        "what does fairness require",
        "justice demands an answer",
        "closing remarks",
    ];
    for (index, text) in debate.iter().enumerate() {
        insert_segment(pool, STREAM_DEBATE, VIDEO_DEBATE, index, text).await;
    }
    insert_segment(
        pool,
        STREAM_LECTURE,
        VIDEO_LECTURE,
        0,
        "justice as a lecture topic",
    )
    .await;
    // SPEAKER_00 speaks 10–20 s, SPEAKER_01 is linked to the third segment directly.
    for (id, label, segment, start, end) in [
        (0xa1u128, "SPEAKER_00", None, Some(10.0), Some(20.0)),
        (
            0xa2,
            "SPEAKER_01",
            Some(segment_id(STREAM_DEBATE, 2)),
            None,
            None,
        ),
    ] {
        sqlx::query(
            "INSERT INTO research_annotations
             (id, corpus_id, video_id, stream_id, segment_id, kind, label, start_seconds,
              end_seconds, source_kind, processor, processor_version)
             VALUES ($1, $2, $3, $4, $5, 'speaker_turn', $6, $7, $8, 'processor',
                     'audio-analysis/diarization', 'test')",
        )
        .bind(Uuid::from_u128(id))
        .bind(CORPUS_DEBATES)
        .bind(VIDEO_DEBATE)
        .bind(STREAM_DEBATE)
        .bind(segment)
        .bind(label)
        .bind(start)
        .bind(end)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn insert_segment(pool: &sqlx::PgPool, stream: Uuid, video: Uuid, index: usize, text: &str) {
    let embedder =
        HashedTextEmbedder::new(TextEmbeddingConfig::default(), CorpusOptions::default()).unwrap();
    let embedding = vector_literal(embedder.embed_text(text).unwrap().as_slice());
    sqlx::query(
        "INSERT INTO transcript_segments
         (id, stream_id, video_id, segment_index, start_seconds, end_seconds, text, language,
          metadata, embedding)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'en', '{}'::jsonb, $8::vector)",
    )
    .bind(segment_id(stream, index as u128))
    .bind(stream)
    .bind(video)
    .bind(index as i64)
    .bind(index as f64 * 10.0)
    .bind(index as f64 * 10.0 + 10.0)
    .bind(text)
    .bind(embedding)
    .execute(pool)
    .await
    .unwrap();
}
