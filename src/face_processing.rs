//! Corpus-owned sampling, within-video tracks and persistence around canonical visual producers.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use image_analysis_core::ImageView;
use image_analysis_detection::{FaceDetectionPreset, FaceDetectorBackend, OnnxFaceDetector};
use image_analysis_embeddings::{FaceEmbedderBackend, FaceEmbeddingPreset, OnnxFaceEmbedder};
use model_runtime::{HuggingFaceDownloader, HuggingFaceModelSpec, ModelBundle, ModelBundleStore};
use runtime_core::CancellationToken;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use video_analysis_core::VideoSource;
use video_analysis_ffmpeg::FfmpegVideoSource;

use crate::multimodal::{
    begin_processing_run_with, link_face_observation_to_track_with, upsert_face_observation_with,
    upsert_face_track_with, BeginProcessingRunRequest, BoundingBox, MediaModality,
    ProcessingProvenance, UpsertFaceObservationRequest, UpsertFaceTrackRequest,
};

pub const FACE_PROCESSOR: &str = "youtube-corpus/faces";
pub const FACE_MODEL: &str = "opencv-yunet+opencv-sface";
pub const YUNET_REVISION: &str = "3cc26e7f1014a5ee5d74a42acee58bafc9d0a310";
pub const SFACE_REVISION: &str = "3d7082438a6e4551e840c9b2bb60b71e8da4b524";
pub const FACE_MODEL_VERSION: &str =
    "yunet:3cc26e7f1014a5ee5d74a42acee58bafc9d0a310;sface:3d7082438a6e4551e840c9b2bb60b71e8da4b524";
const YUNET_FILE: &str = "face_detection_yunet_2023mar.onnx";
const SFACE_FILE: &str = "face_recognition_sface_2021dec.onnx";
const YUNET_SHA256: &str = "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4";
const SFACE_SHA256: &str = "0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceAnalysisConfig {
    pub sample_interval_seconds: f64,
    pub max_track_gap_seconds: f64,
    pub min_track_similarity: f32,
}

impl Default for FaceAnalysisConfig {
    fn default() -> Self {
        Self {
            sample_interval_seconds: 1.0,
            max_track_gap_seconds: 2.0,
            min_track_similarity: 0.6,
        }
    }
}

impl FaceAnalysisConfig {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.sample_interval_seconds.is_finite() && self.sample_interval_seconds > 0.0,
            "face sampling interval must be finite and positive"
        );
        anyhow::ensure!(
            self.max_track_gap_seconds.is_finite()
                && self.max_track_gap_seconds >= self.sample_interval_seconds,
            "face track gap must be finite and at least the sampling interval"
        );
        anyhow::ensure!(
            self.min_track_similarity.is_finite()
                && (-1.0..=1.0).contains(&self.min_track_similarity),
            "face track similarity must be between -1 and 1"
        );
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct FaceAnalysisRequest {
    pub video_id: Uuid,
    pub model_dir: PathBuf,
    pub config: FaceAnalysisConfig,
    pub refresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceAnalysisReport {
    pub video_id: Uuid,
    pub run_id: Uuid,
    pub sampled_frames: u64,
    pub observations: u64,
    pub tracks: u64,
    pub reused: bool,
}

#[derive(Debug)]
struct Observation {
    key: String,
    frame: u64,
    seconds: f64,
    region: BoundingBox,
    score: f64,
    embedding: Vec<f32>,
    attributes: Value,
}

#[derive(Debug)]
struct Track {
    key: String,
    start: f64,
    end: f64,
    best_score: f64,
    embedding: Vec<f32>,
    observations: Vec<usize>,
}

struct Batch {
    sampled_frames: u64,
    observations: Vec<Observation>,
    tracks: Vec<Track>,
}

struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

fn provenance(
    input_hash: String,
    config: &FaceAnalysisConfig,
) -> anyhow::Result<ProcessingProvenance> {
    Ok(ProcessingProvenance {
        processor: FACE_PROCESSOR.to_string(),
        processor_version: crate::provenance::PROCESSOR_SOURCE_REVISION.to_string(),
        model: FACE_MODEL.to_string(),
        model_version: FACE_MODEL_VERSION.to_string(),
        input_hash,
        config_hash: hash_bytes(&serde_json::to_vec(config)?),
        processing_config: json!({ "settings": config, "tracking": "adjacent-sample-greedy-v1",
            "visualAnalysisRevision": crate::visual_processing::VISUAL_ANALYSIS_SCENE_REVISION,
            "runtime": "onnx/cpu", "foundationRevision": "8255347717ca7b8943ede7c435d0c468e5a107c5",
            "models": [
                {"id": "opencv/face_detection_yunet", "revision": YUNET_REVISION, "file": YUNET_FILE, "sha256": YUNET_SHA256, "license": "MIT"},
                {"id": "opencv/face_recognition_sface", "revision": SFACE_REVISION, "file": SFACE_FILE, "sha256": SFACE_SHA256, "license": "Apache-2.0"}
            ] }),
    })
}

async fn media_path(pool: &PgPool, video: Uuid) -> anyhow::Result<PathBuf> {
    let path: Option<String> =
        sqlx::query_scalar("SELECT local_video_path FROM videos WHERE id = $1")
            .bind(video)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| anyhow::anyhow!("video not found"))?;
    let path = path.filter(|p| !p.trim().is_empty()).map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("faces needs retained local video media; download video before running this detector"))?;
    anyhow::ensure!(
        path.is_file(),
        "retained face-analysis media does not exist: {}",
        path.display()
    );
    Ok(path)
}

