//! Verifiable media extracts of a video span (#116): a scene clip, an audio-only excerpt and
//! one keyframe per covering scene, cut with ffmpeg from the corpus's own source media.
//!
//! The clip and the audio excerpt cover the persisted scenes overlapping the requested span,
//! cut at their stored boundaries; no scene is ever invented, so a span without scene evidence
//! gets an audio excerpt of exactly the span and its clip and keyframes are reported skipped.
//! Every extract directory holds a `manifest.json` (`media_extract@1`) recording each file's
//! SHA-256 and size, the source media's recorded SHA-256 and yt-dlp format ids, the exact time
//! range and scene ids, the licence stated in the video's metadata, and the producer, cutter
//! and scene-detector revisions with the cutter's config hash. The manifest is a function of
//! its inputs only, and an unchanged request whose files still match it is reused untouched.
//!
//! Media follows the corpus lifecycle (#127): `process` cuts plan extracts before it releases
//! the downloaded media, and an extract of released media acquires it again through
//! [`crate::media_acquisition`] and deletes it once cut.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::media_acquisition::{MediaAcquisition, MediaNeed, SCENES_MAX_HEIGHT};
use crate::media_evidence::ProcessingEvidenceV1;
use crate::yt_dlp::YtDlpClient;

pub const MEDIA_EXTRACT_SCHEMA: &str = "media_extract@1";
pub const MANIFEST_FILE: &str = "manifest.json";

pub use crate::workflow::{license_allowed, DEFAULT_REQUIRED_LICENSE};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "camelCase")]
pub enum ExtractKind {
    /// The covering scenes as one video clip (with audio when the source has it).
    Clip,
    /// The same range, audio stream only.
    Audio,
    /// One representative frame (the midpoint) of each covering scene.
    Keyframes,
}

impl ExtractKind {
    pub const ALL: [Self; 3] = [Self::Clip, Self::Audio, Self::Keyframes];
}

/// What to extract: a time span, or stored scenes by id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractSelection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_ids: Vec<Uuid>,
}

impl ExtractSelection {
    fn validate(&self) -> anyhow::Result<()> {
        match (
            self.start_seconds,
            self.end_seconds,
            self.scene_ids.is_empty(),
        ) {
            (Some(start), Some(end), true) => anyhow::ensure!(
                start.is_finite() && start >= 0.0 && end.is_finite() && end > start,
                "the span needs 0 <= start < end"
            ),
            (None, None, false) => {}
            _ => anyhow::bail!("select either a span (start and end) or scene ids"),
        }
        Ok(())
    }

