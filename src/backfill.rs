//! Initial history backfill for tracked channels and playlists (Atlas 1.10, 1.12, 1.14, 1.16;
//! #67).
//!
//! Adding a monitored source records a backfill with the user's historical scope. The server's
//! discovery worker lists the source once, in its displayed order, records that snapshot, and
//! selects entries for the scope: `none`, `recent` (published within N days), `limited` (the
//! first N entries in the source's displayed order) or `full` (every listed entry, never capped
//! by the per-check limit). Entries that are not selected are recorded as known history, so
//! later checks for new uploads do not sweep them into analysis.
//!
//! Selected videos follow the source's automatic/review-first admission (1.6, 1.8) and are
//! queued with origin `history`; server checks queue admitted new uploads with origin `new`.
//! One corpus-owned dispatcher processes one queued video at a time and alternates between the
//! two classes whenever both have eligible work, so neither waits for the other to drain.
//! Paused or canceled backfills contribute no eligible work; new uploads continue.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::config::YtDlpConfig;
use crate::ingest::IngestReport;
use crate::subscriptions::{
    admit_item, get_subscription, item_from_row, mark_subscription_item_status,
    settle_pending_elsewhere, subscription_item_status, update_ingested_items,
    upsert_subscription_item, Admission, ProcessingPolicy, Subscription, AWAITING_REVIEW,
};
use crate::youtube::{CollectionSnapshot, VideoItem};

/// The historical scope requested when a source is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryScope {
    None,
    Recent { days: u32 },
    Limited { count: u64 },
    Full,
}

/// Wire form of [`HistoryScope`]: `{"scope": "limited", "limitCount": 20}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryScopeInput {
    pub scope: HistoryScopeKind,
    #[serde(default)]
    pub recent_days: Option<u32>,
    #[serde(default)]
    pub limit_count: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryScopeKind {
    None,
    Recent,
    Limited,
    Full,
}

impl HistoryScopeInput {
    pub fn into_scope(self) -> Result<HistoryScope, String> {
        match self.scope {
            HistoryScopeKind::None => Ok(HistoryScope::None),
            HistoryScopeKind::Full => Ok(HistoryScope::Full),
            HistoryScopeKind::Recent => match self.recent_days {
                Some(days) if days > 0 => Ok(HistoryScope::Recent { days }),
                _ => Err("Recent history needs a positive number of days.".to_string()),
            },
            HistoryScopeKind::Limited => match self.limit_count {
                Some(count) if count > 0 => Ok(HistoryScope::Limited { count }),
                _ => Err("A limited history needs a positive number of videos.".to_string()),
            },
        }
    }
}

impl HistoryScope {
    fn kind(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Recent { .. } => "recent",
            Self::Limited { .. } => "limited",
            Self::Full => "full",
        }
    }

    fn from_row(scope: &str, days: Option<i32>, count: Option<i64>) -> anyhow::Result<Self> {
        Ok(match scope {
            "none" => Self::None,
            "full" => Self::Full,
            "recent" => Self::Recent {
                days: days.unwrap_or(1).max(1) as u32,
            },
            "limited" => Self::Limited {
                count: count.unwrap_or(1).max(1) as u64,
            },
            other => anyhow::bail!("unknown history scope `{other}`"),
        })
    }
}

/// Why a snapshot entry was not selected.
pub const REASON_SCOPE_NONE: &str = "scope_none";
pub const REASON_BEYOND_LIMIT: &str = "beyond_limit";
pub const REASON_OUTSIDE_WINDOW: &str = "outside_recent_window";
pub const REASON_DATE_UNKNOWN: &str = "publish_date_unknown";

/// Selection of one snapshot entry: `Ok(())` when selected, otherwise the reason.
pub type Selection = Result<(), &'static str>;

/// Applies a scope to a snapshot listed in the source's displayed order.
pub fn select_entries(
    scope: HistoryScope,
    items: &[VideoItem],
    today: NaiveDate,
) -> Vec<Selection> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| match scope {
            HistoryScope::None => Err(REASON_SCOPE_NONE),
            HistoryScope::Full => Ok(()),
            HistoryScope::Limited { count } => {
                if (index as u64) < count {
                    Ok(())
                } else {
                    Err(REASON_BEYOND_LIMIT)
                }
            }
            HistoryScope::Recent { days } => match publication_date(item) {
                Some(date) if date >= today - chrono::Duration::days(i64::from(days)) => Ok(()),
                Some(_) => Err(REASON_OUTSIDE_WINDOW),
                None => Err(REASON_DATE_UNKNOWN),
            },
        })
        .collect()
}

