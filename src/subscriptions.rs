use std::path::PathBuf;

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgRow;
use sqlx::types::Json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::config::{CaptionConfig, CorpusSource, YtDlpConfig};
use crate::ingest::{ingest_video_items, IngestReport};
use crate::youtube::{stable_child_id, stable_video_id, VideoItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionSourceKind {
    Channel,
    Playlist,
}

impl SubscriptionSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Playlist => "playlist",
        }
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "channel" => Ok(Self::Channel),
            "playlist" => Ok(Self::Playlist),
            other => anyhow::bail!("unknown subscription source kind `{other}`"),
        }
    }
}

/// Whether newly discovered videos are analyzed right away or held for review (Atlas 1.6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingPolicy {
    #[default]
    Automatic,
    ReviewFirst,
}

impl ProcessingPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::ReviewFirst => "review_first",
        }
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "automatic" => Ok(Self::Automatic),
            "review_first" => Ok(Self::ReviewFirst),
            other => anyhow::bail!("unknown processing policy `{other}`"),
        }
    }
}

/// Item status of a review-first discovery that waits for approval.
pub const AWAITING_REVIEW: &str = "awaiting_review";

#[derive(Debug, Clone)]
pub struct AddSubscriptionRequest {
    pub database_url: String,
    pub source_kind: SubscriptionSourceKind,
    pub source_url: String,
    pub name: Option<String>,
    pub enabled: bool,
    pub work_dir: PathBuf,
    pub caption: CaptionConfig,
    pub yt_dlp: YtDlpConfig,
    pub asr_enabled: bool,
    pub transcriber_command: Option<PathBuf>,
    pub transcriber_args: Vec<String>,
    pub transcriber_timeout_seconds: Option<u64>,
    pub max_items: Option<u64>,
    pub title_contains: Option<String>,
    pub title_excludes: Vec<String>,
    pub duration_min: Option<f64>,
    pub duration_max: Option<f64>,
    pub processing_policy: ProcessingPolicy,
    pub migrate: bool,
}

#[derive(Debug, Clone)]
pub struct CheckSubscriptionsRequest {
    pub database_url: String,
    pub id: Option<Uuid>,
    pub include_disabled: bool,
    pub migrate: bool,
    pub dispatch: CheckDispatch,
}

