//! Deterministic, versioned downstream corpus exports (issue #42).
//!
//! An export is a pure function of a [`CorpusExportSnapshot`] and a caller-supplied revision:
//! the same corpus state, in any vector order, gives the same bytes, and no exporter reads the
//! clock or the environment. [`load_export_snapshot`] loads that state from Postgres; a snapshot
//! can also be read from a JSON file, so exports can be reproduced without a database.
//!
//! Formats:
//!
//! 1. [`export_source_span`]: `source_span_interchange@1`, the generic source/span records
//!    (built by the same code as [`crate::export_source_span_batch`]);
//! 2. [`export_corpus_core_jsonl`]: `moenarch_corpus_core_batch@1` JSONL built from the real
//!    `corpus_core` types, the intake format of philosophy-extractor;
//! 3. [`export_belief_evidence`]: `belief_evidence_interchange@1` (or `@2` once a segment has
//!    timing), belief-lab evidence references, one per transcript segment, copying no text.

use anyhow::{bail, ensure, Context};
use corpus_core::{
    Asset, AssetId, AssetKind, ContentHash, ContentRef, CorpusItemRef, CorpusRecord, Metadata,
    Provenance, Representation, RepresentationId, RepresentationKind, Segment, SegmentId,
    SegmentLocator, Source, SourceId, SourceKind, Span,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use crate::source_span_export::{build_source_span_batch, validate_optional_timed_range};

pub const CORPUS_CORE_BATCH_SCHEMA: &str = "moenarch_corpus_core_batch";
pub const CORPUS_CORE_BATCH_VERSION_V1: u32 = 1;
pub const BELIEF_EVIDENCE_INTERCHANGE_SCHEMA: &str = "belief_evidence_interchange";
pub const BELIEF_EVIDENCE_INTERCHANGE_VERSION_V1: u32 = 1;
/// Version 2 adds each reference's media span and the scene records that cover it.
pub const BELIEF_EVIDENCE_INTERCHANGE_VERSION_V2: u32 = 2;
/// Segment times are exported in microsecond ticks of rational media time.
const MEDIA_TICKS_PER_SECOND: u32 = 1_000_000;

const PRODUCER_NAME: &str = "youtube-corpus";
const CORPUS_CORE_PROVENANCE_PRODUCER: &str = "youtube-corpus/corpus-export";
const TEXT_MEDIA_TYPE: &str = "text/plain; charset=utf-8";
const CORPUS_CORE_TEXT_SEPARATOR: &str = "\n";

/// The corpus state every export is computed from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusExportSnapshot {
    pub videos: Vec<ExportVideo>,
    pub transcript_streams: Vec<ExportTranscriptStream>,
    #[serde(default)]
    pub visual_text_runs: Vec<ExportVisualTextRun>,
    /// Stored scene records (from visual-analysis scene detection), used only to name the
    /// scenes that cover an exported segment.
    #[serde(default)]
    pub scenes: Vec<ExportScene>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportScene {
    pub id: Uuid,
    pub video_id: Uuid,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportVideo {
    pub id: Uuid,
    #[serde(default)]
    pub youtube_id: Option<String>,
    pub source_url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub uploader: Option<String>,
    #[serde(default)]
    pub uploader_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTranscriptStream {
    pub id: Uuid,
    pub video_id: Uuid,
    pub source_kind: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub full_text: Option<String>,
    pub processor: String,
    pub processor_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processor_revision: Option<String>,
    #[serde(default)]
    pub processing_config_hash: Option<String>,
    pub segments: Vec<ExportTranscriptSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTranscriptSegment {
    pub id: Uuid,
    pub segment_index: u64,
    #[serde(default)]
    pub start_seconds: Option<f64>,
    #[serde(default)]
    pub end_seconds: Option<f64>,
    pub text: String,
    #[serde(default)]
    pub language: Option<String>,
    /// The segment's own video when its database row names a different video than its stream
    /// (the schema allows this). `None` means the stream's video. The source-span and
    /// belief-evidence records of the segment use this video, as the source-span export always
    /// did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_id: Option<Uuid>,
}

impl ExportTranscriptSegment {
    /// The video the segment belongs to: its own `video_id`, else its stream's.
    pub fn effective_video_id(&self, stream: &ExportTranscriptStream) -> Uuid {
        self.video_id.unwrap_or(stream.video_id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportVisualTextRun {
    pub id: Uuid,
    pub video_id: Uuid,
    pub processor: String,
    pub processor_version: String,
    pub model: String,
    pub model_version: String,
    pub input_hash: String,
    pub config_hash: String,
    pub tracks: Vec<ExportVisualTextTrack>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportVisualTextTrack {
    pub id: Uuid,
    pub track_key: String,
    pub text: String,
    pub role: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub start_seconds: Option<f64>,
    #[serde(default)]
    pub end_seconds: Option<f64>,
    /// Position of the track in its run as the database orders it (`start_seconds NULLS LAST,
    /// track_key, id` under the database collation). Tracks with an ordinal sort by it first;
    /// the start/key/id order in byte order is the fallback, so a snapshot without ordinals
    /// still exports deterministically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<u64>,
}

impl CorpusExportSnapshot {
    /// Restricts the snapshot to one video the way [`load_export_snapshot`] does: segments whose
    /// (effective) video is `video_id`, the streams that still have segments (or belong to the
    /// video and had none), the video's runs, and every video those reference.
    pub fn retain_video(&mut self, video_id: Uuid) {
        self.transcript_streams.retain_mut(|stream| {
            let had_segments = !stream.segments.is_empty();
            let stream_video = stream.video_id;
            stream
                .segments
                .retain(|segment| segment.video_id.unwrap_or(stream_video) == video_id);
            !stream.segments.is_empty() || (!had_segments && stream_video == video_id)
        });
        self.visual_text_runs.retain(|run| run.video_id == video_id);
        self.scenes.retain(|scene| scene.video_id == video_id);
        let mut referenced = BTreeSet::from([video_id]);
        for stream in &self.transcript_streams {
            referenced.insert(stream.video_id);
            referenced.extend(
                stream
                    .segments
                    .iter()
                    .filter_map(|segment| segment.video_id),
            );
        }
        self.videos.retain(|video| referenced.contains(&video.id));
    }
}

/// A validated, canonically ordered view of a snapshot: videos, streams and runs by id,
/// segments by `segment_index`, tracks by start time (`None` last), track key and id.
pub(crate) struct OrderedSnapshot<'a> {
    pub videos: BTreeMap<Uuid, &'a ExportVideo>,
    pub streams: Vec<OrderedStream<'a>>,
    pub runs: Vec<OrderedRun<'a>>,
}

pub(crate) struct OrderedStream<'a> {
    pub stream: &'a ExportTranscriptStream,
    pub segments: Vec<&'a ExportTranscriptSegment>,
}

pub(crate) struct OrderedRun<'a> {
    pub run: &'a ExportVisualTextRun,
    pub tracks: Vec<&'a ExportVisualTextTrack>,
}

impl<'a> OrderedSnapshot<'a> {
    pub fn video(&self, id: Uuid) -> anyhow::Result<&'a ExportVideo> {
        self.videos
            .get(&id)
            .copied()
            .with_context(|| format!("unknown video {id}"))
    }
}

/// Validates the shared export rules and orders the snapshot canonically.
///
/// `revision_label` names the revision argument in the error message. Visual-text tracks are
/// validated only when `include_visual_text` is set (formats that export them).
pub(crate) fn order_snapshot<'a>(
    snapshot: &'a CorpusExportSnapshot,
    revision: &str,
    revision_label: &str,
    include_visual_text: bool,
) -> anyhow::Result<OrderedSnapshot<'a>> {
    ensure!(
        !revision.trim().is_empty(),
        "a non-empty {revision_label} revision is required"
    );

    let mut videos = BTreeMap::new();
    for video in &snapshot.videos {
        ensure!(
            videos.insert(video.id, video).is_none(),
            "duplicate video {}",
            video.id
        );
    }

    let mut stream_ids = BTreeSet::new();
    let mut streams = Vec::with_capacity(snapshot.transcript_streams.len());
    for stream in &snapshot.transcript_streams {
        ensure!(
            stream_ids.insert(stream.id),
            "duplicate transcript stream {}",
            stream.id
        );
        ensure!(
            videos.contains_key(&stream.video_id),
            "transcript stream {} references unknown video {}",
            stream.id,
            stream.video_id
        );
        let mut segments = stream.segments.iter().collect::<Vec<_>>();
        segments.sort_by_key(|segment| (segment.segment_index, segment.id));
        for pair in segments.windows(2) {
            ensure!(
                pair[0].segment_index != pair[1].segment_index,
                "transcript stream {} has duplicate segment index {}",
                stream.id,
                pair[0].segment_index
            );
        }
        for segment in &segments {
            if let Some(segment_video) = segment.video_id {
                ensure!(
                    videos.contains_key(&segment_video),
                    "transcript segment {} references unknown video {}",
                    segment.id,
                    segment_video
                );
            }
            validate_optional_timed_range(
                segment.id,
                segment.start_seconds,
                segment.end_seconds,
                "transcript segment",
            )?;
        }
        streams.push(OrderedStream { stream, segments });
    }
    streams.sort_by_key(|stream| stream.stream.id);
    let mut segment_ids = BTreeSet::new();
    for segment in streams.iter().flat_map(|stream| &stream.segments) {
        ensure!(
            segment_ids.insert(segment.id),
            "duplicate transcript segment {}",
            segment.id
        );
    }

    let mut run_ids = BTreeSet::new();
    let mut runs = Vec::with_capacity(snapshot.visual_text_runs.len());
    for run in &snapshot.visual_text_runs {
        ensure!(
            run_ids.insert(run.id),
            "duplicate visual text run {}",
            run.id
        );
        ensure!(
            videos.contains_key(&run.video_id),
            "visual text run {} references unknown video {}",
            run.id,
            run.video_id
        );
        let mut tracks = run.tracks.iter().collect::<Vec<_>>();
        tracks.sort_by(|left, right| compare_tracks(left, right));
        if include_visual_text {
            for track in &tracks {
                validate_optional_timed_range(
                    track.id,
                    track.start_seconds,
                    track.end_seconds,
                    "visual text",
                )?;
            }
        }
        runs.push(OrderedRun { run, tracks });
    }
    runs.sort_by_key(|run| run.run.id);
    if include_visual_text {
        let mut track_ids = BTreeSet::new();
        for track in runs.iter().flat_map(|run| &run.tracks) {
            ensure!(
                track_ids.insert(track.id),
                "duplicate visual text track {}",
                track.id
            );
        }
    }

    Ok(OrderedSnapshot {
        videos,
        streams,
        runs,
    })
}

/// Track order inside a run: the database ordinal when present, then `start_seconds` ascending
/// with `None` last (`-0.0` equal to `0.0`, as in SQL), then `track_key`, then `id`.
fn compare_tracks(left: &ExportVisualTextTrack, right: &ExportVisualTextTrack) -> Ordering {
    // Adding +0.0 turns -0.0 into +0.0 and leaves every other value unchanged.
    let start = |track: &ExportVisualTextTrack| track.start_seconds.map(|value| value + 0.0);
    let (left_start, right_start) = (start(left), start(right));
    (left.ordinal.is_none(), left.ordinal)
        .cmp(&(right.ordinal.is_none(), right.ordinal))
        .then(left_start.is_none().cmp(&right_start.is_none()))
        .then_with(|| match (left_start, right_start) {
            (Some(left), Some(right)) => left.total_cmp(&right),
            _ => Ordering::Equal,
        })
        .then_with(|| left.track_key.cmp(&right.track_key))
        .then_with(|| left.id.cmp(&right.id))
}

/// Lowercase hex SHA-256 of the UTF-8 bytes.
pub(crate) fn sha256_hex(value: &str) -> String {
    crate::lower_hex(&Sha256::digest(value.as_bytes()))
}

/// `sha256:`-prefixed lowercase hex SHA-256 of the UTF-8 bytes.
pub(crate) fn sha256_prefixed(value: &str) -> String {
    format!("sha256:{}", sha256_hex(value))
}

/// Compact JSON with object keys sorted by byte order at every level. Every exported revision
/// hashes this form, so revisions do not depend on serde_json's `preserve_order` feature.
pub(crate) fn canonical_json(value: &Value) -> anyhow::Result<String> {
    fn write(value: &Value, output: &mut String) -> anyhow::Result<()> {
        match value {
            Value::Object(map) => {
                let sorted = map.iter().collect::<BTreeMap<_, _>>();
                output.push('{');
                for (index, (key, value)) in sorted.into_iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    output.push_str(&serde_json::to_string(key)?);
                    output.push(':');
                    write(value, output)?;
                }
                output.push('}');
            }
            Value::Array(items) => {
                output.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    write(item, output)?;
                }
                output.push(']');
            }
            scalar => output.push_str(&serde_json::to_string(scalar)?),
        }
        Ok(())
    }
    let mut output = String::new();
    write(value, &mut output)?;
    Ok(output)
}