/// Publication date from yt-dlp's `upload_date` (YYYYMMDD) or, failing that, its timestamps.
pub fn publication_date(item: &VideoItem) -> Option<NaiveDate> {
    item.upload_date
        .as_deref()
        .or(item.metadata.upload_date.as_deref())
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y%m%d").ok())
        .or_else(|| {
            item.metadata
                .timestamp
                .or(item.metadata.release_timestamp)
                .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
                .map(|value| value.date_naive())
        })
}

static DISCOVERY_WAKE: Notify = Notify::const_new();
static WORK_WAKE: Notify = Notify::const_new();

/// Signals the server's background workers that new discovery or processing work exists.
pub fn wake() {
    DISCOVERY_WAKE.notify_one();
    WORK_WAKE.notify_one();
}

/// Records a requested backfill; the discovery worker lists the source in the background.
pub async fn start_backfill(
    pool: &PgPool,
    subscription_id: Uuid,
    corpus_id: Option<Uuid>,
    scope: HistoryScope,
) -> anyhow::Result<BackfillProgress> {
    let id = Uuid::new_v4();
    let (days, count) = match scope {
        HistoryScope::Recent { days } => (Some(days as i32), None),
        HistoryScope::Limited { count } => (None, Some(count as i64)),
        _ => (None, None),
    };
    sqlx::query(
        "INSERT INTO corpus_source_backfills
           (id, subscription_id, corpus_id, scope, recent_days, limit_count, state)
         VALUES ($1, $2, $3, $4, $5, $6, 'discovering')",
    )
    .bind(id)
    .bind(subscription_id)
    .bind(corpus_id)
    .bind(scope.kind())
    .bind(days)
    .bind(count)
    .execute(pool)
    .await?;
    wake();
    get_backfill(pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("backfill disappeared"))
}

/// True while a backfill of this source has not yet recorded its history snapshot.
pub async fn history_discovery_pending(
    pool: &PgPool,
    subscription_id: Uuid,
) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM corpus_source_backfills
                       WHERE subscription_id = $1 AND state = 'discovering')",
    )
    .bind(subscription_id)
    .fetch_one(pool)
    .await?)
}

/// Lists the next backfill awaiting discovery and records its snapshot. Returns false when
/// none is waiting.
pub async fn discover_next(pool: &PgPool) -> anyhow::Result<bool> {
    let Some(backfill_id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM corpus_source_backfills WHERE state = 'discovering'
         ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    else {
        return Ok(false);
    };
    let row =
        sqlx::query("SELECT subscription_id, scope FROM corpus_source_backfills WHERE id = $1")
            .bind(backfill_id)
            .fetch_one(pool)
            .await?;
    let subscription_id: Uuid = row.try_get("subscription_id")?;
    let scope: String = row.try_get("scope")?;
    let Some(subscription) = get_subscription(pool, subscription_id).await? else {
        fail_discovery(pool, backfill_id, "The source no longer exists.").await?;
        return Ok(true);
    };
    let yt_dlp = discovery_config(&subscription.yt_dlp, scope == "recent");
    match crate::youtube::discover_collection_snapshot(&subscription.source_url, &yt_dlp).await {
        Ok(snapshot) => {
            record_snapshot(pool, backfill_id, snapshot, Utc::now().date_naive()).await?
        }
        Err(error) => fail_discovery(pool, backfill_id, &error.to_string()).await?,
    }
    Ok(true)
}

/// Recent-history selection needs publication dates; YouTube's flat channel listing only shows
/// relative ages, which yt-dlp turns into approximate dates on request.
fn discovery_config(config: &YtDlpConfig, needs_dates: bool) -> YtDlpConfig {
    let mut config = config.clone();
    if needs_dates {
        config.args.push("--extractor-args".to_string());
        config.args.push("youtubetab:approximate_date".to_string());
    }
    config
}

async fn fail_discovery(pool: &PgPool, backfill_id: Uuid, message: &str) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE corpus_source_backfills
         SET state = CASE WHEN state = 'canceled' THEN state ELSE 'failed' END,
             discovery_message = $2, finished_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(backfill_id)
    .bind(message)
    .execute(pool)
    .await?;
    Ok(())
}