/// How a check hands admitted new uploads to processing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckDispatch {
    /// Ingest admitted videos before the check returns (CLI checks without a server).
    #[default]
    Inline,
    /// Queue admitted videos as new-upload work for the server's corpus dispatcher, which
    /// interleaves them fairly with historical backfill (Atlas 1.16).
    Enqueue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    pub id: Uuid,
    pub source_kind: SubscriptionSourceKind,
    pub source_url: String,
    pub name: Option<String>,
    pub enabled: bool,
    pub work_dir: PathBuf,
    pub caption: CaptionConfig,
    pub yt_dlp: YtDlpConfig,
    pub asr_enabled: bool,
    pub transcriber_command: Option<PathBuf>,
    pub transcriber_args: Vec<String>,
    pub transcriber_timeout_seconds: Option<u64>,
    pub max_items: Option<u64>,
    pub title_contains: Option<String>,
    pub title_excludes: Vec<String>,
    pub duration_min: Option<f64>,
    pub duration_max: Option<f64>,
    pub processing_policy: ProcessingPolicy,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_ingested_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckSubscriptionsReport {
    pub checked_at: DateTime<Utc>,
    pub subscriptions_checked: u64,
    pub videos_discovered: u64,
    pub new_videos: u64,
    pub videos_indexed: u64,
    pub segments_indexed: u64,
    pub items: Vec<SubscriptionCheckReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionCheckReport {
    pub subscription: SubscriptionSummary,
    pub status: String,
    pub videos_discovered: u64,
    pub new_videos: u64,
    pub videos_indexed: u64,
    pub segments_indexed: u64,
    /// Newly held for review by this check (review-first sources).
    #[serde(default)]
    pub awaiting_review: u64,
    /// Newly queued for background processing by this check (server checks).
    #[serde(default)]
    pub queued: u64,
    pub ingest: Option<IngestReport>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionSummary {
    pub id: Uuid,
    pub source_kind: SubscriptionSourceKind,
    pub source_url: String,
    pub name: Option<String>,
}

impl From<&Subscription> for SubscriptionSummary {
    fn from(value: &Subscription) -> Self {
        Self {
            id: value.id,
            source_kind: value.source_kind,
            source_url: value.source_url.clone(),
            name: value.name.clone(),
        }
    }
}

impl Subscription {
    pub(crate) fn to_ingest_request(&self, database_url: String) -> crate::ingest::IngestRequest {
        crate::ingest::IngestRequest {
            database_url,
            run_id: None,
            source: match self.source_kind {
                SubscriptionSourceKind::Channel => CorpusSource::ChannelUrl {
                    url: self.source_url.clone(),
                },
                SubscriptionSourceKind::Playlist => CorpusSource::PlaylistUrl {
                    url: self.source_url.clone(),
                },
            },
            work_dir: self.work_dir.clone(),
            caption: self.caption.clone(),
            yt_dlp: self.yt_dlp.clone(),
            asr_enabled: self.asr_enabled,
            transcriber_command: self.transcriber_command.clone(),
            transcriber_args: self.transcriber_args.clone(),
            transcriber_timeout_seconds: self.transcriber_timeout_seconds,
            max_items: self.max_items,
            title_contains: self.title_contains.clone(),
            title_excludes: self.title_excludes.clone(),
            duration_min: self.duration_min,
            duration_max: self.duration_max,
            migrate: false,
        }
    }
}

pub async fn add_subscription(request: AddSubscriptionRequest) -> anyhow::Result<Subscription> {
    let pool = crate::db::connect(&request.database_url).await?;
    if request.migrate {
        crate::db::migrate(&pool).await?;
    }

    let id = stable_video_id(&request.source_url);
    let max_items = request.max_items.map(|value| value as i64);
    let transcriber_command = request
        .transcriber_command
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let work_dir = request.work_dir.to_string_lossy().into_owned();
    sqlx::query(
        "INSERT INTO corpus_subscriptions
         (id, source_kind, source_url, name, enabled, work_dir, caption, yt_dlp,
          asr_enabled, transcriber_command, transcriber_args, transcriber_timeout_seconds,
          max_items, title_contains, title_excludes, duration_min, duration_max,
          processing_policy)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
         ON CONFLICT (source_url) DO UPDATE SET
           source_kind = EXCLUDED.source_kind,
           name = EXCLUDED.name,
           enabled = EXCLUDED.enabled,
           work_dir = EXCLUDED.work_dir,
           caption = EXCLUDED.caption,
           yt_dlp = EXCLUDED.yt_dlp,
           asr_enabled = EXCLUDED.asr_enabled,
           transcriber_command = EXCLUDED.transcriber_command,
           transcriber_args = EXCLUDED.transcriber_args,
           transcriber_timeout_seconds = EXCLUDED.transcriber_timeout_seconds,
           max_items = EXCLUDED.max_items,
           title_contains = EXCLUDED.title_contains,
           title_excludes = EXCLUDED.title_excludes,
           duration_min = EXCLUDED.duration_min,
           duration_max = EXCLUDED.duration_max,
           processing_policy = EXCLUDED.processing_policy,
           updated_at = now()",
    )
    .bind(id)
    .bind(request.source_kind.as_str())
    .bind(&request.source_url)
    .bind(&request.name)
    .bind(request.enabled)
    .bind(&work_dir)
    .bind(Json(request.caption))
    .bind(Json(request.yt_dlp))
    .bind(request.asr_enabled)
    .bind(transcriber_command)
    .bind(&request.transcriber_args)
    .bind(
        request
            .transcriber_timeout_seconds
            .map(|value| value as i64),
    )
    .bind(max_items)
    .bind(&request.title_contains)
    .bind(&request.title_excludes)
    .bind(request.duration_min)
    .bind(request.duration_max)
    .bind(request.processing_policy.as_str())
    .execute(&pool)
    .await?;

    get_subscription_by_url(&pool, &request.source_url).await
}

pub async fn list_subscriptions(
    database_url: &str,
    include_disabled: bool,
    migrate: bool,
) -> anyhow::Result<Vec<Subscription>> {
    let pool = crate::db::connect(database_url).await?;
    if migrate {
        crate::db::migrate(&pool).await?;
    }
    load_subscriptions(&pool, None, include_disabled).await
}

pub async fn check_subscriptions(
    request: CheckSubscriptionsRequest,
) -> anyhow::Result<CheckSubscriptionsReport> {
    let pool = crate::db::connect(&request.database_url).await?;
    if request.migrate {
        crate::db::migrate(&pool).await?;
    }

    let subscriptions = load_subscriptions(&pool, request.id, request.include_disabled).await?;
    let checked_at = Utc::now();
    let mut items = Vec::new();
    let mut videos_discovered = 0;
    let mut new_videos = 0;
    let mut videos_indexed = 0;
    let mut segments_indexed = 0;

    for subscription in subscriptions {
        let item = match check_subscription(
            &pool,
            &request.database_url,
            &subscription,
            request.dispatch,
        )
        .await
        {
            Ok(report) => report,
            Err(error) => {
                mark_checked(&pool, subscription.id, false).await?;
                SubscriptionCheckReport {
                    subscription: SubscriptionSummary::from(&subscription),
                    status: "failed".to_string(),
                    videos_discovered: 0,
                    new_videos: 0,
                    videos_indexed: 0,
                    segments_indexed: 0,
                    awaiting_review: 0,
                    queued: 0,
                    ingest: None,
                    message: Some(error.to_string()),
                }
            }
        };
        videos_discovered += item.videos_discovered;
        new_videos += item.new_videos;
        videos_indexed += item.videos_indexed;
        segments_indexed += item.segments_indexed;
        items.push(item);
    }

    Ok(CheckSubscriptionsReport {
        checked_at,
        subscriptions_checked: items.len() as u64,
        videos_discovered,
        new_videos,
        videos_indexed,
        segments_indexed,
        items,
    })
}

async fn check_subscription(
    pool: &PgPool,
    database_url: &str,
    subscription: &Subscription,
    dispatch: CheckDispatch,
) -> anyhow::Result<SubscriptionCheckReport> {
    // Until the initial history snapshot is recorded, every listed video would look new, so a
    // routine check must not sweep excluded history into analysis (Atlas 1.10, 1.18).
    if crate::backfill::history_discovery_pending(pool, subscription.id).await? {
        return Ok(SubscriptionCheckReport {
            subscription: SubscriptionSummary::from(subscription),
            status: "waiting_for_history".to_string(),
            videos_discovered: 0,
            new_videos: 0,
            videos_indexed: 0,
            segments_indexed: 0,
            awaiting_review: 0,
            queued: 0,
            ingest: None,
            message: Some(
                "The source's history is still being listed; checks for new uploads resume once it is recorded."
                    .to_string(),
            ),
        });
    }
    let discovered = crate::youtube::discover_collection(
        &subscription.source_url,
        subscription.max_items,
        &subscription.yt_dlp,
    )
    .await?;
    let mut new_items = Vec::new();
    let mut new_videos = 0;
    for item in &discovered {
        let status = subscription_item_status(pool, subscription.id, item).await?;
        upsert_subscription_item(pool, subscription.id, item).await?;
        if status.is_none() {
            new_videos += 1;
        }
        // A source switched to automatic admits what it previously held for review.
        let released = status.as_deref() == Some(AWAITING_REVIEW)
            && subscription.processing_policy == ProcessingPolicy::Automatic;
        if released || should_ingest_status(status.as_deref()) {
            if crate::youtube::filter_item(
                item,
                &subscription.title_contains,
                &subscription.title_excludes,
                subscription.duration_min,
                subscription.duration_max,
            ) {
                new_items.push(item.clone());
            } else {
                mark_subscription_item_status(pool, subscription.id, &item.source_url, "filtered")
                    .await?;
            }
        }
    }

    let mut admitted: Vec<VideoItem> = Vec::new();
    let mut awaiting_review = 0;
    for item in new_items {
        match admit_item(pool, subscription, &item).await? {
            Admission::Process => admitted.push(item),
            Admission::AlreadyProcessed(status) => {
                mark_subscription_item_status(pool, subscription.id, &item.source_url, &status)
                    .await?;
            }
            Admission::AwaitReview => {
                mark_subscription_item_status(
                    pool,
                    subscription.id,
                    &item.source_url,
                    AWAITING_REVIEW,
                )
                .await?;
                awaiting_review += 1;
            }
        }
    }

    let mut ingest = None;
    let mut videos_indexed = 0;
    let mut segments_indexed = 0;
    let mut queued = 0;
    if dispatch == CheckDispatch::Enqueue {
        for item in &admitted {
            crate::backfill::enqueue_new_upload(pool, subscription.id, &item.source_url).await?;
            queued += 1;
        }
        if queued > 0 {
            crate::backfill::wake();
        }
        admitted.clear();
    }
    if !admitted.is_empty() {
        let report = ingest_video_items(
            subscription.to_ingest_request(database_url.to_string()),
            admitted,
        )
        .await?;
        videos_indexed = report.videos_indexed;
        segments_indexed = report.segments_indexed;
        update_ingested_items(pool, subscription.id, &report).await?;
        ingest = Some(report);
    }
    mark_checked(pool, subscription.id, ingest.is_some()).await?;

    Ok(SubscriptionCheckReport {
        subscription: SubscriptionSummary::from(subscription),
        status: "completed".to_string(),
        videos_discovered: discovered.len() as u64,
        new_videos,
        videos_indexed,
        segments_indexed,
        awaiting_review,
        queued,
        ingest,
        message: None,
    })
}

pub async fn get_subscription(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<Subscription>> {
    Ok(load_subscriptions(pool, Some(id), true)
        .await?
        .into_iter()
        .next())
}

async fn load_subscriptions(
    pool: &PgPool,
    id: Option<Uuid>,
    include_disabled: bool,
) -> anyhow::Result<Vec<Subscription>> {
    let rows = if let Some(id) = id {
        let sql = subscription_select_sql("WHERE id = $1");
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .fetch_all(pool)
            .await?
    } else if include_disabled {
        let sql = subscription_select_sql("");
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .fetch_all(pool)
            .await?
    } else {
        let sql = subscription_select_sql("WHERE enabled = true");
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .fetch_all(pool)
            .await?
    };

    rows.into_iter().map(subscription_from_row).collect()
}

async fn get_subscription_by_url(pool: &PgPool, source_url: &str) -> anyhow::Result<Subscription> {
    let sql = subscription_select_sql("WHERE source_url = $1");
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(source_url)
        .fetch_one(pool)
        .await?;
    subscription_from_row(row)
}

/// Only static `WHERE` clauses are interpolated (callers bind every value), so the resulting SQL is
/// safe to assert for sqlx's dynamic-query check.
fn subscription_select_sql(where_clause: &'static str) -> String {
    format!(
        "SELECT id, source_kind, source_url, name, enabled, work_dir, caption, asr_enabled,
          yt_dlp, transcriber_command, transcriber_args, transcriber_timeout_seconds,
          max_items, title_contains, title_excludes,
          duration_min, duration_max, processing_policy, last_checked_at, last_ingested_at, created_at, updated_at
         FROM corpus_subscriptions
         {where_clause}
         ORDER BY created_at, source_url"
    )
}

fn subscription_from_row(row: PgRow) -> anyhow::Result<Subscription> {
    let source_kind: String = row.try_get("source_kind")?;
    let caption: Json<CaptionConfig> = row.try_get("caption")?;
    let yt_dlp: Json<YtDlpConfig> = row.try_get("yt_dlp")?;
    let max_items: Option<i64> = row.try_get("max_items")?;
    let transcriber_timeout_seconds: Option<i64> = row.try_get("transcriber_timeout_seconds")?;
    let transcriber_command: Option<String> = row.try_get("transcriber_command")?;
    let work_dir: String = row.try_get("work_dir")?;
    Ok(Subscription {
        id: row.try_get("id")?,
        source_kind: parse_source_kind(&source_kind)?,
        source_url: row.try_get("source_url")?,
        name: row.try_get("name")?,
        enabled: row.try_get("enabled")?,
        work_dir: PathBuf::from(work_dir),
        caption: caption.0,
        yt_dlp: yt_dlp.0,
        asr_enabled: row.try_get("asr_enabled")?,
        transcriber_command: transcriber_command.map(PathBuf::from),
        transcriber_args: row.try_get("transcriber_args")?,
        transcriber_timeout_seconds: transcriber_timeout_seconds.map(|value| value as u64),
        max_items: max_items.map(|value| value as u64),
        title_contains: row.try_get("title_contains")?,
        title_excludes: row.try_get("title_excludes")?,
        duration_min: row.try_get("duration_min")?,
        duration_max: row.try_get("duration_max")?,
        processing_policy: ProcessingPolicy::parse(
            &row.try_get::<String, _>("processing_policy")?,
        )?,
        last_checked_at: row.try_get("last_checked_at")?,
        last_ingested_at: row.try_get("last_ingested_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn parse_source_kind(value: &str) -> anyhow::Result<SubscriptionSourceKind> {
    SubscriptionSourceKind::parse(value)
}

pub(crate) fn should_ingest_status(status: Option<&str>) -> bool {
    matches!(status, None | Some("discovered" | "failed"))
}

pub(crate) async fn subscription_item_status(
    pool: &PgPool,
    subscription_id: Uuid,
    item: &VideoItem,
) -> anyhow::Result<Option<String>> {
    let status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM corpus_subscription_items
         WHERE subscription_id = $1
           AND (source_url = $2 OR ($3::text IS NOT NULL AND youtube_id = $3))
         ORDER BY first_seen_at
         LIMIT 1",
    )
    .bind(subscription_id)
    .bind(&item.source_url)
    .bind(&item.youtube_id)
    .fetch_optional(pool)
    .await?;
    Ok(status)
}

pub(crate) async fn upsert_subscription_item(
    pool: &PgPool,
    subscription_id: Uuid,
    item: &VideoItem,
) -> anyhow::Result<()> {
    let id = stable_child_id(subscription_id, &item.source_url);
    sqlx::query(
        "INSERT INTO corpus_subscription_items
         (id, subscription_id, source_url, youtube_id, title, duration_seconds, upload_date,
          channel, channel_id, uploader, uploader_id, view_count, categories, tags, metadata)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
         ON CONFLICT (subscription_id, source_url) DO UPDATE SET
           youtube_id = EXCLUDED.youtube_id,
           title = EXCLUDED.title,
           duration_seconds = EXCLUDED.duration_seconds,
           upload_date = EXCLUDED.upload_date,
           channel = EXCLUDED.channel,
           channel_id = EXCLUDED.channel_id,
           uploader = EXCLUDED.uploader,
           uploader_id = EXCLUDED.uploader_id,
           view_count = EXCLUDED.view_count,
           categories = EXCLUDED.categories,
           tags = EXCLUDED.tags,
           metadata = EXCLUDED.metadata,
           last_seen_at = now()",
    )
    .bind(id)
    .bind(subscription_id)
    .bind(&item.source_url)
    .bind(&item.youtube_id)
    .bind(&item.title)
    .bind(item.duration_seconds)
    .bind(&item.upload_date)
    .bind(&item.metadata.channel)
    .bind(&item.metadata.channel_id)
    .bind(&item.metadata.uploader)
    .bind(&item.metadata.uploader_id)
    .bind(item.metadata.view_count)
    .bind(&item.metadata.categories)
    .bind(&item.metadata.tags)
    .bind(serde_json::to_value(&item.metadata)?)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn mark_subscription_item_status(
    pool: &PgPool,
    subscription_id: Uuid,
    source_url: &str,
    status: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE corpus_subscription_items
         SET status = $3, last_seen_at = now()
         WHERE subscription_id = $1 AND source_url = $2",
    )
    .bind(subscription_id)
    .bind(source_url)
    .bind(status)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn update_ingested_items(
    pool: &PgPool,
    subscription_id: Uuid,
    report: &IngestReport,
) -> anyhow::Result<()> {
    for item in &report.items {
        sqlx::query(
            "UPDATE corpus_subscription_items
             SET status = $3, ingested_at = now(), last_seen_at = now()
             WHERE subscription_id = $1 AND source_url = $2",
        )
        .bind(subscription_id)
        .bind(&item.source_url)
        .bind(&item.status)
        .execute(pool)
        .await?;
        settle_pending_elsewhere(pool, subscription_id, &item.source_url, &item.status).await?;
    }
    Ok(())
}

/// Decision 1.8: once a video is processed through one source, it no longer waits for
/// approval in any other source.
pub async fn settle_pending_elsewhere(
    pool: &PgPool,
    subscription_id: Uuid,
    source_url: &str,
    status: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE corpus_subscription_items o
         SET status = $2, last_seen_at = now()
         FROM corpus_subscription_items i
         WHERE i.subscription_id = $1 AND i.source_url = $3
           AND o.status = 'awaiting_review'
           AND (o.source_url = i.source_url
                OR (i.youtube_id IS NOT NULL AND o.youtube_id = i.youtube_id))",
    )
    .bind(subscription_id)
    .bind(status)
    .bind(source_url)
    .execute(pool)
    .await?;
    Ok(())
}

/// What a source check does with one newly discovered, unfiltered item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// Analyze now (automatic source, or another automatic source applies).
    Process,
    /// The video was already processed through another source; record that status.
    AlreadyProcessed(String),
    /// Hold for explicit approval.
    AwaitReview,
}

/// Admission under Decisions 1.6 and 1.8: review-first holds a video only if no applicable
/// enabled source requests automatic processing, and never re-processes finished work.
pub async fn admit_item(
    pool: &PgPool,
    subscription: &Subscription,
    item: &VideoItem,
) -> anyhow::Result<Admission> {
    let processed = sqlx::query_scalar::<_, String>(
        "SELECT status FROM corpus_subscription_items
         WHERE subscription_id <> $1
           AND (source_url = $2 OR ($3::text IS NOT NULL AND youtube_id = $3))
           AND status IN ('indexed', 'skipped')
         ORDER BY ingested_at DESC NULLS LAST
         LIMIT 1",
    )
    .bind(subscription.id)
    .bind(&item.source_url)
    .bind(&item.youtube_id)
    .fetch_optional(pool)
    .await?;
    if let Some(status) = processed {
        return Ok(Admission::AlreadyProcessed(status));
    }
    if subscription.processing_policy == ProcessingPolicy::Automatic {
        return Ok(Admission::Process);
    }
    let automatic_elsewhere = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
           SELECT 1 FROM corpus_subscription_items i
           JOIN corpus_subscriptions s ON s.id = i.subscription_id
           WHERE i.subscription_id <> $1 AND s.enabled
             AND s.processing_policy = 'automatic'
             AND (i.source_url = $2 OR ($3::text IS NOT NULL AND i.youtube_id = $3)))",
    )
    .bind(subscription.id)
    .bind(&item.source_url)
    .bind(&item.youtube_id)
    .fetch_one(pool)
    .await?;
    Ok(if automatic_elsewhere {
        Admission::Process
    } else {
        Admission::AwaitReview
    })
}

pub async fn set_processing_policy(
    pool: &PgPool,
    subscription_id: Uuid,
    policy: ProcessingPolicy,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE corpus_subscriptions SET processing_policy = $2, updated_at = now() WHERE id = $1",
    )
    .bind(subscription_id)
    .bind(policy.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

/// A discovered video of a review-first source that waits for approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingItem {
    pub source_url: String,
    pub youtube_id: Option<String>,
    pub title: Option<String>,
    pub channel: Option<String>,
    pub duration_seconds: Option<f64>,
    pub upload_date: Option<String>,
    pub first_seen_at: DateTime<Utc>,
}

pub async fn list_pending_items(
    pool: &PgPool,
    subscription_id: Uuid,
) -> anyhow::Result<Vec<PendingItem>> {
    let rows = sqlx::query(
        "SELECT source_url, youtube_id, title, channel, duration_seconds, upload_date,
                first_seen_at
         FROM corpus_subscription_items
         WHERE subscription_id = $1 AND status = 'awaiting_review'
         ORDER BY upload_date DESC NULLS LAST, first_seen_at DESC",
    )
    .bind(subscription_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(PendingItem {
                source_url: row.try_get("source_url")?,
                youtube_id: row.try_get("youtube_id")?,
                title: row.try_get("title")?,
                channel: row.try_get("channel")?,
                duration_seconds: row.try_get("duration_seconds")?,
                upload_date: row.try_get("upload_date")?,
                first_seen_at: row.try_get("first_seen_at")?,
            })
        })
        .collect()
}

/// Approves pending items of a source and processes them with the source's ingest settings.
pub async fn approve_pending_items(
    database_url: &str,
    subscription_id: Uuid,
    source_urls: &[String],
) -> anyhow::Result<IngestReport> {
    let pool = crate::db::connect(database_url).await?;
    let subscription = get_subscription(&pool, subscription_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("source not found"))?;
    let rows = sqlx::query(
        "SELECT source_url, youtube_id, title, duration_seconds, upload_date, metadata
         FROM corpus_subscription_items
         WHERE subscription_id = $1 AND status = 'awaiting_review' AND source_url = ANY($2)",
    )
    .bind(subscription_id)
    .bind(source_urls)
    .fetch_all(&pool)
    .await?;
    if rows.len() != source_urls.len() {
        anyhow::bail!("some videos are not awaiting review for this source");
    }
    let items = rows
        .into_iter()
        .map(item_from_row)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let report = ingest_video_items(
        subscription.to_ingest_request(database_url.to_string()),
        items,
    )
    .await?;
    update_ingested_items(&pool, subscription_id, &report).await?;
    crate::backfill::refresh_for_subscription(&pool, subscription_id).await?;
    Ok(report)
}

/// Rebuilds a discovered video from its `corpus_subscription_items` row (`source_url`,
/// `youtube_id`, `title`, `duration_seconds`, `upload_date`, `metadata`).
pub(crate) fn item_from_row(row: PgRow) -> anyhow::Result<VideoItem> {
    let source_url: String = row.try_get("source_url")?;
    let youtube_id: Option<String> = row.try_get("youtube_id")?;
    let metadata: Option<serde_json::Value> = row.try_get("metadata")?;
    Ok(VideoItem {
        item_id: crate::youtube::sanitize_id(youtube_id.as_deref().unwrap_or(&source_url)),
        youtube_id,
        title: row.try_get("title")?,
        source_url,
        duration_seconds: row.try_get("duration_seconds")?,
        upload_date: row.try_get("upload_date")?,
        metadata: metadata
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default(),
        local_video_path: None,
    })
}

async fn mark_checked(pool: &PgPool, subscription_id: Uuid, ingested: bool) -> anyhow::Result<()> {
    let sql = if ingested {
        "UPDATE corpus_subscriptions
         SET last_checked_at = now(), last_ingested_at = now(), updated_at = now()
         WHERE id = $1"
    } else {
        "UPDATE corpus_subscriptions
         SET last_checked_at = now(), updated_at = now()
         WHERE id = $1"
    };
    sqlx::query(sql).bind(subscription_id).execute(pool).await?;
    Ok(())
}