/// The per-segment revision material shared by the source-span stream revision and the
/// belief-evidence record revision. `language` is the segment's own language.
pub(crate) fn segment_revision_material(segment: &ExportTranscriptSegment) -> Value {
    serde_json::json!({
        "id": segment.id,
        "sequence": segment.segment_index,
        "textHash": sha256_prefixed(&segment.text),
        "startSeconds": segment.start_seconds,
        "endSeconds": segment.end_seconds,
        "language": &segment.language,
    })
}

// ---------------------------------------------------------------------------------------------
// Format 1: source_span_interchange@1
// ---------------------------------------------------------------------------------------------

/// Exports the snapshot as a pretty-printed `source_span_interchange@1` batch.
pub fn export_source_span(
    snapshot: &CorpusExportSnapshot,
    producer_revision: &str,
) -> anyhow::Result<String> {
    let batch = build_source_span_batch(snapshot, producer_revision)?;
    Ok(format!("{}\n", serde_json::to_string_pretty(&batch)?))
}

// ---------------------------------------------------------------------------------------------
// Format 2: moenarch_corpus_core_batch@1 JSONL
// ---------------------------------------------------------------------------------------------

#[derive(Serialize)]
struct CoreBatchHeader<'a> {
    schema: &'a str,
    schema_version: u32,
    producer: CoreBatchProducer<'a>,
}

