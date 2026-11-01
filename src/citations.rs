//! Citation-oriented search output (#140): YouTube ids and timestamp links on each hit, raw
//! diarization labels, a context window around each hit, a plain-text rendering, and the
//! per-span `span_sha256` that truth-core pins a `media_span` passage with.
//!
//! Hits come from the ordinary search providers ([`crate::search`]); this module only enriches
//! them. The span hash is computed over the `source_span_interchange@1` batch built by
//! [`crate::source_span_export`], so it is exactly what a whole-corpus export would yield.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::str::FromStr;

use anyhow::ensure;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::corpus_export::{canonical_json, sha256_prefixed, CorpusExportSnapshot};
use crate::search::{SearchReport, SearchResult};
use crate::source_span_export::{
    build_source_span_batch, SourceLocatorV1, SourceProducerV1, SOURCE_SPAN_INTERCHANGE_SCHEMA,
    SOURCE_SPAN_INTERCHANGE_VERSION_V1,
};

/// Neighbouring segments to return around each hit: `2` (segments on each side) or `30s`
/// (seconds on each side).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContextWindow {
    Segments(u32),
    Seconds(f64),
}

impl FromStr for ContextWindow {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        let invalid =
            || format!("context must be a segment count (`2`) or seconds (`30s`), got {value:?}");
        if let Some(seconds) = value.strip_suffix('s') {
            let seconds: f64 = seconds.trim().parse().map_err(|_| invalid())?;
            if !(seconds.is_finite() && seconds >= 0.0) {
                return Err(invalid());
            }
            return Ok(Self::Seconds(seconds));
        }
        value.parse().map(Self::Segments).map_err(|_| invalid())
    }
}

/// What a reader needs to cite a hit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchCitation {
    pub youtube_id: Option<String>,
    pub channel: Option<String>,
    /// `YYYY-MM-DD`.
    pub upload_date: Option<String>,
    /// `caption_manual`, `caption_auto` or `asr`; absent for visible-text (OCR) hits.
    pub transcript_source: Option<String>,
    /// YouTube link that starts playback at the hit.
    pub timestamp_url: Option<String>,
    /// Raw labels of stored `speaker_turn` annotations overlapping the hit (not identities).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speaker_labels: Vec<String>,
    /// Transcript segments around the hit in time order; the hit (or, for a translated hit, its
    /// original segment) has `isHit`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<ContextSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSegment {
    pub segment_id: Uuid,
    pub segment_index: i64,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speaker_labels: Vec<String>,
    pub is_hit: bool,
}

/// Fills [`SearchResult::citation`] for every hit. Speaker labels come from `corpus_id`'s
/// annotations when given, else from every corpus.
pub async fn attach_citations(
    pool: &PgPool,
    results: &mut [SearchResult],
    corpus_id: Option<Uuid>,
    context: Option<ContextWindow>,
) -> anyhow::Result<()> {
    let video_ids: Vec<Uuid> = results.iter().map(|result| result.video_id).collect();
    // youtube id, channel, upload date
    type VideoCitation = (Option<String>, Option<String>, Option<String>);
    let videos: HashMap<Uuid, VideoCitation> =
        sqlx::query("SELECT id, youtube_id, channel, upload_date FROM videos WHERE id = ANY($1)")
            .bind(&video_ids)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| {
                Ok((
                    row.try_get("id")?,
                    (
                        row.try_get("youtube_id")?,
                        row.try_get("channel")?,
                        row.try_get("upload_date")?,
                    ),
                ))
            })
            .collect::<Result<_, sqlx::Error>>()?;

    let mut contexts = match context {
        Some(window) => load_context(pool, results, window).await?,
        None => HashMap::new(),
    };

    let mut keys = Vec::new();
    for result in results.iter() {
        keys.push((
            result.segment_id,
            result.video_id,
            result.start_seconds,
            result.end_seconds,
        ));
        for segment in contexts.get(&result.segment_id).into_iter().flatten() {
            keys.push((
                segment.segment_id,
                result.video_id,
                segment.start_seconds,
                segment.end_seconds,
            ));
        }
    }
    let labels = speaker_labels(pool, &keys, corpus_id).await?;
    let labels_of = |id: Uuid| labels.get(&id).cloned().unwrap_or_default();

    for result in results.iter_mut() {
        let (youtube_id, channel, upload_date) =
            videos.get(&result.video_id).cloned().unwrap_or_default();
        let mut context = contexts.remove(&result.segment_id).unwrap_or_default();
        for segment in &mut context {
            segment.speaker_labels = labels_of(segment.segment_id);
        }
        result.citation = Some(SearchCitation {
            timestamp_url: youtube_id
                .as_deref()
                .map(|id| timestamp_url(id, result.start_seconds)),
            youtube_id,
            channel,
            upload_date: upload_date.map(|date| display_date(&date)),
            transcript_source: (result.source_kind != "visual_ocr")
                .then(|| result.source_kind.clone()),
            speaker_labels: labels_of(result.segment_id),
            context,
        });
    }
    Ok(())
}

