use std::collections::HashSet;

use url::Url;

use super::types::{DiscoveredYouTubeTarget, DiscoveryKind};

pub fn canonicalize_youtube_target(raw_url: &str) -> Option<DiscoveredYouTubeTarget> {
    let raw_url = trim_url_token(raw_url);
    let url = Url::parse(raw_url).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();

    if host == "youtu.be" {
        let video_id = url.path_segments()?.find(|segment| !segment.is_empty())?;
        return video_target(video_id);
    }

    let is_youtube = matches!(
        host.as_str(),
        "youtube.com"
            | "www.youtube.com"
            | "m.youtube.com"
            | "music.youtube.com"
            | "youtube-nocookie.com"
            | "www.youtube-nocookie.com"
    );
    if !is_youtube {
        return None;
    }

    let segments = url
        .path_segments()
        .map(|parts| parts.filter(|part| !part.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();

    if segments.first() == Some(&"watch") {
        if let Some(video_id) = url
            .query_pairs()
            .find_map(|(key, value)| (key == "v").then(|| value.into_owned()))
        {
            return video_target(&video_id);
        }
        if let Some(playlist_id) = url
            .query_pairs()
            .find_map(|(key, value)| (key == "list").then(|| value.into_owned()))
        {
            return playlist_target(&playlist_id);
        }
    }

    if segments.first() == Some(&"playlist") {
        let playlist_id = url
            .query_pairs()
            .find_map(|(key, value)| (key == "list").then(|| value.into_owned()))?;
        return playlist_target(&playlist_id);
    }

    if matches!(
        segments.first().copied(),
        Some("shorts" | "live" | "embed")
    ) {
        return segments.get(1).and_then(|video_id| video_target(video_id));
    }

    match segments.as_slice() {
        ["channel", channel_id, ..] => channel_target(&format!("channel/{channel_id}")),
        ["c", channel_name, ..] => channel_target(&format!("c/{channel_name}")),
        ["user", user_name, ..] => channel_target(&format!("user/{user_name}")),
        [handle, ..] if handle.starts_with('@') => channel_target(handle),
        _ => None,
    }
}

pub fn extract_youtube_targets(text: &str) -> Vec<DiscoveredYouTubeTarget> {
    let mut seen = HashSet::new();
    text.split_whitespace()
        .filter_map(canonicalize_youtube_target)
        .filter(|target| seen.insert((target.kind, target.canonical_key.clone())))
        .collect()
}

fn trim_url_token(value: &str) -> &str {
    value.trim_matches(|character: char| {
        matches!(
            character,
            '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '"' | '\'' | ',' | ';' | '!' | '?'
        ) || character == '.'
    })
}

fn video_target(video_id: &str) -> Option<DiscoveredYouTubeTarget> {
    let video_id = clean_identifier(video_id)?;
    Some(DiscoveredYouTubeTarget {
        kind: DiscoveryKind::Video,
        canonical_key: format!("youtube:video:{video_id}"),
        target_url: format!("https://www.youtube.com/watch?v={video_id}"),
    })
}

fn playlist_target(playlist_id: &str) -> Option<DiscoveredYouTubeTarget> {
    let playlist_id = clean_identifier(playlist_id)?;
    Some(DiscoveredYouTubeTarget {
        kind: DiscoveryKind::Playlist,
        canonical_key: format!("youtube:playlist:{playlist_id}"),
        target_url: format!("https://www.youtube.com/playlist?list={playlist_id}"),
    })
}

fn channel_target(channel_path: &str) -> Option<DiscoveredYouTubeTarget> {
    let channel_path = channel_path.trim_matches('/').trim();
    if channel_path.is_empty() {
        return None;
    }
    let canonical_path = channel_path.to_ascii_lowercase();
    Some(DiscoveredYouTubeTarget {
        kind: DiscoveryKind::Channel,
        canonical_key: format!("youtube:channel:{canonical_path}"),
        target_url: format!("https://www.youtube.com/{channel_path}"),
    })
}

fn clean_identifier(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')))
    .then_some(value)
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_youtube_target, extract_youtube_targets};
    use crate::discovery::DiscoveryKind;

    #[test]
    fn canonicalizes_supported_youtube_urls() {
        let cases = [
            (
                "https://youtu.be/abc_123?t=3",
                DiscoveryKind::Video,
                "youtube:video:abc_123",
            ),
            (
                "https://www.youtube.com/shorts/short-1",
                DiscoveryKind::Video,
                "youtube:video:short-1",
            ),
            (
                "https://youtube.com/playlist?list=PL_123",
                DiscoveryKind::Playlist,
                "youtube:playlist:PL_123",
            ),
            (
                "https://www.youtube.com/@ExampleCreator/videos",
                DiscoveryKind::Channel,
                "youtube:channel:@examplecreator",
            ),
        ];
        for (url, kind, key) in cases {
            let target = canonicalize_youtube_target(url).unwrap();
            assert_eq!(target.kind, kind);
            assert_eq!(target.canonical_key, key);
        }
    }

    #[test]
    fn extracts_and_deduplicates_youtube_links() {
        let targets = extract_youtube_targets(
            "See (https://youtu.be/abc_123), https://www.youtube.com/watch?v=abc_123 and https://www.youtube.com/playlist?list=PL_1.",
        );
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].canonical_key, "youtube:video:abc_123");
        assert_eq!(targets[1].canonical_key, "youtube:playlist:PL_1");
    }
}