#[derive(Serialize)]
struct CoreBatchProducer<'a> {
    name: &'a str,
    revision: &'a str,
}

#[derive(Serialize)]
struct CoreRecordLine<'a> {
    record: &'a CorpusRecord,
}

#[derive(Serialize)]
struct CorePayloadLine<'a> {
    payload: CorePayload<'a>,
}

#[derive(Serialize)]
struct CorePayload<'a> {
    hash: &'a ContentHash,
    media_type: &'a str,
    text: &'a str,
}

/// One text item that becomes a corpus-core segment of a text representation.
struct CoreTextItem<'a> {
    /// Stable identity component of the segment (the transcript segment or track id).
    key: String,
    text: &'a str,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
    metadata: Metadata,
}

/// Exports the snapshot as `moenarch_corpus_core_batch@1` JSONL in document-search's envelope:
/// a header line, one `{"record": ...}` line per corpus-core record, then one `{"payload": ...}`
/// line per distinct representation text, sorted by hash.
pub fn export_corpus_core_jsonl(
    snapshot: &CorpusExportSnapshot,
    producer_revision: &str,
) -> anyhow::Result<String> {
    let ordered = order_snapshot(snapshot, producer_revision, "producer", true)?;

    let mut records = Vec::new();
    let mut payloads = BTreeMap::<ContentHash, String>::new();
    for video in ordered.videos.values() {
        let video_id = video.id.to_string();
        let source_id = SourceId::derive(&[PRODUCER_NAME, &video_id]);
        let mut metadata = Metadata::new();
        metadata.insert("video_id".to_string(), video_id.clone());
        for (key, value) in [
            ("title", &video.title),
            ("channel", &video.channel),
            ("channel_id", &video.channel_id),
            ("uploader", &video.uploader),
            ("uploader_id", &video.uploader_id),
            ("youtube_id", &video.youtube_id),
        ] {
            if let Some(value) = value.as_deref().filter(|value| !value.trim().is_empty()) {
                metadata.insert(key.to_string(), value.to_string());
            }
        }
        records.push(CorpusRecord::Source(Source {
            id: source_id.clone(),
            kind: source_kind(&video.source_url),
            uri: video.source_url.clone(),
            metadata,
        }));

        for ordered_stream in ordered
            .streams
            .iter()
            .filter(|stream| stream.stream.video_id == video.id)
        {
            let stream = ordered_stream.stream;
            let stream_id = stream.id.to_string();
            let asset_id = AssetId::derive(&[source_id.as_str(), "transcript_stream", &stream_id]);
            let mut asset_metadata = Metadata::new();
            asset_metadata.insert("processor".to_string(), stream.processor.clone());
            asset_metadata.insert(
                "processor_version".to_string(),
                stream.processor_version.clone(),
            );
            asset_metadata.insert("transcript_source".to_string(), stream.source_kind.clone());
            asset_metadata.insert("transcript_stream_id".to_string(), stream_id.clone());
            if let Some(language) = &stream.language {
                asset_metadata.insert("language".to_string(), language.clone());
            }
            let items = ordered_stream
                .segments
                .iter()
                .map(|segment| {
                    let mut metadata = Metadata::new();
                    metadata.insert(
                        "segment_index".to_string(),
                        segment.segment_index.to_string(),
                    );
                    metadata.insert("text_sha256".to_string(), sha256_hex(&segment.text));
                    metadata.insert("transcript_segment_id".to_string(), segment.id.to_string());
                    if let Some(language) =
                        non_empty(&segment.language).or_else(|| non_empty(&stream.language))
                    {
                        metadata.insert("language".to_string(), language.to_string());
                    }
                    CoreTextItem {
                        key: segment.id.to_string(),
                        text: &segment.text,
                        start_seconds: segment.start_seconds,
                        end_seconds: segment.end_seconds,
                        metadata,
                    }
                })
                .collect::<Vec<_>>();
            push_text_asset(
                &mut records,
                &mut payloads,
                TextAsset {
                    asset_id,
                    source_id: source_id.clone(),
                    kind: AssetKind::Subtitle,
                    uri: &video.source_url,
                    metadata: asset_metadata,
                    representation_name: "transcript",
                    representation_kind: RepresentationKind::Transcript,
                    assemble_operation: "assemble_transcript",
                    segment_operation: "segment_transcript",
                    producer_revision,
                },
                items,
            )?;
        }

        for ordered_run in ordered
            .runs
            .iter()
            .filter(|run| run.run.video_id == video.id)
        {
            let run = ordered_run.run;
            let run_id = run.id.to_string();
            let asset_id = AssetId::derive(&[source_id.as_str(), "visual_text_run", &run_id]);
            let mut asset_metadata = Metadata::new();
            for (key, value) in [
                ("config_hash", &run.config_hash),
                ("input_hash", &run.input_hash),
                ("model", &run.model),
                ("model_version", &run.model_version),
                ("processor", &run.processor),
                ("processor_version", &run.processor_version),
                ("visual_text_run_id", &run_id),
            ] {
                asset_metadata.insert(key.to_string(), value.clone());
            }
            let items = ordered_run
                .tracks
                .iter()
                .map(|track| {
                    let mut metadata = Metadata::new();
                    metadata.insert("text_sha256".to_string(), sha256_hex(&track.text));
                    metadata.insert("visual_text_role".to_string(), track.role.clone());
                    metadata.insert("visual_text_track_id".to_string(), track.id.to_string());
                    if let Some(language) = &track.language {
                        metadata.insert("language".to_string(), language.clone());
                    }
                    CoreTextItem {
                        key: track.id.to_string(),
                        text: &track.text,
                        start_seconds: track.start_seconds,
                        end_seconds: track.end_seconds,
                        metadata,
                    }
                })
                .collect::<Vec<_>>();
            push_text_asset(
                &mut records,
                &mut payloads,
                TextAsset {
                    asset_id,
                    source_id: source_id.clone(),
                    kind: AssetKind::Video,
                    uri: &video.source_url,
                    metadata: asset_metadata,
                    representation_name: "ocr_text",
                    representation_kind: RepresentationKind::OcrText,
                    assemble_operation: "assemble_visual_text",
                    segment_operation: "segment_visual_text",
                    producer_revision,
                },
                items,
            )?;
        }
    }

    let mut output = serde_json::to_string(&CoreBatchHeader {
        schema: CORPUS_CORE_BATCH_SCHEMA,
        schema_version: CORPUS_CORE_BATCH_VERSION_V1,
        producer: CoreBatchProducer {
            name: PRODUCER_NAME,
            revision: producer_revision,
        },
    })?;
    output.push('\n');
    for record in &records {
        output.push_str(&serde_json::to_string(&CoreRecordLine { record })?);
        output.push('\n');
    }
    for (hash, text) in &payloads {
        output.push_str(&serde_json::to_string(&CorePayloadLine {
            payload: CorePayload {
                hash,
                media_type: TEXT_MEDIA_TYPE,
                text,
            },
        })?);
        output.push('\n');
    }
    Ok(output)
}