/// Records a listed snapshot for a backfill, selects its scope and queues selected videos.
/// Deterministic seam for tests: `discover_next` calls it with yt-dlp's listing.
pub async fn record_snapshot(
    pool: &PgPool,
    backfill_id: Uuid,
    snapshot: CollectionSnapshot,
    today: NaiveDate,
) -> anyhow::Result<()> {
    let row = sqlx::query(
        "SELECT subscription_id, scope, recent_days, limit_count, state
         FROM corpus_source_backfills WHERE id = $1",
    )
    .bind(backfill_id)
    .fetch_one(pool)
    .await?;
    let subscription_id: Uuid = row.try_get("subscription_id")?;
    let scope = HistoryScope::from_row(
        &row.try_get::<String, _>("scope")?,
        row.try_get("recent_days")?,
        row.try_get("limit_count")?,
    )?;
    let canceled = row.try_get::<String, _>("state")? == "canceled";
    let subscription = get_subscription(pool, subscription_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("source not found"))?;

    let mut seen = std::collections::HashSet::new();
    let items: Vec<VideoItem> = snapshot
        .items
        .into_iter()
        .filter(|item| seen.insert(item.source_url.clone()))
        .collect();
    let selections = select_entries(scope, &items, today);
    for (index, (item, selection)) in items.iter().zip(selections).enumerate() {
        let previous = subscription_item_status(pool, subscription_id, item).await?;
        upsert_subscription_item(pool, subscription_id, item).await?;
        let selected = selection.is_ok() && !canceled;
        sqlx::query(
            "INSERT INTO corpus_backfill_items
               (backfill_id, position, subscription_id, source_url, youtube_id, title,
                upload_date, selected, reason)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT DO NOTHING",
        )
        .bind(backfill_id)
        .bind(index as i32 + 1)
        .bind(subscription_id)
        .bind(&item.source_url)
        .bind(&item.youtube_id)
        .bind(&item.title)
        .bind(publication_date(item).map(|date| date.format("%Y%m%d").to_string()))
        .bind(selection.is_ok())
        .bind(selection.err())
        .execute(pool)
        .await?;
        if selected {
            queue_history_item(pool, &subscription, item, previous.as_deref()).await?;
        } else if matches!(previous.as_deref(), None | Some("discovered")) {
            // Known but unselected history: later new-upload checks must not pick it up.
            mark_subscription_item_status(pool, subscription_id, &item.source_url, "not_selected")
                .await?;
        }
    }

    let discovered = items.len() as i64;
    let reported = snapshot.reported_count.map(|value| value as i64);
    let message = match reported {
        Some(reported) if reported > discovered => Some(format!(
            "YouTube reported {reported} videos but only {discovered} could be listed."
        )),
        _ => None,
    };
    sqlx::query(
        "UPDATE corpus_source_backfills
         SET discovered_count = $2, reported_count = $3, discovery_message = $4,
             discovered_at = now(), updated_at = now(),
             state = CASE WHEN state = 'discovering' THEN 'running' ELSE state END
         WHERE id = $1",
    )
    .bind(backfill_id)
    .bind(discovered)
    .bind(reported)
    .bind(message)
    .execute(pool)
    .await?;
    refresh_state(pool, backfill_id).await?;
    WORK_WAKE.notify_one();
    Ok(())
}

async fn queue_history_item(
    pool: &PgPool,
    subscription: &Subscription,
    item: &VideoItem,
    previous: Option<&str>,
) -> anyhow::Result<()> {
    let candidate = match previous {
        None | Some("discovered" | "failed" | "not_selected" | "canceled") => true,
        Some(status) if status == AWAITING_REVIEW => {
            subscription.processing_policy == ProcessingPolicy::Automatic
        }
        // Already indexed, skipped, filtered, queued or running: reuse, never duplicate.
        Some(_) => false,
    };
    if !candidate {
        return Ok(());
    }
    if !crate::youtube::filter_item(
        item,
        &subscription.title_contains,
        &subscription.title_excludes,
        subscription.duration_min,
        subscription.duration_max,
    ) {
        mark_subscription_item_status(pool, subscription.id, &item.source_url, "filtered").await?;
        return Ok(());
    }
    match admit_item(pool, subscription, item).await? {
        Admission::Process => {
            sqlx::query(
                "UPDATE corpus_subscription_items
                 SET status = 'queued', origin = 'history', queued_at = now(), last_seen_at = now()
                 WHERE subscription_id = $1 AND source_url = $2",
            )
            .bind(subscription.id)
            .bind(&item.source_url)
            .execute(pool)
            .await?;
        }
        Admission::AlreadyProcessed(status) => {
            mark_subscription_item_status(pool, subscription.id, &item.source_url, &status).await?;
        }
        Admission::AwaitReview => {
            mark_subscription_item_status(pool, subscription.id, &item.source_url, AWAITING_REVIEW)
                .await?;
        }
    }
    Ok(())
}

/// Queues a newly published video admitted by a server check.
pub async fn enqueue_new_upload(
    pool: &PgPool,
    subscription_id: Uuid,
    source_url: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE corpus_subscription_items
         SET status = 'queued', origin = 'new', queued_at = now(), last_seen_at = now()
         WHERE subscription_id = $1 AND source_url = $2",
    )
    .bind(subscription_id)
    .bind(source_url)
    .execute(pool)
    .await?;
    Ok(())
}

