use std::path::Path;

use anyhow::Context;
use runtime_core::{OperationId, SurfaceRequest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::io::AsyncReadExt;
use uuid::Uuid;
use video_analysis_ffmpeg::{FfmpegSourceOptions, FfmpegVideoSource};
use video_analysis_ingest::VideoFrameSource;

use crate::multimodal::{
    begin_processing_run, link_face_observation_to_track, upsert_face_observation,
    upsert_face_track, BeginProcessingRunRequest, BoundingBox, MediaModality,
    ProcessingProvenance, UpsertFaceObservationRequest, UpsertFaceTrackRequest,
};

const VISUAL_ANALYSIS_REVISION: &str = "223b4eca141ee0a23f10d8170ca8d16db1918a7c";
const FACE_DETECTOR_MODEL: &str = "opencv-yunet-onnx";
const FACE_EMBEDDING_MODEL: &str = "opencv-sface-onnx";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaAnalysisConfig {
    #[serde(default)]
    pub faces: bool,
    #[serde(default = "default_face_interval_seconds")]
    pub face_interval_seconds: f64,
    #[serde(default = "default_face_track_similarity")]
    pub face_track_similarity: f32,
    #[serde(default)]
    pub model_auto_download: bool,
}

impl Default for MediaAnalysisConfig {
    fn default() -> Self {
        Self {
            faces: false,
            face_interval_seconds: default_face_interval_seconds(),
            face_track_similarity: default_face_track_similarity(),
            model_auto_download: false,
        }
    }
}

impl MediaAnalysisConfig {
    pub fn enabled(&self) -> bool {
        self.faces
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.face_interval_seconds.is_finite() || self.face_interval_seconds <= 0.0 {
            anyhow::bail!("face analysis interval must be finite and greater than zero");
        }
        if !self.face_track_similarity.is_finite()
            || !(0.0..=1.0).contains(&self.face_track_similarity)
        {
            anyhow::bail!("face track similarity must be finite and between zero and one");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaAnalysisReport {
    pub face_observations: usize,
    pub face_tracks: usize,
}

pub async fn analyze_video(
    pool: &PgPool,
    video_id: Uuid,
    media_path: &Path,
    config: &MediaAnalysisConfig,
) -> anyhow::Result<MediaAnalysisReport> {
    config.validate()?;
    if !config.enabled() {
        return Ok(MediaAnalysisReport::default());
    }
    let input_hash = file_sha256(media_path).await?;
    let mut report = MediaAnalysisReport::default();
    if config.faces {
        let face = analyze_faces(pool, video_id, media_path, config, &input_hash).await?;
        report.face_observations = face.face_observations;
        report.face_tracks = face.face_tracks;
    }
    Ok(report)
}

async fn analyze_faces(
    pool: &PgPool,
    video_id: Uuid,
    media_path: &Path,
    config: &MediaAnalysisConfig,
    input_hash: &str,
) -> anyhow::Result<MediaAnalysisReport> {
    let processing_config = json!({
        "frameIntervalSeconds": config.face_interval_seconds,
        "trackSimilarity": config.face_track_similarity,
        "modelAutoDownload": config.model_auto_download,
        "coordinateSpace": "normalized",
    });
    let run = begin_processing_run(
        pool,
        BeginProcessingRunRequest {
            video_id,
            modality: MediaModality::Face,
            provenance: ProcessingProvenance {
                processor: "youtube-corpus-face-analysis".to_string(),
                processor_version: env!("CARGO_PKG_VERSION").to_string(),
                model: format!("{FACE_DETECTOR_MODEL}+{FACE_EMBEDDING_MODEL}"),
                model_version: format!("visual-analysis@{VISUAL_ANALYSIS_REVISION}"),
                input_hash: input_hash.to_string(),
                config_hash: value_sha256(&processing_config)?,
                processing_config,
            },
        },
    )
    .await?;

    let filter = format!("fps=1/{}", config.face_interval_seconds);
    let options = FfmpegSourceOptions::recorded()
        .extra_output_arg("-vf")
        .extra_output_arg(filter);
    let mut source = FfmpegVideoSource::open_path_with_options(media_path, options)
        .with_context(|| format!("failed to decode face-analysis frames from {}", media_path.display()))?;

    let mut tracks = Vec::<FaceTrackState>::new();
    let mut sample_ordinal = 0_u64;
    let mut observation_count = 0_usize;
    while let Some(frame) = source.next_video_frame()? {
        let timestamp = sample_ordinal as f64 * config.face_interval_seconds;
        let image = json!({
            "width": frame.width,
            "height": frame.height,
            "pixelFormat": "rgb24",
            "stride": frame.stride,
            "data": frame.data,
        });
        let detection_response = image_analysis_detection::surface::run_surface_operation(
            SurfaceRequest {
                operation: OperationId::new("image.detection.detectFaces"),
                input: json!({
                    "image": image.clone(),
                    "model": FACE_DETECTOR_MODEL,
                    "autoDownload": config.model_auto_download,
                    "limit": 64,
                }),
            },
        )
        .map_err(|error| anyhow::anyhow!("face detection failed: {error}"))?;
        let detections = detection_response
            .value
            .get("detections")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("face detector returned no detections array"))?;

        for (detection_index, detection) in detections.iter().enumerate() {
            let normalized_region = parse_region(
                detection
                    .get("normalizedRegion")
                    .ok_or_else(|| anyhow::anyhow!("face detection omitted normalizedRegion"))?,
            )?;
            let pixel_region = detection
                .get("region")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("face detection omitted region"))?;
            let detection_score = detection.get("score").and_then(Value::as_f64);
            let embedding_response = image_analysis_embeddings::surface::run_surface_operation(
                SurfaceRequest {
                    operation: OperationId::new("image.embeddings.faceEmbed"),
                    input: json!({
                        "image": image.clone(),
                        "model": FACE_EMBEDDING_MODEL,
                        "region": pixel_region,
                        "autoDownload": config.model_auto_download,
                    }),
                },
            )
            .map_err(|error| anyhow::anyhow!("face embedding failed: {error}"))?;
            let embedding = parse_embedding(
                embedding_response
                    .value
                    .get("vector")
                    .ok_or_else(|| anyhow::anyhow!("face embedder omitted vector"))?,
            )?;

            let observation = upsert_face_observation(
                pool,
                UpsertFaceObservationRequest {
                    run_id: run.id,
                    video_id,
                    observation_key: format!(
                        "sample:{sample_ordinal}:face:{detection_index}"
                    ),
                    start_seconds: timestamp,
                    end_seconds: None,
                    frame_index: None,
                    region: normalized_region,
                    detection_score,
                    embedding: Some(embedding.clone()),
                    quality: json!({
                        "coordinateSpace": "normalized",
                        "sampleOrdinal": sample_ordinal,
                        "sampleIntervalSeconds": config.face_interval_seconds,
                        "detector": FACE_DETECTOR_MODEL,
                        "embedder": FACE_EMBEDDING_MODEL,
                    }),
                },
            )
            .await?;
            observation_count += 1;

            let track_index = best_face_track(
                &tracks,
                &embedding,
                timestamp,
                config.face_interval_seconds * 2.5,
                config.face_track_similarity,
            )
            .unwrap_or_else(|| {
                let index = tracks.len();
                tracks.push(FaceTrackState::new(index, timestamp, &embedding));
                index
            });
            tracks[track_index].observe(timestamp, &embedding, observation.id);
        }
        sample_ordinal += 1;
    }

    for track in &tracks {
        let stored = upsert_face_track(
            pool,
            UpsertFaceTrackRequest {
                run_id: run.id,
                video_id,
                track_key: track.key.clone(),
                start_seconds: track.start_seconds,
                end_seconds: track.end_seconds,
                representative_embedding: Some(track.representative_embedding()),
                quality: json!({
                    "observationCount": track.observation_ids.len(),
                    "assignment": "cosine-nearest-active-track",
                    "similarityThreshold": config.face_track_similarity,
                    "maxGapSeconds": config.face_interval_seconds * 2.5,
                }),
            },
        )
        .await?;
        for observation_id in &track.observation_ids {
            link_face_observation_to_track(pool, stored.id, *observation_id).await?;
        }
    }

    Ok(MediaAnalysisReport {
        face_observations: observation_count,
        face_tracks: tracks.len(),
    })
}

#[derive(Debug, Clone)]
struct FaceTrackState {
    key: String,
    start_seconds: f64,
    end_seconds: f64,
    last_seen_seconds: f64,
    embedding_sum: Vec<f64>,
    embedding_count: usize,
    observation_ids: Vec<Uuid>,
}

impl FaceTrackState {
    fn new(index: usize, timestamp: f64, embedding: &[f32]) -> Self {
        Self {
            key: format!("track:{index}"),
            start_seconds: timestamp,
            end_seconds: timestamp,
            last_seen_seconds: timestamp,
            embedding_sum: embedding.iter().map(|value| f64::from(*value)).collect(),
            embedding_count: 1,
            observation_ids: Vec::new(),
        }
    }

    fn observe(&mut self, timestamp: f64, embedding: &[f32], observation_id: Uuid) {
        if self.embedding_sum.len() == embedding.len() {
            for (sum, value) in self.embedding_sum.iter_mut().zip(embedding) {
                *sum += f64::from(*value);
            }
            self.embedding_count += 1;
        }
        self.end_seconds = timestamp;
        self.last_seen_seconds = timestamp;
        self.observation_ids.push(observation_id);
    }

    fn representative_embedding(&self) -> Vec<f32> {
        let count = self.embedding_count.max(1) as f64;
        normalize_embedding(
            self.embedding_sum
                .iter()
                .map(|value| (*value / count) as f32)
                .collect(),
        )
    }
}

fn best_face_track(
    tracks: &[FaceTrackState],
    embedding: &[f32],
    timestamp: f64,
    max_gap_seconds: f64,
    threshold: f32,
) -> Option<usize> {
    tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| timestamp - track.last_seen_seconds <= max_gap_seconds)
        .filter_map(|(index, track)| {
            let score = cosine_similarity(&track.representative_embedding(), embedding)?;
            (score >= threshold).then_some((index, score))
        })
        .max_by(|(left_index, left_score), (right_index, right_score)| {
            left_score
                .total_cmp(right_score)
                .then_with(|| right_index.cmp(left_index))
        })
        .map(|(index, _)| index)
}

fn parse_region(value: &Value) -> anyhow::Result<BoundingBox> {
    let number = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow::anyhow!("face region omitted numeric {name}"))
    };
    Ok(BoundingBox {
        x: number("x")?,
        y: number("y")?,
        width: number("width")?,
        height: number("height")?,
    })
}

