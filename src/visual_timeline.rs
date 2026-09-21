use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use text_embeddings::{HashedTextEmbedder, TextEmbeddingConfig};
use text_lexical::CorpusOptions;
use uuid::Uuid;

use crate::ingest::vector_literal;
use crate::multimodal::ProcessingProvenance;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualProcessingKind {
    Scene,
    Ocr,
}

impl VisualProcessingKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Scene => "scene",
            Self::Ocr => "ocr",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BeginVisualProcessingRunRequest {
    pub video_id: Uuid,
    pub kind: VisualProcessingKind,
    pub provenance: ProcessingProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualTextRole {
    Subtitle,
    EndCredit,
    PresentationSlide,
    SceneText,
}

impl VisualTextRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Subtitle => "subtitle",
            Self::EndCredit => "end_credit",
            Self::PresentationSlide => "presentation_slide",
            Self::SceneText => "scene_text",
        }
    }

    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "subtitle" => Ok(Self::Subtitle),
            "end_credit" => Ok(Self::EndCredit),
            "presentation_slide" => Ok(Self::PresentationSlide),
            "scene_text" => Ok(Self::SceneText),
            other => anyhow::bail!("unknown visual text role `{other}`"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualBoundingBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoScene {
    pub id: Uuid,
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub scene_index: u64,
    pub start_frame: u64,
    pub end_frame: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub metadata: Value,
}

#[derive(Debug, Clone)]
pub struct UpsertVideoSceneRequest {
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub scene_index: u64,
    pub start_frame: u64,
    pub end_frame: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualTextObservation {
    pub id: Uuid,
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub observation_key: String,
    pub text: String,
    pub language: Option<String>,
    pub frame_index: Option<u64>,
    pub timestamp_seconds: Option<f64>,
    pub scene_index: Option<u64>,
    pub region: Option<VisualBoundingBox>,
    pub confidence: Option<f64>,
    pub attributes: Value,
}

#[derive(Debug, Clone)]
pub struct UpsertVisualTextObservationRequest {
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub observation_key: String,
    pub text: String,
    pub language: Option<String>,
    pub frame_index: Option<u64>,
    pub timestamp_seconds: Option<f64>,
    pub scene_index: Option<u64>,
    pub region: Option<VisualBoundingBox>,
    pub confidence: Option<f64>,
    pub attributes: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualTextTrack {
    pub id: Uuid,
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub track_key: String,
    pub text: String,
    pub role: VisualTextRole,
    pub language: Option<String>,
    pub sample_count: u32,
    pub start_frame: Option<u64>,
    pub end_frame: Option<u64>,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub region: Option<VisualBoundingBox>,
    pub metadata: Value,
}

#[derive(Debug, Clone)]
pub struct UpsertVisualTextTrackRequest {
    pub run_id: Uuid,
    pub video_id: Uuid,
    pub track_key: String,
    pub text: String,
    pub role: VisualTextRole,
    pub language: Option<String>,
    pub sample_count: u32,
    pub start_frame: Option<u64>,
    pub end_frame: Option<u64>,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub region: Option<VisualBoundingBox>,
    pub metadata: Value,
    pub observation_ids: Vec<Uuid>,
    pub scene_ids: Vec<Uuid>,
}

pub async fn begin_visual_processing_run(
    pool: &PgPool,
    request: BeginVisualProcessingRunRequest,
) -> anyhow::Result<Uuid> {
    validate_provenance(&request.provenance)?;
    let identity = format!(
        "youtube-corpus:visual-run:{}:{}:{}:{}:{}:{}:{}:{}",
        request.video_id,
        request.kind.as_str(),
        request.provenance.processor,
        request.provenance.processor_version,
        request.provenance.model,
        request.provenance.model_version,
        request.provenance.input_hash,
        request.provenance.config_hash,
    );
    let id = Uuid::new_v5(&Uuid::NAMESPACE_URL, identity.as_bytes());
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
         RETURNING id",
    )
    .bind(id)
    .bind(request.video_id)
    .bind(request.kind.as_str())
    .bind(request.provenance.processor)
    .bind(request.provenance.processor_version)
    .bind(request.provenance.model)
    .bind(request.provenance.model_version)
    .bind(request.provenance.input_hash)
    .bind(request.provenance.config_hash)
    .bind(request.provenance.processing_config)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

pub async fn upsert_video_scene(
    pool: &PgPool,
    request: UpsertVideoSceneRequest,
) -> anyhow::Result<VideoScene> {
    validate_visual_run(
        pool,
        request.run_id,
        request.video_id,
        VisualProcessingKind::Scene,
    )
    .await?;
    if request.end_frame < request.start_frame {
        anyhow::bail!("scene end frame must be greater than or equal to start frame");
    }
    validate_time_range(request.start_seconds, request.end_seconds, "scene")?;
    let id = Uuid::new_v5(
        &request.run_id,
        format!("scene:{}", request.scene_index).as_bytes(),
    );
    let row = sqlx::query(
        "INSERT INTO video_scenes (
           id, run_id, video_id, scene_index, start_frame, end_frame,
           start_seconds, end_seconds, metadata
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (run_id, scene_index) DO UPDATE SET
           start_frame = EXCLUDED.start_frame,
           end_frame = EXCLUDED.end_frame,
           start_seconds = EXCLUDED.start_seconds,
           end_seconds = EXCLUDED.end_seconds,
           metadata = EXCLUDED.metadata,
           updated_at = now()
         RETURNING id, run_id, video_id, scene_index, start_frame, end_frame,
                   start_seconds, end_seconds, metadata",
    )
    .bind(id)
    .bind(request.run_id)
    .bind(request.video_id)
    .bind(i64_from_u64(request.scene_index, "scene index")?)
    .bind(i64_from_u64(request.start_frame, "scene start frame")?)
    .bind(i64_from_u64(request.end_frame, "scene end frame")?)
    .bind(request.start_seconds)
    .bind(request.end_seconds)
    .bind(request.metadata)
    .fetch_one(pool)
    .await?;
    scene_from_row(row)
}

pub async fn upsert_visual_text_observation(
    pool: &PgPool,
    request: UpsertVisualTextObservationRequest,
) -> anyhow::Result<VisualTextObservation> {
    validate_visual_run(
        pool,
        request.run_id,
        request.video_id,
        VisualProcessingKind::Ocr,
    )
    .await?;
    let observation_key = required_text("observation key", request.observation_key)?;
    let text = required_text("OCR observation text", request.text)?;
    validate_optional_time(request.timestamp_seconds, "OCR observation timestamp")?;
    validate_probability(request.confidence)?;
    validate_region(request.region)?;
    let id = Uuid::new_v5(
        &request.run_id,
        format!("observation:{observation_key}").as_bytes(),
    );
    let (x, y, width, height) = region_columns(request.region);
    let row = sqlx::query(
        "INSERT INTO visual_text_observations (
           id, run_id, video_id, observation_key, text, language, frame_index,
           timestamp_seconds, scene_index, bbox_x, bbox_y, bbox_width, bbox_height,
           confidence, attributes
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
         ON CONFLICT (run_id, observation_key) DO UPDATE SET
           text = EXCLUDED.text,
           language = EXCLUDED.language,
           frame_index = EXCLUDED.frame_index,
           timestamp_seconds = EXCLUDED.timestamp_seconds,
           scene_index = EXCLUDED.scene_index,
           bbox_x = EXCLUDED.bbox_x,
           bbox_y = EXCLUDED.bbox_y,
           bbox_width = EXCLUDED.bbox_width,
           bbox_height = EXCLUDED.bbox_height,
           confidence = EXCLUDED.confidence,
           attributes = EXCLUDED.attributes,
           updated_at = now()
         RETURNING id, run_id, video_id, observation_key, text, language, frame_index,
                   timestamp_seconds, scene_index, bbox_x, bbox_y, bbox_width, bbox_height,
                   confidence, attributes",
    )
    .bind(id)
    .bind(request.run_id)
    .bind(request.video_id)
    .bind(&observation_key)
    .bind(&text)
    .bind(normalize_optional_text(request.language))
    .bind(optional_i64(request.frame_index, "OCR frame index")?)
    .bind(request.timestamp_seconds)
    .bind(optional_i64(request.scene_index, "OCR scene index")?)
    .bind(x)
    .bind(y)
    .bind(width)
    .bind(height)
    .bind(request.confidence)
    .bind(request.attributes)
    .fetch_one(pool)
    .await?;
    observation_from_row(row)
}

pub async fn upsert_visual_text_track(
    pool: &PgPool,
    request: UpsertVisualTextTrackRequest,
) -> anyhow::Result<VisualTextTrack> {
    validate_visual_run(
        pool,
        request.run_id,
        request.video_id,
        VisualProcessingKind::Ocr,
    )
    .await?;
    let track_key = required_text("track key", request.track_key)?;
    let text = required_text("visual text track text", request.text)?;
    if request.sample_count == 0 {
        anyhow::bail!("visual text track sample count must be greater than zero");
    }
    validate_optional_range(request.start_frame, request.end_frame, "visual text frame")?;
    validate_optional_time_range(
        request.start_seconds,
        request.end_seconds,
        "visual text timestamp",
    )?;
    validate_region(request.region)?;
    let embedding = text_embedding_literal(&text)?;
    let id = Uuid::new_v5(&request.run_id, format!("track:{track_key}").as_bytes());
    let (x, y, width, height) = region_columns(request.region);
    let language = normalize_optional_text(request.language);

    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO visual_text_tracks (
           id, run_id, video_id, track_key, text, role, language, sample_count,
           start_frame, end_frame, start_seconds, end_seconds,
           bbox_x, bbox_y, bbox_width, bbox_height, metadata, embedding
         )
         VALUES (
           $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
           $13, $14, $15, $16, $17, CAST(CAST($18 AS text) AS vector)
         )
         ON CONFLICT (run_id, track_key) DO UPDATE SET
           text = EXCLUDED.text,
           role = EXCLUDED.role,
           language = EXCLUDED.language,
           sample_count = EXCLUDED.sample_count,
           start_frame = EXCLUDED.start_frame,
           end_frame = EXCLUDED.end_frame,
           start_seconds = EXCLUDED.start_seconds,
           end_seconds = EXCLUDED.end_seconds,
           bbox_x = EXCLUDED.bbox_x,
           bbox_y = EXCLUDED.bbox_y,
           bbox_width = EXCLUDED.bbox_width,
           bbox_height = EXCLUDED.bbox_height,
           metadata = EXCLUDED.metadata,
           embedding = EXCLUDED.embedding,
           updated_at = now()
         RETURNING id, run_id, video_id, track_key, text, role, language, sample_count,
                   start_frame, end_frame, start_seconds, end_seconds,
                   bbox_x, bbox_y, bbox_width, bbox_height, metadata",
    )
    .bind(id)
    .bind(request.run_id)
    .bind(request.video_id)
    .bind(&track_key)
    .bind(&text)
    .bind(request.role.as_str())
    .bind(language)
    .bind(
        i32::try_from(request.sample_count)
            .map_err(|_| anyhow::anyhow!("sample count exceeds Postgres integer range"))?,
    )
    .bind(optional_i64(
        request.start_frame,
        "visual text start frame",
    )?)
    .bind(optional_i64(request.end_frame, "visual text end frame")?)
    .bind(request.start_seconds)
    .bind(request.end_seconds)
    .bind(x)
    .bind(y)
    .bind(width)
    .bind(height)
    .bind(request.metadata)
    .bind(embedding)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM visual_text_track_observations WHERE track_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for observation_id in dedup_ids(request.observation_ids) {
        let result = sqlx::query(
            "INSERT INTO visual_text_track_observations (track_id, observation_id)
             SELECT $1, id
             FROM visual_text_observations
             WHERE id = $2 AND video_id = $3",
        )
        .bind(id)
        .bind(observation_id)
        .bind(request.video_id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            anyhow::bail!(
                "OCR observation {observation_id} does not belong to video {}",
                request.video_id
            );
        }
    }

    sqlx::query("DELETE FROM visual_text_track_scenes WHERE track_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for scene_id in dedup_ids(request.scene_ids) {
        let result = sqlx::query(
            "INSERT INTO visual_text_track_scenes (track_id, scene_id)
             SELECT $1, id
             FROM video_scenes
             WHERE id = $2 AND video_id = $3",
        )
        .bind(id)
        .bind(scene_id)
        .bind(request.video_id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            anyhow::bail!(
                "scene {scene_id} does not belong to video {}",
                request.video_id
            );
        }
    }

    tx.commit().await?;
    track_from_row(row)
}

pub async fn list_video_scenes(pool: &PgPool, video_id: Uuid) -> anyhow::Result<Vec<VideoScene>> {
    sqlx::query(
        "SELECT id, run_id, video_id, scene_index, start_frame, end_frame,
                start_seconds, end_seconds, metadata
         FROM video_scenes
         WHERE video_id = $1
         ORDER BY start_seconds, scene_index, id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(scene_from_row)
    .collect()
}

pub async fn list_visual_text_tracks(
    pool: &PgPool,
    video_id: Uuid,
) -> anyhow::Result<Vec<VisualTextTrack>> {
    sqlx::query(
        "SELECT id, run_id, video_id, track_key, text, role, language, sample_count,
                start_frame, end_frame, start_seconds, end_seconds,
                bbox_x, bbox_y, bbox_width, bbox_height, metadata
         FROM visual_text_tracks
         WHERE video_id = $1
         ORDER BY start_seconds NULLS LAST, track_key, id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(track_from_row)
    .collect()
}

async fn validate_visual_run(
    pool: &PgPool,
    run_id: Uuid,
    video_id: Uuid,
    expected: VisualProcessingKind,
) -> anyhow::Result<()> {
    let modality = sqlx::query_scalar::<_, String>(
        "SELECT modality FROM media_processing_runs WHERE id = $1 AND video_id = $2",
    )
    .bind(run_id)
    .bind(video_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        anyhow::anyhow!("processing run {run_id} does not belong to video {video_id}")
    })?;
    if modality != expected.as_str() {
        anyhow::bail!(
            "processing run {run_id} has modality `{modality}`, expected `{}`",
            expected.as_str()
        );
    }
    Ok(())
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
        if value.trim().is_empty() {
            anyhow::bail!("{name} is required for visual processing provenance");
        }
    }
    Ok(())
}

fn required_text(name: &str, value: String) -> anyhow::Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        anyhow::bail!("{name} is required");
    }
    Ok(value)
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_time_range(start: f64, end: f64, name: &str) -> anyhow::Result<()> {
    if !start.is_finite() || start < 0.0 || !end.is_finite() || end < start {
        anyhow::bail!("{name} time range must be finite, non-negative, and ordered");
    }
    Ok(())
}

fn validate_optional_time(value: Option<f64>, name: &str) -> anyhow::Result<()> {
    if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
        anyhow::bail!("{name} must be finite and non-negative");
    }
    Ok(())
}

