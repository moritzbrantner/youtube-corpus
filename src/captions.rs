use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use crate::config::{CaptionConfig, SourceKind};
use crate::youtube::VideoItem;

#[derive(Debug, Clone)]
pub struct TranscriptStream {
    pub source_kind: SourceKind,
    pub language: Option<String>,
    pub source_path: Option<PathBuf>,
    pub text: Option<String>,
    pub segments: Vec<text_transcripts::TranscriptSegmentContract>,
    pub message: Option<String>,
}

pub async fn download_and_parse_captions(
    item: &VideoItem,
    dir: &Path,
    config: &CaptionConfig,
) -> anyhow::Result<Vec<TranscriptStream>> {
    if !config.enabled || item.local_video_path.is_some() {
        return Ok(Vec::new());
    }
    crate::youtube::require_command("yt-dlp")?;
    tokio::fs::create_dir_all(dir).await?;
    let langs = config.languages.join(",");
    let template = dir.join(format!("{}-subs.%(id)s.%(ext)s", item.item_id));

    run_caption_download(&item.source_url, &template, &langs, false).await?;
    if config.include_auto_captions {
        let _ = run_caption_download(&item.source_url, &template, &langs, true).await;
    }

    parse_caption_files(dir).await
}

pub async fn parse_caption_files(dir: &Path) -> anyhow::Result<Vec<TranscriptStream>> {
    let mut streams = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if ext != "vtt" && ext != "srt" {
            continue;
        }
        let text = tokio::fs::read_to_string(&path).await?;
        let parsed = if ext == "vtt" {
            text_transcripts::parse_webvtt(&text)
        } else {
            text_transcripts::parse_srt(&text)
        }?;
        let source_kind = if path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.contains(".auto.") || name.contains("auto"))
        {
            SourceKind::CaptionAuto
        } else {
            SourceKind::CaptionManual
        };
        streams.push(TranscriptStream {
            source_kind,
            language: parsed
                .language
                .clone()
                .or_else(|| infer_language_from_path(&path)),
            source_path: Some(path),
            text: parsed.text.clone(),
            segments: parsed
                .segments
                .into_iter()
                .map(text_transcripts::TranscriptSegmentContract::from)
                .collect(),
            message: None,
        });
    }
    streams.sort_by(|left, right| {
        left.source_kind
            .as_str()
            .cmp(right.source_kind.as_str())
            .then_with(|| left.language.cmp(&right.language))
    });
    Ok(streams)
}

async fn run_caption_download(
    url: &str,
    template: &Path,
    languages: &str,
    auto: bool,
) -> anyhow::Result<()> {
    let mut command = Command::new("yt-dlp");
    command
        .arg("--skip-download")
        .arg("--no-playlist")
        .arg("--sub-format")
        .arg("vtt/srt/best")
        .arg("--sub-langs")
        .arg(languages)
        .arg("-o")
        .arg(template)
        .arg(url)
        .stdin(Stdio::null());
    if auto {
        command.arg("--write-auto-subs");
    } else {
        command.arg("--write-subs");
    }
    let output = command.output().await?;
    if !output.status.success() {
        anyhow::bail!(
            "yt-dlp subtitle download failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn infer_language_from_path(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    let parts = file_name.split('.').collect::<Vec<_>>();
    parts
        .iter()
        .rev()
        .skip(1)
        .find(|part| part.len() == 2 || part.len() == 5)
        .map(|value| (*value).to_string())
}