/// Context segments per hit id. Transcript hits use their own stream; translated hits use the
/// stream of the original segment; visible-text hits get none.
async fn load_context(
    pool: &PgPool,
    results: &[SearchResult],
    window: ContextWindow,
) -> anyhow::Result<HashMap<Uuid, Vec<ContextSegment>>> {
    let (hit_ids, anchor_ids): (Vec<Uuid>, Vec<Uuid>) = results
        .iter()
        .filter(|result| result.source_kind != "visual_ocr")
        .map(|result| {
            let anchor = result
                .translation
                .as_ref()
                .map_or(result.segment_id, |translation| {
                    translation.source_segment_id
                });
            (result.segment_id, anchor)
        })
        .unzip();
    let (segments, seconds) = match window {
        ContextWindow::Segments(count) => (Some(i64::from(count)), None),
        ContextWindow::Seconds(seconds) => (None, Some(seconds)),
    };
    let rows = sqlx::query(
        "SELECT h.hit_id, c.id, c.segment_index, c.start_seconds, c.end_seconds, c.text,
                (c.id = a.id) AS is_hit
         FROM unnest($1::uuid[], $2::uuid[]) AS h(hit_id, anchor_id)
         JOIN transcript_segments a ON a.id = h.anchor_id
         JOIN transcript_segments c ON c.stream_id = a.stream_id
         WHERE c.id = a.id
            OR ($3::bigint IS NOT NULL
                AND c.segment_index BETWEEN a.segment_index - $3 AND a.segment_index + $3)
            OR ($4::float8 IS NOT NULL
                AND c.start_seconds < a.end_seconds + $4
                AND c.end_seconds > a.start_seconds - $4)
         ORDER BY h.hit_id, c.start_seconds NULLS LAST, c.segment_index",
    )
    .bind(&hit_ids)
    .bind(&anchor_ids)
    .bind(segments)
    .bind(seconds)
    .fetch_all(pool)
    .await?;
    let mut context = HashMap::<Uuid, Vec<ContextSegment>>::new();
    for row in rows {
        context
            .entry(row.try_get("hit_id")?)
            .or_default()
            .push(ContextSegment {
                segment_id: row.try_get("id")?,
                segment_index: row.try_get("segment_index")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                text: row.try_get("text")?,
                speaker_labels: Vec::new(),
                is_hit: row.try_get("is_hit")?,
            });
    }
    Ok(context)
}