fn validate_optional_time_range(
    start: Option<f64>,
    end: Option<f64>,
    name: &str,
) -> anyhow::Result<()> {
    validate_optional_time(start, name)?;
    validate_optional_time(end, name)?;
    if let (Some(start), Some(end)) = (start, end) {
        if end < start {
            anyhow::bail!("{name} end must be greater than or equal to start");
        }
    }
    Ok(())
}

fn validate_optional_range(start: Option<u64>, end: Option<u64>, name: &str) -> anyhow::Result<()> {
    if let (Some(start), Some(end)) = (start, end) {
        if end < start {
            anyhow::bail!("{name} end must be greater than or equal to start");
        }
    }
    Ok(())
}

fn validate_probability(value: Option<f64>) -> anyhow::Result<()> {
    if value.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
        anyhow::bail!("OCR confidence must be finite and between 0 and 1");
    }
    Ok(())
}

fn validate_region(region: Option<VisualBoundingBox>) -> anyhow::Result<()> {
    if region.is_some_and(|region| region.width == 0 || region.height == 0) {
        anyhow::bail!("visual bounding boxes must have non-zero width and height");
    }
    Ok(())
}

fn region_columns(
    region: Option<VisualBoundingBox>,
) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    region.map_or((None, None, None, None), |region| {
        (
            Some(i64::from(region.x)),
            Some(i64::from(region.y)),
            Some(i64::from(region.width)),
            Some(i64::from(region.height)),
        )
    })
}