/// The two classes of queued work the dispatcher interleaves (Atlas 1.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkClass {
    New,
    History,
}

impl WorkClass {
    fn other(self) -> Self {
        match self {
            Self::New => Self::History,
            Self::History => Self::New,
        }
    }
}

/// Processes one queued video; the production implementation ingests it with the source's
/// settings, tests substitute a deterministic one.
pub trait ItemProcessor: Send + Sync {
    fn process<'a>(
        &'a self,
        subscription: &'a Subscription,
        item: VideoItem,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<IngestReport>> + Send + 'a>>;
}

/// Ingests through the existing per-video ingest path (captions, ASR, detector reuse).
pub struct IngestProcessor {
    pub database_url: String,
}

impl ItemProcessor for IngestProcessor {
    fn process<'a>(
        &'a self,
        subscription: &'a Subscription,
        item: VideoItem,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<IngestReport>> + Send + 'a>> {
        Box::pin(crate::ingest::ingest_video_items(
            subscription.to_ingest_request(self.database_url.clone()),
            vec![item],
        ))
    }
}

/// What one dispatcher step did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchOutcome {
    pub class: WorkClass,
    pub subscription_id: Uuid,
    pub source_url: String,
    pub status: String,
}

/// Round-robin over the two work classes: after serving one class, the other class goes first
/// next time, so while both have eligible work they alternate and neither starves.
#[derive(Debug, Clone)]
pub struct Dispatcher {
    next: WorkClass,
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self {
            next: WorkClass::New,
        }
    }
}

impl Dispatcher {
    pub async fn run_next(
        &mut self,
        pool: &PgPool,
        processor: &dyn ItemProcessor,
    ) -> anyhow::Result<Option<DispatchOutcome>> {
        for class in [self.next, self.next.other()] {
            if let Some((subscription_id, source_url)) = claim(pool, class).await? {
                self.next = class.other();
                let status = process_claimed(pool, processor, subscription_id, &source_url).await?;
                return Ok(Some(DispatchOutcome {
                    class,
                    subscription_id,
                    source_url,
                    status,
                }));
            }
        }
        Ok(None)
    }
}

async fn claim(pool: &PgPool, class: WorkClass) -> anyhow::Result<Option<(Uuid, String)>> {
    let sql = match class {
        WorkClass::New => {
            "UPDATE corpus_subscription_items SET status = 'running', last_seen_at = now()
             WHERE id = (
               SELECT id FROM corpus_subscription_items
               WHERE status = 'queued' AND origin = 'new'
               ORDER BY queued_at NULLS FIRST, first_seen_at
               LIMIT 1 FOR UPDATE SKIP LOCKED)
             RETURNING subscription_id, source_url"
        }
        // Eligible only through a running backfill: paused/canceled backfills contribute none.
        WorkClass::History => {
            "UPDATE corpus_subscription_items SET status = 'running', last_seen_at = now()
             WHERE id = (
               SELECT i.id FROM corpus_subscription_items i
               JOIN corpus_backfill_items bi
                 ON bi.subscription_id = i.subscription_id AND bi.source_url = i.source_url
               JOIN corpus_source_backfills b ON b.id = bi.backfill_id
               WHERE i.status = 'queued' AND i.origin = 'history'
                 AND bi.selected AND bi.outcome IS NULL AND b.state = 'running'
               ORDER BY b.created_at, bi.position
               LIMIT 1 FOR UPDATE OF i SKIP LOCKED)
             RETURNING subscription_id, source_url"
        }
    };
    let row = sqlx::query(sql).fetch_optional(pool).await?;
    row.map(|row| Ok((row.try_get("subscription_id")?, row.try_get("source_url")?)))
        .transpose()
}

async fn process_claimed(
    pool: &PgPool,
    processor: &dyn ItemProcessor,
    subscription_id: Uuid,
    source_url: &str,
) -> anyhow::Result<String> {
    let status = match process_item(pool, processor, subscription_id, source_url).await {
        Ok(status) => status,
        Err(error) => {
            record_failure(pool, subscription_id, source_url, &error.to_string()).await?;
            "failed".to_string()
        }
    };
    refresh_for_item(pool, subscription_id, source_url).await?;
    Ok(status)
}

