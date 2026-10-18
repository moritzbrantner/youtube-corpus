//! Automatic vs review-first admission of discovered videos (#66, Atlas 1.6, 1.8).
//!
//! Discovery and ingestion need yt-dlp and the network, so this drives the admission and
//! settlement steps a source check uses, plus the policy/pending HTTP API, on seeded items.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::subscriptions::{
    admit_item, get_subscription, list_pending_items, settle_pending_elsewhere, Admission,
};
use youtube_corpus::youtube::VideoItem;

async fn source(pool: &PgPool, corpus: Uuid, name: &str, policy: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO corpus_subscriptions
           (id, source_kind, source_url, name, work_dir, caption, processing_policy)
         VALUES ($1, 'playlist', $2, $3, 'use-case-output/test', $5::jsonb, $4)",
    )
    .bind(id)
    .bind(format!("https://www.youtube.com/playlist?list=PL{id}"))
    .bind(name)
    .bind(policy)
    .bind(json!({"enabled": true, "include_auto_captions": true, "languages": ["en"]}))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO corpus_source_memberships (corpus_id, subscription_id) VALUES ($1, $2)",
    )
    .bind(corpus)
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn discover(pool: &PgPool, subscription: Uuid, item: &VideoItem, status: &str) {
    sqlx::query(
        "INSERT INTO corpus_subscription_items
           (id, subscription_id, source_url, youtube_id, title, status)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (subscription_id, source_url) DO UPDATE SET status = EXCLUDED.status",
    )
    .bind(Uuid::new_v4())
    .bind(subscription)
    .bind(&item.source_url)
    .bind(&item.youtube_id)
    .bind(&item.title)
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
}

fn item(tag: &str) -> VideoItem {
    let youtube_id = format!("v{}", &Uuid::new_v4().simple().to_string()[..10]);
    VideoItem {
        item_id: youtube_id.clone(),
        source_url: format!("https://www.youtube.com/watch?v={youtube_id}"),
        youtube_id: Some(youtube_id),
        title: Some(tag.to_string()),
        duration_seconds: None,
        upload_date: None,
        metadata: Default::default(),
        local_video_path: None,
    }
}

async fn status(pool: &PgPool, subscription: Uuid, item: &VideoItem) -> String {
    sqlx::query_scalar(
        "SELECT status FROM corpus_subscription_items WHERE subscription_id = $1 AND source_url = $2",
    )
    .bind(subscription)
    .bind(&item.source_url)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn call(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(body) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn review_first_holds_only_videos_no_automatic_source_wants() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url));

    let corpus = Uuid::new_v4();
    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Review-first test')")
        .bind(corpus)
        .bind(format!("review-{corpus}"))
        .execute(&pool)
        .await
        .unwrap();
    let review = source(&pool, corpus, "Review source", "review_first").await;
    let automatic = source(&pool, corpus, "Automatic source", "automatic").await;
    let review_sub = get_subscription(&pool, review).await.unwrap().unwrap();
    let automatic_sub = get_subscription(&pool, automatic).await.unwrap().unwrap();

    let exclusive = item("review only");
    let shared = item("shared");
    let auto_only = item("automatic only");

    // Review-first source is checked first: its exclusive and the shared video wait.
    for video in [&exclusive, &shared] {
        discover(&pool, review, video, "discovered").await;
        assert_eq!(
            admit_item(&pool, &review_sub, video).await.unwrap(),
            Admission::AwaitReview
        );
        discover(&pool, review, video, "awaiting_review").await;
    }
    let pending = list_pending_items(&pool, review).await.unwrap();
    assert_eq!(pending.len(), 2);

    // The automatic source then finds the shared video and its own: both process, once.
    for video in [&shared, &auto_only] {
        discover(&pool, automatic, video, "discovered").await;
        assert_eq!(
            admit_item(&pool, &automatic_sub, video).await.unwrap(),
            Admission::Process
        );
        discover(&pool, automatic, video, "indexed").await;
        settle_pending_elsewhere(&pool, automatic, &video.source_url, "indexed")
            .await
            .unwrap();
    }
    // The shared video no longer awaits approval, but keeps its review-source membership.
    assert_eq!(status(&pool, review, &shared).await, "indexed");
    assert_eq!(status(&pool, review, &exclusive).await, "awaiting_review");
    let pending = list_pending_items(&pool, review).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].source_url, exclusive.source_url);

    // A later review-first discovery of an already processed video is not re-processed.
    let later = item("processed elsewhere first");
    discover(&pool, automatic, &later, "indexed").await;
    assert_eq!(
        admit_item(&pool, &review_sub, &later).await.unwrap(),
        Admission::AlreadyProcessed("indexed".to_string())
    );
    // ...and one an enabled automatic source has listed but not processed yet goes ahead.
    let queued = item("listed by automatic");
    discover(&pool, automatic, &queued, "discovered").await;
    assert_eq!(
        admit_item(&pool, &review_sub, &queued).await.unwrap(),
        Admission::Process
    );

    // HTTP: sources expose the policy and the pending count; policy is editable; pending
    // items are listed; approving a video that is not pending is rejected.
    let base = format!("/api/corpora/{corpus}/sources");
    let (status_code, sources) = call(&app, Method::GET, &base, None).await;
    assert_eq!(status_code, StatusCode::OK);
    let review_row = sources
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == review.to_string())
        .unwrap();
    assert_eq!(review_row["processingPolicy"], "review_first");
    assert_eq!(review_row["itemsAwaitingReview"], 1);
    let automatic_row = sources
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == automatic.to_string())
        .unwrap();
    assert_eq!(automatic_row["processingPolicy"], "automatic");

    let (status_code, pending) =
        call(&app, Method::GET, &format!("{base}/{review}/pending"), None).await;
    assert_eq!(status_code, StatusCode::OK);
    assert_eq!(pending[0]["title"], "review only");

    let (status_code, _) = call(
        &app,
        Method::POST,
        &format!("{base}/{review}/pending/approve"),
        Some(json!({"sourceUrls": [auto_only.source_url]})),
    )
    .await;
    assert_eq!(status_code, StatusCode::BAD_REQUEST);

    let (status_code, _) = call(
        &app,
        Method::POST,
        &format!("{base}/{automatic}/policy"),
        Some(json!({"policy": "review_first"})),
    )
    .await;
    assert_eq!(status_code, StatusCode::NO_CONTENT);
    let automatic_sub = get_subscription(&pool, automatic).await.unwrap().unwrap();
    assert_eq!(
        automatic_sub.processing_policy,
        youtube_corpus::subscriptions::ProcessingPolicy::ReviewFirst
    );
    // Switching policy does not touch completed items.
    assert_eq!(status(&pool, automatic, &auto_only).await, "indexed");

    let (status_code, _) = call(
        &app,
        Method::POST,
        &format!("/api/corpora/{}/sources/{review}/policy", Uuid::new_v4()),
        Some(json!({"policy": "automatic"})),
    )
    .await;
    assert_eq!(status_code, StatusCode::NOT_FOUND);
}
