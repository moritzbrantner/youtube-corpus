//! Plan-driven source media acquisition (#126) with recorded stream provenance (#120).
//!
//! A processing plan acquires only what its enabled, still-to-run stages need: nothing for
//! metadata, captions, entities and translations; an audio-only stream for ASR and voices;
//! one video (with audio) at the lowest sufficient height for scenes, faces and OCR. The
//! downloaded file is recorded with its yt-dlp format ids, audio track language, byte size
//! and SHA-256, and a later plan that needs more, or finds the file removed, acquires again.
//! Local input files are a read-only source and are never acquired or replaced.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::config::YtDlpConfig;
use crate::yt_dlp::YtDlpClient;

/// Scene cuts survive heavy downscaling.
pub const SCENES_MAX_HEIGHT: u32 = 360;
/// Faces and on-screen text need enough detail for small faces and captions in frame.
pub const FRAME_DETAIL_MAX_HEIGHT: u32 = 720;

/// The source media a set of stages needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MediaNeed {
    Nothing,
    Audio,
    /// Video no taller than `max_height` (the best available at or below it), with audio.
    #[serde(rename_all = "camelCase")]
    Video {
        max_height: u32,
    },
}

impl MediaNeed {
    pub fn for_detector(detector: &str) -> Self {
        match detector {
            "asr" | "voices" => Self::Audio,
            "scenes" => Self::Video {
                max_height: SCENES_MAX_HEIGHT,
            },
            "faces" | "ocr" => Self::Video {
                max_height: FRAME_DETAIL_MAX_HEIGHT,
            },
            _ => Self::Nothing,
        }
    }

    pub fn for_detectors<'a>(detectors: impl IntoIterator<Item = &'a str>) -> Self {
        detectors
            .into_iter()
            .map(Self::for_detector)
            .fold(Self::Nothing, Self::union)
    }

    /// The smallest need that serves both.
    pub fn union(self, other: Self) -> Self {
        match (self, other) {
            (Self::Video { max_height: a }, Self::Video { max_height: b }) => Self::Video {
                max_height: a.max(b),
            },
            (video @ Self::Video { .. }, _) | (_, video @ Self::Video { .. }) => video,
            (Self::Audio, _) | (_, Self::Audio) => Self::Audio,
            _ => Self::Nothing,
        }
    }

    /// Whether media acquired for `acquired` also serves this need. Video always carries
    /// audio, so it serves audio-only stages too.
    pub fn is_covered_by(self, acquired: Self) -> bool {
        match (self, acquired) {
            (Self::Nothing, _) => true,
            (Self::Audio, Self::Audio | Self::Video { .. }) => true,
            (Self::Video { max_height: a }, Self::Video { max_height: b }) => a <= b,
            _ => false,
        }
    }

    /// The yt-dlp `--format` selector. Audio relies on yt-dlp's default `lang` sort, which
    /// ranks YouTube's original-language track above auto-dubbed ones; the resolved track
    /// and its language are recorded in the acquisition.
    pub fn format_selector(self) -> Option<String> {
        match self {
            Self::Nothing => None,
            Self::Audio => Some("ba/wa*".to_string()),
            Self::Video { max_height: h } => {
                Some(format!("bv*[height<={h}]+ba/b[height<={h}]/wv*+ba/w"))
            }
        }
    }
}

/// One yt-dlp format inside the acquired file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquiredFormat {
    pub format_id: String,
    pub language: Option<String>,
    pub format_note: Option<String>,
    pub acodec: Option<String>,
    pub vcodec: Option<String>,
    pub height: Option<u32>,
}

impl AcquiredFormat {
    fn has_audio(&self) -> bool {
        self.acodec.as_deref().is_some_and(|codec| codec != "none")
    }
}

/// The recorded download behind a video's `local_video_path` (`videos.media_acquisition`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaAcquisition {
    pub need: MediaNeed,
    pub format_selector: String,
    /// yt-dlp's combined id, such as `251-5` or `134+251-5`.
    pub format_id: Option<String>,
    pub formats: Vec<AcquiredFormat>,
    pub audio_format_id: Option<String>,
    pub audio_language: Option<String>,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_seconds: Option<f64>,
    pub yt_dlp_args_redacted: Vec<String>,
    pub acquired_at: String,
}

impl MediaAcquisition {
    /// The recorded file is still there with its recorded size. (Hashing every reuse would
    /// read whole videos; stages fingerprint their own input.)
    pub fn file_present(&self) -> bool {
        std::fs::metadata(&self.path).is_ok_and(|meta| meta.is_file() && meta.len() == self.bytes)
    }
}