    /// Directory name of the extract: the span in milliseconds, or a digest of the scene ids.
    fn label(&self) -> String {
        match (self.start_seconds, self.end_seconds) {
            (Some(start), Some(end)) => format!(
                "span-{}-{}",
                (start * 1000.0).round() as u64,
                (end * 1000.0).round() as u64
            ),
            _ => {
                let ids: Vec<String> = self.scene_ids.iter().map(Uuid::to_string).collect();
                format!("scenes-{}", &sha256_hex(ids.join(",").as_bytes())[..16])
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtractRequest {
    pub video_id: Uuid,
    pub selection: ExtractSelection,
    pub kinds: BTreeSet<ExtractKind>,
    /// Tallest video the clip and keyframes are cut from; the default matches the media the
    /// scene stage acquires, so a plan run cuts from the file it already has.
    pub max_height: u32,
    /// Extracts land in `<out_dir>/<youtube id>/<selection>/`.
    pub out_dir: PathBuf,
    pub work_dir: PathBuf,
    /// Case-insensitive text the video's stated licence must contain; empty is the owner's
    /// opt-in for any licence.
    pub required_license: String,
    pub producer_revision: String,
    /// Keep media this extract had to acquire instead of deleting it once cut.
    pub keep_media: bool,
}

impl ExtractRequest {
    pub fn new(video_id: Uuid, selection: ExtractSelection, out_dir: PathBuf) -> Self {
        Self {
            video_id,
            selection,
            kinds: ExtractKind::ALL.into_iter().collect(),
            max_height: SCENES_MAX_HEIGHT,
            out_dir,
            work_dir: PathBuf::from("use-case-output/youtube-corpus"),
            required_license: DEFAULT_REQUIRED_LICENSE.to_string(),
            producer_revision: format!("youtube-corpus@{}", env!("CARGO_PKG_VERSION")),
            keep_media: false,
        }
    }
}

/// `manifest.json` of one extract directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractManifest {
    pub schema: String,
    pub producer: ExtractProducer,
    pub cutter: ExtractCutter,
    pub video: ExtractVideo,
    pub requested: ExtractSelection,
    pub kinds: Vec<ExtractKind>,
    /// The range cut: the covering scenes' bounds, or exactly the span without scenes.
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// The stored scene run the range and keyframes come from; None when no scene evidence
    /// covers the selection.
    pub scene_evidence: Option<SceneEvidence>,
    pub source: ExtractSource,
    pub artifacts: Vec<ExtractArtifact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SkippedExtract>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractProducer {
    pub name: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractCutter {
    pub name: String,
    /// `ffmpeg -version`'s first line.
    pub version: String,
    /// `sha256:` of the canonical JSON of `config`.
    pub config_hash: String,
    pub config: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractVideo {
    pub id: Uuid,
    pub youtube_id: Option<String>,
    pub source_url: String,
    pub title: Option<String>,
    /// The licence the video's metadata states, carried onto every extract.
    pub license: Option<String>,
    /// The owner extracted a video whose licence does not meet the required one.
    pub license_opt_in: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneEvidence {
    pub run: ProcessingEvidenceV1,
    pub scenes: Vec<ExtractScene>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractScene {
    pub id: Uuid,
    pub scene_index: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// The source media file the extracts were cut from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractSource {
    pub sha256: String,
    pub bytes: u64,
    /// The recorded acquisition (#126); None for local input files.
    pub need: Option<MediaNeed>,
    pub format_id: Option<String>,
    pub audio_format_id: Option<String>,
    pub audio_language: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractArtifact {
    pub kind: ExtractKind,
    /// File name inside the extract directory.
    pub file: String,
    pub media_type: String,
    pub bytes: u64,
    pub sha256: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// The frame's time, for keyframes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_seconds: Option<f64>,
    pub scene_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedExtract {
    pub kind: ExtractKind,
    pub reason: String,
}

/// What one extraction did; the manifest is on disk next to the files.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractReport {
    pub dir: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest_sha256: String,
    /// An identical earlier extract was still intact, so nothing was cut.
    pub reused: bool,
    /// The source media had to be downloaded for this extract.
    pub media_acquired: bool,
    /// Media this extract acquired and deleted again.
    pub media_deleted: Vec<PathBuf>,
    pub manifest: ExtractManifest,
}

/// Cuts the requested extracts and writes their manifest.
pub async fn extract_media(
    pool: &PgPool,
    client: &YtDlpClient,
    request: &ExtractRequest,
) -> anyhow::Result<ExtractReport> {
    request.selection.validate()?;
    anyhow::ensure!(!request.kinds.is_empty(), "no extract kind selected");
    anyhow::ensure!(
        !request.producer_revision.trim().is_empty(),
        "the producer revision must not be empty"
    );
    let row = sqlx::query(
        "SELECT youtube_id, source_url, title, metadata->>'license' AS license
         FROM videos WHERE id = $1",
    )
    .bind(request.video_id)
    .fetch_optional(pool)
    .await?
    .with_context(|| format!("video {} not found", request.video_id))?;
    let video = ExtractVideo {
        id: request.video_id,
        youtube_id: row.try_get("youtube_id")?,
        source_url: row.try_get("source_url")?,
        title: row.try_get("title")?,
        license: row.try_get("license")?,
        license_opt_in: false,
    };
    let required = request.required_license.trim();
    let video = if license_allowed(required, video.license.as_deref()) {
        ExtractVideo {
            license_opt_in: !license_allowed(DEFAULT_REQUIRED_LICENSE, video.license.as_deref()),
            ..video
        }
    } else {
        anyhow::bail!(
            "refusing to extract media of {}: its licence ({}) does not contain \"{required}\"; \
             opt in with --allow-non-cc",
            video.youtube_id.as_deref().unwrap_or(&video.source_url),
            video.license.as_deref().unwrap_or("none stated"),
        );
    };

    // Range and scenes come from stored evidence only.
    let scene_evidence = covering_scenes(pool, request).await?;
    let (start_seconds, end_seconds) = match &scene_evidence {
        Some(evidence) => (
            evidence
                .scenes
                .iter()
                .map(|scene| scene.start_seconds)
                .fold(f64::INFINITY, f64::min),
            evidence
                .scenes
                .iter()
                .map(|scene| scene.end_seconds)
                .fold(f64::NEG_INFINITY, f64::max),
        ),
        None => match (
            request.selection.start_seconds,
            request.selection.end_seconds,
        ) {
            (Some(start), Some(end)) => (start, end),
            _ => unreachable!("scene selections always resolve to scene evidence"),
        },
    };
    let mut skipped = Vec::new();
    let mut to_cut = Vec::new();
    for kind in &request.kinds {
        if *kind != ExtractKind::Audio && scene_evidence.is_none() {
            skipped.push(SkippedExtract {
                kind: *kind,
                reason: format!("no scene evidence covers [{start_seconds}, {end_seconds})"),
            });
        } else {
            to_cut.push(*kind);
        }
    }

    let cutter = cutter(request.max_height).await?;
    let dir = request
        .out_dir
        .join(
            video
                .youtube_id
                .clone()
                .unwrap_or_else(|| video.id.to_string()),
        )
        .join(request.selection.label());
    let manifest_path = dir.join(MANIFEST_FILE);
    let pending = ExtractManifest {
        schema: MEDIA_EXTRACT_SCHEMA.to_string(),
        producer: ExtractProducer {
            name: "youtube-corpus".to_string(),
            revision: request.producer_revision.clone(),
        },
        cutter,
        video,
        requested: request.selection.clone(),
        kinds: request.kinds.iter().copied().collect(),
        start_seconds,
        end_seconds,
        scene_evidence,
        source: ExtractSource {
            sha256: String::new(),
            bytes: 0,
            need: None,
            format_id: None,
            audio_format_id: None,
            audio_language: None,
            width: None,
            height: None,
        },
        artifacts: Vec::new(),
        skipped,
    };
    if let Some(existing) = intact_manifest(&manifest_path, &pending).await? {
        let bytes = tokio::fs::read(&manifest_path).await?;
        return Ok(ExtractReport {
            dir,
            manifest_path,
            manifest_sha256: sha256_hex(&bytes),
            reused: true,
            media_acquired: false,
            media_deleted: Vec::new(),
            manifest: existing,
        });
    }

    // Hold the media while cutting so no release deletes it under us.
    let need = if to_cut.iter().any(|kind| *kind != ExtractKind::Audio) {
        MediaNeed::Video {
            max_height: request.max_height,
        }
    } else {
        MediaNeed::Audio
    };
    let lease = crate::media_acquisition::lease(pool, request.video_id).await?;
    let media_acquired =
        !crate::media_acquisition::media_serves(pool, request.video_id, need).await?;
    let cut = async {
        let media = crate::media_acquisition::ensure_media_with(
            pool,
            client,
            request.video_id,
            need,
            &request.work_dir,
        )
        .await
        .context("could not acquire the source media")?
        .context("the video has no source media")?;
        let source = source_record(pool, request.video_id, &media).await?;
        let mut manifest = ExtractManifest { source, ..pending };
        let staging = dir.with_file_name(format!(
            ".{}.partial",
            dir.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("extract")
        ));
        if staging.exists() {
            tokio::fs::remove_dir_all(&staging).await?;
        }
        tokio::fs::create_dir_all(&staging).await?;
        manifest.artifacts = cut_artifacts(&media, &staging, &manifest, &to_cut).await?;
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        tokio::fs::write(staging.join(MANIFEST_FILE), &bytes).await?;
        if dir.exists() {
            tokio::fs::remove_dir_all(&dir).await?;
        }
        tokio::fs::rename(&staging, &dir).await?;
        anyhow::Ok((manifest, bytes))
    }
    .await;
    lease.end().await?;
    let mut media_deleted = Vec::new();
    if media_acquired && !request.keep_media {
        if let Some(lock) =
            crate::media_acquisition::try_release_lock(pool, request.video_id).await?
        {
            media_deleted = lock.finish(Some("media extracts cut")).await?;
        }
    }
    let (manifest, bytes) = cut?;
    Ok(ExtractReport {
        dir,
        manifest_path,
        manifest_sha256: sha256_hex(&bytes),
        reused: false,
        media_acquired,
        media_deleted,
        manifest,
    })
}

/// The scenes of the video's latest scene run that the selection covers.
async fn covering_scenes(
    pool: &PgPool,
    request: &ExtractRequest,
) -> anyhow::Result<Option<SceneEvidence>> {
    let rows = sqlx::query(
        "SELECT s.id, s.scene_index, s.start_seconds, s.end_seconds,
                r.id AS run_id, r.processor, r.processor_version, r.model, r.model_version,
                r.input_hash, r.config_hash, r.processing_config
         FROM video_scenes s
         JOIN media_processing_runs r ON r.id = s.run_id
         WHERE s.video_id = $1 AND s.run_id = (
           SELECT x.id FROM media_processing_runs x
           WHERE x.video_id = $1 AND x.modality = 'scene'
           ORDER BY x.updated_at DESC, x.id DESC LIMIT 1)
         ORDER BY s.start_seconds, s.scene_index",
    )
    .bind(request.video_id)
    .fetch_all(pool)
    .await?;
    let mut run = None;
    let mut scenes = Vec::new();
    for row in &rows {
        let scene = ExtractScene {
            id: row.try_get("id")?,
            scene_index: u64::try_from(row.try_get::<i64, _>("scene_index")?)?,
            start_seconds: row.try_get("start_seconds")?,
            end_seconds: row.try_get("end_seconds")?,
        };
        let selected = match (
            request.selection.start_seconds,
            request.selection.end_seconds,
        ) {
            (Some(start), Some(end)) => scene.start_seconds < end && scene.end_seconds > start,
            _ => request.selection.scene_ids.contains(&scene.id),
        };
        if selected {
            if run.is_none() {
                run = Some(ProcessingEvidenceV1 {
                    run_id: row.try_get::<Uuid, _>("run_id")?.to_string(),
                    processor: row.try_get("processor")?,
                    processor_version: row.try_get("processor_version")?,
                    model: row.try_get("model")?,
                    model_version: row.try_get("model_version")?,
                    input_hash: row.try_get("input_hash")?,
                    config_hash: row.try_get("config_hash")?,
                    processing_config: row.try_get("processing_config")?,
                });
            }
            scenes.push(scene);
        }
    }
    for id in &request.selection.scene_ids {
        anyhow::ensure!(
            scenes.iter().any(|scene| scene.id == *id),
            "scene {id} is not in the video's current scene evidence"
        );
    }
    Ok(run.map(|run| SceneEvidence { run, scenes }))
}

/// The existing manifest when it was made from the same inputs and every file it lists still
/// has its recorded size and SHA-256.
async fn intact_manifest(
    path: &Path,
    pending: &ExtractManifest,
) -> anyhow::Result<Option<ExtractManifest>> {
    let Ok(bytes) = tokio::fs::read(path).await else {
        return Ok(None);
    };
    let Ok(existing) = serde_json::from_slice::<ExtractManifest>(&bytes) else {
        return Ok(None);
    };
    // Compare as parsed JSON on both sides, so float parsing cannot tell equal inputs apart.
    let inputs = |mut value: Value| {
        if let Some(object) = value.as_object_mut() {
            object.remove("source");
            object.remove("artifacts");
        }
        value
    };
    let pending: Value = serde_json::from_slice(&serde_json::to_vec(pending)?)?;
    if inputs(serde_json::from_slice(&bytes)?) != inputs(pending) {
        return Ok(None);
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    for artifact in &existing.artifacts {
        let file = dir.join(&artifact.file);
        let Ok(identity) = digest_file(&file).await else {
            return Ok(None);
        };
        if identity != (artifact.bytes, artifact.sha256.clone()) {
            return Ok(None);
        }
    }
    Ok(Some(existing))
}

/// The cut source: its bytes hashed now and checked against the recorded acquisition.
async fn source_record(
    pool: &PgPool,
    video_id: Uuid,
    media: &Path,
) -> anyhow::Result<ExtractSource> {
    let (bytes, sha256) = digest_file(media).await?;
    let recorded: Option<Value> =
        sqlx::query_scalar("SELECT media_acquisition FROM videos WHERE id = $1")
            .bind(video_id)
            .fetch_one(pool)
            .await?;
    let acquisition = recorded
        .and_then(|value| serde_json::from_value::<MediaAcquisition>(value).ok())
        .filter(|record| record.path == media && record.deleted_at.is_none());
    if let Some(record) = &acquisition {
        anyhow::ensure!(
            record.sha256 == sha256 && record.bytes == bytes,
            "source media {} differs from its recorded acquisition",
            media.display()
        );
    }
    Ok(ExtractSource {
        sha256,
        bytes,
        need: acquisition.as_ref().map(|record| record.need),
        format_id: acquisition
            .as_ref()
            .and_then(|record| record.format_id.clone()),
        audio_format_id: acquisition
            .as_ref()
            .and_then(|record| record.audio_format_id.clone()),
        audio_language: acquisition
            .as_ref()
            .and_then(|record| record.audio_language.clone()),
        width: acquisition.as_ref().and_then(|record| record.width),
        height: acquisition.as_ref().and_then(|record| record.height),
    })
}

/// Encoder settings, fixed so the same source and range give the same bytes. They and the
/// ffmpeg version are the cutter's provenance.
fn cutter_config(max_height: u32) -> Value {
    json!({
        "maxHeight": max_height,
        "seek": "accurate (-ss before -i, re-encoded)",
        "clip": {
            "container": "mp4",
            "args": clip_codec_args(),
        },
        "audio": {
            "container": "m4a",
            "args": audio_codec_args(),
        },
        "keyframes": {
            "frame": "scene midpoint",
            "args": keyframe_codec_args(),
        },
        "bitexact": BITEXACT_ARGS,
    })
}

const BITEXACT_ARGS: [&str; 9] = [
    "-map_metadata",
    "-1",
    "-map_chapters",
    "-1",
    "-fflags",
    "+bitexact",
    "-flags",
    "+bitexact",
    "-bitexact",
];

fn clip_codec_args() -> Vec<&'static str> {
    vec![
        "-map",
        "0:v:0",
        "-map",
        "0:a:0?",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "23",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-b:a",
        "128k",
        "-ac",
        "2",
        "-movflags",
        "+faststart",
    ]
}

fn audio_codec_args() -> Vec<&'static str> {
    vec![
        "-map", "0:a:0", "-vn", "-c:a", "aac", "-b:a", "128k", "-ac", "2",
    ]
}

fn keyframe_codec_args() -> Vec<&'static str> {
    vec![
        "-map",
        "0:v:0",
        "-frames:v",
        "1",
        "-c:v",
        "png",
        "-f",
        "image2",
    ]
}

async fn cutter(max_height: u32) -> anyhow::Result<ExtractCutter> {
    let output = tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
        .context("ffmpeg is required to cut media extracts")?;
    anyhow::ensure!(output.status.success(), "ffmpeg -version failed");
    let version = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    let config = cutter_config(max_height);
    Ok(ExtractCutter {
        name: "ffmpeg".to_string(),
        version,
        config_hash: crate::corpus_export::sha256_prefixed(&crate::corpus_export::canonical_json(
            &config,
        )?),
        config,
    })
}

async fn cut_artifacts(
    media: &Path,
    dir: &Path,
    manifest: &ExtractManifest,
    kinds: &[ExtractKind],
) -> anyhow::Result<Vec<ExtractArtifact>> {
    let (start, end) = (manifest.start_seconds, manifest.end_seconds);
    let scene_ids: Vec<Uuid> = manifest
        .scene_evidence
        .iter()
        .flat_map(|evidence| evidence.scenes.iter().map(|scene| scene.id))
        .collect();
    let mut artifacts = Vec::new();
    for kind in kinds {
        match kind {
            ExtractKind::Clip | ExtractKind::Audio => {
                let (file, media_type, codec) = if *kind == ExtractKind::Clip {
                    ("clip.mp4", "video/mp4", clip_codec_args())
                } else {
                    ("audio.m4a", "audio/mp4", audio_codec_args())
                };
                ffmpeg(media, start, Some(end - start), &codec, &dir.join(file)).await?;
                artifacts.push(
                    artifact(dir, *kind, file, media_type, start, end, None, &scene_ids).await?,
                );
            }
            ExtractKind::Keyframes => {
                for scene in manifest
                    .scene_evidence
                    .iter()
                    .flat_map(|evidence| &evidence.scenes)
                {
                    let at = (scene.start_seconds + scene.end_seconds) / 2.0;
                    let file = format!("keyframe-{:04}.png", scene.scene_index);
                    ffmpeg(media, at, None, &keyframe_codec_args(), &dir.join(&file)).await?;
                    artifacts.push(
                        artifact(
                            dir,
                            *kind,
                            &file,
                            "image/png",
                            scene.start_seconds,
                            scene.end_seconds,
                            Some(at),
                            &[scene.id],
                        )
                        .await?,
                    );
                }
            }
        }
    }
    Ok(artifacts)
}

#[allow(clippy::too_many_arguments)]
async fn artifact(
    dir: &Path,
    kind: ExtractKind,
    file: &str,
    media_type: &str,
    start_seconds: f64,
    end_seconds: f64,
    timestamp_seconds: Option<f64>,
    scene_ids: &[Uuid],
) -> anyhow::Result<ExtractArtifact> {
    let (bytes, sha256) = digest_file(&dir.join(file)).await?;
    Ok(ExtractArtifact {
        kind,
        file: file.to_string(),
        media_type: media_type.to_string(),
        bytes,
        sha256,
        start_seconds,
        end_seconds,
        timestamp_seconds,
        scene_ids: scene_ids.to_vec(),
    })
}

async fn ffmpeg(
    media: &Path,
    seek: f64,
    duration: Option<f64>,
    codec: &[&str],
    output: &Path,
) -> anyhow::Result<()> {
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .args(["-nostdin", "-v", "error", "-y", "-ss"])
        .arg(format!("{seek:.6}"))
        .arg("-i")
        .arg(media);
    if let Some(duration) = duration {
        command.arg("-t").arg(format!("{duration:.6}"));
    }
    let result = command
        .args(codec)
        .args(BITEXACT_ARGS)
        .arg(output)
        .output()
        .await
        .context("failed to run ffmpeg")?;
    anyhow::ensure!(
        result.status.success() && output.is_file(),
        "ffmpeg could not cut {}: {}",
        output.display(),
        String::from_utf8_lossy(&result.stderr).trim()
    );
    Ok(())
}

async fn digest_file(path: &Path) -> anyhow::Result<(u64, String)> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let identity = crate::detector_status::file_identity(&path)?;
        let sha256 = identity["sha256"]
            .as_str()
            .context("file digest missing")?
            .to_string();
        anyhow::Ok((std::fs::metadata(&path)?.len(), sha256))
    })
    .await?
}

fn sha256_hex(bytes: &[u8]) -> String {
    crate::lower_hex(&Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selections_need_a_span_or_scene_ids() {
        let span = ExtractSelection {
            start_seconds: Some(1.5),
            end_seconds: Some(3.0),
            scene_ids: Vec::new(),
        };
        span.validate().unwrap();
        assert_eq!(span.label(), "span-1500-3000");
        let scenes = ExtractSelection {
            start_seconds: None,
            end_seconds: None,
            scene_ids: vec![Uuid::nil()],
        };
        scenes.validate().unwrap();
        assert!(scenes.label().starts_with("scenes-"));
        for invalid in [
            ExtractSelection {
                end_seconds: Some(1.0),
                ..span.clone()
            },
            ExtractSelection {
                scene_ids: vec![Uuid::nil()],
                ..span.clone()
            },
            ExtractSelection {
                start_seconds: None,
                end_seconds: None,
                scene_ids: Vec::new(),
            },
        ] {
            assert!(invalid.validate().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn licence_policy_requires_creative_commons_unless_opted_out() {
        let cc = Some("Creative Commons Attribution license (reuse allowed)");
        assert!(license_allowed(DEFAULT_REQUIRED_LICENSE, cc));
        assert!(!license_allowed(
            DEFAULT_REQUIRED_LICENSE,
            Some("Standard YouTube License")
        ));
        assert!(!license_allowed(DEFAULT_REQUIRED_LICENSE, None));
        assert!(license_allowed("", None));
    }

    #[test]
    fn a_changed_cutter_config_changes_its_hash() {
        let hash = |height| {
            crate::corpus_export::sha256_prefixed(
                &crate::corpus_export::canonical_json(&cutter_config(height)).unwrap(),
            )
        };
        assert_eq!(hash(360), hash(360));
        assert_ne!(hash(360), hash(720));
    }
}
