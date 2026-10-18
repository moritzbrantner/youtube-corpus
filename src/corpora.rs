use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use sqlx::{PgConnection, PgPool, Row};
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
    /// `automatic` or `review_first` (Atlas 1.6).
    pub processing_policy: String,
    /// Discovered videos held for approval.
    pub items_awaiting_review: u64,
    /// User labels assigned to this tracked source (Atlas 1.3).
    pub labels: Vec<CorpusLabel>,
}

/// A user-defined label owned by one corpus. Distinct from YouTube categories/tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CorpusLabel {
    pub id: Uuid,
    pub name: String,
}

/// A tracked source of this corpus whose discovered items contain a video.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusVideoSource {
    pub id: Uuid,
    pub source_kind: String,
    pub name: Option<String>,
}

/// Why a label request failed; mapped to HTTP status codes by the web layer.
#[derive(Debug, thiserror::Error)]
pub enum LabelError {
    #[error("{0}")]
    NotFound(String),
    #[error("label name must not be blank")]
    BlankName,
    #[error("a label named \"{0}\" already exists in this corpus")]
    DuplicateName(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
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
    /// User labels assigned directly to the video in this corpus.
    pub labels: Vec<CorpusLabel>,
    /// Tracked sources of this corpus that discovered the video (Atlas 1.1).
    pub sources: Vec<CorpusVideoSource>,
    /// The video was added to the corpus directly rather than only through a source.
    pub added_directly: bool,
    /// YouTube-supplied categories, kept apart from user labels (Atlas 1.4).
    pub youtube_categories: Vec<String>,
    /// YouTube-supplied tags, kept apart from user labels (Atlas 1.4).
    pub youtube_tags: Vec<String>,
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
                count(i.id) FILTER (WHERE i.status = 'failed')::bigint AS items_failed,
                count(i.id) FILTER (WHERE i.status = 'awaiting_review')::bigint AS items_awaiting_review,
                s.processing_policy,
                coalesce(
                  (SELECT jsonb_agg(jsonb_build_object('id', l.id, 'name', l.name)
                                    ORDER BY lower(l.name), l.id)
                   FROM corpus_source_labels sl
                   JOIN corpus_labels l ON l.id = sl.label_id
                   WHERE sl.corpus_id = m.corpus_id AND sl.subscription_id = s.id),
                  '[]'::jsonb
                ) AS labels
         FROM corpus_source_memberships m
         JOIN corpus_subscriptions s ON s.id = m.subscription_id
         LEFT JOIN corpus_subscription_items i ON i.subscription_id = s.id
         WHERE m.corpus_id = $1
         GROUP BY s.id, s.source_kind, s.source_url, s.name, s.enabled, s.processing_policy,
                  s.last_checked_at, s.last_ingested_at, s.last_check_status, s.last_check_message,
                  m.added_at, m.corpus_id
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
                processing_policy: row.try_get("processing_policy")?,
                items_awaiting_review: row_count(&row, "items_awaiting_review")?,
                labels: row.try_get::<Json<Vec<CorpusLabel>>, _>("labels")?.0,
            })
        })
        .collect()
}

pub async fn list_corpus_videos(
    pool: &PgPool,
    corpus_id: Uuid,
    limit: i64,
) -> anyhow::Result<Vec<CorpusVideoSummary>> {
    list_corpus_videos_filtered(pool, corpus_id, limit, None).await
}

