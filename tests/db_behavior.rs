use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use text_embeddings::{HashedTextEmbedder, TextEmbeddingConfig};
use text_lexical::CorpusOptions;
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::config::{CaptionConfig, SearchMode, SourceKind, YtDlpConfig};
use youtube_corpus::ingest::vector_literal;
use youtube_corpus::search::{search_corpus, SearchRequest};
use youtube_corpus::subscriptions::{
    add_subscription, AddSubscriptionRequest, SubscriptionSourceKind,
};

const VIDEO_ONE: Uuid = Uuid::from_u128(0x10000000000000000000000000000001);
const VIDEO_TWO: Uuid = Uuid::from_u128(0x10000000000000000000000000000002);
const STREAM_MANUAL: Uuid = Uuid::from_u128(0x20000000000000000000000000000001);
const STREAM_AUTO: Uuid = Uuid::from_u128(0x20000000000000000000000000000002);
const STREAM_ASR: Uuid = Uuid::from_u128(0x20000000000000000000000000000003);
const SEG_ALPHA: Uuid = Uuid::from_u128(0x30000000000000000000000000000001);
const SEG_MIDDLE: Uuid = Uuid::from_u128(0x30000000000000000000000000000002);
const SEG_AUTO: Uuid = Uuid::from_u128(0x30000000000000000000000000000003);
const SEG_DE: Uuid = Uuid::from_u128(0x30000000000000000000000000000004);
const SEG_ASR: Uuid = Uuid::from_u128(0x30000000000000000000000000000005);

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn seeded_database_behaviors_work() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    seed(&pool).await;

    let fts = search(&database_url, "alpha neural", SearchMode::Fts).await;
    assert_eq!(fts.results.first().unwrap().segment_id, SEG_ALPHA);

    let semantic = search(
        &database_url,
        "semantic vector query stable",
        SearchMode::Semantic,
    )
    .await;
    assert!(!semantic.results.is_empty());
    assert_eq!(semantic.results.first().unwrap().segment_id, SEG_ASR);

    let hybrid = search(&database_url, "automatic retrieval", SearchMode::Hybrid).await;
    let hybrid_top = hybrid.results.first().unwrap();
    assert!(hybrid_top.fts_score.is_finite());
    assert!(hybrid_top.semantic_score.is_finite());

    assert_filter(&database_url, "automatic", |request| {
        request.source_kind = Some(SourceKind::CaptionAuto)
    })
    .await;
    assert_filter(&database_url, "deutsche", |request| {
        request.language = Some("de".to_string())
    })
    .await;
    assert_filter(&database_url, "alpha", |request| {
        request.upload_date_from = Some("2024-01-01".to_string());
        request.upload_date_to = Some("2024-12-31".to_string());
    })
    .await;
    assert_filter(&database_url, "alpha", |request| {
        request.duration_min = Some(100.0);
        request.duration_max = Some(200.0);
    })
    .await;
    assert_filter(&database_url, "alpha", |request| {
        request.channel_query = Some("Research".to_string())
    })
    .await;
    assert_filter(&database_url, "alpha", |request| {
        request.category_query = Some("Education".to_string())
    })
    .await;
    assert_filter(&database_url, "alpha", |request| {
        request.tag_query = Some("rust".to_string())
    })
    .await;
    assert_filter(&database_url, "alpha", |request| {
        request.view_count_min = Some(1_000);
        request.view_count_max = Some(2_000);
    })
    .await;

    let app = youtube_corpus::web::app_for_tests(Some(database_url.clone()));
    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/transcript-context",
            serde_json::json!({"segmentId": SEG_MIDDLE, "before": 1, "after": 1}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["segments"].as_array().unwrap().len(), 3);
    assert!(body["segments"]
        .as_array()
        .unwrap()
        .iter()
        .any(
            |segment| segment["segmentId"] == SEG_MIDDLE.to_string() && segment["isMatch"] == true
        ));

    let runs = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/ingest-runs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(runs.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(runs.into_body(), usize::MAX).await.unwrap()).unwrap();
    let statuses = body
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run["job"]["status"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(statuses.contains(&"running"));
    assert!(statuses.contains(&"succeeded"));
    assert!(statuses.contains(&"failed"));

    let completed = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/ingest-runs/{}",
                    Uuid::from_u128(0x40000000000000000000000000000002)
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(completed.into_body(), usize::MAX).await.unwrap())
            .unwrap();
    assert_eq!(body["status"], "completed");
    assert_eq!(body["job"]["status"], "succeeded");

    let subscription = subscription_request(&database_url);
    let first = add_subscription(subscription.clone()).await.unwrap();
    let second = add_subscription(subscription).await.unwrap();
    assert_eq!(first.id, second.id);
    let row: (serde_json::Value, Option<i64>) = sqlx::query_as(
        "SELECT yt_dlp, transcriber_timeout_seconds FROM corpus_subscriptions WHERE id = $1",
    )
    .bind(first.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.0["timeoutSeconds"], 12);
    assert_eq!(row.1, Some(34));
}

