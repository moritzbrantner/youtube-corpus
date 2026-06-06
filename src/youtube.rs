use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Deserialize;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct VideoItem {
    pub item_id: String,
    pub youtube_id: Option<String>,
    pub title: Option<String>,
    pub source_url: String,
    pub duration_seconds: Option<f64>,
    pub upload_date: Option<String>,
    pub local_video_path: Option<PathBuf>,
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
    upload_date: Option<String>,
    entries: Option<Vec<Option<YtDlpEntryJson>>>,
}

pub async fn discover_collection(
    url: &str,
    max_items: Option<u64>,
) -> anyhow::Result<Vec<VideoItem>> {
    require_command("yt-dlp")?;
    let mut command = Command::new("yt-dlp");
    command.arg("--flat-playlist");
    if let Some(max_items) = max_items.filter(|value| *value > 0) {
        command.arg("--playlist-end").arg(max_items.to_string());
    }
    let output = command
        .arg("-J")
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .await?;
    if !output.status.success() {
        anyhow::bail!(
            "yt-dlp collection discovery failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_collection_json(&output.stdout, max_items)
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
        items.push(VideoItem {
            item_id,
            youtube_id: entry.id,
            title: entry.title,
            source_url,
            duration_seconds: entry.duration,
            upload_date: entry.upload_date,
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
        local_video_path: Some(path),
    }
}

pub async fn download_video(item: &VideoItem, media_dir: &Path) -> anyhow::Result<PathBuf> {
    if let Some(path) = &item.local_video_path {
        return Ok(path.clone());
    }
    require_command("yt-dlp")?;
    tokio::fs::create_dir_all(media_dir).await?;
    let output_template = media_dir.join(format!("{}-%(id)s.%(ext)s", item.item_id));
    let output = Command::new("yt-dlp")
        .arg("--no-playlist")
        .arg("--merge-output-format")
        .arg("mp4")
        .arg("--print")
        .arg("after_move:filepath")
        .arg("-o")
        .arg(&output_template)
        .arg(&item.source_url)
        .stdin(Stdio::null())
        .output()
        .await?;
    if !output.status.success() {
        anyhow::bail!(
            "yt-dlp failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    for line in String::from_utf8_lossy(&output.stdout).lines().rev() {
        let path = PathBuf::from(line.trim());
        if path.exists() {
            return Ok(path);
        }
    }
    find_video(media_dir).ok_or_else(|| anyhow::anyhow!("yt-dlp completed but no video was found"))
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

fn find_video(dir: &Path) -> Option<PathBuf> {
    let mut entries = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("mp4" | "mkv" | "webm" | "mov")
            )
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries.pop()
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
}