struct TextAsset<'a> {
    asset_id: AssetId,
    source_id: SourceId,
    kind: AssetKind,
    uri: &'a str,
    metadata: Metadata,
    representation_name: &'a str,
    representation_kind: RepresentationKind,
    assemble_operation: &'a str,
    segment_operation: &'a str,
    producer_revision: &'a str,
}

/// Pushes one asset, its single text representation (the item texts joined by `"\n"`) and one
/// segment per item addressing its Unicode-scalar range in that text.
fn push_text_asset(
    records: &mut Vec<CorpusRecord>,
    payloads: &mut BTreeMap<ContentHash, String>,
    asset: TextAsset<'_>,
    items: Vec<CoreTextItem<'_>>,
) -> anyhow::Result<()> {
    let representation_id =
        RepresentationId::derive(&[asset.asset_id.as_str(), asset.representation_name]);
    let provenance = |operation: &str, input: CorpusItemRef| Provenance {
        operation: operation.to_string(),
        producer: CORPUS_CORE_PROVENANCE_PRODUCER.to_string(),
        producer_version: Some(asset.producer_revision.to_string()),
        inputs: vec![input],
        parameters_hash: None,
    };

    let mut text = String::new();
    let mut offset = 0_u64;
    let mut segments = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        if index > 0 {
            text.push_str(CORPUS_CORE_TEXT_SEPARATOR);
            offset += CORPUS_CORE_TEXT_SEPARATOR.chars().count() as u64;
        }
        let length = item.text.chars().count() as u64;
        if length == 0 {
            bail!(
                "{} has empty text; corpus-core segments need a non-empty character range",
                item.key
            );
        }
        text.push_str(item.text);
        let character_range = Span::new(offset, offset + length)?;
        offset += length;
        let time_range_ms = match (item.start_seconds, item.end_seconds) {
            (Some(start), Some(end)) => Some(Span::new(seconds_to_ms(start), seconds_to_ms(end))?),
            _ => None,
        };
        let mut selectors = Metadata::new();
        selectors.insert("character_unit".to_string(), "unicode_scalar".to_string());
        selectors.insert(
            "representation_id".to_string(),
            representation_id.to_string(),
        );
        segments.push(CorpusRecord::Segment(Segment {
            id: SegmentId::derive(&[asset.asset_id.as_str(), &item.key]),
            asset_id: asset.asset_id.clone(),
            locator: SegmentLocator {
                byte_range: None,
                character_range: Some(character_range),
                page_index: None,
                time_range_ms,
                frame_range: None,
                selectors,
            },
            provenance: Some(provenance(
                asset.segment_operation,
                CorpusItemRef::Representation(representation_id.clone()),
            )),
            metadata: item.metadata,
        }));
    }

    let content_hash = ContentHash::from_bytes(text.as_bytes());
    records.push(CorpusRecord::Asset(Asset {
        id: asset.asset_id.clone(),
        source_id: asset.source_id,
        kind: asset.kind,
        content: ContentRef {
            uri: asset.uri.to_string(),
            hash: None,
            media_type: None,
            byte_length: None,
        },
        metadata: asset.metadata,
    }));
    records.push(CorpusRecord::Representation(Representation {
        id: representation_id,
        owner: CorpusItemRef::Asset(asset.asset_id.clone()),
        kind: asset.representation_kind,
        content_hash: content_hash.clone(),
        content: None,
        provenance: provenance(
            asset.assemble_operation,
            CorpusItemRef::Asset(asset.asset_id),
        ),
        metadata: Metadata::new(),
    }));
    records.extend(segments);
    payloads.entry(content_hash).or_insert(text);
    Ok(())
}

