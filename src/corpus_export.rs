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
//! 3. [`export_belief_evidence`]: `belief_evidence_interchange@1`, belief-lab evidence
//!    references, one per transcript segment, copying no text.

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
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use crate::source_span_export::{build_source_span_batch, validate_optional_timed_range};

pub const CORPUS_CORE_BATCH_SCHEMA: &str = "moenarch_corpus_core_batch";
pub const CORPUS_CORE_BATCH_VERSION_V1: u32 = 1;
pub const BELIEF_EVIDENCE_INTERCHANGE_SCHEMA: &str = "belief_evidence_interchange";
pub const BELIEF_EVIDENCE_INTERCHANGE_VERSION_V1: u32 = 1;

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
}

impl CorpusExportSnapshot {
    /// Keeps only the given video and the streams and runs that belong to it.
    pub fn retain_video(&mut self, video_id: Uuid) {
        self.videos.retain(|video| video.id == video_id);
        self.transcript_streams
            .retain(|stream| stream.video_id == video_id);
        self.visual_text_runs.retain(|run| run.video_id == video_id);
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
        tracks.sort_by(|left, right| {
            let start = |track: &ExportVisualTextTrack| {
                (
                    track.start_seconds.is_none(),
                    track.start_seconds.unwrap_or(0.0),
                )
            };
            let (left_none, left_start) = start(left);
            let (right_none, right_start) = start(right);
            left_none
                .cmp(&right_none)
                .then(left_start.total_cmp(&right_start))
                .then_with(|| left.track_key.cmp(&right.track_key))
                .then_with(|| left.id.cmp(&right.id))
        });
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

    let mut evidence = Vec::new();
    for ordered_stream in &ordered.streams {
        let stream = ordered_stream.stream;
        for segment in &ordered_stream.segments {
            let revision = sha256_prefixed(&canonical_json(&segment_revision_material(segment))?);
            evidence.push(BeliefEvidence {
                id: format!("evidence:youtube-corpus:transcript-segment:{}", segment.id),
                class: "transcript",
                family_id: format!("family:transcript-segment:{}", segment.id),
                source: BeliefSource {
                    repository: PRODUCER_NAME,
                    scope_id: format!("video:{}", stream.video_id),
                    record_id: format!("transcript-segment:{}", segment.id),
                    revision,
                },
                producer: BeliefProducer {
                    name: stream.processor.clone(),
                    revision: stream.processor_version.clone(),
                    config_hash: stream.processing_config_hash.clone(),
                },
            });
        }
    }
    evidence.sort_by(|left, right| left.id.cmp(&right.id));
    let revision = sha256_prefixed(&canonical_json(&serde_json::to_value(&evidence)?)?);

    let batch = BeliefEvidenceBatch {
        schema: BELIEF_EVIDENCE_INTERCHANGE_SCHEMA,
        schema_version: BELIEF_EVIDENCE_INTERCHANGE_VERSION_V1,
        exporter: BeliefExporter {
            name: PRODUCER_NAME,
            revision: exporter_revision,
        },
        revision,
        evidence,
    };
    Ok(format!("{}\n", serde_json::to_string_pretty(&batch)?))
}

// ---------------------------------------------------------------------------------------------
// Postgres loader
// ---------------------------------------------------------------------------------------------

/// Loads the export snapshot from Postgres, optionally for one video.
///
/// It reads what the source-span export has always read: transcript streams that have
/// segments, OCR visual-text runs that have tracks, and the videos they belong to. A stream's
/// `processing_config_hash` is `sha256:` of the canonical JSON of its `processing_config`.
pub async fn load_export_snapshot(
    pool: &PgPool,
    video_id: Option<Uuid>,
) -> anyhow::Result<CorpusExportSnapshot> {
    let video_rows = sqlx::query(
        "SELECT v.id, v.youtube_id, v.source_url, v.title, v.channel, v.channel_id,
                v.uploader, v.uploader_id
         FROM videos v
         WHERE ($1::uuid IS NULL OR v.id = $1)
           AND (EXISTS (SELECT 1 FROM transcript_segments ts WHERE ts.video_id = v.id)
                OR EXISTS (SELECT 1 FROM visual_text_tracks t WHERE t.video_id = v.id))
         ORDER BY v.id",
    )
    .bind(video_id)
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

    let stream_rows = sqlx::query(
        "SELECT st.id, st.video_id, st.source_kind, st.language, st.full_text,
                st.processor, st.processor_version, st.processing_config
         FROM transcript_streams st
         WHERE ($1::uuid IS NULL OR st.video_id = $1)
           AND EXISTS (SELECT 1 FROM transcript_segments ts WHERE ts.stream_id = st.id)
         ORDER BY st.id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    let mut streams = BTreeMap::new();
    for row in stream_rows {
        let id: Uuid = row.try_get("id")?;
        let processing_config: Value = row.try_get("processing_config")?;
        streams.insert(
            id,
            ExportTranscriptStream {
                id,
                video_id: row.try_get("video_id")?,
                source_kind: row.try_get("source_kind")?,
                language: row.try_get("language")?,
                full_text: row.try_get("full_text")?,
                processor: row.try_get("processor")?,
                processor_version: row.try_get("processor_version")?,
                processing_config_hash: Some(sha256_prefixed(&canonical_json(&processing_config)?)),
                segments: Vec::new(),
            },
        );
    }

    let segment_rows = sqlx::query(
        "SELECT ts.id, ts.stream_id, ts.segment_index, ts.start_seconds, ts.end_seconds,
                ts.text, ts.language
         FROM transcript_segments ts
         JOIN transcript_streams st ON st.id = ts.stream_id
         WHERE ($1::uuid IS NULL OR st.video_id = $1)
         ORDER BY ts.stream_id, ts.segment_index",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    for row in segment_rows {
        let stream_id: Uuid = row.try_get("stream_id")?;
        let segment_index: i64 = row.try_get("segment_index")?;
        let stream = streams
            .get_mut(&stream_id)
            .with_context(|| format!("transcript segment of unloaded stream {stream_id}"))?;
        stream.segments.push(ExportTranscriptSegment {
            id: row.try_get("id")?,
            segment_index: segment_index
                .try_into()
                .context("transcript segment index must be non-negative")?,
            start_seconds: row.try_get("start_seconds")?,
            end_seconds: row.try_get("end_seconds")?,
            text: row.try_get("text")?,
            language: row.try_get("language")?,
        });
    }

    let run_rows = sqlx::query(
        "SELECT r.id, r.video_id, r.processor, r.processor_version, r.model, r.model_version,
                r.input_hash, r.config_hash
         FROM media_processing_runs r
         WHERE ($1::uuid IS NULL OR r.video_id = $1)
           AND EXISTS (SELECT 1 FROM visual_text_tracks t WHERE t.run_id = r.id)
         ORDER BY r.id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    let mut runs = BTreeMap::new();
    for row in run_rows {
        let id: Uuid = row.try_get("id")?;
        runs.insert(
            id,
            ExportVisualTextRun {
                id,
                video_id: row.try_get("video_id")?,
                processor: row.try_get("processor")?,
                processor_version: row.try_get("processor_version")?,
                model: row.try_get("model")?,
                model_version: row.try_get("model_version")?,
                input_hash: row.try_get("input_hash")?,
                config_hash: row.try_get("config_hash")?,
                tracks: Vec::new(),
            },
        );
    }

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
    for row in track_rows {
        let run_id: Uuid = row.try_get("run_id")?;
        let run = runs
            .get_mut(&run_id)
            .with_context(|| format!("visual text track of unloaded run {run_id}"))?;
        run.tracks.push(ExportVisualTextTrack {
            id: row.try_get("id")?,
            track_key: row.try_get("track_key")?,
            text: row.try_get("text")?,
            role: row.try_get("role")?,
            language: row.try_get("language")?,
            start_seconds: row.try_get("start_seconds")?,
            end_seconds: row.try_get("end_seconds")?,
        });
    }

    Ok(CorpusExportSnapshot {
        videos,
        transcript_streams: streams.into_values().collect(),
        visual_text_runs: runs.into_values().collect(),
    })
}