async fn input_hash(path: PathBuf) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || {
        let identity = crate::detector_status::file_identity(&path)?;
        let hash = identity["sha256"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("media fingerprint is missing"))?;
        Ok(format!("sha256:{hash}"))
    })
    .await?
}

async fn existing_run(
    pool: &PgPool,
    video: Uuid,
    identity: &ProcessingProvenance,
) -> anyhow::Result<Option<FaceAnalysisReport>> {
    let row = sqlx::query(
        "SELECT r.id, r.processing_config,
        (SELECT count(*) FROM face_observations o WHERE o.run_id = r.id) AS observations,
        (SELECT count(*) FROM face_tracks t WHERE t.run_id = r.id) AS tracks
        FROM media_processing_runs r WHERE video_id = $1 AND modality = 'face'
        AND processor = $2 AND processor_version = $3 AND model = $4 AND model_version = $5
        AND input_hash = $6 AND config_hash = $7 AND processing_config->>'complete' = 'true'",
    )
    .bind(video)
    .bind(&identity.processor)
    .bind(&identity.processor_version)
    .bind(&identity.model)
    .bind(&identity.model_version)
    .bind(&identity.input_hash)
    .bind(&identity.config_hash)
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        let config: Value = row.try_get("processing_config")?;
        Ok(FaceAnalysisReport {
            video_id: video,
            run_id: row.try_get("id")?,
            sampled_frames: config["sampledFrames"].as_u64().unwrap_or(0),
            observations: u64::try_from(row.try_get::<i64, _>("observations")?)?,
            tracks: u64::try_from(row.try_get::<i64, _>("tracks")?)?,
            reused: true,
        })
    })
    .transpose()
}

pub(crate) async fn has_current_face_output(pool: &PgPool, video: Uuid) -> anyhow::Result<bool> {
    let path = match media_path(pool, video).await {
        Ok(path) => path,
        Err(_) => return Ok(false),
    };
    let identity = provenance(input_hash(path).await?, &FaceAnalysisConfig::default())?;
    Ok(existing_run(pool, video, &identity).await?.is_some())
}

/// Runs canonical YuNet/SFace over retained media; no YouTube/media acquisition is implied.
/// Missing pinned models are acquired only when this operation is explicitly requested.
pub async fn analyze_stored_video_faces(
    pool: &PgPool,
    request: FaceAnalysisRequest,
) -> anyhow::Result<FaceAnalysisReport> {
    request.config.validate()?;
    let path = media_path(pool, request.video_id).await?;
    let identity = provenance(input_hash(path.clone()).await?, &request.config)?;
    if !request.refresh {
        if let Some(report) = existing_run(pool, request.video_id, &identity).await? {
            return Ok(report);
        }
    }
    let token = CancellationToken::new();
    let _cancel_on_drop = CancelOnDrop(token.clone());
    let config = request.config.clone();
    let models = request.model_dir.join("faces");
    let worker_path = path.clone();
    let batch =
        tokio::task::spawn_blocking(move || analyze_media(&worker_path, &models, &config, &token))
            .await??;
    anyhow::ensure!(
        input_hash(path).await? == identity.input_hash,
        "face-analysis media changed during processing; retry"
    );
    persist_batch(pool, request.video_id, identity, batch).await
}