async fn process_item(
    pool: &PgPool,
    processor: &dyn ItemProcessor,
    subscription_id: Uuid,
    source_url: &str,
) -> anyhow::Result<String> {
    let subscription = get_subscription(pool, subscription_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("source not found"))?;
    let row = sqlx::query(
        "SELECT source_url, youtube_id, title, duration_seconds, upload_date, metadata
         FROM corpus_subscription_items WHERE subscription_id = $1 AND source_url = $2",
    )
    .bind(subscription_id)
    .bind(source_url)
    .fetch_one(pool)
    .await?;
    let item = item_from_row(row)?;
    // Another source may have processed the same video since it was queued (2.2).
    if let Admission::AlreadyProcessed(status) = admit_item(pool, &subscription, &item).await? {
        mark_subscription_item_status(pool, subscription_id, source_url, &status).await?;
        return Ok(status);
    }
    let report = processor.process(&subscription, item).await?;
    let Some(result) = report
        .items
        .iter()
        .find(|entry| entry.source_url == source_url)
    else {
        mark_subscription_item_status(pool, subscription_id, source_url, "filtered").await?;
        return Ok("filtered".to_string());
    };
    let status = result.status.clone();
    let message = result.message.clone();
    update_ingested_items(pool, subscription_id, &report).await?;
    if status == "failed" {
        record_failure(
            pool,
            subscription_id,
            source_url,
            message.as_deref().unwrap_or("processing failed"),
        )
        .await?;
    } else {
        settle_pending_elsewhere(pool, subscription_id, source_url, &status).await?;
        sqlx::query(
            "UPDATE corpus_subscriptions SET last_ingested_at = now(), updated_at = now()
             WHERE id = $1",
        )
        .bind(subscription_id)
        .execute(pool)
        .await?;
    }
    Ok(status)
}

async fn record_failure(
    pool: &PgPool,
    subscription_id: Uuid,
    source_url: &str,
    message: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE corpus_subscription_items
         SET status = 'failed', last_error = $3, last_seen_at = now()
         WHERE subscription_id = $1 AND source_url = $2",
    )
    .bind(subscription_id)
    .bind(source_url)
    .bind(message)
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns work a stopped server left running to the queue, so it resumes rather than being
/// lost or reported finished. Completed videos keep their status and are not redone.
pub async fn recover_interrupted(pool: &PgPool) -> anyhow::Result<u64> {
    let result = sqlx::query(
        "UPDATE corpus_subscription_items SET status = 'queued', last_seen_at = now()
         WHERE status = 'running'",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

async fn refresh_for_item(
    pool: &PgPool,
    subscription_id: Uuid,
    source_url: &str,
) -> anyhow::Result<()> {
    let ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT DISTINCT backfill_id FROM corpus_backfill_items
         WHERE subscription_id = $1 AND source_url = $2",
    )
    .bind(subscription_id)
    .bind(source_url)
    .fetch_all(pool)
    .await?;
    for id in ids {
        refresh_state(pool, id).await?;
    }
    Ok(())
}

/// Re-evaluates the backfills of a source, e.g. after review-first approvals.
pub async fn refresh_for_subscription(pool: &PgPool, subscription_id: Uuid) -> anyhow::Result<()> {
    let ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM corpus_source_backfills
         WHERE subscription_id = $1 AND state IN ('running', 'awaiting_review')",
    )
    .bind(subscription_id)
    .fetch_all(pool)
    .await?;
    for id in ids {
        refresh_state(pool, id).await?;
    }
    Ok(())
}

/// Finishes a running backfill once nothing selected remains queued or running: `completed`
/// only when every selected video finished and the listing was complete, otherwise `partial`;
/// `awaiting_review` while selected videos still wait for approval.
async fn refresh_state(pool: &PgPool, backfill_id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "WITH counts AS (
           SELECT count(*) FILTER (WHERE i.status IN ('queued', 'running')) AS remaining,
                  count(*) FILTER (WHERE i.status = 'awaiting_review') AS awaiting,
                  count(*) FILTER (WHERE i.status = 'failed') AS failed
           FROM corpus_backfill_items bi
           JOIN corpus_subscription_items i
             ON i.subscription_id = bi.subscription_id AND i.source_url = bi.source_url
           WHERE bi.backfill_id = $1 AND bi.selected AND bi.outcome IS NULL)
         UPDATE corpus_source_backfills b
         SET state = CASE
               WHEN c.remaining > 0 THEN 'running'
               WHEN c.awaiting > 0 THEN 'awaiting_review'
               WHEN c.failed > 0 OR b.discovery_message IS NOT NULL THEN 'partial'
               ELSE 'completed' END,
             finished_at = CASE WHEN c.remaining = 0 AND c.awaiting = 0 THEN now() END,
             updated_at = now()
         FROM counts c
         WHERE b.id = $1 AND b.state IN ('running', 'awaiting_review')",
    )
    .bind(backfill_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// User control of a backfill (Atlas 1.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillAction {
    Pause,
    Resume,
    Cancel,
}

