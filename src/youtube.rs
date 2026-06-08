use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::YtDlpConfig;
use crate::yt_dlp::YtDlpClient;

#[derive(Debug, Clone)]
pub struct VideoItem {
    pub item_id: String,
    pub youtube_id: Option<String>,
    pub title: Option<String>,
    pub source_url: String,
    pub duration_seconds: Option<f64>,
    pub upload_date: Option<String>,
    pub metadata: VideoMetadata,
    pub local_video_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VideoMetadata {
    pub youtube_id: Option<String>,
    pub title: Option<String>,
    pub webpage_url: Option<String>,
    pub original_url: Option<String>,
    pub duration_seconds: Option<f64>,
    pub upload_date: Option<String>,
    pub description: Option<String>,
    pub channel: Option<String>,
    pub channel_id: Option<String>,
    pub channel_url: Option<String>,
    pub uploader: Option<String>,
    pub uploader_id: Option<String>,
    pub uploader_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration_string: Option<String>,
    pub timestamp: Option<i64>,
    pub release_timestamp: Option<i64>,
    pub view_count: Option<i64>,
    pub like_count: Option<i64>,
    pub comment_count: Option<i64>,
    pub live_status: Option<String>,
    pub availability: Option<String>,
    pub age_limit: Option<i64>,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub yt_dlp_version: Option<String>,
    pub metadata_downloaded_at: Option<String>,
    pub metadata_source: Option<String>,
    pub yt_dlp_args_redacted: Vec<String>,
    pub media_probe: Option<MediaProbeMetadata>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaProbeMetadata {
    pub input: String,
    pub path: Option<PathBuf>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<String>,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct YtDlpCollectionJson {
    entries: Option<Vec<Option<YtDlpEntryJson>>>,
}

#[derive(Debug, Deserialize)]
struct YtDlpEntryJson {
    #[serde(rename = "_type")]
    entry_type: Option<String>,
    ie_key: Option<String>,
    id: Option<String>,
    title: Option<String>,
    url: Option<String>,
    webpage_url: Option<String>,
    original_url: Option<String>,
    duration: Option<f64>,
    duration_string: Option<String>,
    upload_date: Option<String>,
    timestamp: Option<i64>,
    release_timestamp: Option<i64>,
    description: Option<String>,
    channel: Option<String>,
    channel_id: Option<String>,
    channel_url: Option<String>,
    uploader: Option<String>,
    uploader_id: Option<String>,
    uploader_url: Option<String>,
    thumbnail: Option<String>,
    view_count: Option<i64>,
    like_count: Option<i64>,
    comment_count: Option<i64>,
    live_status: Option<String>,
    availability: Option<String>,
    age_limit: Option<i64>,
    categories: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    entries: Option<Vec<Option<YtDlpEntryJson>>>,
}

impl VideoMetadata {
    fn from_yt_dlp_entry(entry: &YtDlpEntryJson) -> Self {
        Self {
            youtube_id: clean_string(entry.id.clone()),
            title: clean_string(entry.title.clone()),
            webpage_url: clean_string(entry.webpage_url.clone()),
            original_url: clean_string(entry.original_url.clone()),
            duration_seconds: entry.duration,
            upload_date: clean_string(entry.upload_date.clone()),
            description: clean_string(entry.description.clone()),
            channel: clean_string(entry.channel.clone()),
            channel_id: clean_string(entry.channel_id.clone()),
            channel_url: clean_string(entry.channel_url.clone()),
            uploader: clean_string(entry.uploader.clone()),
            uploader_id: clean_string(entry.uploader_id.clone()),
            uploader_url: clean_string(entry.uploader_url.clone()),
            thumbnail_url: clean_string(entry.thumbnail.clone()),
            duration_string: clean_string(entry.duration_string.clone()),
            timestamp: entry.timestamp,
            release_timestamp: entry.release_timestamp,
            view_count: entry.view_count,
            like_count: entry.like_count,
            comment_count: entry.comment_count,
            live_status: clean_string(entry.live_status.clone()),
            availability: clean_string(entry.availability.clone()),
            age_limit: entry.age_limit,
            categories: clean_strings(entry.categories.clone().unwrap_or_default()),
            tags: clean_strings(entry.tags.clone().unwrap_or_default()),
            yt_dlp_version: None,
            metadata_downloaded_at: None,
            metadata_source: None,
            yt_dlp_args_redacted: Vec::new(),
            media_probe: None,
        }
    }
}

pub async fn discover_collection(
    url: &str,
    max_items: Option<u64>,
    yt_dlp: &YtDlpConfig,
) -> anyhow::Result<Vec<VideoItem>> {
    let client = YtDlpClient::new(yt_dlp.clone());
    let (stdout, _) = client.discover_collection_json(url, max_items).await?;
    parse_collection_json(&stdout, max_items)
}

fn parse_collection_json(bytes: &[u8], max_items: Option<u64>) -> anyhow::Result<Vec<VideoItem>> {
    let parsed: YtDlpCollectionJson = serde_json::from_slice(bytes)?;
    let limit = max_items.unwrap_or(u64::MAX) as usize;
    let mut items = Vec::new();
    collect_video_items(parsed.entries.unwrap_or_default(), limit, &mut items);
    Ok(items)
}

fn collect_video_items(
    entries: Vec<Option<YtDlpEntryJson>>,
    limit: usize,
    items: &mut Vec<VideoItem>,
) {
    for mut entry in entries.into_iter().flatten() {
        if items.len() >= limit {
            return;
        }
        if let Some(children) = entry.entries.take() {
            collect_video_items(children, limit, items);
            continue;
        }
        let Some(source_url) = source_url_from_entry(&entry) else {
            continue;
        };
        let fallback_id = format!("item-{}", items.len() + 1);
        let item_id = entry
            .id
            .as_deref()
            .map(sanitize_id)
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback_id);
        let metadata = VideoMetadata::from_yt_dlp_entry(&entry);
        items.push(VideoItem {
            item_id,
            youtube_id: entry.id,
            title: entry.title,
            source_url,
            duration_seconds: entry.duration,
            upload_date: entry.upload_date,
            metadata,
            local_video_path: None,
        });
    }
}

pub fn single_url_item(url: String) -> VideoItem {
    VideoItem {
        item_id: sanitize_id(&url),
        youtube_id: None,
        title: None,
        source_url: url,
        duration_seconds: None,
        upload_date: None,
        metadata: VideoMetadata::default(),
        local_video_path: None,
    }
}

pub fn local_file_item(path: PathBuf) -> VideoItem {
    let item_id = path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(sanitize_id)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "local-file".to_string());
    VideoItem {
        item_id,
        youtube_id: None,
        title: path
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_string),
        source_url: path.to_string_lossy().into_owned(),
        duration_seconds: None,
        upload_date: None,
        metadata: VideoMetadata::default(),
        local_video_path: Some(path),
    }
}

