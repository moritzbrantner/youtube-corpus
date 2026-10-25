//! Automatic per-source check schedule (#68; Atlas 1.18, 1.20).
//!
//! Listing a source needs yt-dlp and the network, so sources are listed by a fixture lister
//! through the same seam the server uses, and the scheduler runs against an injected clock.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use chrono::{DateTime, Duration, DurationRound, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::sync::Notify;
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::backfill::record_snapshot;
use youtube_corpus::source_monitor::{due_sources, run_due_checks};
use youtube_corpus::subscriptions::{
    add_subscription, check_one, get_subscription, recover_interrupted_checks,
    AddSubscriptionRequest, CheckDispatch, SourceLister, Subscription, SubscriptionCheckReport,
    SubscriptionSourceKind, ALREADY_CHECKING, DEFAULT_CHECK_INTERVAL_SECONDS,
};
use youtube_corpus::youtube::{CollectionSnapshot, VideoItem};

/// Lists fixture uploads per source URL and counts listings. Sources in `failing` fail; a
/// source in `gated` blocks until `release` is notified.
#[derive(Default)]
struct FixtureLister {
    uploads: Mutex<HashMap<String, Vec<VideoItem>>>,
    calls: Mutex<HashMap<String, usize>>,
    failing: Mutex<HashSet<String>>,
    gated: Mutex<HashSet<String>>,
    entered: Notify,
    release: Notify,
}

impl SourceLister for FixtureLister {
    fn list<'a>(
        &'a self,
        subscription: &'a Subscription,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<VideoItem>>> + Send + 'a>> {
        Box::pin(async move {
            let url = subscription.source_url.clone();
            *self.calls.lock().unwrap().entry(url.clone()).or_default() += 1;
            if self.gated.lock().unwrap().contains(&url) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            if self.failing.lock().unwrap().contains(&url) {
                anyhow::bail!("fixture listing failed");
            }
            Ok(self
                .uploads
                .lock()
                .unwrap()
                .get(&url)
                .cloned()
                .unwrap_or_default())
        })
    }
}

impl FixtureLister {
    fn calls(&self, url: &str) -> usize {
        self.calls.lock().unwrap().get(url).copied().unwrap_or(0)
    }

    fn set(&self, url: &str, items: &[VideoItem]) {
        self.uploads
            .lock()
            .unwrap()
            .insert(url.to_string(), items.to_vec());
    }
}

fn video(title: &str) -> VideoItem {
    let youtube_id = format!("v{}", &Uuid::new_v4().simple().to_string()[..10]);
    VideoItem {
        item_id: youtube_id.clone(),
        source_url: format!("https://www.youtube.com/watch?v={youtube_id}"),
        youtube_id: Some(youtube_id),
        title: Some(title.to_string()),
        duration_seconds: None,
        upload_date: None,
        metadata: Default::default(),
        local_video_path: None,
    }
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

struct Source {
    id: Uuid,
    url: String,
}

/// Adds a monitored source with a `none` history and records its history snapshot.
async fn add_source(
    app: &axum::Router,
    pool: &PgPool,
    corpus: Uuid,
    interval: Option<u64>,
    policy: &str,
    history: &[VideoItem],
) -> Source {
    let url = format!(
        "https://www.youtube.com/playlist?list=PL{}",
        Uuid::new_v4().simple()
    );
    let mut body = json!({
        "sourceKind": "playlist",
        "sourceUrl": url,
        "monitor": true,
        "processingPolicy": policy,
        "history": {"scope": "none"},
    });
    if let Some(interval) = interval {
        body["checkIntervalSeconds"] = json!(interval);
    }
    let (status, report) = call(
        app,
        Method::POST,
        &format!("/api/corpora/{corpus}/sources"),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{report}");
    assert_eq!(
        report["source"]["checkIntervalSeconds"],
        json!(interval.unwrap_or(DEFAULT_CHECK_INTERVAL_SECONDS))
    );
    let id = report["source"]["id"].as_str().unwrap().parse().unwrap();
    let backfill = report["backfill"]["id"].as_str().unwrap().parse().unwrap();
    record_snapshot(
        pool,
        backfill,
        CollectionSnapshot {
            items: history.to_vec(),
            reported_count: Some(history.len() as u64),
        },
        Utc::now().date_naive(),
    )
    .await
    .unwrap();
    Source { id, url }
}

async fn run(
    pool: &PgPool,
    database_url: &str,
    lister: &Arc<FixtureLister>,
    now: DateTime<Utc>,
) -> Vec<SubscriptionCheckReport> {
    let handles = run_due_checks(pool, database_url, lister.clone(), now)
        .await
        .unwrap();
    let mut reports = Vec::new();
    for handle in handles {
        reports.push(handle.await.unwrap().unwrap());
    }
    reports
}

fn checked(reports: &[SubscriptionCheckReport]) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = reports
        .iter()
        .map(|report| report.subscription.id)
        .collect();
    ids.sort();
    ids
}

fn ids(sources: &[&Source]) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = sources.iter().map(|source| source.id).collect();
    ids.sort();
    ids
}