#[derive(Debug, thiserror::Error)]
pub enum BackfillControlError {
    #[error("backfill not found")]
    NotFound,
    #[error("a {state} backfill cannot be {action}")]
    InvalidState { state: String, action: &'static str },
    #[error(transparent)]
    Database(#[from] anyhow::Error),
}

/// Pause stops launching more of its videos (one already running finishes), resume continues
/// from the saved progress, cancel stops the remaining videos and keeps finished results and
/// source membership.
pub async fn control_backfill(
    pool: &PgPool,
    subscription_id: Uuid,
    backfill_id: Uuid,
    action: BackfillAction,
) -> Result<BackfillProgress, BackfillControlError> {
    let state = sqlx::query_scalar::<_, String>(
        "SELECT state FROM corpus_source_backfills WHERE id = $1 AND subscription_id = $2",
    )
    .bind(backfill_id)
    .bind(subscription_id)
    .fetch_optional(pool)
    .await
    .map_err(anyhow::Error::from)?
    .ok_or(BackfillControlError::NotFound)?;
    let (allowed, next, verb): (&[&str], &str, &'static str) = match action {
        BackfillAction::Pause => (&["running"], "paused", "paused"),
        BackfillAction::Resume => (&["paused"], "running", "resumed"),
        BackfillAction::Cancel => (
            &["discovering", "running", "paused", "awaiting_review"],
            "canceled",
            "canceled",
        ),
    };
    if !allowed.contains(&state.as_str()) {
        return Err(BackfillControlError::InvalidState {
            state,
            action: verb,
        });
    }
    let mut tx = pool.begin().await.map_err(anyhow::Error::from)?;
    sqlx::query(
        "UPDATE corpus_source_backfills
         SET state = $2, updated_at = now(),
             finished_at = CASE WHEN $2 = 'canceled' THEN now() ELSE finished_at END
         WHERE id = $1",
    )
    .bind(backfill_id)
    .bind(next)
    .execute(&mut *tx)
    .await
    .map_err(anyhow::Error::from)?;
    if action == BackfillAction::Cancel {
        // Freeze the not-yet-started videos of this backfill as canceled.
        sqlx::query(
            "UPDATE corpus_backfill_items bi SET outcome = 'canceled'
             FROM corpus_subscription_items i
             WHERE bi.backfill_id = $1 AND bi.selected AND bi.outcome IS NULL
               AND i.subscription_id = bi.subscription_id AND i.source_url = bi.source_url
               AND i.status = 'queued' AND i.origin = 'history'",
        )
        .bind(backfill_id)
        .execute(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
        // Release them unless another active backfill of the source still wants them.
        sqlx::query(
            "UPDATE corpus_subscription_items i SET status = 'canceled', last_seen_at = now()
             FROM corpus_backfill_items bi
             WHERE bi.backfill_id = $1 AND bi.outcome = 'canceled'
               AND i.subscription_id = bi.subscription_id AND i.source_url = bi.source_url
               AND i.status = 'queued' AND i.origin = 'history'
               AND NOT EXISTS (
                 SELECT 1 FROM corpus_backfill_items other
                 JOIN corpus_source_backfills ob ON ob.id = other.backfill_id
                 WHERE other.subscription_id = i.subscription_id
                   AND other.source_url = i.source_url
                   AND other.backfill_id <> $1 AND other.selected AND other.outcome IS NULL
                   AND ob.state IN ('running', 'paused'))",
        )
        .bind(backfill_id)
        .execute(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
    }
    tx.commit().await.map_err(anyhow::Error::from)?;
    if action == BackfillAction::Resume {
        refresh_state(pool, backfill_id).await?;
        WORK_WAKE.notify_one();
    }
    get_backfill(pool, backfill_id)
        .await?
        .ok_or(BackfillControlError::NotFound)
}

/// Truthful progress of one backfill. Counts cover the videos selected from the snapshot;
/// `discoveredCount` is unknown (null) until the listing finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillProgress {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub corpus_id: Option<Uuid>,
    pub scope: String,
    pub recent_days: Option<i32>,
    pub limit_count: Option<i64>,
    pub state: String,
    pub discovered_count: Option<i64>,
    pub reported_count: Option<i64>,
    pub discovery_message: Option<String>,
    pub selected: i64,
    pub not_selected: Vec<ReasonCount>,
    pub queued: i64,
    pub running: i64,
    pub indexed: i64,
    pub skipped: i64,
    pub failed: i64,
    pub awaiting_review: i64,
    pub filtered: i64,
    pub canceled: i64,
    pub created_at: DateTime<Utc>,
    pub discovered_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasonCount {
    pub reason: String,
    pub count: i64,
}

const PROGRESS_SQL: &str = "SELECT b.id, b.subscription_id, b.corpus_id, b.scope, b.recent_days,
       b.limit_count, b.state, b.discovered_count, b.reported_count, b.discovery_message,
       b.created_at, b.discovered_at, b.finished_at,
       count(bi.position) FILTER (WHERE bi.selected) AS selected,
       coalesce((SELECT jsonb_agg(jsonb_build_object('reason', r.reason, 'count', r.n)
                                  ORDER BY r.reason)
                 FROM (SELECT reason, count(*) AS n FROM corpus_backfill_items
                       WHERE backfill_id = b.id AND NOT selected GROUP BY reason) r),
                '[]'::jsonb) AS not_selected,
       count(*) FILTER (WHERE bi.selected AND coalesce(bi.outcome, i.status) = 'queued') AS queued,
       count(*) FILTER (WHERE bi.selected AND coalesce(bi.outcome, i.status) = 'running') AS running,
       count(*) FILTER (WHERE bi.selected AND coalesce(bi.outcome, i.status) = 'indexed') AS indexed,
       count(*) FILTER (WHERE bi.selected AND coalesce(bi.outcome, i.status) = 'skipped') AS skipped,
       count(*) FILTER (WHERE bi.selected AND coalesce(bi.outcome, i.status) = 'failed') AS failed,
       count(*) FILTER (WHERE bi.selected AND coalesce(bi.outcome, i.status) = 'awaiting_review')
         AS awaiting_review,
       count(*) FILTER (WHERE bi.selected AND coalesce(bi.outcome, i.status) = 'filtered') AS filtered,
       count(*) FILTER (WHERE bi.selected AND coalesce(bi.outcome, i.status) = 'canceled') AS canceled
FROM corpus_source_backfills b
LEFT JOIN corpus_backfill_items bi ON bi.backfill_id = b.id
LEFT JOIN corpus_subscription_items i
  ON i.subscription_id = bi.subscription_id AND i.source_url = bi.source_url";

fn progress_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<BackfillProgress> {
    Ok(BackfillProgress {
        id: row.try_get("id")?,
        subscription_id: row.try_get("subscription_id")?,
        corpus_id: row.try_get("corpus_id")?,
        scope: row.try_get("scope")?,
        recent_days: row.try_get("recent_days")?,
        limit_count: row.try_get("limit_count")?,
        state: row.try_get("state")?,
        discovered_count: row.try_get("discovered_count")?,
        reported_count: row.try_get("reported_count")?,
        discovery_message: row.try_get("discovery_message")?,
        selected: row.try_get("selected")?,
        not_selected: row
            .try_get::<sqlx::types::Json<Vec<ReasonCount>>, _>("not_selected")?
            .0,
        queued: row.try_get("queued")?,
        running: row.try_get("running")?,
        indexed: row.try_get("indexed")?,
        skipped: row.try_get("skipped")?,
        failed: row.try_get("failed")?,
        awaiting_review: row.try_get("awaiting_review")?,
        filtered: row.try_get("filtered")?,
        canceled: row.try_get("canceled")?,
        created_at: row.try_get("created_at")?,
        discovered_at: row.try_get("discovered_at")?,
        finished_at: row.try_get("finished_at")?,
    })
}

pub async fn get_backfill(
    pool: &PgPool,
    backfill_id: Uuid,
) -> anyhow::Result<Option<BackfillProgress>> {
    let sql = format!("{PROGRESS_SQL} WHERE b.id = $1 GROUP BY b.id");
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(backfill_id)
        .fetch_optional(pool)
        .await?
        .map(progress_from_row)
        .transpose()
}

/// Backfills of a source, newest first.
pub async fn list_backfills(
    pool: &PgPool,
    subscription_id: Uuid,
) -> anyhow::Result<Vec<BackfillProgress>> {
    let sql = format!(
        "{PROGRESS_SQL} WHERE b.subscription_id = $1 GROUP BY b.id ORDER BY b.created_at DESC"
    );
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(subscription_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(progress_from_row)
        .collect()
}

/// One entry of a recorded discovery snapshot, in the source's displayed order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillEntry {
    pub position: i32,
    pub source_url: String,
    pub youtube_id: Option<String>,
    pub title: Option<String>,
    pub upload_date: Option<String>,
    pub selected: bool,
    pub reason: Option<String>,
    /// Current processing status of a selected entry (`canceled` once a cancel stopped it).
    pub status: Option<String>,
}

pub async fn list_backfill_entries(
    pool: &PgPool,
    subscription_id: Uuid,
    backfill_id: Uuid,
) -> anyhow::Result<Vec<BackfillEntry>> {
    let rows = sqlx::query(
        "SELECT bi.position, bi.source_url, bi.youtube_id, bi.title, bi.upload_date, bi.selected,
                bi.reason, CASE WHEN bi.selected THEN coalesce(bi.outcome, i.status) END AS status
         FROM corpus_backfill_items bi
         JOIN corpus_source_backfills b ON b.id = bi.backfill_id
         LEFT JOIN corpus_subscription_items i
           ON i.subscription_id = bi.subscription_id AND i.source_url = bi.source_url
         WHERE bi.backfill_id = $1 AND b.subscription_id = $2
         ORDER BY bi.position",
    )
    .bind(backfill_id)
    .bind(subscription_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(BackfillEntry {
                position: row.try_get("position")?,
                source_url: row.try_get("source_url")?,
                youtube_id: row.try_get("youtube_id")?,
                title: row.try_get("title")?,
                upload_date: row.try_get("upload_date")?,
                selected: row.try_get("selected")?,
                reason: row.try_get("reason")?,
                status: row.try_get("status")?,
            })
        })
        .collect()
}

/// Starts the server-owned discovery and processing workers. They resume persisted work after a
/// restart and run while the server runs; opening the browser UI starts nothing by itself.
pub fn spawn_background(database_url: String) {
    tokio::spawn(async move {
        let pool = match crate::db::connect(&database_url).await {
            Ok(pool) => pool,
            Err(error) => {
                tracing::warn!(%error, "backfill workers unavailable: database connection failed");
                return;
            }
        };
        match recover_interrupted(&pool).await {
            Ok(0) => {}
            Ok(count) => tracing::info!(count, "requeued interrupted source videos"),
            Err(error) => tracing::warn!(%error, "could not requeue interrupted source videos"),
        }
        let discovery_pool = pool.clone();
        tokio::spawn(async move {
            loop {
                match discover_next(&discovery_pool).await {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(error) => tracing::warn!(%error, "history discovery failed"),
                }
                let _ =
                    tokio::time::timeout(Duration::from_secs(10), DISCOVERY_WAKE.notified()).await;
            }
        });
        let processor = IngestProcessor { database_url };
        let mut dispatcher = Dispatcher::default();
        loop {
            match dispatcher.run_next(&pool, &processor).await {
                Ok(Some(_)) => continue,
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "source video processing failed"),
            }
            let _ = tokio::time::timeout(Duration::from_secs(10), WORK_WAKE.notified()).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(position: usize, upload_date: Option<&str>) -> VideoItem {
        VideoItem {
            item_id: format!("v{position}"),
            youtube_id: Some(format!("v{position}")),
            title: None,
            source_url: format!("https://www.youtube.com/watch?v=v{position}"),
            duration_seconds: None,
            upload_date: upload_date.map(str::to_string),
            metadata: Default::default(),
            local_video_path: None,
        }
    }

    #[test]
    fn limited_takes_the_first_entries_in_displayed_order() {
        // Displayed order deliberately differs from publication order (owner answer #67.Q1).
        let items = vec![
            item(1, Some("20200101")),
            item(2, Some("20240101")),
            item(3, Some("20230101")),
        ];
        let today = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let selection = select_entries(HistoryScope::Limited { count: 2 }, &items, today);
        assert_eq!(selection, vec![Ok(()), Ok(()), Err(REASON_BEYOND_LIMIT)]);
    }

    #[test]
    fn recent_uses_publication_dates_and_reports_unknown_dates() {
        let mut dated_by_timestamp = item(3, None);
        dated_by_timestamp.metadata.timestamp = Some(
            NaiveDate::from_ymd_opt(2026, 10, 20)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp(),
        );
        let items = vec![
            item(1, Some("20261024")),
            item(2, Some("20250101")),
            dated_by_timestamp,
            item(4, None),
        ];
        let today = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let selection = select_entries(HistoryScope::Recent { days: 30 }, &items, today);
        assert_eq!(
            selection,
            vec![
                Ok(()),
                Err(REASON_OUTSIDE_WINDOW),
                Ok(()),
                Err(REASON_DATE_UNKNOWN)
            ]
        );
    }

    #[test]
    fn full_selects_everything_and_none_nothing() {
        let items: Vec<_> = (1..=5).map(|index| item(index, None)).collect();
        let today = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        assert!(select_entries(HistoryScope::Full, &items, today)
            .iter()
            .all(Result::is_ok));
        assert!(select_entries(HistoryScope::None, &items, today)
            .iter()
            .all(|selection| *selection == Err(REASON_SCOPE_NONE)));
    }

    #[test]
    fn scope_input_requires_its_parameter() {
        let input = HistoryScopeInput {
            scope: HistoryScopeKind::Limited,
            recent_days: None,
            limit_count: None,
        };
        assert!(input.into_scope().is_err());
        let input = HistoryScopeInput {
            scope: HistoryScopeKind::Recent,
            recent_days: Some(30),
            limit_count: None,
        };
        assert_eq!(input.into_scope(), Ok(HistoryScope::Recent { days: 30 }));
    }
}
