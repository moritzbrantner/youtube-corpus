use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::corpus_export::{
    canonical_json, load_export_snapshot, order_snapshot, segment_revision_material,
    sha256_prefixed, CorpusExportSnapshot, ExportTranscriptSegment, ExportTranscriptStream,
    ExportVideo, ExportVisualTextRun, ExportVisualTextTrack, OrderedRun, OrderedStream,
};

pub const SOURCE_SPAN_INTERCHANGE_SCHEMA: &str = "source_span_interchange";
pub const SOURCE_SPAN_INTERCHANGE_VERSION_V1: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpanBatchV1 {
    pub schema: String,
    pub schema_version: u32,
    pub producer: SourceProducerV1,
    pub sources: Vec<SourceRecordV1>,
    pub spans: Vec<SourceSpanRecordV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceProducerV1 {
    pub name: String,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRecordV1 {
    pub id: String,
    pub kind: String,
    pub revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub creators: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpanRecordV1 {
    pub id: String,
    pub source_id: String,
    pub sequence: u64,
    pub text: String,
    pub content_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub locator: SourceLocatorV1,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SourceLocatorV1 {
    Text {
        byte_start: usize,
        byte_end: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        paragraph_ordinal: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        page: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        section: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_selector: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        heading_path: Vec<String>,
    },
    Timed {
        segment_index: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        start_seconds: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end_seconds: Option<f64>,
    },
}

/// Exports the `source_span_interchange@1` batch for the corpus in Postgres, optionally for one
/// video. It loads the [`CorpusExportSnapshot`] and builds the batch exactly like
/// [`crate::corpus_export::export_source_span`].
pub async fn export_source_span_batch(
    pool: &PgPool,
    producer_revision: &str,
    video_id: Option<Uuid>,
) -> anyhow::Result<SourceSpanBatchV1> {
    ensure!(
        !producer_revision.trim().is_empty(),
        "a producer revision is required for source-span export"
    );
    let snapshot = load_export_snapshot(pool, video_id).await?;
    build_source_span_batch(&snapshot, producer_revision)
}

/// Builds the source-span batch: transcript sources and spans by stream id then segment index,
/// then `ocr:<run id>` sources and spans by run id and track order.
pub(crate) fn build_source_span_batch(
    snapshot: &CorpusExportSnapshot,
    producer_revision: &str,
) -> anyhow::Result<SourceSpanBatchV1> {
    ensure!(
        !producer_revision.trim().is_empty(),
        "a producer revision is required for source-span export"
    );
    let ordered = order_snapshot(snapshot, producer_revision, "producer", true)?;

    let mut sources = Vec::new();
    let mut spans = Vec::new();
    for ordered_stream in ordered
        .streams
        .iter()
        .filter(|stream| !stream.segments.is_empty())
    {
        // The source describes the video of its first segment, as the source-span export always
        // did (a segment row may name a different video than its stream).
        let first = ordered_stream.segments[0];
        let video = ordered.video(first.effective_video_id(ordered_stream.stream))?;
        sources.push(transcript_source(video, ordered_stream)?);
    }
    for ordered_stream in &ordered.streams {
        let stream = ordered_stream.stream;
        for segment in &ordered_stream.segments {
            spans.push(transcript_span(stream, segment));
        }
    }

    for ordered_run in ordered.runs.iter().filter(|run| !run.tracks.is_empty()) {
        let video = ordered.video(ordered_run.run.video_id)?;
        sources.push(visual_text_source(video, ordered_run)?);
    }
    for ordered_run in &ordered.runs {
        let run = ordered_run.run;
        for (sequence, track) in ordered_run.tracks.iter().enumerate() {
            spans.push(visual_text_span(run, track, sequence)?);
        }
    }

    Ok(SourceSpanBatchV1 {
        schema: SOURCE_SPAN_INTERCHANGE_SCHEMA.to_string(),
        schema_version: SOURCE_SPAN_INTERCHANGE_VERSION_V1,
        producer: SourceProducerV1 {
            name: "youtube-corpus".to_string(),
            revision: producer_revision.to_string(),
        },
        sources,
        spans,
    })
}

fn creators(video: &ExportVideo) -> Vec<String> {
    let mut creators = Vec::new();
    for creator in [&video.channel, &video.uploader].into_iter().flatten() {
        if !creator.trim().is_empty() && !creators.contains(creator) {
            creators.push(creator.clone());
        }
    }
    creators
}

fn transcript_source(
    video: &ExportVideo,
    ordered_stream: &OrderedStream<'_>,
) -> anyhow::Result<SourceRecordV1> {
    let stream = ordered_stream.stream;
    let segments = &ordered_stream.segments;
    let stream_text = stream
        .full_text
        .as_deref()
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        });
    let source_hash = sha256_prefixed(&stream_text);
    let revision_material = serde_json::json!({
        "contentHash": &source_hash,
        "sourceKind": &stream.source_kind,
        "language": &stream.language,
        "spans": segments
            .iter()
            .map(|segment| segment_revision_material(segment))
            .collect::<Vec<_>>(),
    });
    let revision = sha256_prefixed(&canonical_json(&revision_material)?);
    let mut metadata = BTreeMap::new();
    metadata.insert("videoId".to_string(), Value::String(video.id.to_string()));
    metadata.insert("streamId".to_string(), Value::String(stream.id.to_string()));
    metadata.insert(
        "transcriptSource".to_string(),
        Value::String(stream.source_kind.clone()),
    );
    if let Some(youtube_id) = &video.youtube_id {
        metadata.insert("youtubeId".to_string(), Value::String(youtube_id.clone()));
    }
    if let Some(channel_id) = &video.channel_id {
        metadata.insert("channelId".to_string(), Value::String(channel_id.clone()));
    }
    if let Some(uploader_id) = &video.uploader_id {
        metadata.insert("uploaderId".to_string(), Value::String(uploader_id.clone()));
    }

    Ok(SourceRecordV1 {
        id: stream.id.to_string(),
        kind: "youtube_transcript".to_string(),
        revision,
        uri: Some(video.source_url.clone()),
        title: video.title.clone(),
        creators: creators(video),
        language: stream.language.clone(),
        content_hash: source_hash,
        metadata,
    })
}

fn transcript_span(
    stream: &ExportTranscriptStream,
    segment: &ExportTranscriptSegment,
) -> SourceSpanRecordV1 {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "videoId".to_string(),
        Value::String(segment.effective_video_id(stream).to_string()),
    );
    metadata.insert("streamId".to_string(), Value::String(stream.id.to_string()));
    metadata.insert(
        "transcriptSource".to_string(),
        Value::String(stream.source_kind.clone()),
    );
    SourceSpanRecordV1 {
        id: segment.id.to_string(),
        source_id: stream.id.to_string(),
        sequence: segment.segment_index,
        content_hash: sha256_prefixed(&segment.text),
        text: segment.text.clone(),
        language: segment.language.clone().or_else(|| stream.language.clone()),
        locator: SourceLocatorV1::Timed {
            segment_index: segment.segment_index,
            start_seconds: segment.start_seconds,
            end_seconds: segment.end_seconds,
        },
        metadata,
    }
}

fn visual_text_source(
    video: &ExportVideo,
    ordered_run: &OrderedRun<'_>,
) -> anyhow::Result<SourceRecordV1> {
    let run = ordered_run.run;
    let tracks = &ordered_run.tracks;
    let source_text = tracks
        .iter()
        .map(|track| track.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let source_hash = sha256_prefixed(&source_text);
    let revision_material = serde_json::json!({
        "contentHash": &source_hash,
        "processor": &run.processor,
        "processorVersion": &run.processor_version,
        "model": &run.model,
        "modelVersion": &run.model_version,
        "inputHash": &run.input_hash,
        "configHash": &run.config_hash,
        "spans": tracks
            .iter()
            .enumerate()
            .map(|(sequence, track)| serde_json::json!({
                "id": track.id,
                "sequence": sequence,
                "textHash": sha256_prefixed(&track.text),
                "role": &track.role,
                "startSeconds": track.start_seconds,
                "endSeconds": track.end_seconds,
            }))
            .collect::<Vec<_>>(),
    });
    let revision = sha256_prefixed(&canonical_json(&revision_material)?);
    let mut metadata = BTreeMap::new();
    for (key, value) in [
        ("videoId", video.id.to_string()),
        ("runId", run.id.to_string()),
        ("processor", run.processor.clone()),
        ("processorVersion", run.processor_version.clone()),
        ("model", run.model.clone()),
        ("modelVersion", run.model_version.clone()),
        ("inputHash", run.input_hash.clone()),
        ("configHash", run.config_hash.clone()),
    ] {
        metadata.insert(key.to_string(), Value::String(value));
    }

    Ok(SourceRecordV1 {
        id: format!("ocr:{}", run.id),
        kind: "youtube_visual_text".to_string(),
        revision,
        uri: Some(video.source_url.clone()),
        title: video.title.clone(),
        creators: creators(video),
        language: tracks.iter().find_map(|track| track.language.clone()),
        content_hash: source_hash,
        metadata,
    })
}

fn visual_text_span(
    run: &ExportVisualTextRun,
    track: &ExportVisualTextTrack,
    sequence: usize,
) -> anyhow::Result<SourceSpanRecordV1> {
    let sequence = u64::try_from(sequence).context("OCR span sequence exceeds u64 range")?;
    let mut metadata = BTreeMap::new();
    for (key, value) in [
        ("videoId", run.video_id.to_string()),
        ("runId", run.id.to_string()),
        ("visualTextRole", track.role.clone()),
        ("mediaEvidenceRef", track.id.to_string()),
    ] {
        metadata.insert(key.to_string(), Value::String(value));
    }
    Ok(SourceSpanRecordV1 {
        id: track.id.to_string(),
        source_id: format!("ocr:{}", run.id),
        sequence,
        text: track.text.clone(),
        content_hash: sha256_prefixed(&track.text),
        language: track.language.clone(),
        locator: SourceLocatorV1::Timed {
            segment_index: sequence,
            start_seconds: track.start_seconds,
            end_seconds: track.end_seconds,
        },
        metadata,
    })
}

/// Rejects a non-finite or negative start or end time and an end before the start.
pub(crate) fn validate_optional_timed_range(
    id: Uuid,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
    kind: &str,
) -> anyhow::Result<()> {
    ensure!(
        start_seconds.is_none_or(|value| value.is_finite() && value >= 0.0),
        "{kind} {id} has invalid start time"
    );
    ensure!(
        end_seconds.is_none_or(|value| value.is_finite() && value >= 0.0),
        "{kind} {id} has invalid end time"
    );
    ensure!(
        !start_seconds
            .zip(end_seconds)
            .is_some_and(|(start, end)| end < start),
        "{kind} {id} has reversed timestamps"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(segment_index: u64, text: &str, start: f64, end: f64) -> ExportTranscriptSegment {
        ExportTranscriptSegment {
            id: Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                format!("segment:{segment_index}").as_bytes(),
            ),
            segment_index,
            start_seconds: Some(start),
            end_seconds: Some(end),
            text: text.to_string(),
            language: Some("en".to_string()),
            video_id: None,
        }
    }

    fn snapshot(segments: Vec<ExportTranscriptSegment>) -> CorpusExportSnapshot {
        let video_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, b"video");
        CorpusExportSnapshot {
            videos: vec![ExportVideo {
                id: video_id,
                youtube_id: Some("abc123".to_string()),
                source_url: "https://www.youtube.com/watch?v=abc123".to_string(),
                title: Some("Lecture".to_string()),
                channel: Some("Lecture Channel".to_string()),
                channel_id: Some("channel-1".to_string()),
                uploader: Some("Lecturer".to_string()),
                uploader_id: Some("uploader-1".to_string()),
            }],
            transcript_streams: vec![ExportTranscriptStream {
                id: Uuid::new_v5(&Uuid::NAMESPACE_URL, b"stream"),
                video_id,
                source_kind: "caption_manual".to_string(),
                language: Some("en".to_string()),
                full_text: Some("First claim. Second claim.".to_string()),
                processor: "youtube-corpus".to_string(),
                processor_version: "0.1.0".to_string(),
                processing_config_hash: None,
                segments,
            }],
            visual_text_runs: Vec::new(),
        }
    }

    #[test]
    fn builds_one_revisioned_source_with_timestamped_spans() {
        let batch = build_source_span_batch(
            &snapshot(vec![
                segment(1, "Second claim.", 3.0, 5.0),
                segment(0, "First claim.", 0.0, 2.5),
            ]),
            "git:abc123",
        )
        .unwrap();

        assert_eq!(batch.schema, SOURCE_SPAN_INTERCHANGE_SCHEMA);
        assert_eq!(batch.schema_version, 1);
        assert_eq!(batch.producer.revision, "git:abc123");
        assert_eq!(batch.sources.len(), 1);
        assert!(batch.sources[0].content_hash.starts_with("sha256:"));
        assert!(batch.sources[0].revision.starts_with("sha256:"));
        assert_ne!(batch.sources[0].revision, batch.sources[0].content_hash);
        assert_eq!(
            batch
                .spans
                .iter()
                .map(|span| span.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(matches!(
            batch.spans[0].locator,
            SourceLocatorV1::Timed {
                segment_index: 0,
                start_seconds: Some(0.0),
                end_seconds: Some(2.5),
            }
        ));
        assert!(batch
            .spans
            .iter()
            .all(|span| span.content_hash.starts_with("sha256:")));
    }

    #[test]
    fn source_revision_changes_when_timing_changes_but_text_does_not() {
        let first = build_source_span_batch(
            &snapshot(vec![segment(0, "Claim.", 1.0, 2.0)]),
            "git:abc123",
        )
        .unwrap();
        let second = build_source_span_batch(
            &snapshot(vec![segment(0, "Claim.", 1.5, 2.5)]),
            "git:abc123",
        )
        .unwrap();

        assert_eq!(
            first.sources[0].content_hash,
            second.sources[0].content_hash
        );
        assert_ne!(first.sources[0].revision, second.sources[0].revision);
    }

    #[test]
    fn refuses_reversed_timestamp_ranges() {
        let error = build_source_span_batch(
            &snapshot(vec![segment(0, "Claim.", 3.0, 2.0)]),
            "git:abc123",
        )
        .unwrap_err();
        assert!(error.to_string().contains("reversed timestamps"));
    }

    #[test]
    fn serialized_contract_uses_shared_camel_case_field_names() {
        let batch = build_source_span_batch(
            &snapshot(vec![segment(0, "Claim.", 1.0, 2.0)]),
            "git:abc123",
        )
        .unwrap();
        let value = serde_json::to_value(batch).unwrap();

        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(
            value["spans"][0]["sourceId"].as_str(),
            value["sources"][0]["id"].as_str()
        );
        assert_eq!(value["spans"][0]["locator"]["kind"], "timed");
        assert_eq!(value["spans"][0]["locator"]["segmentIndex"], 0);
        assert_eq!(value["spans"][0]["locator"]["startSeconds"], 1.0);
    }
}
