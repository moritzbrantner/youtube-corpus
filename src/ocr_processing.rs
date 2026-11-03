//! Corpus-owned media selection and persistence around visual-analysis' canonical scene-aware OCR.
//! The recognizer, representative-frame sampling and temporal text semantics belong upstream.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use image_analysis_ocr::{OcrLocalModelOptions, OcrPreset, OcrRequest, OnnxTrOcrBackend};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use video_analysis_core::{ObservationKind, Scene};
use video_analysis_ffmpeg::FfmpegVideoSource;
use video_analysis_ingest::surface::{
    detect_content_scenes,
    scene_ocr::{analyze_scene_aware_ocr, SceneAwareOcrAnalysis},
};
use video_analysis_recognition::VideoTextRole;

use crate::multimodal::ProcessingProvenance;
use crate::visual_timeline::{
    begin_visual_processing_run_with, upsert_visual_text_observation_with,
    upsert_visual_text_track_with, BeginVisualProcessingRunRequest,
    UpsertVisualTextObservationRequest, UpsertVisualTextTrackRequest, VisualBoundingBox,
    VisualProcessingKind, VisualTextRole,
};

pub const OCR_MODEL: &str = "trocr-base-printed-onnx";
pub const OCR_CONFIG_VERSION: &str = "scene-aware-ocr-v1";
// The upstream default model spec resolves the Xenova model's `main` bundle into a local
// managed cache. This is distinct from the exact, pinned visual-analysis code revision.
pub(crate) const OCR_MODEL_VERSION: &str = "Xenova/trocr-base-printed:main";

#[derive(Debug, Clone)]
pub struct OcrAnalysisRequest {
    pub video_id: Uuid,
    pub model_dir: PathBuf,
    pub refresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrAnalysisReport {
    pub video_id: Uuid,
    pub run_id: Uuid,
    pub sampled_frames: u64,
    pub observations: u64,
    pub tracks: u64,
    pub reused: bool,
}

struct OcrBatch {
    analysis: SceneAwareOcrAnalysis,
    scenes: Vec<Scene>,
    scene_input_hash: String,
}

struct OcrProjection {
    observations: Vec<UpsertVisualTextObservationRequest>,
    tracks: Vec<UpsertVisualTextTrackRequest>,
}

fn provenance(input_hash: String) -> anyhow::Result<ProcessingProvenance> {
    let settings = json!({
        "adapter": OCR_CONFIG_VERSION,
        "sceneDetector": crate::visual_processing::SCENE_MODEL,
        "sceneThresholdBits": crate::visual_processing::DEFAULT_CONTENT_THRESHOLD.to_bits(),
        "minimumSceneFrames": crate::visual_processing::DEFAULT_MIN_SCENE_LEN,
        "sampling": "visual-analysis/representative_scene_frames",
        "model": OCR_MODEL,
    });
    Ok(ProcessingProvenance {
        processor: "visual-analysis".to_string(),
        processor_version: crate::visual_processing::VISUAL_ANALYSIS_SCENE_REVISION.to_string(),
        model: OCR_MODEL.to_string(),
        model_version: OCR_MODEL_VERSION.to_string(),
        input_hash,
        config_hash: crate::face_processing::hash_bytes(&serde_json::to_vec(&settings)?),
        processing_config: settings,
    })
}

async fn previous_run(
    pool: &PgPool,
    video_id: Uuid,
    identity: &ProcessingProvenance,
) -> anyhow::Result<Option<OcrAnalysisReport>> {
    let row = sqlx::query(
        "SELECT id, processing_config,
            (SELECT count(*) FROM visual_text_observations WHERE run_id = r.id) AS observations,
            (SELECT count(*) FROM visual_text_tracks WHERE run_id = r.id) AS tracks
         FROM media_processing_runs r
         WHERE video_id = $1 AND modality = 'ocr' AND processor = $2
           AND processor_version = $3 AND model = $4 AND model_version = $5
           AND input_hash = $6 AND config_hash = $7
           AND processing_config->>'complete' = 'true'",
    )
    .bind(video_id)
    .bind(&identity.processor)
    .bind(&identity.processor_version)
    .bind(&identity.model)
    .bind(&identity.model_version)
    .bind(&identity.input_hash)
    .bind(&identity.config_hash)
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        let settings: Value = row.try_get("processing_config")?;
        Ok(OcrAnalysisReport {
            video_id,
            run_id: row.try_get("id")?,
            sampled_frames: settings["sampledFrames"].as_u64().unwrap_or(0),
            observations: u64::try_from(row.try_get::<i64, _>("observations")?)?,
            tracks: u64::try_from(row.try_get::<i64, _>("tracks")?)?,
            reused: true,
        })
    })
    .transpose()
}