/// Lists the corpus catalog, optionally narrowed to one user label. A label matches videos
/// labeled directly and the videos of tracked sources carrying that label (Atlas 3.3).
pub async fn list_corpus_videos_filtered(
    pool: &PgPool,
    corpus_id: Uuid,
    limit: i64,
    label_id: Option<Uuid>,
) -> anyhow::Result<Vec<CorpusVideoSummary>> {
    require_corpus(pool, corpus_id).await?;
    if let Some(label_id) = label_id {
        require_label(pool, corpus_id, label_id).await?;
    }
    let rows = sqlx::query(
        "WITH source_video_ids AS (
           SELECT DISTINCT sm.subscription_id, v.id AS video_id
           FROM corpus_source_memberships sm
           JOIN corpus_subscription_items i ON i.subscription_id = sm.subscription_id
           JOIN videos v
             ON v.source_url = i.source_url
             OR (v.youtube_id IS NOT NULL AND i.youtube_id = v.youtube_id)
           WHERE sm.corpus_id = $1
         ),
         scoped_video_ids AS (
           SELECT vm.video_id
           FROM corpus_video_memberships vm
           WHERE vm.corpus_id = $1
           UNION
           SELECT video_id FROM source_video_ids
         ),
         filtered_video_ids AS (
           SELECT scoped.video_id
           FROM scoped_video_ids scoped
           WHERE $3::uuid IS NULL
              OR EXISTS (
                   SELECT 1 FROM corpus_video_labels vl
                   WHERE vl.corpus_id = $1 AND vl.label_id = $3 AND vl.video_id = scoped.video_id)
              OR EXISTS (
                   SELECT 1
                   FROM source_video_ids sv
                   JOIN corpus_source_labels sl
                     ON sl.corpus_id = $1 AND sl.subscription_id = sv.subscription_id
                   WHERE sl.label_id = $3 AND sv.video_id = scoped.video_id)
         )
         SELECT v.id, v.youtube_id, v.source_url, v.title, v.channel, v.thumbnail_url,
                v.duration_seconds, v.upload_date, v.updated_at, v.categories, v.tags,
                count(DISTINCT st.id)::bigint AS transcript_streams,
                count(DISTINCT ts.id)::bigint AS transcript_segments,
                coalesce(
                  jsonb_agg(DISTINCT st.source_kind) FILTER (WHERE st.source_kind IS NOT NULL),
                  '[]'::jsonb
                ) AS transcript_source_kinds,
                max(st.processing_revision)::bigint AS processing_revision,
                max(st.retrieved_at) AS last_retrieved_at,
                coalesce(
                  (SELECT jsonb_agg(jsonb_build_object('id', l.id, 'name', l.name)
                                    ORDER BY lower(l.name), l.id)
                   FROM corpus_video_labels vl
                   JOIN corpus_labels l ON l.id = vl.label_id
                   WHERE vl.corpus_id = $1 AND vl.video_id = v.id),
                  '[]'::jsonb
                ) AS labels,
                coalesce(
                  (SELECT jsonb_agg(jsonb_build_object(
                             'id', s.id, 'sourceKind', s.source_kind, 'name', s.name)
                           ORDER BY lower(coalesce(s.name, s.source_url)), s.id)
                   FROM source_video_ids sv
                   JOIN corpus_subscriptions s ON s.id = sv.subscription_id
                   WHERE sv.video_id = v.id),
                  '[]'::jsonb
                ) AS sources,
                EXISTS (
                  SELECT 1 FROM corpus_video_memberships vm
                  WHERE vm.corpus_id = $1 AND vm.video_id = v.id
                ) AS added_directly
         FROM filtered_video_ids scoped
         JOIN videos v ON v.id = scoped.video_id
         LEFT JOIN transcript_streams st ON st.video_id = v.id
         LEFT JOIN transcript_segments ts ON ts.stream_id = st.id
         GROUP BY v.id, v.youtube_id, v.source_url, v.title, v.channel, v.thumbnail_url,
                  v.duration_seconds, v.upload_date, v.updated_at, v.categories, v.tags
         ORDER BY v.updated_at DESC, v.source_url
         LIMIT $2",
    )
    .bind(corpus_id)
    .bind(limit.clamp(1, 500))
    .bind(label_id)
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
                labels: row.try_get::<Json<Vec<CorpusLabel>>, _>("labels")?.0,
                sources: row.try_get::<Json<Vec<CorpusVideoSource>>, _>("sources")?.0,
                added_directly: row.try_get("added_directly")?,
                youtube_categories: row.try_get("categories")?,
                youtube_tags: row.try_get("tags")?,
            })
        })
        .collect()
}

pub async fn list_labels(pool: &PgPool, corpus_id: Uuid) -> Result<Vec<CorpusLabel>, LabelError> {
    require_corpus_for_labels(pool, corpus_id).await?;
    Ok(sqlx::query_as::<_, CorpusLabel>(
        "SELECT id, name FROM corpus_labels WHERE corpus_id = $1 ORDER BY lower(name), id",
    )
    .bind(corpus_id)
    .fetch_all(pool)
    .await?)
}

pub async fn create_label(
    pool: &PgPool,
    corpus_id: Uuid,
    name: &str,
) -> Result<CorpusLabel, LabelError> {
    require_corpus_for_labels(pool, corpus_id).await?;
    let name = label_name(name)?;
    sqlx::query_as::<_, CorpusLabel>(
        "INSERT INTO corpus_labels (id, corpus_id, name) VALUES ($1, $2, $3) RETURNING id, name",
    )
    .bind(Uuid::new_v4())
    .bind(corpus_id)
    .bind(&name)
    .fetch_one(pool)
    .await
    .map_err(|error| duplicate_or(error, &name))
}

pub async fn rename_label(
    pool: &PgPool,
    corpus_id: Uuid,
    label_id: Uuid,
    name: &str,
) -> Result<CorpusLabel, LabelError> {
    require_corpus_for_labels(pool, corpus_id).await?;
    require_label(pool, corpus_id, label_id).await?;
    let name = label_name(name)?;
    sqlx::query_as::<_, CorpusLabel>(
        "UPDATE corpus_labels SET name = $3, updated_at = now()
         WHERE corpus_id = $1 AND id = $2
         RETURNING id, name",
    )
    .bind(corpus_id)
    .bind(label_id)
    .bind(&name)
    .fetch_optional(pool)
    .await
    .map_err(|error| duplicate_or(error, &name))?
    .ok_or_else(label_not_found)
}

