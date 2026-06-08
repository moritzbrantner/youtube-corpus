use std::path::{Path, PathBuf};

use crate::config::{CaptionConfig, SourceKind, YtDlpConfig};
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
    yt_dlp: &YtDlpConfig,
) -> anyhow::Result<Vec<TranscriptStream>> {
    let tracks = video_analysis_youtube::download_youtube_captions(
        item,
        dir,
        video_analysis_youtube::CaptionDownloadRequest {
            enabled: config.enabled,
            include_auto_captions: config.include_auto_captions,
            languages: config.languages.clone(),
        },
        yt_dlp,
    )
    .await?;
    Ok(tracks.into_iter().map(track_to_stream).collect())
}

pub async fn parse_caption_files(dir: &Path) -> anyhow::Result<Vec<TranscriptStream>> {
    let tracks = video_analysis_youtube::parse_youtube_caption_files(dir).await?;
    Ok(tracks.into_iter().map(track_to_stream).collect())
}

fn track_to_stream(track: video_analysis_youtube::YoutubeCaptionTrack) -> TranscriptStream {
    let source_kind = match track.kind {
        video_analysis_youtube::YoutubeCaptionKind::Manual => SourceKind::CaptionManual,
        video_analysis_youtube::YoutubeCaptionKind::Auto => SourceKind::CaptionAuto,
    };
    TranscriptStream {
        source_kind,
        language: track.language.or(track.transcript.language.clone()),
        source_path: Some(track.source_path),
        text: track.transcript.text.clone(),
        segments: track.transcript.segments,
        message: None,
    }
}