fn parse_embedding(value: &Value) -> anyhow::Result<Vec<f32>> {
    let values = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("embedding vector must be an array"))?;
    if values.is_empty() {
        anyhow::bail!("embedding vector must not be empty");
    }
    values
        .iter()
        .map(|value| {
            let value = value
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("embedding values must be numbers"))?;
            if !value.is_finite() {
                anyhow::bail!("embedding values must be finite");
            }
            Ok(value as f32)
        })
        .collect()
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (left, right) in left.iter().zip(right) {
        dot += f64::from(*left) * f64::from(*right);
        left_norm += f64::from(*left) * f64::from(*left);
        right_norm += f64::from(*right) * f64::from(*right);
    }
    if left_norm <= f64::EPSILON || right_norm <= f64::EPSILON {
        return None;
    }
    Some((dot / (left_norm.sqrt() * right_norm.sqrt())) as f32)
}

fn normalize_embedding(mut values: Vec<f32>) -> Vec<f32> {
    let norm = values
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt();
    if norm > f64::EPSILON {
        for value in &mut values {
            *value = (f64::from(*value) / norm) as f32;
        }
    }
    values
}

async fn file_sha256(path: &Path) -> anyhow::Result<String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("failed to open {} for hashing", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn value_sha256(value: &Value) -> anyhow::Result<String> {
    let bytes = serde_json::to_vec(value)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn default_face_interval_seconds() -> f64 {
    5.0
}

fn default_face_track_similarity() -> f32 {
    0.65
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_analysis_defaults_to_disabled() {
        let config = MediaAnalysisConfig::default();
        assert!(!config.enabled());
        assert_eq!(config.face_interval_seconds, 5.0);
    }

    #[test]
    fn config_rejects_invalid_face_controls() {
        let mut config = MediaAnalysisConfig::default();
        config.face_interval_seconds = 0.0;
        assert!(config.validate().is_err());
        config.face_interval_seconds = 5.0;
        config.face_track_similarity = 1.5;
        assert!(config.validate().is_err());
    }

    #[test]
    fn face_tracking_prefers_similarity_then_stable_index() {
        let mut tracks = vec![
            FaceTrackState::new(0, 0.0, &[1.0, 0.0]),
            FaceTrackState::new(1, 0.0, &[0.9, 0.1]),
        ];
        tracks[0].last_seen_seconds = 4.0;
        tracks[1].last_seen_seconds = 4.0;
        assert_eq!(
            best_face_track(&tracks, &[1.0, 0.0], 5.0, 10.0, 0.5),
            Some(0)
        );
    }

    #[test]
    fn face_tracking_ignores_stale_tracks() {
        let tracks = vec![FaceTrackState::new(0, 0.0, &[1.0, 0.0])];
        assert_eq!(
            best_face_track(&tracks, &[1.0, 0.0], 20.0, 10.0, 0.5),
            None
        );
    }

    #[test]
    fn config_hash_is_deterministic() {
        let value = json!({"a": 1, "b": true});
        assert_eq!(value_sha256(&value).unwrap(), value_sha256(&value).unwrap());
    }
}