fn resolve_bundle(
    store: &ModelBundleStore,
    spec: HuggingFaceModelSpec,
    file: &str,
    expected: &str,
    token: &CancellationToken,
) -> anyhow::Result<ModelBundle> {
    let manifest = store.bundle_dir(&spec).join("manifest.json");
    let bundle = if manifest.exists() {
        store.load(
            &spec.name,
            spec.revision_value()
                .context("face model revision is missing")?,
        )?
    } else {
        store.download_cancellable(&spec, Some(token))?
    };
    let path = bundle
        .file_path(file)
        .ok_or_else(|| anyhow::anyhow!("model bundle is missing {file}"))?;
    let identity = crate::detector_status::file_identity(&path)?;
    anyhow::ensure!(
        identity["sha256"].as_str() == Some(expected),
        "{file} checksum does not match the pinned model; repair its local bundle"
    );
    Ok(bundle)
}

fn analyze_media(
    path: &Path,
    models: &Path,
    config: &FaceAnalysisConfig,
    token: &CancellationToken,
) -> anyhow::Result<Batch> {
    // Validate retained visual media before any optional model acquisition.
    let mut source = FfmpegVideoSource::open(path)
        .context("cannot decode retained video for faces; ensure it contains a visual stream")?;
    let store =
        ModelBundleStore::new(models).downloader(HuggingFaceDownloader::new().max_retries(2));
    let detector_bundle = resolve_bundle(
        &store,
        FaceDetectionPreset::OpenCvYuNet
            .model_spec()
            .revision(YUNET_REVISION)
            .file("LICENSE"),
        YUNET_FILE,
        YUNET_SHA256,
        token,
    )?;
    let embedder_bundle = resolve_bundle(
        &store,
        FaceEmbeddingPreset::OpenCvSFace
            .model_spec()
            .revision(SFACE_REVISION)
            .file("LICENSE"),
        SFACE_FILE,
        SFACE_SHA256,
        token,
    )?;
    let mut detector = OnnxFaceDetector::<runtime_onnx::OnnxSession>::from_bundle(detector_bundle)?;
    let mut embedder = OnnxFaceEmbedder::<runtime_onnx::OnnxSession>::from_bundle(embedder_bundle)?;
    let mut batch = Batch {
        sampled_frames: 0,
        observations: Vec::new(),
        tracks: Vec::new(),
    };
    let mut next_sample = 0.0;
    while let Some(frame) = source.next_frame()? {
        anyhow::ensure!(!token.is_cancelled(), "face analysis cancelled");
        let seconds = frame.position.timestamp.seconds();
        if seconds + 1e-6 < next_sample {
            continue;
        }
        next_sample = seconds + config.sample_interval_seconds;
        batch.sampled_frames += 1;
        let borrowed = frame.as_frame();
        let image = ImageView::from_video_frame(&borrowed)?;
        let mut used_tracks = BTreeSet::new();
        for (index, detection) in detector.detect_faces(&image)?.into_iter().enumerate() {
            anyhow::ensure!(!token.is_cancelled(), "face analysis cancelled");
            let embedding = embedder.embed_face(&image, Some(&detection))?;
            let bounds = detection.bbox;
            let observation = Observation {
                key: format!("frame-{}-face-{index}", frame.position.frame_index),
                frame: frame.position.frame_index,
                seconds,
                region: BoundingBox {
                    x: f64::from(bounds.x),
                    y: f64::from(bounds.y),
                    width: f64::from(bounds.width),
                    height: f64::from(bounds.height),
                },
                score: f64::from(detection.confidence),
                embedding: embedding.vector,
                attributes: json!({ "embedding": embedding.attributes, "landmarks": detection.landmarks.map(|l| l.points) }),
            };
            let track = assign_track(
                &mut batch.tracks,
                &observation,
                batch.observations.len(),
                &used_tracks,
                config,
            )?;
            used_tracks.insert(track);
            batch.observations.push(observation);
        }
    }
    anyhow::ensure!(!token.is_cancelled(), "face analysis cancelled");
    Ok(batch)
}

fn assign_track(
    tracks: &mut Vec<Track>,
    observation: &Observation,
    index: usize,
    used: &BTreeSet<usize>,
    config: &FaceAnalysisConfig,
) -> anyhow::Result<usize> {
    let mut best = None;
    for (track_index, track) in tracks.iter().enumerate() {
        if used.contains(&track_index)
            || observation.seconds - track.end > config.max_track_gap_seconds
        {
            continue;
        }
        let score =
            vector_analysis_core::cosine_similarity(&track.embedding, &observation.embedding)?;
        if score >= config.min_track_similarity && best.is_none_or(|(_, previous)| score > previous)
        {
            best = Some((track_index, score));
        }
    }
    if let Some((track_index, _)) = best {
        let track = &mut tracks[track_index];
        track.end = observation.seconds;
        if observation.score > track.best_score {
            track.embedding = observation.embedding.clone();
            track.best_score = observation.score;
        }
        track.observations.push(index);
        Ok(track_index)
    } else {
        tracks.push(Track {
            key: observation.key.clone(),
            start: observation.seconds,
            end: observation.seconds,
            best_score: observation.score,
            embedding: observation.embedding.clone(),
            observations: vec![index],
        });
        Ok(tracks.len() - 1)
    }
}