fn text_embedding_literal(text: &str) -> anyhow::Result<String> {
    let embedder =
        HashedTextEmbedder::new(TextEmbeddingConfig::default(), CorpusOptions::default())?;
    let embedding = embedder.embed_text(text)?;
    Ok(vector_literal(embedding.as_slice()))
}

fn i64_from_u64(value: u64, name: &str) -> anyhow::Result<i64> {
    i64::try_from(value).map_err(|_| anyhow::anyhow!("{name} exceeds Postgres bigint range"))
}

fn optional_i64(value: Option<u64>, name: &str) -> anyhow::Result<Option<i64>> {
    value.map(|value| i64_from_u64(value, name)).transpose()
}

fn u64_from_i64(value: i64, name: &str) -> anyhow::Result<u64> {
    u64::try_from(value).map_err(|_| anyhow::anyhow!("stored {name} is negative"))
}

fn optional_u64(value: Option<i64>, name: &str) -> anyhow::Result<Option<u64>> {
    value.map(|value| u64_from_i64(value, name)).transpose()
}

fn dedup_ids(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn scene_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<VideoScene> {
    Ok(VideoScene {
        id: row.try_get("id")?,
        run_id: row.try_get("run_id")?,
        video_id: row.try_get("video_id")?,
        scene_index: u64_from_i64(row.try_get("scene_index")?, "scene index")?,
        start_frame: u64_from_i64(row.try_get("start_frame")?, "scene start frame")?,
        end_frame: u64_from_i64(row.try_get("end_frame")?, "scene end frame")?,
        start_seconds: row.try_get("start_seconds")?,
        end_seconds: row.try_get("end_seconds")?,
        metadata: row.try_get("metadata")?,
    })
}

fn observation_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<VisualTextObservation> {
    let x: Option<i64> = row.try_get("bbox_x")?;
    let y: Option<i64> = row.try_get("bbox_y")?;
    let width: Option<i64> = row.try_get("bbox_width")?;
    let height: Option<i64> = row.try_get("bbox_height")?;
    Ok(VisualTextObservation {
        id: row.try_get("id")?,
        run_id: row.try_get("run_id")?,
        video_id: row.try_get("video_id")?,
        observation_key: row.try_get("observation_key")?,
        text: row.try_get("text")?,
        language: row.try_get("language")?,
        frame_index: optional_u64(row.try_get("frame_index")?, "OCR frame index")?,
        timestamp_seconds: row.try_get("timestamp_seconds")?,
        scene_index: optional_u64(row.try_get("scene_index")?, "OCR scene index")?,
        region: region_from_columns(x, y, width, height)?,
        confidence: row.try_get("confidence")?,
        attributes: row.try_get("attributes")?,
    })
}

fn track_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<VisualTextTrack> {
    let role: String = row.try_get("role")?;
    let sample_count: i32 = row.try_get("sample_count")?;
    let x: Option<i64> = row.try_get("bbox_x")?;
    let y: Option<i64> = row.try_get("bbox_y")?;
    let width: Option<i64> = row.try_get("bbox_width")?;
    let height: Option<i64> = row.try_get("bbox_height")?;
    Ok(VisualTextTrack {
        id: row.try_get("id")?,
        run_id: row.try_get("run_id")?,
        video_id: row.try_get("video_id")?,
        track_key: row.try_get("track_key")?,
        text: row.try_get("text")?,
        role: VisualTextRole::parse(&role)?,
        language: row.try_get("language")?,
        sample_count: u32::try_from(sample_count)
            .map_err(|_| anyhow::anyhow!("stored sample count is negative"))?,
        start_frame: optional_u64(row.try_get("start_frame")?, "visual text start frame")?,
        end_frame: optional_u64(row.try_get("end_frame")?, "visual text end frame")?,
        start_seconds: row.try_get("start_seconds")?,
        end_seconds: row.try_get("end_seconds")?,
        region: region_from_columns(x, y, width, height)?,
        metadata: row.try_get("metadata")?,
    })
}

fn region_from_columns(
    x: Option<i64>,
    y: Option<i64>,
    width: Option<i64>,
    height: Option<i64>,
) -> anyhow::Result<Option<VisualBoundingBox>> {
    match (x, y, width, height) {
        (None, None, None, None) => Ok(None),
        (Some(x), Some(y), Some(width), Some(height)) => Ok(Some(VisualBoundingBox {
            x: u32::try_from(x).map_err(|_| anyhow::anyhow!("stored bbox x is out of range"))?,
            y: u32::try_from(y).map_err(|_| anyhow::anyhow!("stored bbox y is out of range"))?,
            width: u32::try_from(width)
                .map_err(|_| anyhow::anyhow!("stored bbox width is out of range"))?,
            height: u32::try_from(height)
                .map_err(|_| anyhow::anyhow!("stored bbox height is out of range"))?,
        })),
        _ => anyhow::bail!("stored visual bounding box is partially null"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_run_identity_is_stable_through_uuid_v5_inputs() {
        let run = Uuid::new_v5(&Uuid::NAMESPACE_URL, b"fixture-run");
        let first = Uuid::new_v5(&run, b"scene:4");
        let second = Uuid::new_v5(&run, b"scene:4");
        assert_eq!(first, second);
    }

    #[test]
    fn visual_text_role_round_trips_storage_vocabulary() {
        for role in [
            VisualTextRole::Subtitle,
            VisualTextRole::EndCredit,
            VisualTextRole::PresentationSlide,
            VisualTextRole::SceneText,
        ] {
            assert_eq!(VisualTextRole::parse(role.as_str()).unwrap(), role);
        }
    }

    #[test]
    fn rejects_invalid_visual_ranges() {
        assert!(validate_time_range(2.0, 1.0, "scene").is_err());
        assert!(validate_optional_time(Some(f64::NAN), "timestamp").is_err());
        assert!(validate_optional_range(Some(9), Some(3), "frame").is_err());
    }
}
