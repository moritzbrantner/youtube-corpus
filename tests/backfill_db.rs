//! Initial history backfill, its controls and fair interleaving with new uploads (#67; Atlas
//! 1.10, 1.12, 1.14, 1.16).
//!
//! Listing and ingesting need yt-dlp and the network, so this records fixture snapshots through
//! the same seam the discovery worker uses and processes queued videos with a deterministic
//! processor through the real dispatcher.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use chrono::NaiveDate;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::backfill::{
    control_backfill, enqueue_new_upload, get_backfill, record_snapshot, recover_interrupted,
    BackfillAction, Dispatcher, ItemProcessor, WorkClass,
};
use youtube_corpus::ingest::{IngestItemReport, IngestReport};
use youtube_corpus::subscriptions::Subscription;
use youtube_corpus::youtube::{CollectionSnapshot, VideoItem};

/// Records processed videos; videos titled `fail` fail.
#[derive(Default)]
struct FakeProcessor {
    processed: Mutex<Vec<String>>,
}

impl ItemProcessor for FakeProcessor {
    fn process<'a>(
        &'a self,
        _subscription: &'a Subscription,
        item: VideoItem,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<IngestReport>> + Send + 'a>> {
        Box::pin(async move {
            self.processed.lock().unwrap().push(item.source_url.clone());
            let failed = item.title.as_deref() == Some("fail");
            Ok(IngestReport {
                workflow: "test".to_string(),
                run_id: Uuid::new_v4(),
                videos_seen: 1,
                videos_indexed: u64::from(!failed),
                segments_indexed: 0,
                items: vec![IngestItemReport {
                    video_id: None,
                    source_url: item.source_url,
                    title: item.title,
                    status: if failed { "failed" } else { "indexed" }.to_string(),
                    streams_indexed: 0,
                    segments_indexed: 0,
                    message: failed.then(|| "fixture failure".to_string()),
                }],
            })
        })
    }
}

impl FakeProcessor {
    fn count(&self) -> usize {
        self.processed.lock().unwrap().len()
    }
}

fn video(title: &str, upload_date: Option<&str>) -> VideoItem {
    let youtube_id = format!("v{}", &Uuid::new_v4().simple().to_string()[..10]);
    VideoItem {
        item_id: youtube_id.clone(),
        source_url: format!("https://www.youtube.com/watch?v={youtube_id}"),
        youtube_id: Some(youtube_id),
        title: Some(title.to_string()),
        duration_seconds: None,
        upload_date: upload_date.map(str::to_string),
        metadata: Default::default(),
        local_video_path: None,
    }
}

fn snapshot(items: &[VideoItem]) -> CollectionSnapshot {
    CollectionSnapshot {
        items: items.to_vec(),
        reported_count: Some(items.len() as u64),
    }
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 25).unwrap()
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

async fn corpus(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Backfill test')")
        .bind(id)
        .bind(format!("backfill-{id}"))
        .execute(pool)
        .await
        .unwrap();
    id
}

