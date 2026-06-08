pub use video_analysis_youtube::{
    local_file_item, single_url_item, stable_child_id, YoutubeVideoItem as VideoItem,
    YoutubeVideoMetadata as VideoMetadata,
};

use crate::config::YtDlpConfig;

pub async fn discover_collection(
    url: &str,
    max_items: Option<u64>,
    yt_dlp: &YtDlpConfig,
) -> anyhow::Result<Vec<VideoItem>> {
    video_analysis_youtube::discover_youtube_collection(url, max_items, yt_dlp)
        .await
        .map_err(Into::into)
}

pub async fn enrich_video_metadata(
    item: &mut VideoItem,
    metadata_dir: &std::path::Path,
    yt_dlp: &YtDlpConfig,
) -> anyhow::Result<()> {
    video_analysis_youtube::enrich_youtube_metadata(item, metadata_dir, yt_dlp)
        .await
        .map_err(Into::into)
}

pub async fn download_video(
    item: &mut VideoItem,
    media_dir: &std::path::Path,
    yt_dlp: &YtDlpConfig,
) -> anyhow::Result<std::path::PathBuf> {
    video_analysis_youtube::download_youtube_media(item, media_dir, yt_dlp)
        .await
        .map_err(Into::into)
}

pub fn filter_item(
    item: &VideoItem,
    title_contains: &Option<String>,
    title_excludes: &[String],
    duration_min: Option<f64>,
    duration_max: Option<f64>,
) -> bool {
    video_analysis_youtube::filter_youtube_item(
        item,
        &video_analysis_youtube::YoutubeItemFilter {
            title_contains: title_contains.clone(),
            title_excludes: title_excludes.to_vec(),
            duration_min,
            duration_max,
        },
    )
}

pub fn stable_video_id(source_url: &str) -> uuid::Uuid {
    video_analysis_youtube::stable_youtube_source_id(source_url)
}

pub fn require_command(command: &str) -> anyhow::Result<()> {
    video_analysis_youtube::source::require_command(command).map_err(Into::into)
}