/// Makes sure the video's source media serves `need`, downloading it when the recorded
/// acquisition is missing, insufficient or its file was removed. Returns the media path, or
/// None when nothing is needed. Local input files are returned as they are.
pub async fn ensure_media(
    pool: &PgPool,
    video_id: Uuid,
    need: MediaNeed,
    work_dir: &Path,
    yt_dlp: &YtDlpConfig,
) -> anyhow::Result<Option<PathBuf>> {
    ensure_media_with(
        pool,
        &YtDlpClient::new(yt_dlp.clone()),
        video_id,
        need,
        work_dir,
    )
    .await
}

pub async fn ensure_media_with(
    pool: &PgPool,
    client: &YtDlpClient,
    video_id: Uuid,
    need: MediaNeed,
    work_dir: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    if need == MediaNeed::Nothing {
        return Ok(None);
    }
    let row = sqlx::query(
        "SELECT source_url, local_video_path, media_acquisition FROM videos WHERE id = $1",
    )
    .bind(video_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("video not found"))?;
    let source_url: String = row.try_get("source_url")?;
    let local_path: Option<String> = row.try_get("local_video_path")?;
    let local_path = local_path
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from);
    if !is_remote(&source_url) {
        return Ok(local_path);
    }
    let recorded: Option<serde_json::Value> = row.try_get("media_acquisition")?;
    match recorded.map(serde_json::from_value::<MediaAcquisition>) {
        Some(Ok(acquired)) => {
            if need.is_covered_by(acquired.need)
                && local_path.as_deref() == Some(acquired.path.as_path())
                && acquired.file_present()
            {
                return Ok(Some(acquired.path));
            }
        }
        // Media retained before acquisitions were recorded was a full-quality download.
        None => {
            if let Some(path) = local_path.filter(|path| path.is_file()) {
                return Ok(Some(path));
            }
        }
        Some(Err(_)) => {}
    }
    let item_id = crate::youtube::sanitize_id(&source_url);
    let media_dir = work_dir.join(&item_id).join("media");
    let acquisition = download(client, &source_url, &media_dir, &item_id, need).await?;
    sqlx::query(
        "UPDATE videos SET local_video_path = $2, media_acquisition = $3, updated_at = now()
        WHERE id = $1",
    )
    .bind(video_id)
    .bind(acquisition.path.to_string_lossy().into_owned())
    .bind(serde_json::to_value(&acquisition)?)
    .execute(pool)
    .await?;
    Ok(Some(acquisition.path))
}

fn is_remote(source_url: &str) -> bool {
    source_url.starts_with("https://") || source_url.starts_with("http://")
}

/// Downloads the media `need` asks for into `media_dir` and describes the stored file.
pub async fn download(
    client: &YtDlpClient,
    source_url: &str,
    media_dir: &Path,
    file_stem: &str,
    need: MediaNeed,
) -> anyhow::Result<MediaAcquisition> {
    let selector = need
        .format_selector()
        .ok_or_else(|| anyhow::anyhow!("no media is needed"))?;
    tokio::fs::create_dir_all(media_dir).await?;
    let temp_dir = media_dir.join(format!(".tmp-{file_stem}"));
    if temp_dir.exists() {
        tokio::fs::remove_dir_all(&temp_dir).await?;
    }
    tokio::fs::create_dir_all(&temp_dir).await?;
    let output_template = temp_dir.join(format!("{file_stem}-%(id)s.%(ext)s"));
    let result = download_into(
        client,
        source_url,
        media_dir,
        &temp_dir,
        &output_template,
        need,
        selector,
    )
    .await;
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    result
}