/// Adds a monitored source through the HTTP API; returns (source id, backfill id).
async fn add_source(
    app: &axum::Router,
    corpus: Uuid,
    history: Option<Value>,
    policy: &str,
) -> (Uuid, Uuid, Value) {
    let mut body = json!({
        "sourceKind": "playlist",
        "sourceUrl": format!("https://www.youtube.com/playlist?list=PL{}", Uuid::new_v4().simple()),
        "monitor": true,
        "maxItems": 2,
        "processingPolicy": policy,
    });
    if let Some(history) = history {
        body["history"] = history;
    }
    let (status, report) = call(
        app,
        Method::POST,
        &format!("/api/corpora/{corpus}/sources"),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{report}");
    let source = report["source"]["id"].as_str().unwrap().parse().unwrap();
    let backfill = report["backfill"]["id"].as_str().unwrap().parse().unwrap();
    (source, backfill, report)
}

async fn status(pool: &PgPool, source: Uuid, item: &VideoItem) -> String {
    sqlx::query_scalar(
        "SELECT status FROM corpus_subscription_items WHERE subscription_id = $1 AND source_url = $2",
    )
    .bind(source)
    .bind(&item.source_url)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn new_upload(pool: &PgPool, source: Uuid, item: &VideoItem) {
    sqlx::query(
        "INSERT INTO corpus_subscription_items (id, subscription_id, source_url, youtube_id, title)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(source)
    .bind(&item.source_url)
    .bind(&item.youtube_id)
    .bind(&item.title)
    .execute(pool)
    .await
    .unwrap();
    enqueue_new_upload(pool, source, &item.source_url)
        .await
        .unwrap();
}

async fn drain(
    pool: &PgPool,
    dispatcher: &mut Dispatcher,
    processor: &FakeProcessor,
) -> Vec<WorkClass> {
    let mut classes = Vec::new();
    while let Some(outcome) = dispatcher.run_next(pool, processor).await.unwrap() {
        classes.push(outcome.class);
    }
    classes
}

/// One test so the shared dispatcher queue is not raced by a parallel test in this binary.
#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn initial_history_backfill() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    // Leftovers of earlier runs must not interfere with exact dispatch counts and order.
    sqlx::query("UPDATE corpus_subscription_items SET status = 'canceled' WHERE status IN ('queued', 'running')")
        .execute(&pool)
        .await
        .unwrap();
    history_scopes_select_from_the_recorded_snapshot(&pool, &database_url).await;
    backfill_controls_fairness_review_and_recovery(&pool, &database_url).await;
}

async fn history_scopes_select_from_the_recorded_snapshot(pool: &PgPool, database_url: &str) {
    let pool = pool.clone();
    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url.to_string()));
    let corpus = corpus(&pool).await;
    let processor = FakeProcessor::default();
    let mut dispatcher = Dispatcher::default();

    // Displayed order deliberately differs from publication order.
    let listing = [
        video("old pinned", Some("20190101")),
        video("recent", Some("20261020")),
        video("middle", Some("20240101")),
        video("undated", None),
    ];

    // Omitting the scope applies the full-history default; nothing runs before the listing.
    let (full, full_backfill, report) = add_source(&app, corpus, None, "automatic").await;
    assert_eq!(report["backfill"]["scope"], "full");
    assert_eq!(report["backfill"]["state"], "discovering");
    assert_eq!(report["backfill"]["discoveredCount"], Value::Null);
    assert!(drain(&pool, &mut dispatcher, &processor).await.is_empty());
    // A check before the snapshot is recorded must not treat history as new uploads.
    let (code, check) = call(
        &app,
        Method::POST,
        &format!("/api/corpora/{corpus}/sources/{full}/check"),
        Some(json!({})),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(check["items"][0]["status"], "waiting_for_history");

    record_snapshot(&pool, full_backfill, snapshot(&listing), today())
        .await
        .unwrap();
    let progress = get_backfill(&pool, full_backfill).await.unwrap().unwrap();
    assert_eq!(progress.discovered_count, Some(4));
    // Full history ignores the per-check maximum of 2.
    assert_eq!(progress.selected, 4);
    assert_eq!(progress.state, "running");

    // Limited: the first N entries in displayed order, not the newest publications.
    let (limited, limited_backfill, _) = add_source(
        &app,
        corpus,
        Some(json!({"scope": "limited", "limitCount": 2})),
        "automatic",
    )
    .await;
    let limited_listing = [
        video("old pinned", Some("20190101")),
        video("recent", Some("20261020")),
        video("middle", Some("20240101")),
    ];
    record_snapshot(&pool, limited_backfill, snapshot(&limited_listing), today())
        .await
        .unwrap();
    assert_eq!(status(&pool, limited, &limited_listing[0]).await, "queued");
    assert_eq!(status(&pool, limited, &limited_listing[1]).await, "queued");
    assert_eq!(
        status(&pool, limited, &limited_listing[2]).await,
        "not_selected"
    );
    let (_, entries) = call(
        &app,
        Method::GET,
        &format!("/api/corpora/{corpus}/sources/{limited}/backfills/{limited_backfill}/entries"),
        None,
    )
    .await;
    let entries = entries.as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0]["position"], 1);
    assert_eq!(
        entries[0]["sourceUrl"],
        limited_listing[0].source_url.as_str()
    );
    assert_eq!(entries[2]["reason"], "beyond_limit");

    // Recent: publication window; unknown dates are reported, not guessed.
    let (recent, recent_backfill, _) = add_source(
        &app,
        corpus,
        Some(json!({"scope": "recent", "recentDays": 30})),
        "automatic",
    )
    .await;
    let recent_listing = [
        video("recent", Some("20261020")),
        video("old", Some("20200101")),
        video("undated", None),
    ];
    record_snapshot(&pool, recent_backfill, snapshot(&recent_listing), today())
        .await
        .unwrap();
    let progress = get_backfill(&pool, recent_backfill).await.unwrap().unwrap();
    assert_eq!(progress.selected, 1);
    let reasons: Vec<_> = progress
        .not_selected
        .iter()
        .map(|entry| (entry.reason.as_str(), entry.count))
        .collect();
    assert_eq!(
        reasons,
        vec![("outside_recent_window", 1), ("publish_date_unknown", 1)]
    );
    assert_eq!(
        status(&pool, recent, &recent_listing[1]).await,
        "not_selected"
    );

    // None: nothing processed, history known, the backfill is complete; monitoring stays on.
    let (none, none_backfill, _) =
        add_source(&app, corpus, Some(json!({"scope": "none"})), "automatic").await;
    let none_listing = [video("a", None), video("b", None)];
    record_snapshot(&pool, none_backfill, snapshot(&none_listing), today())
        .await
        .unwrap();
    let progress = get_backfill(&pool, none_backfill).await.unwrap().unwrap();
    assert_eq!(
        (progress.selected, progress.state.as_str()),
        (0, "completed")
    );
    assert_eq!(status(&pool, none, &none_listing[0]).await, "not_selected");

    // Invalid scope parameters are rejected before anything is registered.
    let (code, _) = call(
        &app,
        Method::POST,
        &format!("/api/corpora/{corpus}/sources"),
        Some(json!({
            "sourceKind": "playlist",
            "sourceUrl": "https://www.youtube.com/playlist?list=PLinvalid",
            "history": {"scope": "limited"},
        })),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);

    // Process everything queued so far; each selected video exactly once.
    drain(&pool, &mut dispatcher, &processor).await;
    assert_eq!(processor.count(), 4 + 2 + 1);
    let progress = get_backfill(&pool, full_backfill).await.unwrap().unwrap();
    assert_eq!(
        (progress.indexed, progress.state.as_str()),
        (4, "completed")
    );

    // Repeating the full backfill (e.g. adding the source to another corpus) reuses results.
    let other = self::corpus(&pool).await;
    let source_url: String =
        sqlx::query_scalar("SELECT source_url FROM corpus_subscriptions WHERE id = $1")
            .bind(full)
            .fetch_one(&pool)
            .await
            .unwrap();
    let (_, report) = call(
        &app,
        Method::POST,
        &format!("/api/corpora/{other}/sources"),
        Some(json!({"sourceKind": "playlist", "sourceUrl": source_url, "monitor": true})),
    )
    .await;
    let repeat: Uuid = report["backfill"]["id"].as_str().unwrap().parse().unwrap();
    record_snapshot(&pool, repeat, snapshot(&listing), today())
        .await
        .unwrap();
    drain(&pool, &mut dispatcher, &processor).await;
    assert_eq!(processor.count(), 7);
    let progress = get_backfill(&pool, repeat).await.unwrap().unwrap();
    assert_eq!(
        (progress.indexed, progress.state.as_str()),
        (4, "completed")
    );
}

