//! Server-owned automatic source checks (Atlas 1.18, 1.20; #68).
//!
//! While the local server runs, every enabled monitored source is checked when its own
//! `next_check_at` is due. Each due source is checked in its own task, so a slow or failing
//! source never holds back another. A completed or failed check schedules the next one a full
//! interval after it ran, which makes a restart after downtime check each overdue source once
//! and then resume its cadence. Checks reuse the regular source check, which coalesces with a
//! running check of the same source and queues admitted uploads for the corpus dispatcher.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::backfill::HistoryScope;
use crate::subscriptions::{
    check_one, get_subscription, recover_interrupted_checks, CheckDispatch, SourceLister,
    SubscriptionCheckReport, YtDlpLister, STALE_CHECK_SECONDS,
};

/// Longest pause between scheduler passes; interval edits and toggles wake it sooner.
const MAX_IDLE: Duration = Duration::from_secs(60);

static SCHEDULE_WAKE: Notify = Notify::const_new();

/// Re-evaluates due times now, e.g. after an interval edit or enabling a source.
pub fn wake() {
    SCHEDULE_WAKE.notify_one();
}

/// Enabled sources due at `now` that are not being checked. A source whose initial history is
/// still being listed waits until its snapshot is recorded (Atlas 1.10).
pub async fn due_sources(pool: &PgPool, now: DateTime<Utc>) -> anyhow::Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT s.id FROM corpus_subscriptions s
         WHERE s.enabled
           AND coalesce(s.next_check_at,
                        s.created_at + s.check_interval_seconds * interval '1 second') <= $1
           AND (s.check_started_at IS NULL
                OR s.check_started_at < $1 - $2 * interval '1 second')
           AND NOT EXISTS (SELECT 1 FROM corpus_source_backfills b
                           WHERE b.subscription_id = s.id AND b.state = 'discovering')
         ORDER BY s.next_check_at NULLS FIRST, s.id",
    )
    .bind(now)
    .bind(STALE_CHECK_SECONDS)
    .fetch_all(pool)
    .await?)
}

/// A never-checked source without a recorded history selection (e.g. added by the CLI) would see
/// its whole existing history as new uploads; its first automatic check records the current
/// uploads as an empty (`none`) history baseline instead of analyzing them.
async fn needs_history_baseline(pool: &PgPool, subscription_id: Uuid) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT s.last_checked_at IS NULL
                AND NOT EXISTS (SELECT 1 FROM corpus_source_backfills b
                                WHERE b.subscription_id = s.id)
         FROM corpus_subscriptions s WHERE s.id = $1",
    )
    .bind(subscription_id)
    .fetch_optional(pool)
    .await?
    .unwrap_or(false))
}

/// Starts one check task per source due at `now` (the injected scheduler clock). Tests await
/// the returned handles; the server lets them run detached.
pub async fn run_due_checks(
    pool: &PgPool,
    database_url: &str,
    lister: Arc<dyn SourceLister>,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<JoinHandle<anyhow::Result<SubscriptionCheckReport>>>> {
    let mut handles = Vec::new();
    for id in due_sources(pool, now).await? {
        if needs_history_baseline(pool, id).await? {
            crate::backfill::start_backfill(pool, id, None, HistoryScope::None).await?;
            continue;
        }
        let Some(subscription) = get_subscription(pool, id).await? else {
            continue;
        };
        let pool = pool.clone();
        let database_url = database_url.to_string();
        let lister = lister.clone();
        handles.push(tokio::spawn(async move {
            check_one(
                &pool,
                &database_url,
                &subscription,
                CheckDispatch::Enqueue,
                lister.as_ref(),
                now,
            )
            .await
        }));
    }
    Ok(handles)
}

/// How long the scheduler may sleep before the earliest enabled source becomes due.
async fn idle_time(pool: &PgPool, now: DateTime<Utc>) -> anyhow::Result<Duration> {
    let next: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT min(s.next_check_at) FROM corpus_subscriptions s
         WHERE s.enabled AND s.check_started_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM corpus_source_backfills b
                           WHERE b.subscription_id = s.id AND b.state = 'discovering')",
    )
    .fetch_one(pool)
    .await?;
    Ok(match next {
        None => MAX_IDLE,
        Some(next) => (next - now)
            .to_std()
            .unwrap_or(Duration::ZERO)
            .clamp(Duration::from_secs(1), MAX_IDLE),
    })
}

/// Starts the scheduler with the server. Checks only run while the server runs; loading the
/// browser UI requests nothing. Claims left by a stopped server are released first so every
/// overdue enabled source gets its one prompt catch-up check.
pub fn spawn_scheduler(database_url: String) {
    tokio::spawn(async move {
        let pool = match crate::db::connect(&database_url).await {
            Ok(pool) => pool,
            Err(error) => {
                tracing::warn!(%error, "source scheduler unavailable: database connection failed");
                return;
            }
        };
        if let Err(error) = recover_interrupted_checks(&pool).await {
            tracing::warn!(%error, "could not release interrupted source checks");
        }
        let lister: Arc<dyn SourceLister> = Arc::new(YtDlpLister);
        loop {
            match run_due_checks(&pool, &database_url, lister.clone(), Utc::now()).await {
                Ok(handles) => {
                    for handle in handles {
                        tokio::spawn(async move {
                            match handle.await {
                                Ok(Ok(report)) if report.status == "failed" => tracing::warn!(
                                    source = %report.subscription.source_url,
                                    message = report.message.as_deref().unwrap_or_default(),
                                    "scheduled source check failed"
                                ),
                                Ok(Ok(_)) => {}
                                Ok(Err(error)) => {
                                    tracing::warn!(%error, "scheduled source check failed")
                                }
                                Err(error) => {
                                    tracing::warn!(%error, "scheduled source check panicked")
                                }
                            }
                        });
                    }
                }
                Err(error) => tracing::warn!(%error, "source scheduler pass failed"),
            }
            let idle = idle_time(&pool, Utc::now()).await.unwrap_or(MAX_IDLE);
            let _ = tokio::time::timeout(idle, SCHEDULE_WAKE.notified()).await;
        }
    });
}
