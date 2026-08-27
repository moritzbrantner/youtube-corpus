use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationSourceKind {
    User,
    Processor,
}

impl AnnotationSourceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Processor => "processor",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchAnnotation {
    pub id: Uuid,
    pub corpus_id: Uuid,
    pub video_id: Uuid,
    pub stream_id: Option<Uuid>,
    pub segment_id: Option<Uuid>,
    pub kind: String,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub label: Option<String>,
    pub text: Option<String>,
    pub payload: Value,
    pub source_kind: AnnotationSourceKind,
    pub processor: Option<String>,
    pub processor_version: Option<String>,
    pub processing_config: Value,
    pub revision: u64,
    pub content_checksum: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateAnnotationRequest {
    pub corpus_id: Uuid,
    pub video_id: Uuid,
    pub stream_id: Option<Uuid>,
    pub segment_id: Option<Uuid>,
    pub kind: String,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub label: Option<String>,
    pub text: Option<String>,
    pub payload: Value,
    pub source_kind: AnnotationSourceKind,
    pub processor: Option<String>,
    pub processor_version: Option<String>,
    pub processing_config: Value,
}

pub async fn create_annotation(
    pool: &PgPool,
    request: CreateAnnotationRequest,
) -> anyhow::Result<ResearchAnnotation> {
    let allowed = crate::corpora::corpus_video_ids(pool, request.corpus_id).await?;
    if !allowed.contains(&request.video_id) {
        anyhow::bail!("video is not part of this corpus");
    }
    validate_kind(&request.kind)?;
    validate_range(request.start_seconds, request.end_seconds)?;

    if let Some(stream_id) = request.stream_id {
        let belongs = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
               SELECT 1 FROM transcript_streams
               WHERE id = $1 AND video_id = $2
             )",
        )
        .bind(stream_id)
        .bind(request.video_id)
        .fetch_one(pool)
        .await?;
        if !belongs {
            anyhow::bail!("stream does not belong to the annotated video");
        }
    }

    if let Some(segment_id) = request.segment_id {
        let belongs = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
               SELECT 1 FROM transcript_segments
               WHERE id = $1 AND video_id = $2
             )",
        )
        .bind(segment_id)
        .bind(request.video_id)
        .fetch_one(pool)
        .await?;
        if !belongs {
            anyhow::bail!("segment does not belong to the annotated video");
        }
    }

    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO research_annotations (
           id, corpus_id, video_id, stream_id, segment_id, kind,
           start_seconds, end_seconds, label, text, payload, source_kind,
           processor, processor_version, processing_config
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
         RETURNING id, corpus_id, video_id, stream_id, segment_id, kind,
                   start_seconds, end_seconds, label, text, payload, source_kind,
                   processor, processor_version, processing_config, revision,
                   content_checksum, created_at, updated_at",
    )
    .bind(id)
    .bind(request.corpus_id)
    .bind(request.video_id)
    .bind(request.stream_id)
    .bind(request.segment_id)
    .bind(request.kind)
    .bind(request.start_seconds)
    .bind(request.end_seconds)
    .bind(normalize_optional(request.label))
    .bind(normalize_optional(request.text))
    .bind(request.payload)
    .bind(request.source_kind.as_str())
    .bind(normalize_optional(request.processor))
    .bind(normalize_optional(request.processor_version))
    .bind(request.processing_config)
    .fetch_one(pool)
    .await?;
    annotation_from_row(row)
}

pub async fn list_annotations(
    pool: &PgPool,
    corpus_id: Uuid,
    kind: Option<&str>,
    video_id: Option<Uuid>,
    limit: i64,
) -> anyhow::Result<Vec<ResearchAnnotation>> {
    if crate::corpora::get_corpus(pool, corpus_id).await?.is_none() {
        anyhow::bail!("corpus not found");
    }
    let rows = sqlx::query(
        "SELECT id, corpus_id, video_id, stream_id, segment_id, kind,
                start_seconds, end_seconds, label, text, payload, source_kind,
                processor, processor_version, processing_config, revision,
                content_checksum, created_at, updated_at
         FROM research_annotations
         WHERE corpus_id = $1
           AND ($2::text IS NULL OR kind = $2)
           AND ($3::uuid IS NULL OR video_id = $3)
         ORDER BY created_at DESC, id
         LIMIT $4",
    )
    .bind(corpus_id)
    .bind(kind)
    .bind(video_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(annotation_from_row).collect()
}

fn annotation_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<ResearchAnnotation> {
    let source_kind: String = row.try_get("source_kind")?;
    let revision: i64 = row.try_get("revision")?;
    Ok(ResearchAnnotation {
        id: row.try_get("id")?,
        corpus_id: row.try_get("corpus_id")?,
        video_id: row.try_get("video_id")?,
        stream_id: row.try_get("stream_id")?,
        segment_id: row.try_get("segment_id")?,
        kind: row.try_get("kind")?,
        start_seconds: row.try_get("start_seconds")?,
        end_seconds: row.try_get("end_seconds")?,
        label: row.try_get("label")?,
        text: row.try_get("text")?,
        payload: row.try_get("payload")?,
        source_kind: match source_kind.as_str() {
            "processor" => AnnotationSourceKind::Processor,
            _ => AnnotationSourceKind::User,
        },
        processor: row.try_get("processor")?,
        processor_version: row.try_get("processor_version")?,
        processing_config: row.try_get("processing_config")?,
        revision: revision.max(0) as u64,
        content_checksum: row.try_get("content_checksum")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn validate_kind(kind: &str) -> anyhow::Result<()> {
    let kind = kind.trim();
    if kind.is_empty() || kind.len() > 64 {
        anyhow::bail!("annotation kind must be 1-64 characters");
    }
    let mut characters = kind.chars();
    let Some(first) = characters.next() else {
        anyhow::bail!("annotation kind is required");
    };
    if !first.is_ascii_lowercase()
        || !characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-')
        })
    {
        anyhow::bail!("annotation kind must be a lowercase slug");
    }
    Ok(())
}

fn validate_range(start: Option<f64>, end: Option<f64>) -> anyhow::Result<()> {
    if start.is_some_and(|value| !value.is_finite() || value < 0.0)
        || end.is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        anyhow::bail!("annotation timestamps must be finite and non-negative");
    }
    if let (Some(start), Some(end)) = (start, end) {
        if end < start {
            anyhow::bail!("annotation end must not precede start");
        }
    }
    Ok(())
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{validate_kind, validate_range};

    #[test]
    fn annotation_kinds_are_open_but_slug_shaped() {
        assert!(validate_kind("claim").is_ok());
        assert!(validate_kind("speaker_turn").is_ok());
        assert!(validate_kind("visual-event").is_ok());
        assert!(validate_kind("Speaker Turn").is_err());
    }

    #[test]
    fn annotation_ranges_reject_reversed_timestamps() {
        assert!(validate_range(Some(2.0), Some(3.0)).is_ok());
        assert!(validate_range(Some(3.0), Some(2.0)).is_err());
    }
}