async fn persist_batch(
    pool: &PgPool,
    video: Uuid,
    mut identity: ProcessingProvenance,
    batch: Batch,
) -> anyhow::Result<FaceAnalysisReport> {
    identity.processing_config["complete"] = json!(true);
    identity.processing_config["sampledFrames"] = json!(batch.sampled_frames);
    let mut tx = pool.begin().await?;
    let run = begin_processing_run_with(
        &mut tx,
        BeginProcessingRunRequest {
            video_id: video,
            modality: MediaModality::Face,
            provenance: identity,
        },
    )
    .await?;
    let mut observation_ids = Vec::new();
    for observation in &batch.observations {
        let stored = upsert_face_observation_with(
            &mut tx,
            UpsertFaceObservationRequest {
                run_id: run.id,
                video_id: video,
                observation_key: observation.key.clone(),
                start_seconds: observation.seconds,
                end_seconds: None,
                frame_index: Some(i64::try_from(observation.frame)?),
                region: observation.region,
                detection_score: Some(observation.score),
                embedding: Some(observation.embedding.clone()),
                quality: observation.attributes.clone(),
            },
        )
        .await?;
        observation_ids.push(stored.id);
    }
    let mut track_ids = Vec::new();
    sqlx::query("DELETE FROM face_track_observations l USING face_tracks t WHERE l.track_id = t.id AND t.run_id = $1").bind(run.id).execute(&mut *tx).await?;
    for track in &batch.tracks {
        let stored = upsert_face_track_with(&mut tx, UpsertFaceTrackRequest { run_id: run.id, video_id: video, track_key: track.key.clone(),
            start_seconds: track.start, end_seconds: track.end, representative_embedding: Some(track.embedding.clone()),
            quality: json!({ "inferred": true, "verifiedIdentity": false, "observationCount": track.observations.len(), "bestDetectionScore": track.best_score }) }).await?;
        track_ids.push(stored.id);
        for index in &track.observations {
            link_face_observation_to_track_with(&mut tx, stored.id, observation_ids[*index])
                .await?;
        }
    }
    // Reconcile only this exact run; other models/configurations retain their evidence.
    sqlx::query("DELETE FROM face_tracks WHERE run_id = $1 AND NOT (id = ANY($2))")
        .bind(run.id)
        .bind(&track_ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM face_observations WHERE run_id = $1 AND NOT (id = ANY($2))")
        .bind(run.id)
        .bind(&observation_ids)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(FaceAnalysisReport {
        video_id: video,
        run_id: run.id,
        sampled_frames: batch.sampled_frames,
        observations: u64::try_from(observation_ids.len())?,
        tracks: u64::try_from(track_ids.len())?,
        reused: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation(seconds: f64, embedding: Vec<f32>) -> Observation {
        Observation {
            key: format!("at-{seconds}"),
            frame: seconds as u64,
            seconds,
            region: BoundingBox {
                x: 0.0,
                y: 0.0,
                width: 0.2,
                height: 0.2,
            },
            score: 0.9,
            embedding,
            attributes: json!({}),
        }
    }
    #[test]
    fn tracks_reuse_adjacent_matching_faces_without_merging_people_in_one_frame() {
        let config = FaceAnalysisConfig::default();
        let mut tracks = Vec::new();
        assert_eq!(
            assign_track(
                &mut tracks,
                &observation(0.0, vec![1.0, 0.0]),
                0,
                &BTreeSet::new(),
                &config
            )
            .unwrap(),
            0
        );
        assert_eq!(
            assign_track(
                &mut tracks,
                &observation(1.0, vec![1.0, 0.0]),
                1,
                &BTreeSet::new(),
                &config
            )
            .unwrap(),
            0
        );
        assert_eq!(
            assign_track(
                &mut tracks,
                &observation(1.0, vec![1.0, 0.0]),
                2,
                &BTreeSet::from([0]),
                &config
            )
            .unwrap(),
            1
        );
        assert_eq!(
            assign_track(
                &mut tracks,
                &observation(5.0, vec![1.0, 0.0]),
                3,
                &BTreeSet::new(),
                &config
            )
            .unwrap(),
            2
        );
        assert_eq!(tracks[0].observations, vec![0, 1]);
        assert_eq!(
            assign_track(
                &mut tracks,
                &observation(5.0, vec![0.0, 1.0]),
                4,
                &BTreeSet::new(),
                &config
            )
            .unwrap(),
            3
        );
    }
}