fn source_kind(uri: &str) -> SourceKind {
    let lower = uri.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        SourceKind::Url
    } else {
        SourceKind::FileSystem
    }
}

/// Milliseconds for a validated (finite, non-negative) seconds value.
fn seconds_to_ms(seconds: f64) -> u64 {
    (seconds * 1000.0).round() as u64
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.is_empty())
}

// ---------------------------------------------------------------------------------------------
// Format 3: belief_evidence_interchange@1
// ---------------------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BeliefEvidenceBatch<'a> {
    schema: &'a str,
    schema_version: u32,
    exporter: BeliefExporter<'a>,
    revision: String,
    evidence: Vec<BeliefEvidence>,
}

#[derive(Serialize)]
struct BeliefExporter<'a> {
    name: &'a str,
    revision: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BeliefEvidence {
    id: String,
    class: &'static str,
    family_id: String,
    source: BeliefSource,
    producer: BeliefProducer,
    #[serde(skip_serializing_if = "Option::is_none")]
    media_span: Option<BeliefMediaSpan>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    scene_ids: Vec<String>,
}

#[derive(Serialize)]
struct BeliefMediaSpan {
    start: BeliefMediaTime,
    end: BeliefMediaTime,
}

#[derive(Serialize)]
struct BeliefMediaTime {
    pts: i64,
    timebase: BeliefTimebase,
}

#[derive(Serialize)]
struct BeliefTimebase {
    num: u32,
    den: u32,
}

/// Microsecond ticks of a stored time, or `None` when it is missing, negative or not finite.
fn media_ticks(seconds: Option<f64>) -> Option<i64> {
    let seconds = seconds.filter(|value| value.is_finite() && *value >= 0.0)?;
    let ticks = (seconds * f64::from(MEDIA_TICKS_PER_SECOND)).round();
    (ticks <= i64::MAX as f64).then_some(ticks as i64)
}

