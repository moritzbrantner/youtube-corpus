use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::types::Json;
use sqlx::Row;
use uuid::Uuid;

use crate::subscriptions::SubscriptionSourceKind;

#[derive(Debug, Clone)]
pub struct ListVideosRequest {
    pub database_url: String,
    pub downloaded_only: bool,
    pub parsed_only: bool,
    pub limit: Option<i64>,
    pub migrate: bool,
}

#[derive(Debug, Clone)]
pub struct CorpusStatusRequest {
    pub database_url: String,
    pub include_disabled: bool,
    pub downloaded_only: bool,
    pub parsed_only: bool,
    pub limit: Option<i64>,
    pub migrate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusStatusReport {
    pub monitored_sources: Vec<MonitoredSourceStatus>,
    pub videos: Vec<VideoStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitoredSourceStatus {
    pub id: Uuid,
    pub source_kind: SubscriptionSourceKind,
    pub source_url: String,
    pub name: Option<String>,
    pub enabled: bool,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_ingested_at: Option<DateTime<Utc>>,
    pub items_seen: u64,
    pub items_discovered: u64,
    pub items_indexed: u64,
    pub items_without_transcript: u64,
    pub items_failed: u64,
    pub items_filtered: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoStatus {
    pub id: Uuid,
    pub youtube_id: Option<String>,
    pub source_url: String,
    pub title: Option<String>,
    pub media_downloaded: bool,
    pub local_video_path: Option<PathBuf>,
    pub caption_files_downloaded: bool,
    pub parsed: bool,
    pub transcript_streams: u64,
    pub transcript_segments: u64,
    pub transcript_source_kinds: Vec<String>,
    pub subscription_names: Vec<String>,
    pub subscription_source_urls: Vec<String>,
    pub duration_seconds: Option<f64>,
    pub upload_date: Option<String>,
    pub description: Option<String>,
    pub channel: Option<String>,
    pub channel_id: Option<String>,
    pub channel_url: Option<String>,
    pub uploader: Option<String>,
    pub uploader_id: Option<String>,
    pub uploader_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration_string: Option<String>,
    pub timestamp: Option<i64>,
    pub release_timestamp: Option<i64>,
    pub view_count: Option<i64>,
    pub like_count: Option<i64>,
    pub comment_count: Option<i64>,
    pub live_status: Option<String>,
    pub availability: Option<String>,
    pub age_limit: Option<i64>,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn corpus_status(request: CorpusStatusRequest) -> anyhow::Result<CorpusStatusReport> {
    let pool = crate::db::connect(&request.database_url).await?;
    if request.migrate {
        crate::db::migrate(&pool).await?;
    }

    let monitored_sources = list_monitored_sources(&pool, request.include_disabled).await?;
    let videos = list_videos_with_pool(
        &pool,
        request.downloaded_only,
        request.parsed_only,
        request.limit,
    )
    .await?;

    Ok(CorpusStatusReport {
        monitored_sources,
        videos,
    })
}

pub async fn list_videos(request: ListVideosRequest) -> anyhow::Result<Vec<VideoStatus>> {
    let pool = crate::db::connect(&request.database_url).await?;
    if request.migrate {
        crate::db::migrate(&pool).await?;
    }
    list_videos_with_pool(
        &pool,
        request.downloaded_only,
        request.parsed_only,
        request.limit,
    )
    .await
}

pub async fn list_monitored_sources(
    pool: &sqlx::PgPool,
    include_disabled: bool,
) -> anyhow::Result<Vec<MonitoredSourceStatus>> {
    let rows = sqlx::query(
        "SELECT s.id, s.source_kind, s.source_url, s.name, s.enabled,
                s.last_checked_at, s.last_ingested_at, s.created_at, s.updated_at,
                count(i.id)::bigint AS items_seen,
                count(i.id) FILTER (WHERE i.status = 'discovered')::bigint AS items_discovered,
                count(i.id) FILTER (WHERE i.status = 'indexed')::bigint AS items_indexed,
                count(i.id) FILTER (WHERE i.status = 'no_transcript')::bigint AS items_without_transcript,
                count(i.id) FILTER (WHERE i.status = 'failed')::bigint AS items_failed,
                count(i.id) FILTER (WHERE i.status = 'filtered')::bigint AS items_filtered
         FROM corpus_subscriptions s
         LEFT JOIN corpus_subscription_items i ON i.subscription_id = s.id
         WHERE ($1::boolean OR s.enabled)
         GROUP BY s.id, s.source_kind, s.source_url, s.name, s.enabled,
                  s.last_checked_at, s.last_ingested_at, s.created_at, s.updated_at
         ORDER BY s.created_at, s.source_url",
    )
    .bind(include_disabled)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let source_kind: String = row.try_get("source_kind")?;
            Ok(MonitoredSourceStatus {
                id: row.try_get("id")?,
                source_kind: SubscriptionSourceKind::parse(&source_kind)?,
                source_url: row.try_get("source_url")?,
                name: row.try_get("name")?,
                enabled: row.try_get("enabled")?,
                last_checked_at: row.try_get("last_checked_at")?,
                last_ingested_at: row.try_get("last_ingested_at")?,
                items_seen: row_count(&row, "items_seen")?,
                items_discovered: row_count(&row, "items_discovered")?,
                items_indexed: row_count(&row, "items_indexed")?,
                items_without_transcript: row_count(&row, "items_without_transcript")?,
                items_failed: row_count(&row, "items_failed")?,
                items_filtered: row_count(&row, "items_filtered")?,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .collect()
}

async fn list_videos_with_pool(
    pool: &sqlx::PgPool,
    downloaded_only: bool,
    parsed_only: bool,
    limit: Option<i64>,
) -> anyhow::Result<Vec<VideoStatus>> {
    let rows = sqlx::query(
        "SELECT v.id, v.youtube_id, v.source_url, v.title, v.local_video_path,
                v.duration_seconds, v.upload_date, v.created_at, v.updated_at,
                v.description, v.channel, v.channel_id, v.channel_url,
                v.uploader, v.uploader_id, v.uploader_url, v.thumbnail_url,
                v.duration_string, v.timestamp, v.release_timestamp, v.view_count,
                v.like_count, v.comment_count, v.live_status, v.availability,
                v.age_limit, v.categories, v.tags, v.metadata,
                count(DISTINCT st.id)::bigint AS transcript_streams,
                count(DISTINCT ts.id)::bigint AS transcript_segments,
                count(DISTINCT st.id) FILTER (WHERE st.source_path IS NOT NULL)::bigint
                  AS transcript_files,
                coalesce(
                  jsonb_agg(DISTINCT st.source_kind) FILTER (WHERE st.source_kind IS NOT NULL),
                  '[]'::jsonb
                ) AS transcript_source_kinds,
                coalesce(
                  jsonb_agg(DISTINCT s.name) FILTER (WHERE s.name IS NOT NULL),
                  '[]'::jsonb
                ) AS subscription_names,
                coalesce(
                  jsonb_agg(DISTINCT s.source_url) FILTER (WHERE s.source_url IS NOT NULL),
                  '[]'::jsonb
                ) AS subscription_source_urls
         FROM videos v
         LEFT JOIN transcript_streams st ON st.video_id = v.id
         LEFT JOIN transcript_segments ts ON ts.stream_id = st.id
         LEFT JOIN corpus_subscription_items i
           ON i.source_url = v.source_url
           OR (v.youtube_id IS NOT NULL AND i.youtube_id = v.youtube_id)
         LEFT JOIN corpus_subscriptions s ON s.id = i.subscription_id
         WHERE ($1::boolean = false OR v.local_video_path IS NOT NULL)
           AND (
             $2::boolean = false
             OR EXISTS (
               SELECT 1 FROM transcript_segments parsed
               WHERE parsed.video_id = v.id
             )
           )
         GROUP BY v.id, v.youtube_id, v.source_url, v.title, v.local_video_path,
                  v.duration_seconds, v.upload_date, v.created_at, v.updated_at,
                  v.description, v.channel, v.channel_id, v.channel_url,
                  v.uploader, v.uploader_id, v.uploader_url, v.thumbnail_url,
                  v.duration_string, v.timestamp, v.release_timestamp, v.view_count,
                  v.like_count, v.comment_count, v.live_status, v.availability,
                  v.age_limit, v.categories, v.tags, v.metadata
         ORDER BY v.updated_at DESC, v.source_url
         LIMIT $3",
    )
    .bind(downloaded_only)
    .bind(parsed_only)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let local_video_path: Option<String> = row.try_get("local_video_path")?;
            let transcript_streams = row_count(&row, "transcript_streams")?;
            let transcript_segments = row_count(&row, "transcript_segments")?;
            let transcript_files = row_count(&row, "transcript_files")?;
            let transcript_source_kinds: Json<Vec<String>> =
                row.try_get("transcript_source_kinds")?;
            let subscription_names: Json<Vec<String>> = row.try_get("subscription_names")?;
            let subscription_source_urls: Json<Vec<String>> =
                row.try_get("subscription_source_urls")?;
            let categories: Vec<String> = row.try_get("categories")?;
            let tags: Vec<String> = row.try_get("tags")?;
            let metadata: Json<Value> = row.try_get("metadata")?;
            Ok(VideoStatus {
                id: row.try_get("id")?,
                youtube_id: row.try_get("youtube_id")?,
                source_url: row.try_get("source_url")?,
                title: row.try_get("title")?,
                media_downloaded: local_video_path.is_some(),
                local_video_path: local_video_path.map(PathBuf::from),
                caption_files_downloaded: transcript_files > 0,
                parsed: transcript_segments > 0,
                transcript_streams,
                transcript_segments,
                transcript_source_kinds: transcript_source_kinds.0,
                subscription_names: subscription_names.0,
                subscription_source_urls: subscription_source_urls.0,
                duration_seconds: row.try_get("duration_seconds")?,
                upload_date: row.try_get("upload_date")?,
                description: row.try_get("description")?,
                channel: row.try_get("channel")?,
                channel_id: row.try_get("channel_id")?,
                channel_url: row.try_get("channel_url")?,
                uploader: row.try_get("uploader")?,
                uploader_id: row.try_get("uploader_id")?,
                uploader_url: row.try_get("uploader_url")?,
                thumbnail_url: row.try_get("thumbnail_url")?,
                duration_string: row.try_get("duration_string")?,
                timestamp: row.try_get("timestamp")?,
                release_timestamp: row.try_get("release_timestamp")?,
                view_count: row.try_get("view_count")?,
                like_count: row.try_get("like_count")?,
                comment_count: row.try_get("comment_count")?,
                live_status: row.try_get("live_status")?,
                availability: row.try_get("availability")?,
                age_limit: row.try_get("age_limit")?,
                categories,
                tags,
                metadata: metadata.0,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .collect()
}

fn row_count(row: &sqlx::postgres::PgRow, name: &str) -> anyhow::Result<u64> {
    let value: i64 = row.try_get(name)?;
    Ok(value as u64)
}