async fn download_into(
    client: &YtDlpClient,
    source_url: &str,
    media_dir: &Path,
    temp_dir: &Path,
    output_template: &Path,
    need: MediaNeed,
    selector: String,
) -> anyhow::Result<MediaAcquisition> {
    let (stdout, report) = client
        .download_media(source_url, output_template, &selector)
        .await?;
    let printed = printed_download_path(&stdout)
        .ok_or_else(|| anyhow::anyhow!("yt-dlp completed but did not print after_move:filepath"))?;
    let downloaded = printed.canonicalize()?;
    if !downloaded.starts_with(temp_dir.canonicalize()?) {
        anyhow::bail!(
            "yt-dlp reported output outside temporary directory: {}",
            downloaded.display()
        );
    }
    let info = match info_json_path(&downloaded) {
        Some(path) if path.is_file() => Some((path.clone(), tokio::fs::read(&path).await?)),
        _ => None,
    };
    let formats = info
        .as_ref()
        .map(|(_, bytes)| parse_formats(bytes))
        .transpose()?
        .unwrap_or_default();

    // Probe and fingerprint before publishing, so a rejected download leaves nothing behind.
    let path = downloaded.clone();
    let (probe, bytes, sha256) = tokio::task::spawn_blocking(move || {
        let probe = probe(&path, need)?;
        let bytes = std::fs::metadata(&path)?.len();
        let identity = crate::detector_status::file_identity(&path)?;
        let sha256 = identity["sha256"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("media fingerprint missing"))?
            .to_string();
        anyhow::Ok((probe, bytes, sha256))
    })
    .await??;

    let file_name = downloaded
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("yt-dlp reported output without a file name"))?;
    let final_path = media_dir.join(file_name);
    tokio::fs::rename(&downloaded, &final_path).await?;
    if let Some((info_path, _)) = &info {
        if let Some(name) = info_path.file_name() {
            tokio::fs::rename(info_path, media_dir.join(name)).await?;
        }
    }

    let audio = formats.iter().find(|format| format.has_audio());
    Ok(MediaAcquisition {
        need,
        format_selector: selector,
        format_id: info
            .as_ref()
            .and_then(|(_, bytes)| serde_json::from_slice::<InfoFormat>(bytes).ok())
            .and_then(|top| top.format_id),
        audio_format_id: audio.map(|format| format.format_id.clone()),
        audio_language: audio.and_then(|format| format.language.clone()),
        formats,
        path: final_path,
        bytes,
        sha256,
        width: probe.width,
        height: probe.height,
        duration_seconds: probe.duration_seconds,
        yt_dlp_args_redacted: report.args_redacted,
        acquired_at: chrono::Utc::now().to_rfc3339(),
    })
}

#[derive(Debug, Default)]
struct Probe {
    width: Option<u32>,
    height: Option<u32>,
    duration_seconds: Option<f64>,
}