fn media_time(pts: i64) -> BeliefMediaTime {
    BeliefMediaTime {
        pts,
        timebase: BeliefTimebase {
            num: 1,
            den: MEDIA_TICKS_PER_SECOND,
        },
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BeliefSource {
    repository: &'static str,
    scope_id: String,
    record_id: String,
    revision: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BeliefProducer {
    name: String,
    revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_hash: Option<String>,
}

/// Exports one belief-lab evidence reference per transcript segment, ordered by evidence id.
/// References carry the segment's revision but never its text.
pub fn export_belief_evidence(
    snapshot: &CorpusExportSnapshot,
    exporter_revision: &str,
) -> anyhow::Result<String> {
    let ordered = order_snapshot(snapshot, exporter_revision, "exporter", false)?;

    require_immutable_revision("exporter.revision", exporter_revision)?;
    let mut scenes = snapshot.scenes.iter().collect::<Vec<_>>();
    scenes.sort_by(|left, right| {
        left.start_seconds
            .total_cmp(&right.start_seconds)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut evidence = Vec::new();
    for ordered_stream in &ordered.streams {
        let stream = ordered_stream.stream;
        let producer_revision = stream
            .processor_revision
            .as_deref()
            .unwrap_or(&stream.processor_version);
        if !ordered_stream.segments.is_empty() {
            require_immutable_revision("producer.revision", producer_revision).with_context(|| format!(
                "transcript stream {} (processor {}, release {}) has no recorded immutable processor identity; re-ingest/re-segment it or restore its original processor receipt; changing the exporter revision cannot recover historical provenance",
                stream.id, stream.processor, stream.processor_version))?;
        }
        for segment in &ordered_stream.segments {
            let revision = sha256_prefixed(&canonical_json(&segment_revision_material(segment))?);
            let video = segment.effective_video_id(stream);
            // Only stored timing becomes a span; an untimed or empty segment gets none.
            let span = match (
                media_ticks(segment.start_seconds),
                media_ticks(segment.end_seconds),
            ) {
                (Some(start), Some(end)) if start < end => Some((start, end)),
                _ => None,
            };
            let scene_ids = span
                .map(|(start, end)| {
                    scenes
                        .iter()
                        .filter(|scene| {
                            scene.video_id == video
                                && media_ticks(Some(scene.start_seconds))
                                    .zip(media_ticks(Some(scene.end_seconds)))
                                    .is_some_and(|(from, to)| from < end && start < to)
                        })
                        .map(|scene| format!("scene:{}", scene.id))
                        .collect()
                })
                .unwrap_or_default();
            evidence.push(BeliefEvidence {
                id: format!("evidence:youtube-corpus:transcript-segment:{}", segment.id),
                class: "transcript",
                family_id: format!("family:transcript-segment:{}", segment.id),
                source: BeliefSource {
                    repository: PRODUCER_NAME,
                    scope_id: format!("video:{}", segment.effective_video_id(stream)),
                    record_id: format!("transcript-segment:{}", segment.id),
                    revision,
                },
                producer: BeliefProducer {
                    name: stream.processor.clone(),
                    revision: producer_revision.to_string(),
                    config_hash: stream.processing_config_hash.clone(),
                },
                media_span: span.map(|(start, end)| BeliefMediaSpan {
                    start: media_time(start),
                    end: media_time(end),
                }),
                scene_ids,
            });
        }
    }
    evidence.sort_by(|left, right| left.id.cmp(&right.id));
    let revision = sha256_prefixed(&canonical_json(&serde_json::to_value(&evidence)?)?);

    let batch = BeliefEvidenceBatch {
        schema: BELIEF_EVIDENCE_INTERCHANGE_SCHEMA,
        // Version 1 consumers keep working until a reference carries media fields.
        schema_version: if evidence.iter().any(|item| item.media_span.is_some()) {
            BELIEF_EVIDENCE_INTERCHANGE_VERSION_V2
        } else {
            BELIEF_EVIDENCE_INTERCHANGE_VERSION_V1
        },
        exporter: BeliefExporter {
            name: PRODUCER_NAME,
            revision: exporter_revision,
        },
        revision,
        evidence,
    };
    Ok(format!("{}\n", serde_json::to_string_pretty(&batch)?))
}

fn require_immutable_revision(field: &str, value: &str) -> anyhow::Result<()> {
    let prefixed = ["sha256:", "git:", "commit:"]
        .iter()
        .any(|prefix| value.starts_with(prefix) && value.len() > prefix.len());
    let hex = matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    anyhow::ensure!(
        prefixed || hex,
        "{field} must be an immutable revision, got {value:?}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Postgres loader
// ---------------------------------------------------------------------------------------------

/// Loads the export snapshot from Postgres, optionally for one video.
///
/// It reads exactly what the source-span export has always read, with the same scope:
///
/// - transcript segments, filtered by the segment row's own `video_id` (which the schema allows
///   to differ from its stream's; such a segment carries its own [`ExportTranscriptSegment::video_id`]);
/// - the transcript streams of those segments;
/// - OCR visual-text tracks, filtered by their `video_id`, with their runs, each track carrying
///   its database order ([`ExportVisualTextTrack::ordinal`]: `start_seconds NULLS LAST,
///   track_key, id` under the database collation);
/// - every video those rows reference.
///
/// A stream's `processing_config_hash` is `sha256:` of the canonical JSON of its
/// `processing_config`.
pub async fn load_export_snapshot(
    pool: &PgPool,
    video_id: Option<Uuid>,
) -> anyhow::Result<CorpusExportSnapshot> {
    let segment_rows = sqlx::query(
        "SELECT ts.id, ts.stream_id, ts.video_id, ts.segment_index, ts.start_seconds,
                ts.end_seconds, ts.text, ts.language
         FROM transcript_segments ts
         WHERE ($1::uuid IS NULL OR ts.video_id = $1)
         ORDER BY ts.stream_id, ts.segment_index",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    let mut segments_by_stream = BTreeMap::<Uuid, Vec<(Uuid, ExportTranscriptSegment)>>::new();
    for row in segment_rows {
        let segment_index: i64 = row.try_get("segment_index")?;
        let segment_video: Uuid = row.try_get("video_id")?;
        segments_by_stream
            .entry(row.try_get("stream_id")?)
            .or_default()
            .push((
                segment_video,
                ExportTranscriptSegment {
                    id: row.try_get("id")?,
                    segment_index: segment_index
                        .try_into()
                        .context("transcript segment index must be non-negative")?,
                    start_seconds: row.try_get("start_seconds")?,
                    end_seconds: row.try_get("end_seconds")?,
                    text: row.try_get("text")?,
                    language: row.try_get("language")?,
                    video_id: None,
                },
            ));
    }

    let stream_ids = segments_by_stream.keys().copied().collect::<Vec<_>>();
    let stream_rows = sqlx::query(
        "SELECT st.id, st.video_id, st.source_kind, st.language, st.full_text,
                st.processor, st.processor_version, st.processor_revision, st.processing_config
         FROM transcript_streams st
         WHERE st.id = ANY($1)
         ORDER BY st.id",
    )
    .bind(&stream_ids)
    .fetch_all(pool)
    .await?;
    let mut referenced_videos = BTreeSet::new();
    let mut transcript_streams = Vec::with_capacity(stream_rows.len());
    for row in stream_rows {
        let id: Uuid = row.try_get("id")?;
        let stream_video: Uuid = row.try_get("video_id")?;
        let processing_config: Value = row.try_get("processing_config")?;
        let segments = segments_by_stream
            .remove(&id)
            .unwrap_or_default()
            .into_iter()
            .map(|(segment_video, mut segment)| {
                referenced_videos.insert(segment_video);
                if segment_video != stream_video {
                    segment.video_id = Some(segment_video);
                }
                segment
            })
            .collect();
        referenced_videos.insert(stream_video);
        transcript_streams.push(ExportTranscriptStream {
            id,
            video_id: stream_video,
            source_kind: row.try_get("source_kind")?,
            language: row.try_get("language")?,
            full_text: row.try_get("full_text")?,
            processor: row.try_get("processor")?,
            processor_version: row.try_get("processor_version")?,
            processor_revision: row.try_get("processor_revision")?,
            processing_config_hash: Some(sha256_prefixed(&canonical_json(&processing_config)?)),
            segments,
        });
    }
    ensure!(
        segments_by_stream.is_empty(),
        "transcript segments reference missing streams"
    );

    let track_rows = sqlx::query(
        "SELECT t.id, t.run_id, t.track_key, t.text, t.role, t.language,
                t.start_seconds, t.end_seconds
         FROM visual_text_tracks t
         WHERE ($1::uuid IS NULL OR t.video_id = $1)
         ORDER BY t.run_id, t.start_seconds NULLS LAST, t.track_key, t.id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    let mut tracks_by_run = BTreeMap::<Uuid, Vec<ExportVisualTextTrack>>::new();
    for row in track_rows {
        let tracks = tracks_by_run.entry(row.try_get("run_id")?).or_default();
        let ordinal = u64::try_from(tracks.len()).context("visual text track count")?;
        tracks.push(ExportVisualTextTrack {
            id: row.try_get("id")?,
            track_key: row.try_get("track_key")?,
            text: row.try_get("text")?,
            role: row.try_get("role")?,
            language: row.try_get("language")?,
            start_seconds: row.try_get("start_seconds")?,
            end_seconds: row.try_get("end_seconds")?,
            ordinal: Some(ordinal),
        });
    }

    let run_ids = tracks_by_run.keys().copied().collect::<Vec<_>>();
    let run_rows = sqlx::query(
        "SELECT r.id, r.video_id, r.processor, r.processor_version, r.model, r.model_version,
                r.input_hash, r.config_hash
         FROM media_processing_runs r
         WHERE r.id = ANY($1)
         ORDER BY r.id",
    )
    .bind(&run_ids)
    .fetch_all(pool)
    .await?;
    let mut visual_text_runs = Vec::with_capacity(run_rows.len());
    for row in run_rows {
        let id: Uuid = row.try_get("id")?;
        let run_video: Uuid = row.try_get("video_id")?;
        referenced_videos.insert(run_video);
        visual_text_runs.push(ExportVisualTextRun {
            id,
            video_id: run_video,
            processor: row.try_get("processor")?,
            processor_version: row.try_get("processor_version")?,
            model: row.try_get("model")?,
            model_version: row.try_get("model_version")?,
            input_hash: row.try_get("input_hash")?,
            config_hash: row.try_get("config_hash")?,
            tracks: tracks_by_run.remove(&id).unwrap_or_default(),
        });
    }
    ensure!(
        tracks_by_run.is_empty(),
        "visual text tracks reference missing runs"
    );

    let referenced_videos = referenced_videos.into_iter().collect::<Vec<_>>();
    let video_rows = sqlx::query(
        "SELECT v.id, v.youtube_id, v.source_url, v.title, v.channel, v.channel_id,
                v.uploader, v.uploader_id
         FROM videos v
         WHERE v.id = ANY($1)
         ORDER BY v.id",
    )
    .bind(&referenced_videos)
    .fetch_all(pool)
    .await?;
    let mut videos = Vec::with_capacity(video_rows.len());
    for row in video_rows {
        videos.push(ExportVideo {
            id: row.try_get("id")?,
            youtube_id: row.try_get("youtube_id")?,
            source_url: row.try_get("source_url")?,
            title: row.try_get("title")?,
            channel: row.try_get("channel")?,
            channel_id: row.try_get("channel_id")?,
            uploader: row.try_get("uploader")?,
            uploader_id: row.try_get("uploader_id")?,
        });
    }

    let scene_rows = sqlx::query(
        "SELECT s.id, s.video_id, s.start_seconds, s.end_seconds
         FROM video_scenes s
         WHERE ($1::uuid IS NULL OR s.video_id = $1)
         ORDER BY s.video_id, s.start_seconds, s.id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    let scenes = scene_rows
        .into_iter()
        .map(|row| {
            Ok(ExportScene {
                id: row.try_get("id")?,
                video_id: row.try_get("video_id")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(CorpusExportSnapshot {
        videos,
        transcript_streams,
        visual_text_runs,
        scenes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(
        id: u128,
        key: &str,
        start: Option<f64>,
        ordinal: Option<u64>,
    ) -> ExportVisualTextTrack {
        ExportVisualTextTrack {
            id: Uuid::from_u128(id),
            track_key: key.to_string(),
            text: format!("text {key}"),
            role: "subtitle".to_string(),
            language: None,
            start_seconds: start,
            end_seconds: start,
            ordinal,
        }
    }

    fn keys(tracks: &mut [ExportVisualTextTrack]) -> Vec<String> {
        tracks.sort_by(compare_tracks);
        tracks.iter().map(|track| track.track_key.clone()).collect()
    }

    #[test]
    fn negative_zero_start_ties_with_zero() {
        let mut tracks = vec![
            track(1, "b", Some(0.0), None),
            track(2, "a", Some(-0.0), None),
        ];
        assert_eq!(keys(&mut tracks), ["a", "b"]);
        tracks.reverse();
        assert_eq!(keys(&mut tracks), ["a", "b"]);
    }

    #[test]
    fn database_ordinal_wins_over_byte_order_and_is_input_order_independent() {
        // Collation order ("a" before "B") differs from byte order ("B" before "a").
        let mut tracks = vec![
            track(1, "B", Some(1.0), Some(1)),
            track(2, "a", Some(1.0), Some(0)),
            track(3, "late", None, Some(2)),
        ];
        assert_eq!(keys(&mut tracks), ["a", "B", "late"]);
        tracks.reverse();
        assert_eq!(keys(&mut tracks), ["a", "B", "late"]);

        let mut without = tracks
            .into_iter()
            .map(|track| ExportVisualTextTrack {
                ordinal: None,
                ..track
            })
            .collect::<Vec<_>>();
        assert_eq!(keys(&mut without), ["B", "a", "late"]);
    }

    #[test]
    fn retain_video_follows_the_segment_video() {
        let (stream_video, other_video) = (Uuid::from_u128(10), Uuid::from_u128(11));
        let video = |id| ExportVideo {
            id,
            youtube_id: None,
            source_url: format!("https://youtube.test/{id}"),
            title: None,
            channel: None,
            channel_id: None,
            uploader: None,
            uploader_id: None,
        };
        let segment = |index: u64, video_id| ExportTranscriptSegment {
            id: Uuid::from_u128(100 + u128::from(index)),
            segment_index: index,
            start_seconds: None,
            end_seconds: None,
            text: format!("segment {index}"),
            language: None,
            video_id,
        };
        let snapshot = CorpusExportSnapshot {
            videos: vec![video(stream_video), video(other_video)],
            transcript_streams: vec![ExportTranscriptStream {
                id: Uuid::from_u128(20),
                video_id: stream_video,
                source_kind: "asr".to_string(),
                language: None,
                full_text: None,
                processor: "youtube-corpus".to_string(),
                processor_version: "0.1.0".to_string(),
                processor_revision: None,
                processing_config_hash: None,
                segments: vec![segment(0, None), segment(1, Some(other_video))],
            }],
            visual_text_runs: Vec::new(),
            scenes: Vec::new(),
        };

        let mut other = snapshot.clone();
        other.retain_video(other_video);
        assert_eq!(other.transcript_streams.len(), 1);
        assert_eq!(other.transcript_streams[0].segments.len(), 1);
        assert_eq!(other.transcript_streams[0].segments[0].segment_index, 1);
        assert_eq!(other.videos.len(), 2, "the stream's video stays referenced");

        let mut own = snapshot;
        own.retain_video(stream_video);
        assert_eq!(own.transcript_streams[0].segments.len(), 1);
        assert_eq!(own.videos.len(), 1);
    }
}
