use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub const DEFAULT_CORPUS_ID: Uuid = Uuid::from_u128(0x00000000000050008000000000000001);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Corpus {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub video_count: u64,
    pub source_count: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateCorpusRequest {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusSourceSummary {
    pub id: Uuid,
    pub source_kind: String,
    pub source_url: String,
    pub name: Option<String>,
    pub enabled: bool,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_ingested_at: Option<DateTime<Utc>>,
    pub last_check_status: Option<String>,
    pub last_check_message: Option<String>,
    pub items_seen: u64,
    pub items_indexed: u64,
    pub items_failed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusVideoSummary {
    pub id: Uuid,
    pub youtube_id: Option<String>,
    pub source_url: String,
    pub title: Option<String>,
    pub channel: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration_seconds: Option<f64>,
    pub upload_date: Option<String>,
    pub transcript_streams: u64,
    pub transcript_segments: u64,
    pub transcript_source_kinds: Vec<String>,
    pub processing_revision: Option<u64>,
    pub last_retrieved_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

pub fn normalize_slug(value: &str) -> anyhow::Result<String> {
    let mut slug = String::new();
    let mut last_was_separator = false;

    for character in value.trim().chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_was_separator = false;
        } else if matches!(character, '-' | '_' | ' ') && !slug.is_empty() && !last_was_separator {
            slug.push('-');
            last_was_separator = true;
        } else if !character.is_ascii() {
            anyhow::bail!("corpus slug must contain only ASCII letters, numbers, and separators");
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        anyhow::bail!("corpus slug must not be empty");
    }
    if slug.len() > 80 {
        anyhow::bail!("corpus slug must be at most 80 characters");
    }
    Ok(slug)
}

pub async fn list_corpora(pool: &PgPool) -> anyhow::Result<Vec<Corpus>> {
    let rows = sqlx::query(
        "SELECT c.id, c.slug, c.name, c.description, c.is_default, c.created_at, c.updated_at,
                (
                  SELECT count(*)::bigint
                  FROM (
                    SELECT vm.video_id
                    FROM corpus_video_memberships vm
                    WHERE vm.corpus_id = c.id
                    UNION
                    SELECT v.id
                    FROM corpus_source_memberships sm
                    JOIN corpus_subscription_items i ON i.subscription_id = sm.subscription_id
                    JOIN videos v
                      ON v.source_url = i.source_url
                      OR (v.youtube_id IS NOT NULL AND i.youtube_id = v.youtube_id)
                    WHERE sm.corpus_id = c.id
                  ) scoped_videos
                ) AS video_count,
                (
                  SELECT count(*)::bigint
                  FROM corpus_source_memberships sm
                  WHERE sm.corpus_id = c.id
                ) AS source_count
         FROM corpora c
         ORDER BY c.is_default DESC, lower(c.name), c.created_at",
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(corpus_from_row).collect()
}

pub async fn get_corpus(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<Corpus>> {
    let row = sqlx::query(
        "SELECT c.id, c.slug, c.name, c.description, c.is_default, c.created_at, c.updated_at,
                (
                  SELECT count(*)::bigint
                  FROM (
                    SELECT vm.video_id
                    FROM corpus_video_memberships vm
                    WHERE vm.corpus_id = c.id
                    UNION
                    SELECT v.id
                    FROM corpus_source_memberships sm
                    JOIN corpus_subscription_items i ON i.subscription_id = sm.subscription_id
                    JOIN videos v
                      ON v.source_url = i.source_url
                      OR (v.youtube_id IS NOT NULL AND i.youtube_id = v.youtube_id)
                    WHERE sm.corpus_id = c.id
                  ) scoped_videos
                ) AS video_count,
                (
                  SELECT count(*)::bigint
                  FROM corpus_source_memberships sm
                  WHERE sm.corpus_id = c.id
                ) AS source_count
         FROM corpora c
         WHERE c.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    row.map(corpus_from_row).transpose()
}

pub async fn create_corpus(pool: &PgPool, request: CreateCorpusRequest) -> anyhow::Result<Corpus> {
    let name = request.name.trim();
    if name.is_empty() {
        anyhow::bail!("corpus name is required");
    }
    let slug = normalize_slug(request.slug.as_deref().unwrap_or(name))?;
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("youtube-corpus:corpus:{slug}").as_bytes(),
    );
    let description = request
        .description
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    sqlx::query(
        "INSERT INTO corpora (id, slug, name, description)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(&slug)
    .bind(name)
    .bind(description)
    .execute(pool)
    .await?;

    get_corpus(pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("created corpus could not be loaded"))
}

pub async fn link_source(
    pool: &PgPool,
    corpus_id: Uuid,
    subscription_id: Uuid,
) -> anyhow::Result<()> {
    require_corpus(pool, corpus_id).await?;
    sqlx::query(
        "INSERT INTO corpus_source_memberships (corpus_id, subscription_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(corpus_id)
    .bind(subscription_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn link_video(pool: &PgPool, corpus_id: Uuid, video_id: Uuid) -> anyhow::Result<()> {
    require_corpus(pool, corpus_id).await?;
    sqlx::query(
        "INSERT INTO corpus_video_memberships (corpus_id, video_id, membership_kind)
         VALUES ($1, $2, 'direct')
         ON CONFLICT DO NOTHING",
    )
    .bind(corpus_id)
    .bind(video_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn source_belongs_to_corpus(
    pool: &PgPool,
    corpus_id: Uuid,
    subscription_id: Uuid,
) -> anyhow::Result<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
           SELECT 1
           FROM corpus_source_memberships
           WHERE corpus_id = $1 AND subscription_id = $2
         )",
    )
    .bind(corpus_id)
    .bind(subscription_id)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

pub async fn set_source_enabled(
    pool: &PgPool,
    corpus_id: Uuid,
    subscription_id: Uuid,
    enabled: bool,
) -> anyhow::Result<()> {
    if !source_belongs_to_corpus(pool, corpus_id, subscription_id).await? {
        anyhow::bail!("source is not part of this corpus");
    }
    sqlx::query(
        "UPDATE corpus_subscriptions
         SET enabled = $2, updated_at = now()
         WHERE id = $1",
    )
    .bind(subscription_id)
    .bind(enabled)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_check_outcome(
    pool: &PgPool,
    subscription_id: Uuid,
    status: &str,
    message: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE corpus_subscriptions
         SET last_check_status = $2, last_check_message = $3, updated_at = now()
         WHERE id = $1",
    )
    .bind(subscription_id)
    .bind(status)
    .bind(message)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_corpus_sources(
    pool: &PgPool,
    corpus_id: Uuid,
) -> anyhow::Result<Vec<CorpusSourceSummary>> {
    require_corpus(pool, corpus_id).await?;
    let rows = sqlx::query(
        "SELECT s.id, s.source_kind, s.source_url, s.name, s.enabled,
                s.last_checked_at, s.last_ingested_at, s.last_check_status, s.last_check_message,
                count(i.id)::bigint AS items_seen,
                count(i.id) FILTER (WHERE i.status = 'indexed')::bigint AS items_indexed,
                count(i.id) FILTER (WHERE i.status = 'failed')::bigint AS items_failed
         FROM corpus_source_memberships m
         JOIN corpus_subscriptions s ON s.id = m.subscription_id
         LEFT JOIN corpus_subscription_items i ON i.subscription_id = s.id
         WHERE m.corpus_id = $1
         GROUP BY s.id, s.source_kind, s.source_url, s.name, s.enabled,
                  s.last_checked_at, s.last_ingested_at, s.last_check_status, s.last_check_message,
                  m.added_at
         ORDER BY m.added_at, lower(coalesce(s.name, s.source_url))",
    )
    .bind(corpus_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(CorpusSourceSummary {
                id: row.try_get("id")?,
                source_kind: row.try_get("source_kind")?,
                source_url: row.try_get("source_url")?,
                name: row.try_get("name")?,
                enabled: row.try_get("enabled")?,
                last_checked_at: row.try_get("last_checked_at")?,
                last_ingested_at: row.try_get("last_ingested_at")?,
                last_check_status: row.try_get("last_check_status")?,
                last_check_message: row.try_get("last_check_message")?,
                items_seen: row_count(&row, "items_seen")?,
                items_indexed: row_count(&row, "items_indexed")?,
                items_failed: row_count(&row, "items_failed")?,
            })
        })
        .collect()
}

pub async fn list_corpus_videos(
    pool: &PgPool,
    corpus_id: Uuid,
    limit: i64,
) -> anyhow::Result<Vec<CorpusVideoSummary>> {
    require_corpus(pool, corpus_id).await?;
    let rows = sqlx::query(
        "WITH scoped_video_ids AS (
           SELECT vm.video_id
           FROM corpus_video_memberships vm
           WHERE vm.corpus_id = $1
           UNION
           SELECT v.id
           FROM corpus_source_memberships sm
           JOIN corpus_subscription_items i ON i.subscription_id = sm.subscription_id
           JOIN videos v
             ON v.source_url = i.source_url
             OR (v.youtube_id IS NOT NULL AND i.youtube_id = v.youtube_id)
           WHERE sm.corpus_id = $1
         )
         SELECT v.id, v.youtube_id, v.source_url, v.title, v.channel, v.thumbnail_url,
                v.duration_seconds, v.upload_date, v.updated_at,
                count(DISTINCT st.id)::bigint AS transcript_streams,
                count(DISTINCT ts.id)::bigint AS transcript_segments,
                coalesce(
                  jsonb_agg(DISTINCT st.source_kind) FILTER (WHERE st.source_kind IS NOT NULL),
                  '[]'::jsonb
                ) AS transcript_source_kinds,
                max(st.processing_revision)::bigint AS processing_revision,
                max(st.retrieved_at) AS last_retrieved_at
         FROM scoped_video_ids scoped
         JOIN videos v ON v.id = scoped.video_id
         LEFT JOIN transcript_streams st ON st.video_id = v.id
         LEFT JOIN transcript_segments ts ON ts.stream_id = st.id
         GROUP BY v.id, v.youtube_id, v.source_url, v.title, v.channel, v.thumbnail_url,
                  v.duration_seconds, v.upload_date, v.updated_at
         ORDER BY v.updated_at DESC, v.source_url
         LIMIT $2",
    )
    .bind(corpus_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let source_kinds: Json<Vec<String>> = row.try_get("transcript_source_kinds")?;
            let processing_revision: Option<i64> = row.try_get("processing_revision")?;
            Ok(CorpusVideoSummary {
                id: row.try_get("id")?,
                youtube_id: row.try_get("youtube_id")?,
                source_url: row.try_get("source_url")?,
                title: row.try_get("title")?,
                channel: row.try_get("channel")?,
                thumbnail_url: row.try_get("thumbnail_url")?,
                duration_seconds: row.try_get("duration_seconds")?,
                upload_date: row.try_get("upload_date")?,
                transcript_streams: row_count(&row, "transcript_streams")?,
                transcript_segments: row_count(&row, "transcript_segments")?,
                transcript_source_kinds: source_kinds.0,
                processing_revision: processing_revision.map(|value| value as u64),
                last_retrieved_at: row.try_get("last_retrieved_at")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .collect()
}

pub async fn corpus_video_ids(pool: &PgPool, corpus_id: Uuid) -> anyhow::Result<HashSet<Uuid>> {
    require_corpus(pool, corpus_id).await?;
    let ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT vm.video_id
         FROM corpus_video_memberships vm
         WHERE vm.corpus_id = $1
         UNION
         SELECT v.id
         FROM corpus_source_memberships sm
         JOIN corpus_subscription_items i ON i.subscription_id = sm.subscription_id
         JOIN videos v
           ON v.source_url = i.source_url
           OR (v.youtube_id IS NOT NULL AND i.youtube_id = v.youtube_id)
         WHERE sm.corpus_id = $1",
    )
    .bind(corpus_id)
    .fetch_all(pool)
    .await?;
    Ok(ids.into_iter().collect())
}

async fn require_corpus(pool: &PgPool, id: Uuid) -> anyhow::Result<()> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM corpora WHERE id = $1)")
            .bind(id)
            .fetch_one(pool)
            .await?;
    if !exists {
        anyhow::bail!("corpus not found");
    }
    Ok(())
}

fn corpus_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<Corpus> {
    Ok(Corpus {
        id: row.try_get("id")?,
        slug: row.try_get("slug")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        is_default: row.try_get("is_default")?,
        video_count: row_count(&row, "video_count")?,
        source_count: row_count(&row, "source_count")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn row_count(row: &sqlx::postgres::PgRow, name: &str) -> anyhow::Result<u64> {
    let value: i64 = row.try_get(name)?;
    Ok(value.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::normalize_slug;

    #[test]
    fn normalizes_shareable_corpus_slugs() {
        assert_eq!(
            normalize_slug(" Church History ").unwrap(),
            "church-history"
        );
        assert_eq!(
            normalize_slug("rust__media---tools").unwrap(),
            "rust-media-tools"
        );
    }

    #[test]
    fn rejects_non_ascii_corpus_slugs() {
        assert!(normalize_slug("thomístic").is_err());
    }
}
