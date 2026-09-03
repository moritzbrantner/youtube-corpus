use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaModality {
    Face,
    Voice,
}

impl MediaModality {
    fn as_str(self) -> &'static str {
        match self {
            Self::Face => "face",
            Self::Voice => "voice",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingProvenance {
    pub processor: String,
    pub processor_version: String,
    pub model: String,
    pub model_version: String,
    pub input_hash: String,
    pub config_hash: String,
    pub processing_config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaProcessingRun {
    pub id: Uuid,
    pub video_id: Uuid,
    pub modality: MediaModality,
    pub provenance: ProcessingProvenance,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct BeginProcessingRunRequest {
    pub video_id: Uuid,
    pub modality: MediaModality,
    pub provenance: ProcessingProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceObservation {
    pub id: Uuid,
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub observation_key: String,
    pub start_seconds: f64,
    pub end_seconds: Option<f64>,
    pub frame_index: Option<i64>,
    pub region: BoundingBox,
    pub detection_score: Option<f64>,
    pub embedding_dimensions: Option<u32>,
    pub quality: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertFaceObservationRequest {
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub observation_key: String,
    pub start_seconds: f64,
    pub end_seconds: Option<f64>,
    pub frame_index: Option<i64>,
    pub region: BoundingBox,
    pub detection_score: Option<f64>,
    pub embedding: Option<Vec<f32>>,
    pub quality: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceTrack {
    pub id: Uuid,
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub track_key: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub embedding_dimensions: Option<u32>,
    pub quality: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertFaceTrackRequest {
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub track_key: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub representative_embedding: Option<Vec<f32>>,
    pub quality: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceObservation {
    pub id: Uuid,
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub observation_key: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub confidence: Option<f64>,
    pub transcript_segment_id: Option<Uuid>,
    pub embedding_dimensions: Option<u32>,
    pub quality: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertVoiceObservationRequest {
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub observation_key: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub confidence: Option<f64>,
    pub transcript_segment_id: Option<Uuid>,
    pub embedding: Option<Vec<f32>>,
    pub quality: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTrack {
    pub id: Uuid,
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub track_key: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub embedding_dimensions: Option<u32>,
    pub quality: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertVoiceTrackRequest {
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub track_key: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub representative_embedding: Option<Vec<f32>>,
    pub quality: Value,
}

pub async fn begin_processing_run(
    pool: &PgPool,
    request: BeginProcessingRunRequest,
) -> anyhow::Result<MediaProcessingRun> {
    validate_provenance(&request.provenance)?;
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO media_processing_runs (
           id, video_id, modality, processor, processor_version, model, model_version,
           input_hash, config_hash, processing_config
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (
           video_id, modality, processor, processor_version, model, model_version,
           input_hash, config_hash
         ) DO UPDATE SET
           processing_config = EXCLUDED.processing_config,
           updated_at = now()
         RETURNING id, video_id, modality, processor, processor_version, model, model_version,
                   input_hash, config_hash, processing_config, created_at, updated_at",
    )
    .bind(id)
    .bind(request.video_id)
    .bind(request.modality.as_str())
    .bind(request.provenance.processor)
    .bind(request.provenance.processor_version)
    .bind(request.provenance.model)
    .bind(request.provenance.model_version)
    .bind(request.provenance.input_hash)
    .bind(request.provenance.config_hash)
    .bind(request.provenance.processing_config)
    .fetch_one(pool)
    .await?;
    processing_run_from_row(row)
}

pub async fn upsert_face_observation(
    pool: &PgPool,
    request: UpsertFaceObservationRequest,
) -> anyhow::Result<FaceObservation> {
    validate_run(pool, request.run_id, request.video_id, MediaModality::Face).await?;
    validate_key("observation key", &request.observation_key)?;
    validate_optional_range(request.start_seconds, request.end_seconds)?;
    validate_region(request.region)?;
    validate_probability("detection score", request.detection_score)?;
    if request.frame_index.is_some_and(|value| value < 0) {
        anyhow::bail!("frame index must be non-negative");
    }
    let (embedding, embedding_dimensions) = prepare_embedding(request.embedding.as_deref())?;
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO face_observations (
           id, run_id, video_id, observation_key, start_seconds, end_seconds, frame_index,
           bbox_x, bbox_y, bbox_width, bbox_height, detection_score,
           embedding, embedding_dimensions, quality
         )
         VALUES (
           $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
           CAST(CAST($13 AS text) AS vector), $14, $15
         )
         ON CONFLICT (run_id, observation_key) DO UPDATE SET
           start_seconds = EXCLUDED.start_seconds,
           end_seconds = EXCLUDED.end_seconds,
           frame_index = EXCLUDED.frame_index,
           bbox_x = EXCLUDED.bbox_x,
           bbox_y = EXCLUDED.bbox_y,
           bbox_width = EXCLUDED.bbox_width,
           bbox_height = EXCLUDED.bbox_height,
           detection_score = EXCLUDED.detection_score,
           embedding = EXCLUDED.embedding,
           embedding_dimensions = EXCLUDED.embedding_dimensions,
           quality = EXCLUDED.quality,
           updated_at = now()
         RETURNING id, run_id, video_id, observation_key, start_seconds, end_seconds, frame_index,
                   bbox_x, bbox_y, bbox_width, bbox_height, detection_score,
                   embedding_dimensions, quality, created_at, updated_at",
    )
    .bind(id)
    .bind(request.run_id)
    .bind(request.video_id)
    .bind(request.observation_key)
    .bind(request.start_seconds)
    .bind(request.end_seconds)
    .bind(request.frame_index)
    .bind(request.region.x)
    .bind(request.region.y)
    .bind(request.region.width)
    .bind(request.region.height)
    .bind(request.detection_score)
    .bind(embedding)
    .bind(embedding_dimensions)
    .bind(request.quality)
    .fetch_one(pool)
    .await?;
    face_observation_from_row(row)
}

pub async fn upsert_face_track(
    pool: &PgPool,
    request: UpsertFaceTrackRequest,
) -> anyhow::Result<FaceTrack> {
    validate_run(pool, request.run_id, request.video_id, MediaModality::Face).await?;
    validate_key("track key", &request.track_key)?;
    validate_range(request.start_seconds, request.end_seconds)?;
    let (embedding, embedding_dimensions) =
        prepare_embedding(request.representative_embedding.as_deref())?;
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO face_tracks (
           id, run_id, video_id, track_key, start_seconds, end_seconds,
           representative_embedding, embedding_dimensions, quality
         )
         VALUES ($1, $2, $3, $4, $5, $6, CAST(CAST($7 AS text) AS vector), $8, $9)
         ON CONFLICT (run_id, track_key) DO UPDATE SET
           start_seconds = EXCLUDED.start_seconds,
           end_seconds = EXCLUDED.end_seconds,
           representative_embedding = EXCLUDED.representative_embedding,
           embedding_dimensions = EXCLUDED.embedding_dimensions,
           quality = EXCLUDED.quality,
           updated_at = now()
         RETURNING id, run_id, video_id, track_key, start_seconds, end_seconds,
                   embedding_dimensions, quality, created_at, updated_at",
    )
    .bind(id)
    .bind(request.run_id)
    .bind(request.video_id)
    .bind(request.track_key)
    .bind(request.start_seconds)
    .bind(request.end_seconds)
    .bind(embedding)
    .bind(embedding_dimensions)
    .bind(request.quality)
    .fetch_one(pool)
    .await?;
    track_from_row(row, MediaModality::Face).map(|track| FaceTrack {
        id: track.id,
        run_id: track.run_id,
        video_id: track.video_id,
        track_key: track.track_key,
        start_seconds: track.start_seconds,
        end_seconds: track.end_seconds,
        embedding_dimensions: track.embedding_dimensions,
        quality: track.quality,
        created_at: track.created_at,
        updated_at: track.updated_at,
    })
}

pub async fn link_face_observation_to_track(
    pool: &PgPool,
    track_id: Uuid,
    observation_id: Uuid,
) -> anyhow::Result<()> {
    let result = sqlx::query(
        "INSERT INTO face_track_observations (track_id, observation_id)
         SELECT t.id, o.id
         FROM face_tracks t
         JOIN face_observations o ON o.run_id = t.run_id AND o.video_id = t.video_id
         WHERE t.id = $1 AND o.id = $2
         ON CONFLICT DO NOTHING",
    )
    .bind(track_id)
    .bind(observation_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(
               SELECT 1 FROM face_track_observations
               WHERE track_id = $1 AND observation_id = $2
             )",
        )
        .bind(track_id)
        .bind(observation_id)
        .fetch_one(pool)
        .await?;
        if !exists {
            anyhow::bail!("face track and observation must belong to the same processing run");
        }
    }
    Ok(())
}

pub async fn upsert_voice_observation(
    pool: &PgPool,
    request: UpsertVoiceObservationRequest,
) -> anyhow::Result<VoiceObservation> {
    validate_run(pool, request.run_id, request.video_id, MediaModality::Voice).await?;
    validate_key("observation key", &request.observation_key)?;
    validate_range(request.start_seconds, request.end_seconds)?;
    validate_probability("confidence", request.confidence)?;
    validate_transcript_segment(pool, request.video_id, request.transcript_segment_id).await?;
    let (embedding, embedding_dimensions) = prepare_embedding(request.embedding.as_deref())?;
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO voice_observations (
           id, run_id, video_id, observation_key, start_seconds, end_seconds, confidence,
           transcript_segment_id, embedding, embedding_dimensions, quality
         )
         VALUES (
           $1, $2, $3, $4, $5, $6, $7, $8,
           CAST(CAST($9 AS text) AS vector), $10, $11
         )
         ON CONFLICT (run_id, observation_key) DO UPDATE SET
           start_seconds = EXCLUDED.start_seconds,
           end_seconds = EXCLUDED.end_seconds,
           confidence = EXCLUDED.confidence,
           transcript_segment_id = EXCLUDED.transcript_segment_id,
           embedding = EXCLUDED.embedding,
           embedding_dimensions = EXCLUDED.embedding_dimensions,
           quality = EXCLUDED.quality,
           updated_at = now()
         RETURNING id, run_id, video_id, observation_key, start_seconds, end_seconds,
                   confidence, transcript_segment_id, embedding_dimensions, quality,
                   created_at, updated_at",
    )
    .bind(id)
    .bind(request.run_id)
    .bind(request.video_id)
    .bind(request.observation_key)
    .bind(request.start_seconds)
    .bind(request.end_seconds)
    .bind(request.confidence)
    .bind(request.transcript_segment_id)
    .bind(embedding)
    .bind(embedding_dimensions)
    .bind(request.quality)
    .fetch_one(pool)
    .await?;
    voice_observation_from_row(row)
}

pub async fn upsert_voice_track(
    pool: &PgPool,
    request: UpsertVoiceTrackRequest,
) -> anyhow::Result<VoiceTrack> {
    validate_run(pool, request.run_id, request.video_id, MediaModality::Voice).await?;
    validate_key("track key", &request.track_key)?;
    validate_range(request.start_seconds, request.end_seconds)?;
    let (embedding, embedding_dimensions) =
        prepare_embedding(request.representative_embedding.as_deref())?;
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO voice_tracks (
           id, run_id, video_id, track_key, start_seconds, end_seconds,
           representative_embedding, embedding_dimensions, quality
         )
         VALUES ($1, $2, $3, $4, $5, $6, CAST(CAST($7 AS text) AS vector), $8, $9)
         ON CONFLICT (run_id, track_key) DO UPDATE SET
           start_seconds = EXCLUDED.start_seconds,
           end_seconds = EXCLUDED.end_seconds,
           representative_embedding = EXCLUDED.representative_embedding,
           embedding_dimensions = EXCLUDED.embedding_dimensions,
           quality = EXCLUDED.quality,
           updated_at = now()
         RETURNING id, run_id, video_id, track_key, start_seconds, end_seconds,
                   embedding_dimensions, quality, created_at, updated_at",
    )
    .bind(id)
    .bind(request.run_id)
    .bind(request.video_id)
    .bind(request.track_key)
    .bind(request.start_seconds)
    .bind(request.end_seconds)
    .bind(embedding)
    .bind(embedding_dimensions)
    .bind(request.quality)
    .fetch_one(pool)
    .await?;
    track_from_row(row, MediaModality::Voice).map(|track| VoiceTrack {
        id: track.id,
        run_id: track.run_id,
        video_id: track.video_id,
        track_key: track.track_key,
        start_seconds: track.start_seconds,
        end_seconds: track.end_seconds,
        embedding_dimensions: track.embedding_dimensions,
        quality: track.quality,
        created_at: track.created_at,
        updated_at: track.updated_at,
    })
}

pub async fn link_voice_observation_to_track(
    pool: &PgPool,
    track_id: Uuid,
    observation_id: Uuid,
) -> anyhow::Result<()> {
    let result = sqlx::query(
        "INSERT INTO voice_track_observations (track_id, observation_id)
         SELECT t.id, o.id
         FROM voice_tracks t
         JOIN voice_observations o ON o.run_id = t.run_id AND o.video_id = t.video_id
         WHERE t.id = $1 AND o.id = $2
         ON CONFLICT DO NOTHING",
    )
    .bind(track_id)
    .bind(observation_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(
               SELECT 1 FROM voice_track_observations
               WHERE track_id = $1 AND observation_id = $2
             )",
        )
        .bind(track_id)
        .bind(observation_id)
        .fetch_one(pool)
        .await?;
        if !exists {
            anyhow::bail!("voice track and observation must belong to the same processing run");
        }
    }
    Ok(())
}

pub async fn list_face_observations(
    pool: &PgPool,
    video_id: Uuid,
) -> anyhow::Result<Vec<FaceObservation>> {
    let rows = sqlx::query(
        "SELECT id, run_id, video_id, observation_key, start_seconds, end_seconds, frame_index,
                bbox_x, bbox_y, bbox_width, bbox_height, detection_score,
                embedding_dimensions, quality, created_at, updated_at
         FROM face_observations
         WHERE video_id = $1
         ORDER BY start_seconds, frame_index NULLS LAST, id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(face_observation_from_row).collect()
}

pub async fn list_voice_observations(
    pool: &PgPool,
    video_id: Uuid,
) -> anyhow::Result<Vec<VoiceObservation>> {
    let rows = sqlx::query(
        "SELECT id, run_id, video_id, observation_key, start_seconds, end_seconds,
                confidence, transcript_segment_id, embedding_dimensions, quality,
                created_at, updated_at
         FROM voice_observations
         WHERE video_id = $1
         ORDER BY start_seconds, end_seconds, id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(voice_observation_from_row).collect()
}

async fn validate_run(
    pool: &PgPool,
    run_id: Uuid,
    video_id: Uuid,
    expected_modality: MediaModality,
) -> anyhow::Result<()> {
    let row = sqlx::query("SELECT video_id, modality FROM media_processing_runs WHERE id = $1")
        .bind(run_id)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        anyhow::bail!("processing run not found");
    };
    let actual_video_id: Uuid = row.try_get("video_id")?;
    let modality: String = row.try_get("modality")?;
    if actual_video_id != video_id {
        anyhow::bail!("processing run belongs to a different video");
    }
    if modality != expected_modality.as_str() {
        anyhow::bail!("processing run has the wrong modality");
    }
    Ok(())
}

async fn validate_transcript_segment(
    pool: &PgPool,
    video_id: Uuid,
    segment_id: Option<Uuid>,
) -> anyhow::Result<()> {
    let Some(segment_id) = segment_id else {
        return Ok(());
    };
    let belongs = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
           SELECT 1 FROM transcript_segments
           WHERE id = $1 AND video_id = $2
         )",
    )
    .bind(segment_id)
    .bind(video_id)
    .fetch_one(pool)
    .await?;
    if !belongs {
        anyhow::bail!("transcript segment does not belong to the voice observation video");
    }
    Ok(())
}

fn processing_run_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<MediaProcessingRun> {
    let modality: String = row.try_get("modality")?;
    Ok(MediaProcessingRun {
        id: row.try_get("id")?,
        video_id: row.try_get("video_id")?,
        modality: match modality.as_str() {
            "face" => MediaModality::Face,
            "voice" => MediaModality::Voice,
            value => anyhow::bail!("unsupported media modality {value}"),
        },
        provenance: ProcessingProvenance {
            processor: row.try_get("processor")?,
            processor_version: row.try_get("processor_version")?,
            model: row.try_get("model")?,
            model_version: row.try_get("model_version")?,
            input_hash: row.try_get("input_hash")?,
            config_hash: row.try_get("config_hash")?,
            processing_config: row.try_get("processing_config")?,
        },
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn face_observation_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<FaceObservation> {
    let embedding_dimensions: Option<i32> = row.try_get("embedding_dimensions")?;
    Ok(FaceObservation {
        id: row.try_get("id")?,
        run_id: row.try_get("run_id")?,
        video_id: row.try_get("video_id")?,
        observation_key: row.try_get("observation_key")?,
        start_seconds: row.try_get("start_seconds")?,
        end_seconds: row.try_get("end_seconds")?,
        frame_index: row.try_get("frame_index")?,
        region: BoundingBox {
            x: row.try_get("bbox_x")?,
            y: row.try_get("bbox_y")?,
            width: row.try_get("bbox_width")?,
            height: row.try_get("bbox_height")?,
        },
        detection_score: row.try_get("detection_score")?,
        embedding_dimensions: embedding_dimensions.map(|value| value.max(0) as u32),
        quality: row.try_get("quality")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn voice_observation_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<VoiceObservation> {
    let embedding_dimensions: Option<i32> = row.try_get("embedding_dimensions")?;
    Ok(VoiceObservation {
        id: row.try_get("id")?,
        run_id: row.try_get("run_id")?,
        video_id: row.try_get("video_id")?,
        observation_key: row.try_get("observation_key")?,
        start_seconds: row.try_get("start_seconds")?,
        end_seconds: row.try_get("end_seconds")?,
        confidence: row.try_get("confidence")?,
        transcript_segment_id: row.try_get("transcript_segment_id")?,
        embedding_dimensions: embedding_dimensions.map(|value| value.max(0) as u32),
        quality: row.try_get("quality")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

struct TrackRow {
    id: Uuid,
    run_id: Uuid,
    video_id: Uuid,
    track_key: String,
    start_seconds: f64,
    end_seconds: f64,
    embedding_dimensions: Option<u32>,
    quality: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn track_from_row(
    row: sqlx::postgres::PgRow,
    _modality: MediaModality,
) -> anyhow::Result<TrackRow> {
    let embedding_dimensions: Option<i32> = row.try_get("embedding_dimensions")?;
    Ok(TrackRow {
        id: row.try_get("id")?,
        run_id: row.try_get("run_id")?,
        video_id: row.try_get("video_id")?,
        track_key: row.try_get("track_key")?,
        start_seconds: row.try_get("start_seconds")?,
        end_seconds: row.try_get("end_seconds")?,
        embedding_dimensions: embedding_dimensions.map(|value| value.max(0) as u32),
        quality: row.try_get("quality")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn validate_provenance(provenance: &ProcessingProvenance) -> anyhow::Result<()> {
    for (name, value) in [
        ("processor", provenance.processor.as_str()),
        ("processor version", provenance.processor_version.as_str()),
        ("model", provenance.model.as_str()),
        ("model version", provenance.model_version.as_str()),
        ("input hash", provenance.input_hash.as_str()),
        ("config hash", provenance.config_hash.as_str()),
    ] {
        validate_key(name, value)?;
    }
    Ok(())
}

fn validate_key(name: &str, value: &str) -> anyhow::Result<()> {
    let value = value.trim();
    if value.is_empty() || value.len() > 512 {
        anyhow::bail!("{name} must be 1-512 characters");
    }
    Ok(())
}

fn validate_optional_range(start: f64, end: Option<f64>) -> anyhow::Result<()> {
    if !start.is_finite() || start < 0.0 {
        anyhow::bail!("start timestamp must be finite and non-negative");
    }
    if let Some(end) = end {
        validate_range(start, end)?;
    }
    Ok(())
}

fn validate_range(start: f64, end: f64) -> anyhow::Result<()> {
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end < start {
        anyhow::bail!("timestamps must be finite, non-negative, and ordered");
    }
    Ok(())
}

fn validate_region(region: BoundingBox) -> anyhow::Result<()> {
    if !region.x.is_finite()
        || !region.y.is_finite()
        || !region.width.is_finite()
        || !region.height.is_finite()
        || region.x < 0.0
        || region.y < 0.0
        || region.width <= 0.0
        || region.height <= 0.0
    {
        anyhow::bail!(
            "face bounding box must contain finite non-negative coordinates and positive size"
        );
    }
    Ok(())
}

fn validate_probability(name: &str, value: Option<f64>) -> anyhow::Result<()> {
    if value.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
        anyhow::bail!("{name} must be between 0 and 1");
    }
    Ok(())
}

fn prepare_embedding(embedding: Option<&[f32]>) -> anyhow::Result<(Option<String>, Option<i32>)> {
    let Some(embedding) = embedding else {
        return Ok((None, None));
    };
    if embedding.is_empty() {
        anyhow::bail!("embedding must not be empty");
    }
    if embedding.len() > i32::MAX as usize {
        anyhow::bail!("embedding is too large");
    }
    if embedding.iter().any(|value| !value.is_finite()) {
        anyhow::bail!("embedding values must be finite");
    }
    let literal = format!(
        "[{}]",
        embedding
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    Ok((Some(literal), Some(embedding.len() as i32)))
}

#[cfg(test)]
mod tests {
    use super::{prepare_embedding, validate_probability, validate_region, BoundingBox};

    #[test]
    fn embeddings_are_dimension_neutral_and_reject_non_finite_values() {
        let (literal, dimensions) = prepare_embedding(Some(&[0.25, -0.5, 1.0])).unwrap();
        assert_eq!(literal.as_deref(), Some("[0.25,-0.5,1]"));
        assert_eq!(dimensions, Some(3));
        assert!(prepare_embedding(Some(&[])).is_err());
        assert!(prepare_embedding(Some(&[f32::NAN])).is_err());
    }

    #[test]
    fn face_regions_require_positive_finite_geometry() {
        assert!(validate_region(BoundingBox {
            x: 1.0,
            y: 2.0,
            width: 20.0,
            height: 30.0,
        })
        .is_ok());
        assert!(validate_region(BoundingBox {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 1.0,
        })
        .is_err());
    }

    #[test]
    fn confidences_are_probabilities() {
        assert!(validate_probability("confidence", Some(0.5)).is_ok());
        assert!(validate_probability("confidence", Some(1.1)).is_err());
        assert!(validate_probability("confidence", Some(f64::NAN)).is_err());
    }
}