/// A completed OCR run with no recognized text is reusable. Zero tracks must not mean
/// "unprocessed" or force repeated media downloads when slides contain no text.
pub(crate) async fn has_current_ocr_output(pool: &PgPool, video_id: Uuid) -> anyhow::Result<bool> {
    let Some(hash) = crate::media_acquisition::source_input_hash(pool, video_id).await? else {
        return Ok(false);
    };
    Ok(previous_run(pool, video_id, &provenance(hash)?)
        .await?
        .is_some())
}

/// Run TrOCR on representative scene frames and persist raw OCR observations plus semantic tracks.
/// This function does not download media: the corpus plan runner owns acquisition and release.
pub async fn analyze_stored_video_ocr(
    pool: &PgPool,
    request: OcrAnalysisRequest,
) -> anyhow::Result<OcrAnalysisReport> {
    if !request.refresh {
        if let Some(hash) =
            crate::media_acquisition::source_input_hash(pool, request.video_id).await?
        {
            if let Some(report) = previous_run(pool, request.video_id, &provenance(hash)?).await? {
                return Ok(report);
            }
        }
    }

    let path: Option<String> =
        sqlx::query_scalar("SELECT local_video_path FROM videos WHERE id = $1")
            .bind(request.video_id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| anyhow::anyhow!("video not found"))?;
    let path = path
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            anyhow::anyhow!("OCR needs local video media; acquire it before running this detector")
        })?;
    anyhow::ensure!(
        path.is_file(),
        "OCR video media does not exist: {}",
        path.display()
    );

    let input = source_hash(path.clone()).await?;
    let identity = provenance(input.clone())?;
    let model_dir = request.model_dir.join("ocr");
    let video_id = request.video_id;
    let worker_path = path.clone();
    // Scene detection, model loading and inference are synchronous and can be expensive.
    // Keep them off the async executor and outside any SQL transaction.
    let batch =
        tokio::task::spawn_blocking(move || analyze_media(&worker_path, &model_dir, video_id))
            .await??;
    anyhow::ensure!(
        source_hash(path).await? == input,
        "OCR media changed during processing; retry the stage"
    );
    persist_batch(pool, video_id, identity, batch).await
}

async fn source_hash(path: PathBuf) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || {
        let identity = crate::detector_status::file_identity(&path)?;
        let hash = identity["sha256"]
            .as_str()
            .context("OCR media hash is missing")?;
        Ok(format!("sha256:{hash}"))
    })
    .await?
}

fn analyze_media(path: &Path, model_dir: &Path, video_id: Uuid) -> anyhow::Result<OcrBatch> {
    let scene_input_hash = crate::visual_processing::fingerprint_file(path)?;
    let mut scene_source = FfmpegVideoSource::open(path)
        .context("cannot decode video frames for OCR scene detection")?;
    let scene_result = detect_content_scenes(
        &mut scene_source,
        crate::visual_processing::DEFAULT_CONTENT_THRESHOLD,
        crate::visual_processing::DEFAULT_MIN_SCENE_LEN,
    )?;
    anyhow::ensure!(
        !scene_result.scenes.is_empty(),
        "OCR cannot process a video with no decodable scene frames"
    );
    drop(scene_source);

    let backend = OnnxTrOcrBackend::from_local_model(OcrLocalModelOptions {
        preset: OcrPreset::TrOcrBasePrintedOnnx,
        bundle_root: model_dir.to_path_buf(),
        auto_download: true,
        max_retries: 2,
        ..OcrLocalModelOptions::default()
    })
    .context(
        "cannot load the visual-analysis TrOCR model; check model download and ONNX Runtime",
    )?;
    let mut ocr_source =
        FfmpegVideoSource::open(path).context("cannot decode video frames for OCR recognition")?;
    let analysis = analyze_scene_aware_ocr(
        &mut ocr_source,
        backend,
        OcrRequest::new().model_preset(OcrPreset::TrOcrBasePrintedOnnx),
        &video_id.to_string(),
        &scene_result.scenes,
        None,
    )?;
    Ok(OcrBatch {
        analysis,
        scenes: scene_result.scenes,
        scene_input_hash,
    })
}

fn region(value: Option<video_analysis_core::BoundingBox>) -> Option<VisualBoundingBox> {
    value.map(|region| VisualBoundingBox {
        x: region.x,
        y: region.y,
        width: region.width,
        height: region.height,
    })
}

fn role(value: VideoTextRole) -> VisualTextRole {
    match value {
        VideoTextRole::Subtitle => VisualTextRole::Subtitle,
        VideoTextRole::EndCredit => VisualTextRole::EndCredit,
        VideoTextRole::PresentationSlide => VisualTextRole::PresentationSlide,
        VideoTextRole::SceneText => VisualTextRole::SceneText,
    }
}