/// Distinct raw labels of `speaker_turn` annotations linked to each segment or overlapping it.
async fn speaker_labels(
    pool: &PgPool,
    keys: &[(Uuid, Uuid, Option<f64>, Option<f64>)],
    corpus_id: Option<Uuid>,
) -> anyhow::Result<HashMap<Uuid, Vec<String>>> {
    let ids: Vec<Uuid> = keys.iter().map(|key| key.0).collect();
    let videos: Vec<Uuid> = keys.iter().map(|key| key.1).collect();
    let starts: Vec<Option<f64>> = keys.iter().map(|key| key.2).collect();
    let ends: Vec<Option<f64>> = keys.iter().map(|key| key.3).collect();
    let rows = sqlx::query(
        "SELECT k.id, array_agg(DISTINCT ra.label ORDER BY ra.label) AS labels
         FROM unnest($1::uuid[], $2::uuid[], $3::float8[], $4::float8[])
              AS k(id, video_id, start_seconds, end_seconds)
         JOIN research_annotations ra
           ON ra.kind = 'speaker_turn' AND ra.label IS NOT NULL AND ra.video_id = k.video_id
          AND ($5::uuid IS NULL OR ra.corpus_id = $5)
          AND (ra.segment_id = k.id
               OR (ra.start_seconds < k.end_seconds AND ra.end_seconds > k.start_seconds))
         GROUP BY k.id",
    )
    .bind(&ids)
    .bind(&videos)
    .bind(&starts)
    .bind(&ends)
    .bind(corpus_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| Ok((row.try_get("id")?, row.try_get("labels")?)))
        .collect()
}

/// `https://www.youtube.com/watch?v=<id>&t=<whole seconds>s`.
pub fn timestamp_url(youtube_id: &str, start_seconds: Option<f64>) -> String {
    match start_seconds {
        Some(start) if start.is_finite() && start >= 0.0 => {
            format!(
                "https://www.youtube.com/watch?v={youtube_id}&t={}s",
                start.floor() as u64
            )
        }
        _ => format!("https://www.youtube.com/watch?v={youtube_id}"),
    }
}

fn display_date(date: &str) -> String {
    if date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..])
    } else {
        date.to_string()
    }
}

/// `mm:ss`, or `h:mm:ss` from one hour on.
pub fn clock(seconds: f64) -> String {
    let whole = seconds.max(0.0).floor() as u64;
    let (hours, minutes, secs) = (whole / 3600, whole / 60 % 60, whole % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes:02}:{secs:02}")
    }
}

fn range(start: Option<f64>, end: Option<f64>) -> String {
    let side = |value: Option<f64>| value.map_or_else(|| "?".to_string(), clock);
    format!("[{}–{}]", side(start), side(end))
}

fn speaker_prefix(labels: &[String]) -> String {
    if labels.is_empty() {
        String::new()
    } else {
        format!("{}: ", labels.join("/"))
    }
}

/// Plain text: a header per hit, then `[mm:ss–mm:ss] speaker: text` lines with the hit marked
/// `>`.
pub fn render_search_text(report: &SearchReport) -> String {
    let mut output = String::new();
    for (rank, result) in report.results.iter().enumerate() {
        let citation = result.citation.clone().unwrap_or_default();
        let mut header = vec![citation
            .youtube_id
            .clone()
            .unwrap_or_else(|| result.video_id.to_string())];
        if let Some(title) = &result.title {
            header.push(format!("\"{title}\""));
        }
        header.extend(citation.channel.clone());
        header.extend(citation.upload_date.clone());
        header.push(match &result.language {
            Some(language) => format!("{} {language}", result.source_kind),
            None => result.source_kind.clone(),
        });
        if let Some(translation) = &result.translation {
            header.push(format!("translated from {}", translation.source_language));
        }
        let _ = writeln!(output, "#{} {}", rank + 1, header.join(" · "));
        let _ = writeln!(
            output,
            "  {}",
            citation
                .timestamp_url
                .as_deref()
                .unwrap_or(&result.source_url)
        );
        let hit_line = format!(
            "> {} {}{}",
            range(result.start_seconds, result.end_seconds),
            speaker_prefix(&citation.speaker_labels),
            result.text
        );
        // A transcript hit is printed in place of its context segment; a translated hit is
        // printed first, followed by the original-language context with its source marked `=`.
        let inline_hit = result.translation.is_none() && !citation.context.is_empty();
        if !inline_hit {
            let _ = writeln!(output, "{hit_line}");
        }
        for segment in &citation.context {
            if segment.is_hit && inline_hit {
                let _ = writeln!(output, "{hit_line}");
                continue;
            }
            let marker = if segment.is_hit { "=" } else { " " };
            let _ = writeln!(
                output,
                "{marker} {} {}{}",
                range(segment.start_seconds, segment.end_seconds),
                speaker_prefix(&segment.speaker_labels),
                segment.text
            );
        }
        output.push('\n');
    }
    output
}