pub async fn enrich_video_metadata(
    item: &mut VideoItem,
    metadata_dir: &Path,
    yt_dlp: &YtDlpConfig,
) -> anyhow::Result<()> {
    if item.local_video_path.is_some() {
        return Ok(());
    }
    tokio::fs::create_dir_all(metadata_dir).await?;
    let client = YtDlpClient::new(yt_dlp.clone());
    let (stdout, report) = client.fetch_metadata_json(&item.source_url).await?;
    let entry: YtDlpEntryJson = serde_json::from_slice(&stdout)?;
    item.merge_yt_dlp_entry(&entry);
    item.metadata.yt_dlp_version = report.yt_dlp_version;
    item.metadata.metadata_downloaded_at = Some(chrono::Utc::now().to_rfc3339());
    item.metadata.metadata_source = Some("yt-dlp --dump-json".to_string());
    item.metadata.yt_dlp_args_redacted = report.args_redacted;

    let metadata_path = metadata_dir.join(format!("{}.metadata.json", item.item_id));
    let metadata = serde_json::to_vec_pretty(&item.metadata)?;
    tokio::fs::write(metadata_path, metadata).await?;
    Ok(())
}

impl VideoItem {
    fn merge_yt_dlp_entry(&mut self, entry: &YtDlpEntryJson) {
        if let Some(id) = clean_string(entry.id.clone()) {
            self.youtube_id = Some(id);
        }
        if let Some(title) = clean_string(entry.title.clone()) {
            self.title = Some(title);
        }
        if entry.duration.is_some() {
            self.duration_seconds = entry.duration;
        }
        if let Some(upload_date) = clean_string(entry.upload_date.clone()) {
            self.upload_date = Some(upload_date);
        }
        self.metadata = VideoMetadata::from_yt_dlp_entry(entry);
    }
}

