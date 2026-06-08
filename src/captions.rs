use std::path::{Path, PathBuf};

use crate::config::{CaptionConfig, SourceKind, YtDlpConfig};
use crate::youtube::VideoItem;
use crate::yt_dlp::YtDlpClient;

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
    yt_dlp: &YtDlpConfig,
) -> anyhow::Result<Vec<TranscriptStream>> {
    if !config.enabled || item.local_video_path.is_some() {
        return Ok(Vec::new());
    }
    tokio::fs::create_dir_all(dir).await?;
    let langs = config.languages.join(",");
    let template = dir.join(format!("{}-subs.%(id)s.%(ext)s", item.item_id));

    let mut messages = Vec::new();
    if let Err(error) =
        run_caption_download(&item.source_url, &template, &langs, false, yt_dlp).await
    {
        messages.push(TranscriptStream {
            source_kind: SourceKind::CaptionManual,
            language: None,
            source_path: None,
            text: None,
            segments: Vec::new(),
            message: Some(format!("manual captions unavailable: {error}")),
        });
    }
    if config.include_auto_captions {
        if let Err(error) =
            run_caption_download(&item.source_url, &template, &langs, true, yt_dlp).await
        {
            messages.push(TranscriptStream {
                source_kind: SourceKind::CaptionAuto,
                language: None,
                source_path: None,
                text: None,
                segments: Vec::new(),
                message: Some(format!("auto captions unavailable: {error}")),
            });
        }
    }

    let mut streams = parse_caption_files(dir).await?;
    streams.extend(messages);
    Ok(streams)
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
        let segments = parsed
            .segments
            .into_iter()
            .filter_map(|segment| {
                let mut segment = text_transcripts::TranscriptSegmentContract::from(segment);
                segment.text = normalize_subtitle_text(&segment.text);
                (!segment.text.is_empty()).then_some(segment)
            })
            .collect::<Vec<_>>();
        let text = parsed
            .text
            .as_deref()
            .map(normalize_subtitle_text)
            .filter(|text| !text.is_empty())
            .or_else(|| {
                (!segments.is_empty()).then(|| {
                    segments
                        .iter()
                        .map(|segment| segment.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
            });
        streams.push(TranscriptStream {
            source_kind,
            language: parsed
                .language
                .clone()
                .or_else(|| infer_language_from_path(&path)),
            source_path: Some(path),
            text,
            segments,
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
    yt_dlp: &YtDlpConfig,
) -> anyhow::Result<()> {
    let client = YtDlpClient::new(yt_dlp.clone());
    client
        .download_captions(url, template.to_path_buf(), languages, auto)
        .await
        .map(|_| ())
        .map_err(Into::into)
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

fn normalize_subtitle_text(text: &str) -> String {
    let without_markup = strip_subtitle_markup(text);
    let decoded = decode_basic_entities(&without_markup);
    collapse_whitespace(&decoded)
}

fn strip_subtitle_markup(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '<' {
            output.push(ch);
            continue;
        }

        let mut tag = String::new();
        let mut closed = false;
        for tag_ch in chars.by_ref() {
            if tag_ch == '>' {
                closed = true;
                break;
            }
            tag.push(tag_ch);
        }

        if !closed {
            output.push('<');
            output.push_str(&tag);
            break;
        }

        let tag = tag.trim();
        if tag.is_empty() {
            continue;
        }

        if is_subtitle_timestamp(tag) {
            output.push(' ');
        } else if is_subtitle_tag(tag) {
            continue;
        } else {
            output.push('<');
            output.push_str(tag);
            output.push('>');
        }
    }
    output
}

fn is_subtitle_tag(tag: &str) -> bool {
    let tag = tag.trim_start_matches('/');
    let name = tag
        .split(|ch: char| ch.is_whitespace() || ch == '.')
        .next()
        .unwrap_or_default();
    matches!(name, "b" | "c" | "i" | "lang" | "rt" | "ruby" | "u" | "v")
}

fn is_subtitle_timestamp(value: &str) -> bool {
    let value = value.replace(',', ".");
    let parts = value.split(':').collect::<Vec<_>>();
    let [hours, minutes, seconds] = parts.as_slice() else {
        return false;
    };
    is_two_digits(hours)
        && is_two_digits(minutes)
        && seconds.len() == 6
        && seconds.as_bytes().get(2) == Some(&b'.')
        && seconds[..2].bytes().all(|byte| byte.is_ascii_digit())
        && seconds[3..].bytes().all(|byte| byte.is_ascii_digit())
}

fn is_two_digits(value: &str) -> bool {
    value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn decode_basic_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