async fn backfill_controls_fairness_review_and_recovery(pool: &PgPool, database_url: &str) {
    let pool = pool.clone();
    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url.to_string()));
    let corpus = corpus(&pool).await;
    let processor = FakeProcessor::default();
    let mut dispatcher = Dispatcher::default();

    let (source, backfill, _) = add_source(&app, corpus, None, "automatic").await;
    let mut history: Vec<_> = (0..8)
        .map(|index| video(&format!("h{index}"), None))
        .collect();
    history[1].title = Some("fail".to_string());
    record_snapshot(&pool, backfill, snapshot(&history), today())
        .await
        .unwrap();

    // Fairness: while new uploads keep arriving, both classes advance in alternation.
    let mut classes = Vec::new();
    for round in 0..3 {
        new_upload(&pool, source, &video(&format!("new{round}"), None)).await;
        for _ in 0..2 {
            classes.push(
                dispatcher
                    .run_next(&pool, &processor)
                    .await
                    .unwrap()
                    .unwrap()
                    .class,
            );
        }
    }
    assert_eq!(
        classes
            .iter()
            .filter(|class| **class == WorkClass::New)
            .count(),
        3
    );
    assert_eq!(
        classes
            .iter()
            .filter(|class| **class == WorkClass::History)
            .count(),
        3
    );

    // Pause: no more history is launched, new uploads still run.
    let base = format!("/api/corpora/{corpus}/sources/{source}/backfills/{backfill}");
    let (code, paused) = call(
        &app,
        Method::POST,
        &format!("{base}/pause"),
        Some(json!({})),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(paused["state"], "paused");
    let (code, _) = call(
        &app,
        Method::POST,
        &format!("{base}/pause"),
        Some(json!({})),
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT);
    new_upload(&pool, source, &video("new while paused", None)).await;
    assert_eq!(
        drain(&pool, &mut dispatcher, &processor).await,
        vec![WorkClass::New]
    );

    // An interrupted run is requeued on restart, then resumes without redoing finished work.
    sqlx::query(
        "UPDATE corpus_subscription_items SET status = 'running'
         WHERE subscription_id = $1 AND source_url = $2",
    )
    .bind(source)
    .bind(&history[5].source_url)
    .execute(&pool)
    .await
    .unwrap();
    assert!(recover_interrupted(&pool).await.unwrap() >= 1);
    let before = processor.count();
    control_backfill(&pool, source, backfill, BackfillAction::Resume)
        .await
        .unwrap();
    assert_eq!(
        dispatcher
            .run_next(&pool, &processor)
            .await
            .unwrap()
            .unwrap()
            .class,
        WorkClass::History
    );
    assert_eq!(processor.count(), before + 1);

    // Cancel: remaining videos stop, finished results and membership stay.
    let canceled = control_backfill(&pool, source, backfill, BackfillAction::Cancel)
        .await
        .unwrap();
    assert_eq!(canceled.state, "canceled");
    assert_eq!(canceled.failed, 1);
    assert_eq!(canceled.indexed, 3);
    assert_eq!(canceled.canceled, 4);
    assert!(drain(&pool, &mut dispatcher, &processor).await.is_empty());
    assert_eq!(status(&pool, source, &history[0]).await, "indexed");
    assert_eq!(status(&pool, source, &history[7]).await, "canceled");
    let (_, sources) = call(
        &app,
        Method::GET,
        &format!("/api/corpora/{corpus}/sources"),
        None,
    )
    .await;
    let row = &sources.as_array().unwrap()[0];
    assert_eq!(row["backfill"]["state"], "canceled");
    assert_eq!(row["queue"]["historyQueued"], 0);

    // Review-first: selected history waits for approval and is not processed meanwhile.
    let (review, review_backfill, _) = add_source(&app, corpus, None, "review_first").await;
    let review_listing = [video("r1", None), video("r2", None)];
    record_snapshot(&pool, review_backfill, snapshot(&review_listing), today())
        .await
        .unwrap();
    assert!(drain(&pool, &mut dispatcher, &processor).await.is_empty());
    assert_eq!(
        status(&pool, review, &review_listing[0]).await,
        "awaiting_review"
    );
    let progress = get_backfill(&pool, review_backfill).await.unwrap().unwrap();
    assert_eq!(
        (progress.awaiting_review, progress.state.as_str()),
        (2, "awaiting_review")
    );
}