async fn subscription(pool: &PgPool, source: &Source) -> Subscription {
    get_subscription(pool, source.id).await.unwrap().unwrap()
}

async fn item_status(pool: &PgPool, source: &Source, item: &VideoItem) -> (String, String) {
    sqlx::query_as(
        "SELECT status, origin FROM corpus_subscription_items
         WHERE subscription_id = $1 AND source_url = $2",
    )
    .bind(source.id)
    .bind(&item.source_url)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn item_count(pool: &PgPool, source: &Source) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM corpus_subscription_items WHERE subscription_id = $1")
        .bind(source.id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn sources_are_checked_on_their_own_schedules() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    // Sources of other tests must not become due under the injected clock.
    sqlx::query("UPDATE corpus_subscriptions SET enabled = false")
        .execute(&pool)
        .await
        .unwrap();
    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url.clone()));
    let corpus = Uuid::new_v4();
    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Schedule test')")
        .bind(corpus)
        .bind(format!("schedule-{corpus}"))
        .execute(&pool)
        .await
        .unwrap();
    let lister = Arc::new(FixtureLister::default());
    // Sources are created at real time; the injected clock starts from there.
    let t0 = Utc::now().duration_round(Duration::seconds(1)).unwrap() + Duration::seconds(1);
    let at = |minutes: i64| t0 + Duration::minutes(minutes);

    let (a1, a2) = (video("a history 1"), video("a history 2"));
    let hourly = add_source(
        &app,
        &pool,
        corpus,
        Some(3600),
        "automatic",
        &[a1.clone(), a2.clone()],
    )
    .await;
    let b1 = video("b history");
    let review = add_source(
        &app,
        &pool,
        corpus,
        Some(7200),
        "review_first",
        std::slice::from_ref(&b1),
    )
    .await;
    let daily = add_source(&app, &pool, corpus, Some(86_400), "automatic", &[]).await;
    let weekly = add_source(&app, &pool, corpus, Some(7 * 86_400), "automatic", &[]).await;
    let defaulted = add_source(&app, &pool, corpus, None, "automatic", &[]).await;
    let disabled = add_source(&app, &pool, corpus, Some(3600), "automatic", &[]).await;
    let (code, _) = call(
        &app,
        Method::POST,
        &format!("/api/corpora/{corpus}/sources/{}/enabled", disabled.id),
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(code, StatusCode::NO_CONTENT);

    let a3 = video("a new upload");
    lister.set(&hourly.url, &[a3.clone(), a1.clone(), a2.clone()]);
    let b2 = video("b new upload");
    lister.set(&review.url, &[b2.clone(), b1.clone()]);

    // Nothing is due before the first interval elapses; loading sources checks nothing.
    let (code, sources) = call(
        &app,
        Method::GET,
        &format!("/api/corpora/{corpus}/sources"),
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert!(sources
        .as_array()
        .unwrap()
        .iter()
        .all(|source| source["lastCheckedAt"].is_null() && !source["nextCheckAt"].is_null()));
    assert!(run(&pool, &database_url, &lister, at(30)).await.is_empty());

    // The hourly source is due first; the disabled one with the same interval is not.
    let reports = run(&pool, &database_url, &lister, at(61)).await;
    assert_eq!(checked(&reports), ids(&[&hourly]));
    assert_eq!(reports[0].status, "completed");
    assert_eq!(reports[0].new_videos, 1);
    assert_eq!(reports[0].queued, 1);
    // The new upload is queued as new work; the excluded history stays excluded (1.10).
    assert_eq!(
        item_status(&pool, &hourly, &a3).await,
        ("queued".into(), "new".into())
    );
    assert_eq!(item_status(&pool, &hourly, &a1).await.0, "not_selected");
    assert_eq!(item_status(&pool, &hourly, &a2).await.0, "not_selected");
    let source = subscription(&pool, &hourly).await;
    assert_eq!(source.last_checked_at, Some(at(61)));
    assert_eq!(source.next_check_at, Some(at(121)));
    assert_eq!(source.check_started_at, None);

    // Two hours in, both the hourly and the two-hourly source are due on their own cadences.
    let reports = run(&pool, &database_url, &lister, at(122)).await;
    assert_eq!(checked(&reports), ids(&[&hourly, &review]));
    assert_eq!(item_status(&pool, &review, &b2).await.0, "awaiting_review");
    assert_eq!(item_status(&pool, &review, &b1).await.0, "not_selected");
    assert_eq!(
        item_count(&pool, &hourly).await,
        3,
        "repeat checks keep one row per video"
    );
    assert_eq!(lister.calls(&disabled.url), 0);

    // A failing check is reported truthfully and rescheduled a full interval later.
    lister.failing.lock().unwrap().insert(hourly.url.clone());
    let reports = run(&pool, &database_url, &lister, at(185)).await;
    assert_eq!(checked(&reports), ids(&[&hourly]));
    assert_eq!(reports[0].status, "failed");
    let source = subscription(&pool, &hourly).await;
    assert_eq!(source.last_checked_at, Some(at(185)));
    assert_eq!(source.next_check_at, Some(at(245)));
    let (_, sources) = call(
        &app,
        Method::GET,
        &format!("/api/corpora/{corpus}/sources"),
        None,
    )
    .await;
    let listed = sources
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["id"] == json!(hourly.id))
        .unwrap();
    assert_eq!(listed["lastCheckStatus"], "failed");
    assert_eq!(listed["lastCheckMessage"], "fixture listing failed");
    assert_eq!(listed["checkInProgress"], false);
    lister.failing.lock().unwrap().clear();

    // Editing one interval moves only that source's next check.
    let hourly_next = subscription(&pool, &hourly).await.next_check_at;
    let daily_next = subscription(&pool, &daily).await.next_check_at;
    let (code, _) = call(
        &app,
        Method::POST,
        &format!("/api/corpora/{corpus}/sources/{}/interval", review.id),
        Some(json!({"intervalSeconds": 600})),
    )
    .await;
    assert_eq!(code, StatusCode::NO_CONTENT);
    let (code, _) = call(
        &app,
        Method::POST,
        &format!("/api/corpora/{corpus}/sources/{}/interval", review.id),
        Some(json!({"intervalSeconds": 5})),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let edited = subscription(&pool, &review).await;
    assert_eq!(edited.check_interval_seconds, 600);
    assert_eq!(edited.next_check_at, Some(at(132)));
    assert_eq!(
        subscription(&pool, &hourly).await.next_check_at,
        hourly_next
    );
    assert_eq!(subscription(&pool, &daily).await.next_check_at, daily_next);

    // Check now while the scheduled check of the same source runs joins it instead of listing
    // and queueing a second time.
    let a4 = video("a second new upload");
    lister.set(
        &hourly.url,
        &[a4.clone(), a3.clone(), a1.clone(), a2.clone()],
    );
    lister.gated.lock().unwrap().insert(hourly.url.clone());
    let calls_before = lister.calls(&hourly.url);
    let handles = run_due_checks(&pool, &database_url, lister.clone(), at(246))
        .await
        .unwrap();
    lister.entered.notified().await;
    assert!(!due_sources(&pool, at(246))
        .await
        .unwrap()
        .contains(&hourly.id));
    let manual = check_one(
        &pool,
        &database_url,
        &subscription(&pool, &hourly).await,
        CheckDispatch::Enqueue,
        lister.as_ref(),
        Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(manual.status, ALREADY_CHECKING);
    let (_, sources) = call(
        &app,
        Method::GET,
        &format!("/api/corpora/{corpus}/sources"),
        None,
    )
    .await;
    assert!(sources
        .as_array()
        .unwrap()
        .iter()
        .any(|source| source["id"] == json!(hourly.id) && source["checkInProgress"] == true));
    lister.gated.lock().unwrap().clear();
    lister.release.notify_one();
    for handle in handles {
        handle.await.unwrap().unwrap();
    }
    assert_eq!(lister.calls(&hourly.url), calls_before + 1);
    assert_eq!(
        item_status(&pool, &hourly, &a4).await,
        ("queued".into(), "new".into())
    );
    assert_eq!(item_count(&pool, &hourly).await, 4);

    // A legacy source without a history selection records a `none` baseline before its first
    // automatic check instead of treating its existing uploads as new.
    let legacy_url = format!(
        "https://www.youtube.com/playlist?list=PL{}",
        Uuid::new_v4().simple()
    );
    let legacy = add_subscription(AddSubscriptionRequest {
        database_url: database_url.clone(),
        source_kind: SubscriptionSourceKind::Playlist,
        source_url: legacy_url.clone(),
        name: None,
        enabled: true,
        work_dir: "use-case-output/youtube-corpus".into(),
        caption: Default::default(),
        yt_dlp: Default::default(),
        asr_enabled: false,
        transcriber_command: None,
        transcriber_args: Vec::new(),
        transcriber_timeout_seconds: None,
        max_items: None,
        title_contains: None,
        title_excludes: Vec::new(),
        duration_min: None,
        duration_max: None,
        processing_policy: Default::default(),
        check_interval_seconds: Some(3600),
        migrate: false,
    })
    .await
    .unwrap();
    lister.set(&legacy_url, &[video("legacy old")]);
    let reports = run(&pool, &database_url, &lister, at(300)).await;
    assert!(!checked(&reports).contains(&legacy.id));
    assert_eq!(lister.calls(&legacy_url), 0);
    let baseline: String =
        sqlx::query_scalar("SELECT scope FROM corpus_source_backfills WHERE subscription_id = $1")
            .bind(legacy.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(baseline, "none");
    assert!(!due_sources(&pool, at(300))
        .await
        .unwrap()
        .contains(&legacy.id));
    sqlx::query("UPDATE corpus_subscriptions SET enabled = false WHERE id = $1")
        .bind(legacy.id)
        .execute(&pool)
        .await
        .unwrap();

    // The server stops for many intervals; a check was running when it stopped.
    sqlx::query("UPDATE corpus_subscriptions SET check_started_at = $2 WHERE id = $1")
        .bind(review.id)
        .bind(at(29 * 60))
        .execute(&pool)
        .await
        .unwrap();
    let restart = at(30 * 60);
    assert!(!due_sources(&pool, restart)
        .await
        .unwrap()
        .contains(&review.id));
    assert!(recover_interrupted_checks(&pool).await.unwrap() >= 1);
    let calls: Vec<usize> = [&hourly, &review, &daily, &weekly, &defaulted, &disabled]
        .iter()
        .map(|source| lister.calls(&source.url))
        .collect();
    // One prompt catch-up check per overdue enabled source, not one per missed interval; the
    // weekly source is not yet due and the disabled one is never checked.
    let reports = run(&pool, &database_url, &lister, restart).await;
    assert_eq!(
        checked(&reports),
        ids(&[&hourly, &review, &daily, &defaulted])
    );
    let after: Vec<usize> = [&hourly, &review, &daily, &weekly, &defaulted, &disabled]
        .iter()
        .map(|source| lister.calls(&source.url))
        .collect();
    assert_eq!(
        after,
        vec![
            calls[0] + 1,
            calls[1] + 1,
            calls[2] + 1,
            calls[3],
            calls[4] + 1,
            0
        ]
    );
    assert!(run(&pool, &database_url, &lister, restart).await.is_empty());
    // Afterwards each source resumes its own interval from the catch-up check.
    assert_eq!(
        subscription(&pool, &hourly).await.next_check_at,
        Some(restart + Duration::hours(1))
    );
    assert_eq!(
        subscription(&pool, &review).await.next_check_at,
        Some(restart + Duration::minutes(10))
    );
    assert_eq!(
        subscription(&pool, &daily).await.next_check_at,
        Some(restart + Duration::days(1))
    );
    let reports = run(
        &pool,
        &database_url,
        &lister,
        restart + Duration::minutes(11),
    )
    .await;
    assert_eq!(checked(&reports), ids(&[&review]));
    assert_eq!(
        item_count(&pool, &review).await,
        2,
        "catch-up checks keep one row per video"
    );
}