pub async fn download_video(
    item: &mut VideoItem,
    media_dir: &Path,
    yt_dlp: &YtDlpConfig,
) -> anyhow::Result<PathBuf> {
    if let Some(path) = &item.local_video_path {
        return Ok(path.clone());
    }
    tokio::fs::create_dir_all(media_dir).await?;
    let temp_dir = media_dir.join(format!(".tmp-{}", item.item_id));
    if temp_dir.exists() {
        tokio::fs::remove_dir_all(&temp_dir).await?;
    }
    tokio::fs::create_dir_all(&temp_dir).await?;
    let output_template = temp_dir.join(format!("{}-%(id)s.%(ext)s", item.item_id));
    let archive = media_dir.parent().and_then(Path::parent).map(|work_dir| {
        work_dir
            .join("archives")
            .join("yt-dlp-media-download-archive.txt")
    });
    if let Some(archive) = archive.as_ref().and_then(|path| path.parent()) {
        tokio::fs::create_dir_all(archive).await?;
    }

    let client = YtDlpClient::new(yt_dlp.clone());
    let (stdout, report) = client
        .download_media(&item.source_url, &output_template, archive)
        .await?;
    let printed_path = printed_download_path(&stdout)
        .ok_or_else(|| anyhow::anyhow!("yt-dlp completed but did not print after_move:filepath"))?;
    let temp_root = temp_dir.canonicalize()?;
    let canonical_path = printed_path.canonicalize()?;
    if !canonical_path.starts_with(&temp_root) {
        anyhow::bail!(
            "yt-dlp reported output outside temporary directory: {}",
            canonical_path.display()
        );
    }
    let probe = probe_downloaded_media(&canonical_path)?;
    let file_name = canonical_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("yt-dlp reported output without a file name"))?;
    let final_path = media_dir.join(file_name);
    if final_path.exists() {
        tokio::fs::remove_file(&final_path).await?;
    }
    tokio::fs::rename(&canonical_path, &final_path).await?;

    if let Some(info_path) = info_json_path(&canonical_path) {
        if info_path.exists() {
            if let Ok(bytes) = tokio::fs::read(&info_path).await {
                if let Ok(entry) = serde_json::from_slice::<YtDlpEntryJson>(&bytes) {
                    item.merge_yt_dlp_entry(&entry);
                }
            }
            if let Some(file_name) = info_path.file_name() {
                let final_info_path = media_dir.join(file_name);
                if final_info_path.exists() {
                    tokio::fs::remove_file(&final_info_path).await?;
                }
                tokio::fs::rename(&info_path, final_info_path).await?;
            }
        }
    }
    item.metadata.yt_dlp_version = report.yt_dlp_version;
    item.metadata.metadata_downloaded_at = Some(chrono::Utc::now().to_rfc3339());
    item.metadata.metadata_source = Some("yt-dlp --write-info-json".to_string());
    item.metadata.yt_dlp_args_redacted = report.args_redacted;
    item.metadata.media_probe = Some(probe);
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    Ok(final_path)
}

pub fn filter_item(
    item: &VideoItem,
    title_contains: &Option<String>,
    title_excludes: &[String],
    duration_min: Option<f64>,
    duration_max: Option<f64>,
) -> bool {
    let title = item.title.as_deref().unwrap_or("").to_ascii_lowercase();
    if let Some(needle) = title_contains {
        if !title.contains(&needle.to_ascii_lowercase()) {
            return false;
        }
    }
    if title_excludes
        .iter()
        .any(|needle| !needle.is_empty() && title.contains(&needle.to_ascii_lowercase()))
    {
        return false;
    }
    if let Some(min) = duration_min {
        if item
            .duration_seconds
            .map(|value| value < min)
            .unwrap_or(false)
        {
            return false;
        }
    }
    if let Some(max) = duration_max {
        if item
            .duration_seconds
            .map(|value| value > max)
            .unwrap_or(false)
        {
            return false;
        }
    }
    true
}

pub fn stable_video_id(source_url: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, source_url.as_bytes())
}

pub fn stable_child_id(parent: uuid::Uuid, label: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&parent, label.as_bytes())
}