/// Deletes a label and its assignments; videos, sources and evidence are untouched.
pub async fn delete_label(
    pool: &PgPool,
    corpus_id: Uuid,
    label_id: Uuid,
) -> Result<(), LabelError> {
    require_corpus_for_labels(pool, corpus_id).await?;
    let deleted = sqlx::query("DELETE FROM corpus_labels WHERE corpus_id = $1 AND id = $2")
        .bind(corpus_id)
        .bind(label_id)
        .execute(pool)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(label_not_found());
    }
    Ok(())
}

/// Assigns (idempotently) or removes a label on a tracked source of the corpus.
pub async fn set_source_label(
    pool: &PgPool,
    corpus_id: Uuid,
    source_id: Uuid,
    label_id: Uuid,
    assigned: bool,
) -> Result<(), LabelError> {
    require_corpus_for_labels(pool, corpus_id).await?;
    require_label(pool, corpus_id, label_id).await?;
    let belongs = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM corpus_source_memberships
                       WHERE corpus_id = $1 AND subscription_id = $2)",
    )
    .bind(corpus_id)
    .bind(source_id)
    .fetch_one(pool)
    .await?;
    if !belongs {
        return Err(LabelError::NotFound(
            "source is not part of this corpus".to_string(),
        ));
    }
    let query = if assigned {
        "INSERT INTO corpus_source_labels (corpus_id, label_id, subscription_id)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"
    } else {
        "DELETE FROM corpus_source_labels
         WHERE corpus_id = $1 AND label_id = $2 AND subscription_id = $3"
    };
    sqlx::query(query)
        .bind(corpus_id)
        .bind(label_id)
        .bind(source_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Assigns (idempotently) or removes a label on a video of the corpus.
pub async fn set_video_label(
    pool: &PgPool,
    corpus_id: Uuid,
    video_id: Uuid,
    label_id: Uuid,
    assigned: bool,
) -> Result<(), LabelError> {
    require_corpus_for_labels(pool, corpus_id).await?;
    require_label(pool, corpus_id, label_id).await?;
    let in_corpus = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
           SELECT 1 FROM corpus_video_memberships vm
           WHERE vm.corpus_id = $1 AND vm.video_id = $2
           UNION ALL
           SELECT 1
           FROM corpus_source_memberships sm
           JOIN corpus_subscription_items i ON i.subscription_id = sm.subscription_id
           JOIN videos v
             ON v.source_url = i.source_url
             OR (v.youtube_id IS NOT NULL AND i.youtube_id = v.youtube_id)
           WHERE sm.corpus_id = $1 AND v.id = $2
         )",
    )
    .bind(corpus_id)
    .bind(video_id)
    .fetch_one(pool)
    .await?;
    if !in_corpus {
        return Err(LabelError::NotFound(
            "video is not part of this corpus".to_string(),
        ));
    }
    let query = if assigned {
        "INSERT INTO corpus_video_labels (corpus_id, label_id, video_id)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"
    } else {
        "DELETE FROM corpus_video_labels WHERE corpus_id = $1 AND label_id = $2 AND video_id = $3"
    };
    sqlx::query(query)
        .bind(corpus_id)
        .bind(label_id)
        .bind(video_id)
        .execute(pool)
        .await?;
    Ok(())
}

fn label_name(name: &str) -> Result<String, LabelError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(LabelError::BlankName);
    }
    Ok(name.to_string())
}

fn label_not_found() -> LabelError {
    LabelError::NotFound("label not found in this corpus".to_string())
}

fn duplicate_or(error: sqlx::Error, name: &str) -> LabelError {
    match &error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            LabelError::DuplicateName(name.to_string())
        }
        _ => LabelError::Database(error),
    }
}

async fn require_corpus_for_labels(pool: &PgPool, corpus_id: Uuid) -> Result<(), LabelError> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM corpora WHERE id = $1)")
            .bind(corpus_id)
            .fetch_one(pool)
            .await?;
    if !exists {
        return Err(LabelError::NotFound("corpus not found".to_string()));
    }
    Ok(())
}

async fn require_label(pool: &PgPool, corpus_id: Uuid, label_id: Uuid) -> Result<(), LabelError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM corpus_labels WHERE corpus_id = $1 AND id = $2)",
    )
    .bind(corpus_id)
    .bind(label_id)
    .fetch_one(pool)
    .await?;
    if !exists {
        return Err(label_not_found());
    }
    Ok(())
}

pub async fn corpus_video_ids(pool: &PgPool, corpus_id: Uuid) -> anyhow::Result<HashSet<Uuid>> {
    let mut connection = pool.acquire().await?;
    corpus_video_ids_with(&mut connection, corpus_id).await
}

pub(crate) async fn corpus_video_ids_with(
    connection: &mut PgConnection,
    corpus_id: Uuid,
) -> anyhow::Result<HashSet<Uuid>> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM corpora WHERE id=$1)")
        .bind(corpus_id)
        .fetch_one(&mut *connection)
        .await?;
    anyhow::ensure!(exists, "corpus not found");
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
    .fetch_all(&mut *connection)
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