async fn search(
    database_url: &str,
    query: &str,
    mode: SearchMode,
) -> youtube_corpus::search::SearchReport {
    search_corpus(SearchRequest {
        database_url: database_url.to_string(),
        query: query.to_string(),
        top_k: 5,
        mode,
        source_kind: None,
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
    })
    .await
    .unwrap()
}

async fn assert_filter(database_url: &str, query: &str, update: impl FnOnce(&mut SearchRequest)) {
    let mut request = SearchRequest {
        database_url: database_url.to_string(),
        query: query.to_string(),
        top_k: 5,
        mode: SearchMode::Hybrid,
        source_kind: None,
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
    };
    update(&mut request);
    let report = search_corpus(request).await.unwrap();
    assert!(
        !report.results.is_empty(),
        "filter query {query:?} returned no results"
    );
}

fn json_request(method: Method, uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn subscription_request(database_url: &str) -> AddSubscriptionRequest {
    AddSubscriptionRequest {
        database_url: database_url.to_string(),
        source_kind: SubscriptionSourceKind::Channel,
        source_url: "https://www.youtube.com/@seeded-channel".to_string(),
        name: Some("Seeded channel".to_string()),
        enabled: true,
        work_dir: "use-case-output/youtube-corpus-test".into(),
        caption: CaptionConfig {
            enabled: true,
            include_auto_captions: true,
            languages: vec!["en".to_string()],
        },
        yt_dlp: YtDlpConfig {
            args: vec!["--flat-playlist".to_string()],
            timeout_seconds: Some(12),
        },
        asr_enabled: false,
        transcriber_command: None,
        transcriber_args: Vec::new(),
        transcriber_timeout_seconds: Some(34),
        max_items: Some(2),
        title_contains: None,
        title_excludes: Vec::new(),
        duration_min: None,
        duration_max: None,
        migrate: false,
    }
}

async fn seed(pool: &sqlx::PgPool) {
    sqlx::query(
        "TRUNCATE corpus_subscription_items, corpus_subscriptions, transcript_segments,
                  transcript_streams, videos, ingest_runs CASCADE",
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO videos
         (id, youtube_id, source_url, title, duration_seconds, upload_date, channel, channel_id,
          uploader, uploader_id, view_count, categories, tags, metadata)
         VALUES
         ($1, 'one', 'https://youtube.test/one', 'Alpha Research Video', 150, '20240115',
          'Research Channel', 'channel-one', 'Research Uploader', 'uploader-one', 1500,
          ARRAY['Education'], ARRAY['rust','search'], '{\"topic\":\"alpha\"}'::jsonb),
         ($2, 'two', 'https://youtube.test/two', 'Semantic Retrieval Video', 420, '20230505',
          'Archive Channel', 'channel-two', 'Archive Uploader', 'uploader-two', 9000,
          ARRAY['Technology'], ARRAY['vector'], '{\"topic\":\"semantic\"}'::jsonb)",
    )
    .bind(VIDEO_ONE)
    .bind(VIDEO_TWO)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status, full_text)
         VALUES
         ($1, $4, 'caption_manual', 'en', 'completed', 'manual text'),
         ($2, $4, 'caption_auto', 'en', 'completed', 'auto text'),
         ($3, $5, 'asr', 'en', 'completed', 'asr text')",
    )
    .bind(STREAM_MANUAL)
    .bind(STREAM_AUTO)
    .bind(STREAM_ASR)
    .bind(VIDEO_ONE)
    .bind(VIDEO_TWO)
    .execute(pool)
    .await
    .unwrap();

    insert_segment(
        pool,
        SEG_ALPHA,
        STREAM_MANUAL,
        VIDEO_ONE,
        0,
        "alpha neural transcript segment",
        "en",
    )
    .await;
    insert_segment(
        pool,
        SEG_MIDDLE,
        STREAM_MANUAL,
        VIDEO_ONE,
        1,
        "middle context has transformer attention",
        "en",
    )
    .await;
    insert_segment(
        pool,
        Uuid::from_u128(0x30000000000000000000000000000006),
        STREAM_MANUAL,
        VIDEO_ONE,
        2,
        "closing searchable archive",
        "en",
    )
    .await;
    insert_segment(
        pool,
        SEG_AUTO,
        STREAM_AUTO,
        VIDEO_ONE,
        0,
        "automatic captions mention retrieval",
        "en",
    )
    .await;
    insert_segment(
        pool,
        SEG_DE,
        STREAM_MANUAL,
        VIDEO_ONE,
        3,
        "deutsche suche alpha",
        "de",
    )
    .await;
    insert_segment(
        pool,
        SEG_ASR,
        STREAM_ASR,
        VIDEO_TWO,
        0,
        "semantic vector query stable",
        "en",
    )
    .await;

    for (id, status) in [
        (
            Uuid::from_u128(0x40000000000000000000000000000001),
            "running",
        ),
        (
            Uuid::from_u128(0x40000000000000000000000000000002),
            "completed",
        ),
        (
            Uuid::from_u128(0x40000000000000000000000000000003),
            "failed",
        ),
    ] {
        let report = match status {
            "running" => {
                serde_json::json!({"workflow":"youtube_corpus_ingest","status":"running","progress":{"completed":0,"total":null,"unit":"videos","message":"Queued"}})
            }
            "completed" => {
                serde_json::json!({"workflow":"youtube_corpus_ingest","runId":id,"videosSeen":1,"videosIndexed":1,"segmentsIndexed":2,"items":[]})
            }
            _ => {
                serde_json::json!({"workflow":"youtube_corpus_ingest","status":"failed","failure":{"message":"seed failure"}})
            }
        };
        sqlx::query(
            "INSERT INTO ingest_runs
             (id, source_url, status, videos_seen, videos_indexed, segments_indexed, report)
             VALUES ($1, 'https://youtube.test/run', $2, 1, 1, 2, $3)",
        )
        .bind(id)
        .bind(status)
        .bind(report)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn insert_segment(
    pool: &sqlx::PgPool,
    id: Uuid,
    stream_id: Uuid,
    video_id: Uuid,
    index: i64,
    text: &str,
    language: &str,
) {
    let embedder =
        HashedTextEmbedder::new(TextEmbeddingConfig::default(), CorpusOptions::default()).unwrap();
    let embedding = vector_literal(embedder.embed_text(text).unwrap().as_slice());
    sqlx::query(
        "INSERT INTO transcript_segments
         (id, stream_id, video_id, segment_index, start_seconds, end_seconds, text, language, metadata, embedding)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '{}'::jsonb, $9::vector)",
    )
    .bind(id)
    .bind(stream_id)
    .bind(video_id)
    .bind(index)
    .bind(index as f64 * 10.0)
    .bind(index as f64 * 10.0 + 5.0)
    .bind(text)
    .bind(language)
    .bind(embedding)
    .execute(pool)
    .await
    .unwrap();
}