pub fn require_command(command: &str) -> anyhow::Result<()> {
    if resolve_command(command).is_some() {
        Ok(())
    } else {
        anyhow::bail!("required command `{command}` was not found on PATH")
    }
}

fn resolve_command(command: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(command))
            .find(|path| path.is_file())
    })
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

fn probe_downloaded_media(path: &Path) -> anyhow::Result<MediaProbeMetadata> {
    let metadata = video_analysis_ffmpeg::probe(path)?;
    if metadata.duration_seconds.unwrap_or(0.0) <= 0.0 {
        anyhow::bail!(
            "ffprobe returned no positive duration for {}",
            path.display()
        );
    }
    if metadata.width == 0 || metadata.height == 0 {
        anyhow::bail!("ffprobe returned no video stream for {}", path.display());
    }
    Ok(MediaProbeMetadata {
        input: metadata.input,
        path: metadata.path,
        width: Some(metadata.width),
        height: Some(metadata.height),
        frame_rate: Some(format!(
            "{}/{}",
            metadata.frame_rate.numer(),
            metadata.frame_rate.denom()
        )),
        duration_seconds: metadata.duration_seconds,
    })
}

fn source_url_from_entry(entry: &YtDlpEntryJson) -> Option<String> {
    if !is_video_entry(entry) {
        return None;
    }
    for value in [&entry.webpage_url, &entry.original_url, &entry.url]
        .into_iter()
        .flatten()
    {
        if is_youtube_video_url(value) {
            return Some(value.clone());
        }
    }
    if let Some(id) = entry.id.as_deref().or(entry.url.as_deref()) {
        if is_probable_youtube_video_id(id) {
            return Some(format!("https://www.youtube.com/watch?v={id}"));
        }
    }
    None
}

fn is_video_entry(entry: &YtDlpEntryJson) -> bool {
    entry.ie_key.as_deref() == Some("Youtube")
        || entry.entry_type.as_deref() == Some("url")
        || entry
            .webpage_url
            .as_deref()
            .is_some_and(is_youtube_video_url)
        || entry.url.as_deref().is_some_and(is_youtube_video_url)
        || entry
            .id
            .as_deref()
            .is_some_and(is_probable_youtube_video_id)
}

fn is_youtube_video_url(value: &str) -> bool {
    value.starts_with("https://www.youtube.com/watch?")
        || value.starts_with("https://youtube.com/watch?")
        || value.starts_with("https://youtu.be/")
        || value.starts_with("https://www.youtube.com/shorts/")
        || value.starts_with("https://youtube.com/shorts/")
}