/// Selects the transcript stream(s) and interval of a span citation.
#[derive(Debug, Clone)]
pub struct SpanQuery {
    pub youtube_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub stream_id: Option<Uuid>,
    pub transcript_source: Option<String>,
    pub language: Option<String>,
}

/// Which stored segmentation a span is cut from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum SpanRepresentation {
    /// The indexed segments: de-rolled passages for automatic captions (#92), cues otherwise.
    #[default]
    Segments,
    /// The raw display cues kept for passage streams: what automatic captions indexed before
    /// passages, so citations made then reproduce their `span_sha256`.
    SourceCues,
}

/// The pinnable citation of `[start, end)` in every matching transcript stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanReport {
    /// `source_span_interchange@1`: the export the hash is defined over.
    pub schema: String,
    pub producer: SourceProducerV1,
    #[serde(default)]
    pub representation: SpanRepresentation,
    pub youtube_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub spans: Vec<MediaSpan>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSpan {
    pub stream_id: String,
    pub transcript_source: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub timestamp_url: String,
    pub segment_count: usize,
    /// SHA-256 of the canonical JSON list of each overlapping segment's `segmentIndex`,
    /// `startSeconds`, `endSeconds` and `textHash`, in time order (truth-core's `span_sha256`).
    pub span_sha256: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speaker_labels: Vec<String>,
    pub segments: Vec<SpanSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanSegment {
    pub segment_index: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speaker_labels: Vec<String>,
}

/// Builds the span citation from the same `source_span_interchange@1` batch `export --format
/// source-span` writes: the timed spans of each `youtube_transcript` source of the video whose
/// interval overlaps `[start, end)`, sorted by start time then segment index. Streams without
/// an overlapping segment are left out.
pub fn media_spans(
    snapshot: &CorpusExportSnapshot,
    producer_revision: &str,
    query: &SpanQuery,
) -> anyhow::Result<SpanReport> {
    let (start, end) = (query.start_seconds, query.end_seconds);
    ensure!(
        start.is_finite() && start >= 0.0 && end.is_finite() && end > start,
        "the span needs 0 <= start < end"
    );
    let batch = build_source_span_batch(snapshot, producer_revision)?;
    let metadata = |source: &crate::source_span_export::SourceRecordV1, key: &str| {
        source
            .metadata
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let mut spans = Vec::new();
    for source in batch.sources.iter().filter(|source| {
        source.kind == "youtube_transcript"
            && metadata(source, "youtubeId").as_deref() == Some(query.youtube_id.as_str())
            && query.stream_id.is_none_or(|id| source.id == id.to_string())
            && query
                .transcript_source
                .as_ref()
                .is_none_or(|kind| metadata(source, "transcriptSource").as_ref() == Some(kind))
            && query
                .language
                .as_ref()
                .is_none_or(|language| source.language.as_ref() == Some(language))
    }) {
        let stream_id = Uuid::parse_str(&source.id)?;
        let mut segments = Vec::new();
        for span in batch
            .spans
            .iter()
            .filter(|span| span.source_id == source.id)
        {
            let SourceLocatorV1::Timed {
                segment_index,
                start_seconds: Some(seg_start),
                end_seconds: Some(seg_end),
            } = span.locator
            else {
                continue;
            };
            if !(seg_start < end && seg_end > start) {
                continue;
            }
            let segment_id = Uuid::parse_str(&span.id)?;
            segments.push(SpanSegment {
                segment_index,
                start_seconds: seg_start,
                end_seconds: seg_end,
                text: span.text.clone(),
                content_hash: span.content_hash.clone(),
                speaker_labels: snapshot_speaker_labels(
                    snapshot, stream_id, segment_id, seg_start, seg_end,
                ),
            });
        }
        if segments.is_empty() {
            continue;
        }
        segments.sort_by(|a, b| {
            a.start_seconds
                .total_cmp(&b.start_seconds)
                .then(a.segment_index.cmp(&b.segment_index))
        });
        let speaker_labels = segments
            .iter()
            .flat_map(|segment| segment.speaker_labels.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        spans.push(MediaSpan {
            stream_id: source.id.clone(),
            transcript_source: metadata(source, "transcriptSource").unwrap_or_default(),
            language: source.language.clone(),
            title: source.title.clone(),
            timestamp_url: timestamp_url(&query.youtube_id, Some(start)),
            segment_count: segments.len(),
            span_sha256: span_sha256(&segments)?,
            speaker_labels,
            segments,
        });
    }
    Ok(SpanReport {
        schema: format!("{SOURCE_SPAN_INTERCHANGE_SCHEMA}@{SOURCE_SPAN_INTERCHANGE_VERSION_V1}"),
        producer: batch.producer,
        representation: SpanRepresentation::Segments,
        youtube_id: query.youtube_id.clone(),
        start_seconds: start,
        end_seconds: end,
        spans,
    })
}

/// truth-core's `media_span_hash`: database ids are left out because they differ between
/// ingests.
pub fn span_sha256(segments: &[SpanSegment]) -> anyhow::Result<String> {
    let material = Value::Array(
        segments
            .iter()
            .map(|segment| {
                serde_json::json!({
                    "endSeconds": segment.end_seconds,
                    "segmentIndex": segment.segment_index,
                    "startSeconds": segment.start_seconds,
                    "textHash": sha256_prefixed(&segment.text),
                })
            })
            .collect(),
    );
    Ok(sha256_prefixed(&canonical_json(&material)?))
}

fn snapshot_speaker_labels(
    snapshot: &CorpusExportSnapshot,
    stream_id: Uuid,
    segment_id: Uuid,
    start: f64,
    end: f64,
) -> Vec<String> {
    snapshot
        .speaker_turns
        .iter()
        .filter(|turn| turn.stream_id.is_none_or(|id| id == stream_id))
        .filter(|turn| {
            turn.segment_id == Some(segment_id)
                || matches!((turn.start_seconds, turn.end_seconds),
                    (Some(turn_start), Some(turn_end)) if turn_start < end && turn_end > start)
        })
        .filter_map(|turn| turn.speaker_label.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Plain text of a span report: one block per stream with its hash and segments.
pub fn render_span_text(report: &SpanReport) -> String {
    let mut output = String::new();
    for span in &report.spans {
        let _ = writeln!(
            output,
            "{} · {} {} · {} segments · {}",
            report.youtube_id,
            span.transcript_source,
            span.language.as_deref().unwrap_or("?"),
            span.segment_count,
            span.span_sha256
        );
        let _ = writeln!(output, "  {}", span.timestamp_url);
        for segment in &span.segments {
            let _ = writeln!(
                output,
                "  {} {}{}",
                range(Some(segment.start_seconds), Some(segment.end_seconds)),
                speaker_prefix(&segment.speaker_labels),
                segment.text
            );
        }
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_segment_and_second_windows() {
        assert_eq!("2".parse(), Ok(ContextWindow::Segments(2)));
        assert_eq!("30s".parse(), Ok(ContextWindow::Seconds(30.0)));
        assert_eq!("1.5s".parse(), Ok(ContextWindow::Seconds(1.5)));
        assert!("-1".parse::<ContextWindow>().is_err());
        assert!("-3s".parse::<ContextWindow>().is_err());
        assert!("two".parse::<ContextWindow>().is_err());
    }

    #[test]
    fn formats_clocks_and_timestamp_links() {
        assert_eq!(clock(65.9), "01:05");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(
            timestamp_url("abc", Some(12.75)),
            "https://www.youtube.com/watch?v=abc&t=12s"
        );
        assert_eq!(
            timestamp_url("abc", None),
            "https://www.youtube.com/watch?v=abc"
        );
        assert_eq!(display_date("20240115"), "2024-01-15");
    }
}
