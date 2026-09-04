use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;
use video_analysis_ffmpeg::FfmpegVideoSource;

use crate::multimodal::ProcessingProvenance;
use crate::visual_timeline::{
    begin_visual_processing_run, upsert_video_scene, BeginVisualProcessingRunRequest,
    UpsertVideoSceneRequest, VideoScene, VisualProcessingKind,
};

/// Exact visual-analysis revision consumed by this source-first integration.
pub const VISUAL_ANALYSIS_SCENE_REVISION: &str =
    "a25d3e540d2ed90f1001fe76704ebc11815bc9db";
const SCENE_MODEL: &str = "scenedetect-core:content";
const SCENE_MODEL_VERSION: &str = "0.1.0";
const DEFAULT_CONTENT_THRESHOLD: f32 = 27.0;
const DEFAULT_MIN_SCENE_LEN: u64 = 15;

#[derive(Debug, Clone)]
pub struct SceneAnalysisRequest {
    pub video_id: Uuid,
    pub media_path: PathBuf,
    pub threshold: f32,
    pub min_scene_len: u64,
}

impl SceneAnalysisRequest {
    pub fn new(video_id: Uuid, media_path: impl Into<PathBuf>) -> Self {
        Self {
            video_id,
            media_path: media_path.into(),
            threshold: DEFAULT_CONTENT_THRESHOLD,
            min_scene_len: DEFAULT_MIN_SCENE_LEN,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneAnalysisReport {
    pub video_id: Uuid,
    pub run_id: Uuid,
    pub media_path: PathBuf,
    pub input_hash: String,
    pub config_hash: String,
    pub frames_processed: u64,
    pub scenes: Vec<VideoScene>,
}

/// Runs canonical content-scene analysis for an explicitly supplied local media file
/// and persists the resulting scene spans with idempotent processing provenance.
pub async fn analyze_and_persist_scenes(
    pool: &PgPool,
    request: SceneAnalysisRequest,
) -> anyhow::Result<SceneAnalysisReport> {
    validate_scene_config(request.threshold, request.min_scene_len)?;
    if !request.media_path.is_file() {
        anyhow::bail!(
            "scene analysis media file does not exist: {}",
            request.media_path.display()
        );
    }

    let input_hash = fingerprint_file(&request.media_path)?;
    let config_identity = format!(
        "content-threshold={:08x};min-scene-len={}",
        request.threshold.to_bits(),
        request.min_scene_len
    );
    let config_hash = fingerprint_bytes(config_identity.as_bytes());
    let run_id = begin_visual_processing_run(
        pool,
        BeginVisualProcessingRunRequest {
            video_id: request.video_id,
            kind: VisualProcessingKind::Scene,
            provenance: ProcessingProvenance {
                processor: "visual-analysis".to_string(),
                processor_version: VISUAL_ANALYSIS_SCENE_REVISION.to_string(),
                model: SCENE_MODEL.to_string(),
                model_version: SCENE_MODEL_VERSION.to_string(),
                input_hash: input_hash.clone(),
                config_hash: config_hash.clone(),
                processing_config: json!({
                    "adapter": "video-analysis-ingest.detect_content_scenes",
                    "threshold": request.threshold,
                    "minSceneLen": request.min_scene_len,
                    "visualAnalysisRevision": VISUAL_ANALYSIS_SCENE_REVISION,
                }),
            },
        },
    )
    .await?;

    let mut source = FfmpegVideoSource::open(&request.media_path)?;
    let result = video_analysis_ingest::surface::detect_content_scenes(
        &mut source,
        request.threshold,
        request.min_scene_len,
    )?;

    let mut scenes = Vec::with_capacity(result.scenes.len());
    for (index, scene) in result.scenes.into_iter().enumerate() {
        let scene_index = u64::try_from(index)
            .map_err(|_| anyhow::anyhow!("scene index exceeds u64 range"))?;
        scenes.push(
            upsert_video_scene(
                pool,
                UpsertVideoSceneRequest {
                    run_id,
                    video_id: request.video_id,
                    scene_index,
                    start_frame: scene.start.frame_index,
                    end_frame: scene.end.frame_index,
                    start_seconds: scene.start.timestamp.seconds(),
                    end_seconds: scene.end.timestamp.seconds(),
                    metadata: json!({
                        "detector": "content",
                        "threshold": request.threshold,
                        "minSceneLen": request.min_scene_len,
                    }),
                },
            )
            .await?,
        );
    }

    Ok(SceneAnalysisReport {
        video_id: request.video_id,
        run_id,
        media_path: request.media_path,
        input_hash,
        config_hash,
        frames_processed: result.frames_processed,
        scenes,
    })
}

/// Runs scene analysis for the media path already retained by the corpus.
///
/// This intentionally does not download missing media. Transcript-only corpora therefore
/// remain cheap unless the caller explicitly chooses a visual processing workflow.
pub async fn analyze_stored_video_scenes(
    pool: &PgPool,
    video_id: Uuid,
) -> anyhow::Result<SceneAnalysisReport> {
    let row = sqlx::query("SELECT local_video_path FROM videos WHERE id = $1")
        .bind(video_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("video not found"))?;
    let local_video_path: Option<String> = row.try_get("local_video_path")?;
    let path = local_video_path
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "video {video_id} has no retained local media; download or ingest media before scene analysis"
            )
        })?;
    analyze_and_persist_scenes(pool, SceneAnalysisRequest::new(video_id, path)).await
}

fn validate_scene_config(threshold: f32, min_scene_len: u64) -> anyhow::Result<()> {
    if !threshold.is_finite() || threshold < 0.0 {
        anyhow::bail!("scene threshold must be finite and non-negative");
    }
    if min_scene_len == 0 {
        anyhow::bail!("minimum scene length must be greater than zero");
    }
    Ok(())
}

fn fingerprint_file(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)?;
    let mut state = FNV1A_OFFSET;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        state = fnv1a_update(state, &buffer[..read]);
    }
    Ok(format!("fnv1a64:{state:016x}"))
}

fn fingerprint_bytes(bytes: &[u8]) -> String {
    let state = fnv1a_update(FNV1A_OFFSET, bytes);
    format!("fnv1a64:{state:016x}")
}

const FNV1A_OFFSET: u64 = 0xcbf29ce484222325;
const FNV1A_PRIME: u64 = 0x100000001b3;

fn fnv1a_update(mut state: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(FNV1A_PRIME);
    }
    state
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn content_fingerprint_is_stable_and_content_sensitive() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"scene fixture").unwrap();
        file.flush().unwrap();
        let first = fingerprint_file(file.path()).unwrap();
        let second = fingerprint_file(file.path()).unwrap();
        assert_eq!(first, second);

        file.as_file_mut().set_len(0).unwrap();
        file.as_file_mut().write_all(b"changed fixture").unwrap();
        file.as_file_mut().flush().unwrap();
        let changed = fingerprint_file(file.path()).unwrap();
        assert_ne!(first, changed);
    }

    #[test]
    fn scene_config_rejects_non_finite_thresholds_and_zero_length() {
        assert!(validate_scene_config(f32::NAN, 15).is_err());
        assert!(validate_scene_config(27.0, 0).is_err());
        assert!(validate_scene_config(27.0, 15).is_ok());
    }

    #[test]
    fn scene_config_identity_is_deterministic() {
        let first = fingerprint_bytes(b"content-threshold=41d80000;min-scene-len=15");
        let second = fingerprint_bytes(b"content-threshold=41d80000;min-scene-len=15");
        assert_eq!(first, second);
    }
}