fn is_probable_youtube_video_id(value: &str) -> bool {
    value.len() == 11
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn sanitize_id(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    while sanitized.contains("--") {
        sanitized = sanitized.replace("--", "-");
    }
    if sanitized.is_empty() {
        "item".to_string()
    } else {
        sanitized
    }
}

fn clean_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn clean_strings(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .filter_map(|value| clean_string(Some(value)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_video_entries() {
        let json = br#"{
            "entries": [
                {
                    "_type": "url",
                    "ie_key": "Youtube",
                    "id": "vA8-T5R8zDY",
                    "title": "Documentary",
                    "url": "https://www.youtube.com/watch?v=vA8-T5R8zDY",
                    "duration": 42730.0
                }
            ]
        }"#;

        let items = parse_collection_json(json, None).unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].youtube_id.as_deref(), Some("vA8-T5R8zDY"));
        assert_eq!(
            items[0].source_url,
            "https://www.youtube.com/watch?v=vA8-T5R8zDY"
        );
    }

    #[test]
    fn flattens_channel_tab_entries_and_ignores_collection_rows() {
        let json = br#"{
            "entries": [
                {
                    "_type": "playlist",
                    "id": "UCyvKnffD7Mh8t7kPMh70yIw",
                    "title": "Distinguo - Videos",
                    "entries": [
                        {
                            "_type": "url",
                            "ie_key": "Youtube",
                            "id": "P316q0D4q0Y",
                            "title": "A Complete DESTRUCTION of FAITH ALONE",
                            "url": "https://www.youtube.com/watch?v=P316q0D4q0Y"
                        }
                    ]
                },
                {
                    "_type": "playlist",
                    "id": "UCyvKnffD7Mh8t7kPMh70yIw",
                    "title": "Distinguo - Shorts"
                }
            ]
        }"#;

        let items = parse_collection_json(json, None).unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].youtube_id.as_deref(), Some("P316q0D4q0Y"));
        assert_eq!(
            items[0].source_url,
            "https://www.youtube.com/watch?v=P316q0D4q0Y"
        );
    }

    #[test]
    fn respects_collection_limit_across_nested_entries() {
        let json = br#"{
            "entries": [
                {
                    "entries": [
                        {"_type": "url", "ie_key": "Youtube", "id": "aaaaaaaaaaa"},
                        {"_type": "url", "ie_key": "Youtube", "id": "bbbbbbbbbbb"}
                    ]
                }
            ]
        }"#;

        let items = parse_collection_json(json, Some(1)).unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].source_url,
            "https://www.youtube.com/watch?v=aaaaaaaaaaa"
        );
    }

    #[test]
    fn parses_important_video_metadata_fields() {
        let entry: YtDlpEntryJson = serde_json::from_slice(
            br#"{
            "id": "vA8-T5R8zDY",
            "title": "Me at the zoo",
            "description": "First YouTube upload",
            "channel": "jawed",
            "channel_id": "UC4QobU6STFB0P71PMvOGN5A",
            "channel_url": "https://www.youtube.com/channel/UC4QobU6STFB0P71PMvOGN5A",
            "uploader": "jawed",
            "uploader_id": "jawed",
            "thumbnail": "https://i.ytimg.com/vi/vA8-T5R8zDY/hqdefault.jpg",
            "duration": 19.0,
            "duration_string": "19",
            "upload_date": "20050424",
            "timestamp": 1114329600,
            "view_count": 123,
            "like_count": 45,
            "comment_count": 6,
            "live_status": "not_live",
            "availability": "public",
            "age_limit": 0,
            "categories": ["People & Blogs"],
            "tags": ["zoo", "youtube"]
        }"#,
        )
        .unwrap();

        let metadata = VideoMetadata::from_yt_dlp_entry(&entry);

        assert_eq!(metadata.youtube_id.as_deref(), Some("vA8-T5R8zDY"));
        assert_eq!(metadata.title.as_deref(), Some("Me at the zoo"));
        assert_eq!(metadata.duration_seconds, Some(19.0));
        assert_eq!(metadata.upload_date.as_deref(), Some("20050424"));
        assert_eq!(metadata.channel.as_deref(), Some("jawed"));
        assert_eq!(
            metadata.channel_id.as_deref(),
            Some("UC4QobU6STFB0P71PMvOGN5A")
        );
        assert_eq!(metadata.view_count, Some(123));
        assert_eq!(metadata.categories, vec!["People & Blogs"]);
        assert_eq!(metadata.tags, vec!["zoo", "youtube"]);
    }

    #[test]
    fn merges_downloaded_metadata_into_video_item() {
        let mut item = single_url_item("https://www.youtube.com/watch?v=vA8-T5R8zDY".to_string());
        let entry: YtDlpEntryJson = serde_json::from_slice(
            br#"{
            "id": "vA8-T5R8zDY",
            "title": "Me at the zoo",
            "duration": 19.0,
            "upload_date": "20050424",
            "channel": "jawed",
            "view_count": 123
        }"#,
        )
        .unwrap();

        item.merge_yt_dlp_entry(&entry);

        assert_eq!(item.youtube_id.as_deref(), Some("vA8-T5R8zDY"));
        assert_eq!(item.title.as_deref(), Some("Me at the zoo"));
        assert_eq!(item.duration_seconds, Some(19.0));
        assert_eq!(item.upload_date.as_deref(), Some("20050424"));
        assert_eq!(item.metadata.channel.as_deref(), Some("jawed"));
        assert_eq!(item.metadata.view_count, Some(123));
    }

    #[test]
    fn parses_last_printed_download_path() {
        let path = printed_download_path(b"[download] progress\n/tmp/video.mp4\n").unwrap();

        assert_eq!(path, PathBuf::from("/tmp/video.mp4"));
    }

    #[test]
    fn rejects_missing_printed_download_path() {
        assert!(printed_download_path(b"\n\n").is_none());
    }

    #[test]
    fn builds_info_json_path_next_to_media() {
        assert_eq!(
            info_json_path(Path::new("/tmp/video.mp4")).unwrap(),
            PathBuf::from("/tmp/video.info.json")
        );
    }
}
