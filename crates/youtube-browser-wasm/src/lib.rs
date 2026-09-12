//! Browser-safe YouTube extraction primitives.
//!
//! Network access deliberately stays in the browser host. This crate owns the
//! deterministic parsing and selection boundary for player responses and
//! caption payloads so GitHub Pages can reuse Rust without pretending to ship
//! the full yt-dlp runtime.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use wasm_bindgen::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerResponse {
    #[serde(default)]
    playability_status: Option<PlayabilityStatus>,
    #[serde(default)]
    video_details: Option<VideoDetails>,
    #[serde(default)]
    captions: Option<Captions>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayabilityStatus {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoDetails {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    length_seconds: Option<String>,
    #[serde(default)]
    view_count: Option<String>,
    #[serde(default)]
    short_description: Option<String>,
    #[serde(default)]
    thumbnail: Option<ThumbnailSet>,
}

#[derive(Clone, Debug, Deserialize)]
struct ThumbnailSet {
    #[serde(default)]
    thumbnails: Vec<Thumbnail>,
}

#[derive(Clone, Debug, Deserialize)]
struct Thumbnail {
    url: String,
    #[serde(default)]
    width: Option<u64>,
    #[serde(default)]
    height: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Captions {
    #[serde(default)]
    player_captions_tracklist_renderer: Option<CaptionTrackList>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptionTrackList {
    #[serde(default)]
    caption_tracks: Vec<RawCaptionTrack>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCaptionTrack {
    base_url: String,
    language_code: String,
    #[serde(default)]
    name: Option<TextValue>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    vss_id: Option<String>,
    #[serde(default)]
    is_translatable: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextValue {
    #[serde(default)]
    simple_text: Option<String>,
    #[serde(default)]
    runs: Vec<TextRun>,
}

#[derive(Clone, Debug, Deserialize)]
struct TextRun {
    text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPlayerEvidence {
    playability_status: String,
    playability_reason: Option<String>,
    title: Option<String>,
    author: Option<String>,
    channel_id: Option<String>,
    duration_seconds: Option<u64>,
    view_count: Option<u64>,
    description: Option<String>,
    thumbnail_url: Option<String>,
    caption_tracks: Vec<CaptionTrack>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionTrack {
    base_url: String,
    language_code: String,
    name: String,
    source_kind: String,
    is_translatable: bool,
    vss_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Json3Transcript {
    #[serde(default)]
    events: Vec<Json3Event>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Json3Event {
    #[serde(default)]
    t_start_ms: Option<u64>,
    #[serde(default)]
    d_duration_ms: Option<u64>,
    #[serde(default)]
    segs: Vec<Json3Segment>,
}

#[derive(Clone, Debug, Deserialize)]
struct Json3Segment {
    #[serde(default)]
    utf8: String,
}

#[derive(Clone, Debug, PartialEq)]
struct Cue {
    start_ms: u64,
    duration_ms: Option<u64>,
    text: String,
}

/// Extracts the stable subset of an InnerTube player response needed by the
/// static analyzer. Unknown fields remain ignored so YouTube can add response
/// data without changing this boundary.
#[wasm_bindgen(js_name = extractPlayerResponse)]
pub fn extract_player_response(value: JsValue) -> Result<JsValue, JsValue> {
    let response: PlayerResponse = serde_wasm_bindgen::from_value(value).map_err(js_error)?;
    let evidence = player_evidence(response);
    serde_wasm_bindgen::to_value(&evidence).map_err(js_error)
}

/// Chooses one caption track deterministically. Language preference wins first,
/// then human-authored captions win over ASR for otherwise equivalent tracks.
#[wasm_bindgen(js_name = selectCaptionTrack)]
pub fn select_caption_track(tracks: JsValue, preferred_languages: JsValue) -> Result<JsValue, JsValue> {
    let mut tracks: Vec<CaptionTrack> =
        serde_wasm_bindgen::from_value(tracks).map_err(js_error)?;
    let preferred_languages: Vec<String> =
        serde_wasm_bindgen::from_value(preferred_languages).map_err(js_error)?;

    let preferred_languages = preferred_languages
        .into_iter()
        .map(|language| language.to_ascii_lowercase())
        .collect::<Vec<_>>();

    tracks.sort_by_key(|track| track_rank(track, &preferred_languages));
    serde_wasm_bindgen::to_value(&tracks.into_iter().next()).map_err(js_error)
}

/// Converts YouTube's json3 timed-text representation to deterministic WebVTT.
/// WebVTT is then consumed by the existing browser transcript parser.
#[wasm_bindgen(js_name = json3ToWebVtt)]
pub fn json3_to_web_vtt(input: &str) -> Result<String, JsValue> {
    json3_to_web_vtt_impl(input).map_err(js_error)
}

fn player_evidence(response: PlayerResponse) -> BrowserPlayerEvidence {
    let playability_status = response
        .playability_status
        .as_ref()
        .and_then(|status| status.status.clone())
        .unwrap_or_else(|| "UNKNOWN".to_string());
    let playability_reason = response
        .playability_status
        .and_then(|status| status.reason);

    let video = response.video_details;
    let caption_tracks = response
        .captions
        .and_then(|captions| captions.player_captions_tracklist_renderer)
        .map(|renderer| {
            renderer
                .caption_tracks
                .into_iter()
                .map(|track| CaptionTrack {
                    name: text_value(track.name.as_ref())
                        .unwrap_or_else(|| track.language_code.clone()),
                    source_kind: if track.kind.as_deref() == Some("asr") {
                        "caption_auto".to_string()
                    } else {
                        "caption_manual".to_string()
                    },
                    base_url: track.base_url,
                    language_code: track.language_code,
                    is_translatable: track.is_translatable,
                    vss_id: track.vss_id,
                })
                .collect()
        })
        .unwrap_or_default();

    BrowserPlayerEvidence {
        title: video.as_ref().and_then(|video| video.title.clone()),
        author: video.as_ref().and_then(|video| video.author.clone()),
        channel_id: video.as_ref().and_then(|video| video.channel_id.clone()),
        duration_seconds: video
            .as_ref()
            .and_then(|video| parse_u64(video.length_seconds.as_deref())),
        view_count: video
            .as_ref()
            .and_then(|video| parse_u64(video.view_count.as_deref())),
        description: video.and_then(|video| video.short_description),
        thumbnail_url: response
            .video_details
            .as_ref()
            .and_then(|video| video.thumbnail.as_ref())
            .and_then(best_thumbnail),
        playability_status,
        playability_reason,
        caption_tracks,
    }
}

fn best_thumbnail(set: &ThumbnailSet) -> Option<String> {
    set.thumbnails
        .iter()
        .max_by_key(|thumbnail| {
            thumbnail
                .width
                .unwrap_or_default()
                .saturating_mul(thumbnail.height.unwrap_or_default())
        })
        .map(|thumbnail| thumbnail.url.clone())
}

fn text_value(value: Option<&TextValue>) -> Option<String> {
    let value = value?;
    if let Some(simple_text) = value.simple_text.as_ref() {
        return Some(simple_text.clone());
    }
    let joined = value
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect::<String>();
    (!joined.is_empty()).then_some(joined)
}

fn parse_u64(value: Option<&str>) -> Option<u64> {
    value?.parse().ok()
}

fn track_rank(track: &CaptionTrack, preferred_languages: &[String]) -> (usize, usize, String, String) {
    let language = track.language_code.to_ascii_lowercase();
    let base_language = language.split('-').next().unwrap_or(&language);
    let language_rank = preferred_languages
        .iter()
        .position(|preferred| preferred == &language)
        .map(|rank| rank * 2)
        .or_else(|| {
            preferred_languages.iter().position(|preferred| {
                preferred.split('-').next().unwrap_or(preferred.as_str()) == base_language
            })
            .map(|rank| rank * 2 + 1)
        })
        .unwrap_or(usize::MAX / 4);
    let source_rank = usize::from(track.source_kind == "caption_auto");
    (
        language_rank,
        source_rank,
        language,
        track.name.to_ascii_lowercase(),
    )
}

fn json3_to_web_vtt_impl(input: &str) -> Result<String, String> {
    let transcript: Json3Transcript =
        serde_json::from_str(input).map_err(|error| format!("invalid json3 captions: {error}"))?;

    let cues = transcript
        .events
        .into_iter()
        .filter_map(|event| {
            if event.segs.is_empty() {
                return None;
            }
            let text = event
                .segs
                .into_iter()
                .map(|segment| segment.utf8)
                .collect::<String>()
                .replace(['\n', '\r'], " ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if text.is_empty() {
                return None;
            }
            Some(Cue {
                start_ms: event.t_start_ms.unwrap_or_default(),
                duration_ms: event.d_duration_ms,
                text,
            })
        })
        .collect::<Vec<_>>();

    if cues.is_empty() {
        return Err("json3 captions contain no text cues".to_string());
    }

    let mut output = String::from("WEBVTT\n\n");
    for (index, cue) in cues.iter().enumerate() {
        let fallback_end = cues
            .get(index + 1)
            .map(|next| next.start_ms)
            .filter(|next_start| *next_start > cue.start_ms)
            .unwrap_or_else(|| cue.start_ms.saturating_add(2_000));
        let end_ms = cue
            .duration_ms
            .filter(|duration| *duration > 0)
            .map(|duration| cue.start_ms.saturating_add(duration))
            .unwrap_or(fallback_end)
            .max(cue.start_ms.saturating_add(1));

        output.push_str(&(index + 1).to_string());
        output.push('\n');
        output.push_str(&format_timestamp(cue.start_ms));
        output.push_str(" --> ");
        output.push_str(&format_timestamp(end_ms));
        output.push('\n');
        output.push_str(&cue.text);
        output.push_str("\n\n");
    }

    Ok(output)
}

fn format_timestamp(milliseconds: u64) -> String {
    let hours = milliseconds / 3_600_000;
    let minutes = (milliseconds % 3_600_000) / 60_000;
    let seconds = (milliseconds % 60_000) / 1_000;
    let millis = milliseconds % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    js_sys::Error::new(&error.to_string()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_player_metadata_and_caption_provenance() {
        let response: PlayerResponse = serde_json::from_value(serde_json::json!({
            "playabilityStatus": {"status": "OK"},
            "videoDetails": {
                "title": "Example",
                "author": "Channel",
                "channelId": "channel-1",
                "lengthSeconds": "42",
                "viewCount": "1234",
                "thumbnail": {"thumbnails": [
                    {"url": "small", "width": 120, "height": 90},
                    {"url": "large", "width": 480, "height": 360}
                ]}
            },
            "captions": {"playerCaptionsTracklistRenderer": {"captionTracks": [
                {
                    "baseUrl": "https://www.youtube.com/api/timedtext?lang=en",
                    "languageCode": "en",
                    "name": {"simpleText": "English"},
                    "isTranslatable": true
                },
                {
                    "baseUrl": "https://www.youtube.com/api/timedtext?lang=de&kind=asr",
                    "languageCode": "de",
                    "name": {"simpleText": "German (auto)"},
                    "kind": "asr"
                }
            ]}}
        }))
        .expect("player response");

        let evidence = player_evidence(response);
        assert_eq!(evidence.playability_status, "OK");
        assert_eq!(evidence.duration_seconds, Some(42));
        assert_eq!(evidence.view_count, Some(1234));
        assert_eq!(evidence.thumbnail_url.as_deref(), Some("large"));
        assert_eq!(evidence.caption_tracks.len(), 2);
        assert_eq!(evidence.caption_tracks[0].source_kind, "caption_manual");
        assert_eq!(evidence.caption_tracks[1].source_kind, "caption_auto");
    }

    #[test]
    fn selects_preferred_manual_track_before_asr() {
        let preferred = vec!["de-de".to_string(), "en".to_string()];
        let mut tracks = [
            CaptionTrack {
                base_url: "auto".into(),
                language_code: "de".into(),
                name: "German auto".into(),
                source_kind: "caption_auto".into(),
                is_translatable: true,
                vss_id: None,
            },
            CaptionTrack {
                base_url: "manual".into(),
                language_code: "de-DE".into(),
                name: "German".into(),
                source_kind: "caption_manual".into(),
                is_translatable: true,
                vss_id: None,
            },
        ];
        tracks.sort_by_key(|track| track_rank(track, &preferred));
        assert_eq!(tracks[0].base_url, "manual");
    }

    #[test]
    fn converts_json3_to_timed_webvtt() {
        let vtt = json3_to_web_vtt_impl(
            r#"{"events":[{"tStartMs":1000,"dDurationMs":1500,"segs":[{"utf8":"Hello "},{"utf8":"world"}]},{"tStartMs":3000,"segs":[{"utf8":"Again"}]}]}"#,
        )
        .expect("json3 should parse");

        assert!(vtt.starts_with("WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.500"));
        assert!(vtt.contains("Hello world"));
        assert!(vtt.contains("00:00:03.000 --> 00:00:05.000"));
    }
}