fn probe(path: &Path, need: MediaNeed) -> anyhow::Result<Probe> {
    if matches!(need, MediaNeed::Video { .. }) {
        let metadata = video_analysis_ffmpeg::probe(path)?;
        if metadata.width == 0 || metadata.height == 0 {
            anyhow::bail!("ffprobe returned no video stream for {}", path.display());
        }
        if metadata.duration_seconds.unwrap_or(0.0) <= 0.0 {
            anyhow::bail!(
                "ffprobe returned no positive duration for {}",
                path.display()
            );
        }
        return Ok(Probe {
            width: Some(metadata.width),
            height: Some(metadata.height),
            duration_seconds: metadata.duration_seconds,
        });
    }
    // WebM audio (YouTube's opus tracks) often carries no stream duration; a decodable
    // audio stream is what ASR and voices need.
    let metadata = video_analysis_ffmpeg::probe_audio(path)?;
    if metadata.sample_rate == 0 || metadata.channels == 0 {
        anyhow::bail!("ffprobe returned no audio stream for {}", path.display());
    }
    Ok(Probe {
        duration_seconds: metadata.duration_seconds,
        ..Probe::default()
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
struct InfoFormat {
    format_id: Option<String>,
    language: Option<String>,
    format_note: Option<String>,
    acodec: Option<String>,
    vcodec: Option<String>,
    height: Option<f64>,
}

impl InfoFormat {
    fn into_acquired(self) -> Option<AcquiredFormat> {
        Some(AcquiredFormat {
            format_id: self.format_id?,
            language: self.language,
            format_note: self.format_note,
            acodec: self.acodec,
            vcodec: self.vcodec,
            height: self.height.map(|height| height as u32),
        })
    }
}

#[derive(Debug, Deserialize)]
struct InfoJson {
    #[serde(flatten)]
    top: InfoFormat,
    requested_formats: Option<Vec<InfoFormat>>,
    #[serde(default)]
    formats: Vec<InfoFormat>,
}

/// The formats inside the downloaded file: each merged part, or the single format. yt-dlp's
/// written info JSON carries the combined `format_id` (`243+251`) and the full `formats`
/// list but not `requested_formats`, so parts are resolved against that list.
fn parse_formats(info_json: &[u8]) -> anyhow::Result<Vec<AcquiredFormat>> {
    let info: InfoJson = serde_json::from_slice(info_json)?;
    if let Some(parts) = info.requested_formats.filter(|parts| !parts.is_empty()) {
        return Ok(parts
            .into_iter()
            .filter_map(InfoFormat::into_acquired)
            .collect());
    }
    let Some(combined) = info.top.format_id.clone() else {
        return Ok(Vec::new());
    };
    let parts: Option<Vec<_>> = combined
        .split('+')
        .map(|id| {
            info.formats
                .iter()
                .find(|format| format.format_id.as_deref() == Some(id))
                .cloned()
                .and_then(InfoFormat::into_acquired)
        })
        .collect();
    Ok(parts.unwrap_or_else(|| info.top.into_acquired().into_iter().collect()))
}

fn printed_download_path(stdout: &[u8]) -> Option<PathBuf> {
    String::from_utf8_lossy(stdout)
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
}

fn info_json_path(media_path: &Path) -> Option<PathBuf> {
    let parent = media_path.parent()?;
    let stem = media_path.file_stem()?.to_str()?;
    Some(parent.join(format!("{stem}.info.json")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yt_dlp::{CommandFuture, CommandRunner, CommandRunnerOutput, YtDlpCommandSpec};
    use std::sync::{Arc, Mutex};

    #[test]
    fn derives_need_from_enabled_stages() {
        assert_eq!(
            MediaNeed::for_detectors(["captions", "entities", "translate_spoken"]),
            MediaNeed::Nothing
        );
        assert_eq!(
            MediaNeed::for_detectors(["captions", "asr"]),
            MediaNeed::Audio
        );
        assert_eq!(MediaNeed::for_detectors(["voices"]), MediaNeed::Audio);
        assert_eq!(
            MediaNeed::for_detectors(["asr", "scenes"]),
            MediaNeed::Video { max_height: 360 }
        );
        assert_eq!(
            MediaNeed::for_detectors(["scenes", "faces", "voices"]),
            MediaNeed::Video { max_height: 720 }
        );
        assert_eq!(MediaNeed::Nothing.format_selector(), None);
        assert_eq!(MediaNeed::Audio.format_selector().unwrap(), "ba/wa*");
        assert_eq!(
            MediaNeed::Video { max_height: 720 }
                .format_selector()
                .unwrap(),
            "bv*[height<=720]+ba/b[height<=720]/wv*+ba/w"
        );
    }

    #[test]
    fn video_serves_audio_but_audio_does_not_serve_video() {
        let low = MediaNeed::Video { max_height: 360 };
        let high = MediaNeed::Video { max_height: 720 };
        assert!(MediaNeed::Audio.is_covered_by(low));
        assert!(low.is_covered_by(high));
        assert!(!high.is_covered_by(low));
        assert!(!low.is_covered_by(MediaNeed::Audio));
        assert!(MediaNeed::Nothing.is_covered_by(MediaNeed::Nothing));
    }

    #[test]
    fn records_merged_and_single_format_ids() {
        let merged = br#"{"format_id": "134+251-5", "requested_formats": [
            {"format_id": "134", "vcodec": "avc1.4d401e", "acodec": "none", "height": 360},
            {"format_id": "251-5", "vcodec": "none", "acodec": "opus", "language": "en",
             "format_note": "English original (default), medium"}]}"#;
        let formats = parse_formats(merged).unwrap();
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].height, Some(360));
        assert_eq!(formats[1].format_id, "251-5");
        assert_eq!(formats[1].language.as_deref(), Some("en"));

        // What yt-dlp actually writes: the combined id and the full format list.
        let written = br#"{"format_id": "243+251", "acodec": "opus", "vcodec": "vp9", "formats": [
            {"format_id": "140", "acodec": "mp4a.40.2", "vcodec": "none"},
            {"format_id": "243", "acodec": "none", "vcodec": "vp9", "height": 360},
            {"format_id": "251", "acodec": "opus", "vcodec": "none", "language": "en"}]}"#;
        let formats = parse_formats(written).unwrap();
        let ids: Vec<_> = formats.iter().map(|f| f.format_id.as_str()).collect();
        assert_eq!(ids, ["243", "251"]);
        assert_eq!(formats[1].language.as_deref(), Some("en"));

        let single =
            br#"{"format_id": "251", "acodec": "opus", "vcodec": "none", "language": "de"}"#;
        let formats = parse_formats(single).unwrap();
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].format_id, "251");
    }

    #[test]
    fn parses_last_printed_download_path() {
        let path = printed_download_path(b"[download] progress\n/tmp/video.mp4\n").unwrap();
        assert_eq!(path, PathBuf::from("/tmp/video.mp4"));
        assert!(printed_download_path(b"\n\n").is_none());
        assert_eq!(
            info_json_path(Path::new("/tmp/video.mp4")).unwrap(),
            PathBuf::from("/tmp/video.info.json")
        );
    }

    /// Stands in for yt-dlp: "downloads" a fixture file to the requested template.
    struct FakeDownloader {
        fixture: PathBuf,
        ext: &'static str,
        info: serde_json::Value,
        calls: Mutex<Vec<YtDlpCommandSpec>>,
    }

    impl CommandRunner for FakeDownloader {
        fn run<'a>(&'a self, spec: YtDlpCommandSpec) -> CommandFuture<'a> {
            Box::pin(async move {
                let template =
                    spec.args[spec.args.iter().position(|a| a == "-o").unwrap() + 1].clone();
                let base = template.replace("%(id)s", "fake").replace(".%(ext)s", "");
                let media = format!("{base}.{}", self.ext);
                std::fs::copy(&self.fixture, &media).unwrap();
                std::fs::write(format!("{base}.info.json"), self.info.to_string()).unwrap();
                self.calls.lock().unwrap().push(spec);
                Ok(CommandRunnerOutput {
                    stdout: format!("{media}\n").into_bytes(),
                    stderr: Vec::new(),
                    exit_status: Some(0),
                })
            })
        }
    }

    fn ffmpeg_fixture(dir: &Path, name: &str, args: &[&str]) -> Option<PathBuf> {
        let path = dir.join(name);
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y"])
            .args(args)
            .arg(&path)
            .status()
            .ok()?;
        status.success().then_some(path)
    }

    #[tokio::test]
    async fn audio_download_records_format_ids_size_and_digest_of_the_stored_file() {
        let dir = tempfile::tempdir().unwrap();
        let Some(fixture) = ffmpeg_fixture(
            dir.path(),
            "tone.webm",
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-c:a",
                "libopus",
            ],
        ) else {
            eprintln!("ffmpeg with libopus unavailable; skipping");
            return;
        };
        let runner = Arc::new(FakeDownloader {
            fixture,
            ext: "webm",
            info: serde_json::json!({"format_id": "251-5", "acodec": "opus", "vcodec": "none",
                "language": "en", "format_note": "English original (default), medium"}),
            calls: Mutex::new(Vec::new()),
        });
        let client = YtDlpClient::with_runner(YtDlpConfig::default(), runner.clone());
        let media_dir = dir.path().join("media");

        let acquired = download(
            &client,
            "https://www.youtube.com/watch?v=aaaaaaaaaaa",
            &media_dir,
            "item",
            MediaNeed::Audio,
        )
        .await
        .unwrap();

        let args = &runner.calls.lock().unwrap()[0].args;
        assert_eq!(args[1..3], ["--format".to_string(), "ba/wa*".to_string()]);
        assert!(!args.iter().any(|arg| arg == "--download-archive"));
        let bytes = std::fs::read(&acquired.path).unwrap();
        {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            assert_eq!(acquired.sha256, digest);
        }
        assert_eq!(acquired.bytes, bytes.len() as u64);
        assert_eq!(acquired.path, media_dir.join("item-fake.webm"));
        assert!(media_dir.join("item-fake.info.json").is_file());
        assert!(!media_dir.join(".tmp-item").exists());
        assert_eq!(acquired.format_id.as_deref(), Some("251-5"));
        assert_eq!(acquired.audio_format_id.as_deref(), Some("251-5"));
        assert_eq!(acquired.audio_language.as_deref(), Some("en"));
        assert_eq!(acquired.height, None);
        assert!(acquired.file_present());

        // A removed file no longer counts as present, so the next plan acquires again.
        std::fs::remove_file(&acquired.path).unwrap();
        assert!(!acquired.file_present());
    }

    #[tokio::test]
    async fn video_download_requires_a_video_stream() {
        let dir = tempfile::tempdir().unwrap();
        let Some(fixture) = ffmpeg_fixture(
            dir.path(),
            "tone.m4a",
            &["-f", "lavfi", "-i", "sine=frequency=440:duration=1"],
        ) else {
            eprintln!("ffmpeg unavailable; skipping");
            return;
        };
        let runner = Arc::new(FakeDownloader {
            fixture,
            ext: "m4a",
            info: serde_json::json!({"format_id": "140"}),
            calls: Mutex::new(Vec::new()),
        });
        let client = YtDlpClient::with_runner(YtDlpConfig::default(), runner);
        let error = download(
            &client,
            "https://www.youtube.com/watch?v=aaaaaaaaaaa",
            &dir.path().join("media"),
            "item",
            MediaNeed::Video { max_height: 720 },
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("video"), "{error:#}");
    }
}