/// Project the canonical model output without inventing OCR scores or attaching tracks to
/// unrelated observations. The upstream tracker identifies members by raw observation index.
fn project_batch(
    video_id: Uuid,
    run_id: Uuid,
    analysis: &SceneAwareOcrAnalysis,
    scene_ids: &BTreeMap<u64, Uuid>,
) -> anyhow::Result<OcrProjection> {
    let mut observations = Vec::new();
    let mut ids = vec![None; analysis.observations.len()];
    for (index, raw) in analysis.observations.iter().enumerate() {
        if !matches!(raw.kind, ObservationKind::Text) {
            continue;
        }
        let Some(text) = raw
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        if let Some(score) = raw.score {
            anyhow::ensure!(
                score.is_finite() && (0.0..=1.0).contains(&score),
                "invalid OCR confidence"
            );
        }
        let key = format!("observation-{index}");
        ids[index] = Some(Uuid::new_v5(
            &run_id,
            format!("observation:{key}").as_bytes(),
        ));
        observations.push(UpsertVisualTextObservationRequest {
            run_id,
            video_id,
            observation_key: key,
            text: text.to_string(),
            language: raw.attributes.get("language").cloned(),
            frame_index: raw.frame.map(|frame| frame.frame_index),
            timestamp_seconds: raw.timestamp.or_else(|| raw.frame.map(|f| f.timestamp))
                .map(|timestamp| timestamp.seconds()),
            scene_index: raw.scene_index,
            region: region(raw.region),
            confidence: raw.score.map(f64::from),
            attributes: json!({"analyzer": raw.analyzer, "label": raw.label, "details": raw.attributes}),
        });
    }

    let mut tracks = Vec::new();
    for track in &analysis.text_semantics.tracks {
        let observation_ids = track
            .observation_indices
            .iter()
            .map(|index| {
                ids.get(*index).copied().flatten().ok_or_else(|| {
                    anyhow::anyhow!(
                        "OCR track {} refers to a missing or non-text observation {index}",
                        track.id.as_str()
                    )
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        anyhow::ensure!(
            observation_ids.len() == usize::try_from(track.sample_count)?,
            "OCR track {} has inconsistent sample count",
            track.id.as_str()
        );
        tracks.push(UpsertVisualTextTrackRequest {
            run_id,
            video_id,
            track_key: track.id.as_str().to_string(),
            text: track.text.clone(),
            role: role(track.role),
            language: track.observation_indices.iter().find_map(|index| {
                analysis.observations[*index].attributes.get("language").cloned()
            }),
            sample_count: track.sample_count,
            start_frame: track.start_frame,
            end_frame: track.end_frame,
            start_seconds: track.start_seconds,
            end_seconds: track.end_seconds,
            region: region(track.representative_region),
            metadata: json!({"semanticEntityId": track.id.as_str(), "sceneIndices": track.scene_indices,
                "roleInferred": true, "tracker": "visual-analysis"}),
            observation_ids,
            scene_ids: track.scene_indices.iter().filter_map(|index| scene_ids.get(index).copied()).collect(),
        });
    }
    Ok(OcrProjection {
        observations,
        tracks,
    })
}

async fn matching_scene_ids(
    pool: &PgPool,
    video_id: Uuid,
    batch: &OcrBatch,
) -> anyhow::Result<BTreeMap<u64, Uuid>> {
    let rows = sqlx::query(
        "SELECT s.id, s.scene_index, s.start_frame, s.end_frame
         FROM video_scenes s JOIN media_processing_runs r ON r.id = s.run_id
         WHERE s.video_id = $1 AND r.modality = 'scene'
           AND r.processor = 'visual-analysis' AND r.processor_version = $2
           AND r.model = $3 AND r.input_hash = $4",
    )
    .bind(video_id)
    .bind(crate::visual_processing::VISUAL_ANALYSIS_SCENE_REVISION)
    .bind(crate::visual_processing::SCENE_MODEL)
    .bind(&batch.scene_input_hash)
    .fetch_all(pool)
    .await?;
    let mut result = BTreeMap::new();
    for row in rows {
        let index = u64::try_from(row.try_get::<i64, _>("scene_index")?)?;
        if let Some(scene) = batch.scenes.get(usize::try_from(index)?) {
            if scene.start.frame_index == u64::try_from(row.try_get::<i64, _>("start_frame")?)?
                && scene.end.frame_index == u64::try_from(row.try_get::<i64, _>("end_frame")?)?
            {
                result.insert(index, row.try_get("id")?);
            }
        }
    }
    Ok(result)
}

async fn persist_batch(
    pool: &PgPool,
    video_id: Uuid,
    mut identity: ProcessingProvenance,
    batch: OcrBatch,
) -> anyhow::Result<OcrAnalysisReport> {
    let scene_ids = matching_scene_ids(pool, video_id, &batch).await?;
    identity.processing_config["complete"] = json!(true);
    identity.processing_config["sampledFrames"] = json!(batch.analysis.sampled_frames.len());
    identity.processing_config["sceneCount"] = json!(batch.scenes.len());
    let mut tx = pool.begin().await?;
    let run_id = begin_visual_processing_run_with(
        &mut tx,
        BeginVisualProcessingRunRequest {
            video_id,
            kind: VisualProcessingKind::Ocr,
            provenance: identity,
        },
    )
    .await?;
    let projection = project_batch(video_id, run_id, &batch.analysis, &scene_ids)?;
    let mut observation_ids = Vec::new();
    for observation in projection.observations {
        let stored = upsert_visual_text_observation_with(&mut tx, observation).await?;
        observation_ids.push(stored.id);
    }
    let mut track_ids = Vec::new();
    for track in projection.tracks {
        let stored = upsert_visual_text_track_with(&mut tx, track).await?;
        track_ids.push(stored.id);
    }
    // A refreshed scan of the same input may change its detections. Reconcile only this
    // exact producer/input/config run; never erase another version's retained evidence.
    sqlx::query("DELETE FROM visual_text_tracks WHERE run_id = $1 AND NOT (id = ANY($2))")
        .bind(run_id)
        .bind(&track_ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM visual_text_observations WHERE run_id = $1 AND NOT (id = ANY($2))")
        .bind(run_id)
        .bind(&observation_ids)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(OcrAnalysisReport {
        video_id,
        run_id,
        sampled_frames: u64::try_from(batch.analysis.sampled_frames.len())?,
        observations: u64::try_from(observation_ids.len())?,
        tracks: u64::try_from(track_ids.len())?,
        reused: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use video_analysis_core::{BoundingBox, FramePosition, Observation, Timebase, Timestamp};
    use video_analysis_recognition::{analyze_video_text_semantics, VideoTextSemanticContext};

    fn position(index: u64) -> FramePosition {
        FramePosition {
            frame_index: index,
            timestamp: Timestamp::new(index as i64, Timebase::new(1, 30)),
        }
    }

    #[test]
    fn projection_preserves_original_frame_evidence_and_deduplicated_track_members() {
        let video = Uuid::new_v4();
        let run = Uuid::new_v4();
        let scenes = [Scene {
            start: position(0),
            end: position(100),
        }];
        let observations = vec![
            Observation::new("ocr", ObservationKind::Text)
                .text("Written on slide")
                .at_frame(position(30))
                .in_scene(0)
                .region(BoundingBox::new(10, 10, 200, 30).unwrap()),
            Observation::new("ocr", ObservationKind::Text)
                .text("Written on slide")
                .at_frame(position(60))
                .in_scene(0)
                .region(BoundingBox::new(10, 10, 200, 30).unwrap()),
            Observation::new("ocr", ObservationKind::Text)
                .text(" ")
                .at_frame(position(90))
                .in_scene(0),
        ];
        let semantics = analyze_video_text_semantics(
            VideoTextSemanticContext {
                video_id: "fixture",
                width: 320,
                height: 240,
                duration_seconds: Some(100.0 / 30.0),
                scenes: &scenes,
            },
            &observations,
        )
        .unwrap();
        let analysis = SceneAwareOcrAnalysis {
            sampled_frames: vec![30, 60, 90],
            observations,
            text_semantics: semantics,
        };
        let scene_id = Uuid::new_v4();
        let projection =
            project_batch(video, run, &analysis, &BTreeMap::from([(0, scene_id)])).unwrap();
        assert_eq!(projection.observations.len(), 2);
        assert_eq!(projection.tracks.len(), 1);
        let track = &projection.tracks[0];
        assert_eq!(track.sample_count, 2);
        assert_eq!(track.observation_ids.len(), 2);
        assert_eq!(track.scene_ids, vec![scene_id]);
        assert_eq!(track.start_seconds, Some(1.0));
        assert_eq!(track.end_seconds, Some(2.0));
        assert_ne!(
            projection.observations[0].observation_key,
            projection.observations[1].observation_key
        );
    }

    #[test]
    fn blank_ocr_output_creates_no_searchable_tracks() {
        let video = Uuid::new_v4();
        let run = Uuid::new_v4();
        let analysis = SceneAwareOcrAnalysis {
            sampled_frames: vec![0],
            observations: vec![],
            text_semantics: video_analysis_recognition::VideoTextSemanticAnalysis {
                tracks: vec![],
                annotations: vec![],
                relations: vec![],
            },
        };
        let projection = project_batch(video, run, &analysis, &BTreeMap::new()).unwrap();
        assert!(projection.observations.is_empty());
        assert!(projection.tracks.is_empty());
    }
}
