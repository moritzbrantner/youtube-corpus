use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::captions::{download_and_parse_captions, TranscriptStream};
use crate::config::{CaptionConfig, SourceKind, YtDlpConfig};
use crate::youtube::{enrich_video_metadata, single_url_item};

#[derive(Clone)]
struct BridgeState {
    yt_dlp: YtDlpConfig,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptionBridgeRequest {
    source_url: String,
    #[serde(default)]
    preferred_languages: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptionBridgeResponse {
    video_id: Option<String>,
    transcript_text: String,
    track: CaptionBridgeTrack,
    player: CaptionBridgePlayer,
    yt_dlp_profile: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptionBridgeTrack {
    language_code: String,
    name: String,
    source_kind: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptionBridgePlayer {
    playability_status: String,
    title: Option<String>,
    author: Option<String>,
    channel_id: Option<String>,
    duration_seconds: Option<f64>,
    view_count: Option<i64>,
    description: Option<String>,
    thumbnail_url: Option<String>,
}

#[derive(Debug, Serialize)]
struct BridgeErrorBody {
    error: BridgeErrorMessage,
}

#[derive(Debug, Serialize)]
struct BridgeErrorMessage {
    message: String,
}

type BridgeError = (StatusCode, Json<BridgeErrorBody>);

struct CaptionAttempt {
    profile: String,
    streams: Vec<TranscriptStream>,
}

pub fn router(yt_dlp: YtDlpConfig) -> Router {
    Router::new()
        .route("/api/youtube-captions", post(fetch_youtube_captions))
        .with_state(BridgeState { yt_dlp })
}

async fn fetch_youtube_captions(
    State(state): State<BridgeState>,
    Json(input): Json<CaptionBridgeRequest>,
) -> Result<Json<CaptionBridgeResponse>, BridgeError> {
    if youtube_video_id(&input.source_url).is_none() {
        return Err(bridge_error(
            StatusCode::BAD_REQUEST,
            "sourceUrl must be an HTTPS YouTube video URL.",
        ));
    }

    let preferred_languages = normalize_languages(&input.preferred_languages);
    let work_dir =
        std::env::temp_dir().join(format!("youtube-corpus-caption-bridge-{}", Uuid::new_v4()));
    let result = extract_with_yt_dlp(
        &input.source_url,
        &preferred_languages,
        &work_dir,
        &state.yt_dlp,
    )
    .await;
    let _ = tokio::fs::remove_dir_all(&work_dir).await;

    result.map(Json).map_err(|message| {
        let status = if message.contains("not found on PATH") || message.contains("MissingBinary") {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::BAD_GATEWAY
        };
        bridge_error(status, message)
    })
}

async fn extract_with_yt_dlp(
    source_url: &str,
    preferred_languages: &[String],
    work_dir: &std::path::Path,
    yt_dlp: &YtDlpConfig,
) -> Result<CaptionBridgeResponse, String> {
    let metadata_dir = work_dir.join("metadata");
    let captions_dir = work_dir.join("captions");
    let mut item = single_url_item(source_url.to_string());

    let metadata_error = enrich_video_metadata(&mut item, &metadata_dir, yt_dlp)
        .await
        .err()
        .map(|error| error.to_string());

    let caption_config = CaptionConfig {
        enabled: true,
        include_auto_captions: true,
        languages: preferred_languages.to_vec(),
    };
    let caption_attempt = download_caption_profiles(&item, &captions_dir, &caption_config, yt_dlp)
        .await
        .map_err(|mut messages| {
            if let Some(error) = metadata_error {
                messages.push(format!("metadata unavailable: {error}"));
            }
            if messages.is_empty() {
                "yt-dlp completed without a usable manual or automatic caption file.".to_string()
            } else {
                messages.join(" · ")
            }
        })?;

    let selected = select_caption_stream(&caption_attempt.streams, preferred_languages)
        .ok_or_else(|| "yt-dlp produced caption files but none were usable.".to_string())?;
    let source_path = selected
        .source_path
        .as_ref()
        .ok_or_else(|| "selected caption stream has no source file".to_string())?;
    let transcript_text = tokio::fs::read_to_string(source_path)
        .await
        .map_err(|error| format!("failed to read yt-dlp caption output: {error}"))?;
    if transcript_text.trim().is_empty() {
        return Err("yt-dlp returned an empty caption file.".to_string());
    }

    let language = selected
        .language
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let source_kind = selected.source_kind.as_str().to_string();
    let track_name = match selected.source_kind {
        SourceKind::CaptionManual => format!("{language} manual captions"),
        SourceKind::CaptionAuto => format!("{language} automatic captions"),
        SourceKind::Asr => format!("{language} ASR"),
    };
    let metadata = &item.metadata;

    Ok(CaptionBridgeResponse {
        video_id: item.youtube_id.clone(),
        transcript_text,
        track: CaptionBridgeTrack {
            language_code: language,
            name: track_name,
            source_kind,
        },
        player: CaptionBridgePlayer {
            playability_status: "OK".to_string(),
            title: item.title.clone().or_else(|| metadata.title.clone()),
            author: metadata
                .channel
                .clone()
                .or_else(|| metadata.uploader.clone()),
            channel_id: metadata.channel_id.clone(),
            duration_seconds: item.duration_seconds.or(metadata.duration_seconds),
            view_count: metadata.view_count,
            description: metadata.description.clone(),
            thumbnail_url: metadata.thumbnail_url.clone(),
        },
        yt_dlp_profile: caption_attempt.profile,
    })
}

async fn download_caption_profiles(
    item: &crate::youtube::VideoItem,
    captions_dir: &std::path::Path,
    caption_config: &CaptionConfig,
    yt_dlp: &YtDlpConfig,
) -> Result<CaptionAttempt, Vec<String>> {
    let mut messages = Vec::new();
    for (index, (profile, config)) in caption_profiles(yt_dlp).into_iter().enumerate() {
        let attempt_dir = captions_dir.join(format!("{index}-{profile}"));
        match download_and_parse_captions(item, &attempt_dir, caption_config, &config).await {
            Ok(streams) => {
                if streams.iter().any(|stream| stream.source_path.is_some()) {
                    return Ok(CaptionAttempt { profile, streams });
                }
                messages.extend(streams.into_iter().filter_map(|stream| {
                    stream
                        .message
                        .map(|message| format!("{profile}: {message}"))
                }));
            }
            Err(error) => messages.push(format!("{profile}: {error}")),
        }
    }
    Err(messages)
}

fn caption_profiles(config: &YtDlpConfig) -> Vec<(String, YtDlpConfig)> {
    let mut profiles = vec![("default".to_string(), config.clone())];
    if has_explicit_youtube_player_client(&config.args) {
        return profiles;
    }

    for client in ["web_embedded", "mweb", "tv"] {
        let mut fallback = config.clone();
        fallback.args.push("--extractor-args".to_string());
        fallback
            .args
            .push(format!("youtube:player_client={client}"));
        profiles.push((client.to_string(), fallback));
    }
    profiles
}

fn has_explicit_youtube_player_client(args: &[String]) -> bool {
    args.iter().any(|arg| {
        let arg = arg.to_ascii_lowercase();
        arg.contains("youtube:")
            && (arg.contains("player_client=") || arg.contains("player-client="))
    })
}

fn select_caption_stream<'a>(
    streams: &'a [TranscriptStream],
    preferred_languages: &[String],
) -> Option<&'a TranscriptStream> {
    streams
        .iter()
        .filter(|stream| stream.source_path.is_some())
        .min_by_key(|stream| stream_rank(stream, preferred_languages))
}

fn stream_rank(
    stream: &TranscriptStream,
    preferred_languages: &[String],
) -> (usize, usize, String) {
    let language = stream
        .language
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let base_language = language
        .split(['-', '_'])
        .next()
        .unwrap_or(language.as_str());
    let language_rank = preferred_languages
        .iter()
        .position(|preferred| preferred == &language)
        .map(|rank| rank * 2)
        .or_else(|| {
            preferred_languages
                .iter()
                .position(|preferred| {
                    preferred
                        .split(['-', '_'])
                        .next()
                        .unwrap_or(preferred.as_str())
                        == base_language
                })
                .map(|rank| rank * 2 + 1)
        })
        .unwrap_or(usize::MAX / 4);
    let source_rank = match stream.source_kind {
        SourceKind::CaptionManual => 0,
        SourceKind::CaptionAuto => 1,
        SourceKind::Asr => 2,
    };
    (language_rank, source_rank, language)
}

fn normalize_languages(input: &[String]) -> Vec<String> {
    let mut languages = Vec::new();
    for raw in input.iter().take(8) {
        let language = raw.trim().to_ascii_lowercase();
        if language.is_empty()
            || language.len() > 35
            || !language
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        {
            continue;
        }
        push_unique(&mut languages, language.clone());
        if let Some(base) = language.split(['-', '_']).next() {
            if base.len() >= 2 {
                push_unique(&mut languages, base.to_string());
            }
        }
    }
    if languages.is_empty() {
        languages.push("en".to_string());
    }
    languages
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn youtube_video_id(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(host.as_str());

    let candidate = if host == "youtu.be" {
        url.path_segments()?
            .find(|segment| !segment.is_empty())?
            .to_string()
    } else if host == "youtube.com" || host.ends_with(".youtube.com") {
        if url.path() == "/watch" {
            url.query_pairs()
                .find_map(|(key, value)| (key == "v").then(|| value.into_owned()))?
        } else {
            let mut segments = url.path_segments()?.filter(|segment| !segment.is_empty());
            match segments.next()? {
                "shorts" | "live" | "embed" => segments.next()?.to_string(),
                _ => return None,
            }
        }
    } else {
        return None;
    };

    is_probable_video_id(&candidate).then_some(candidate)
}

fn is_probable_video_id(value: &str) -> bool {
    value.len() == 11
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn bridge_error(status: StatusCode, message: impl Into<String>) -> BridgeError {
    (
        status,
        Json(BridgeErrorBody {
            error: BridgeErrorMessage {
                message: message.into(),
            },
        }),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn accepts_only_youtube_video_urls() {
        assert_eq!(
            youtube_video_id("https://youtu.be/jNQXAC9IVRw").as_deref(),
            Some("jNQXAC9IVRw")
        );
        assert_eq!(
            youtube_video_id("https://www.youtube.com/watch?v=jNQXAC9IVRw").as_deref(),
            Some("jNQXAC9IVRw")
        );
        assert_eq!(
            youtube_video_id("https://m.youtube.com/shorts/jNQXAC9IVRw").as_deref(),
            Some("jNQXAC9IVRw")
        );
        assert!(youtube_video_id("https://example.com/watch?v=jNQXAC9IVRw").is_none());
        assert!(youtube_video_id("http://www.youtube.com/watch?v=jNQXAC9IVRw").is_none());
    }

    #[test]
    fn normalizes_browser_languages_for_yt_dlp() {
        assert_eq!(
            normalize_languages(&[
                "de-DE".to_string(),
                "en-US".to_string(),
                "de".to_string(),
                "../../bad".to_string(),
            ]),
            vec!["de-de", "de", "en-us", "en"]
        );
        assert_eq!(normalize_languages(&[]), vec!["en"]);
    }

    #[test]
    fn prefers_language_before_manual_vs_auto_source() {
        let streams = vec![
            TranscriptStream {
                source_kind: SourceKind::CaptionManual,
                language: Some("en".to_string()),
                source_path: Some(PathBuf::from("manual-en.vtt")),
                text: None,
                segments: Vec::new(),
                message: None,
            },
            TranscriptStream {
                source_kind: SourceKind::CaptionAuto,
                language: Some("de".to_string()),
                source_path: Some(PathBuf::from("auto-de.vtt")),
                text: None,
                segments: Vec::new(),
                message: None,
            },
        ];
        let selected = select_caption_stream(&streams, &["de".to_string(), "en".to_string()])
            .expect("caption stream");
        assert_eq!(selected.language.as_deref(), Some("de"));
    }

    #[test]
    fn adds_bounded_player_client_fallbacks() {
        let profiles = caption_profiles(&YtDlpConfig::default());
        assert_eq!(
            profiles
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec!["default", "web_embedded", "mweb", "tv"]
        );
        assert!(profiles[1]
            .1
            .args
            .windows(2)
            .any(|args| { args == ["--extractor-args", "youtube:player_client=web_embedded"] }));
    }

    #[test]
    fn preserves_explicit_player_client_policy() {
        let config = YtDlpConfig {
            args: vec![
                "--extractor-args".to_string(),
                "youtube:player_client=ios;fetch_pot=always".to_string(),
            ],
            ..YtDlpConfig::default()
        };
        let profiles = caption_profiles(&config);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].0, "default");
    }
}
