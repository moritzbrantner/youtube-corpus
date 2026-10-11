//! Acceptance tests for deterministic, versioned downstream corpus exports (issue #42).
//!
//! These tests define the contract; they fail to compile until `youtube_corpus::corpus_export`
//! and the `export` CLI subcommand exist. `tests/export_fixture_contracts.rs` validates the
//! committed expected files against the consumer readers in `tests/support/mod.rs` and runs
//! without the implementation.
//!
//! # Public API (module `youtube_corpus::corpus_export`)
//!
//! ```text
//! pub const CORPUS_CORE_BATCH_SCHEMA: &str = "moenarch_corpus_core_batch";
//! pub const CORPUS_CORE_BATCH_VERSION_V1: u32 = 1;
//! pub const BELIEF_EVIDENCE_INTERCHANGE_SCHEMA: &str = "belief_evidence_interchange";
//! pub const BELIEF_EVIDENCE_INTERCHANGE_VERSION_V1: u32 = 1;
//!
//! pub fn export_source_span(snapshot: &CorpusExportSnapshot, producer_revision: &str)
//!     -> anyhow::Result<String>;
//! pub fn export_corpus_core_jsonl(snapshot: &CorpusExportSnapshot, producer_revision: &str)
//!     -> anyhow::Result<String>;
//! pub fn export_belief_evidence(snapshot: &CorpusExportSnapshot, exporter_revision: &str)
//!     -> anyhow::Result<String>;
//! pub async fn load_export_snapshot(pool: &sqlx::PgPool, video_id: Option<uuid::Uuid>)
//!     -> anyhow::Result<CorpusExportSnapshot>;
//! ```
//!
//! The exporters are pure: the same snapshot (in any vector order) gives the same bytes. They
//! never read the clock or the environment.
//!
//! ## Export snapshot
//!
//! The corpus state the DB queries load, serde camelCase, all types `Debug + Clone + PartialEq
//! + Serialize + Deserialize`; missing `Option` fields deserialize as `None`. Committed example:
//! `tests/fixtures/exports/corpus-snapshot.json`.
//!
//! ```text
//! CorpusExportSnapshot { videos: Vec<ExportVideo>, transcript_streams: Vec<ExportTranscriptStream>,
//!                        #[serde(default)] visual_text_runs: Vec<ExportVisualTextRun> }
//! ExportVideo { id: Uuid, youtube_id: Option<String>, source_url: String, title: Option<String>,
//!               channel: Option<String>, channel_id: Option<String>, uploader: Option<String>,
//!               uploader_id: Option<String> }
//! ExportTranscriptStream { id: Uuid, video_id: Uuid, source_kind: String, language: Option<String>,
//!               full_text: Option<String>, processor: String, processor_version: String,
//!               processing_config_hash: Option<String>, segments: Vec<ExportTranscriptSegment> }
//! ExportTranscriptSegment { id: Uuid, segment_index: u64, start_seconds: Option<f64>,
//!               end_seconds: Option<f64>, text: String, language: Option<String> }
//! ExportVisualTextRun { id: Uuid, video_id: Uuid, processor: String, processor_version: String,
//!               model: String, model_version: String, input_hash: String, config_hash: String,
//!               tracks: Vec<ExportVisualTextTrack> }
//! ExportVisualTextTrack { id: Uuid, track_key: String, text: String, role: String,
//!               language: Option<String>, start_seconds: Option<f64>, end_seconds: Option<f64> }
//! ```
//!
//! `load_export_snapshot` loads exactly what `export_source_span_batch` reads today
//! (`videos`, `transcript_streams`, `transcript_segments`, OCR `visual_text_tracks` with their
//! `media_processing_runs`), optionally for one video. `processing_config_hash` is the
//! loader's choice (e.g. `sha256:` of the stream's canonical `processing_config`) or `None`.
//!
//! ## Shared definitions
//!
//! - `sha256(x)` is lowercase hex SHA-256 of UTF-8 bytes; `sha256:`-prefixed where stated.
//! - Canonical JSON: compact, object keys sorted by byte order (serde_json `Value` order).
//! - Segment order inside a stream: `segment_index`. Track order inside a run: `start_seconds`
//!   ascending with `None` last, then `track_key`, then `id` (the existing SQL order).
//! - Segment revision material (shared by formats 1 and 3): canonical JSON of
//!   `{"endSeconds", "id", "language" (the segment's own, not the stream's), "sequence"
//!   (= segment_index), "startSeconds", "textHash" (= "sha256:" + sha256(text))}`.
//! - Validation, all three formats: an empty or whitespace producer/exporter revision fails
//!   (message contains "revision"); a non-finite or negative start/end fails ("invalid start
//!   time" / "invalid end time"); end before start fails ("reversed timestamps"); a stream or run
//!   whose `video_id` is not in `videos` fails ("unknown video"); two segments of one stream
//!   with the same `segment_index` fail ("duplicate segment index"). Formats 1 and 2 apply the
//!   time checks to visual-text tracks too (format 3 does not export tracks).
//!
//! ## Format 1: `source_span_interchange@1`
//!
//! `serde_json::to_string_pretty(&SourceSpanBatchV1) + "\n"`, built exactly like the existing
//! `export_source_span_batch` (same sources, spans, metadata, revisions and ordering: transcript
//! sources/spans by stream id then segment index, then `ocr:<run id>` sources/spans by run id
//! and track order). `export_source_span_batch` must keep producing the same batch.
//!
//! ## Format 2: `moenarch_corpus_core_batch@1` JSONL
//!
//! document-search's envelope (`serializeCorpusCoreBatch`): header line
//! `{"schema":"moenarch_corpus_core_batch","schema_version":1,"producer":{"name":"youtube-corpus","revision":<rev>}}`,
//! then `{"record":<corpus_core::CorpusRecord>}` lines, then
//! `{"payload":{"hash","media_type":"text/plain; charset=utf-8","text"}}` lines (unique, sorted
//! by hash), each line `serde_json::to_string` of the real `corpus_core` types, `\n`-terminated.
//! Records, videos ordered by id:
//!
//! - source: `SourceId::derive(["youtube-corpus", video_id])`, kind `url` for `http(s)://`
//!   URLs else `file_system`, uri `source_url`, metadata `video_id` plus the present, non-blank
//!   `title`, `channel`, `channel_id`, `uploader`, `uploader_id`, `youtube_id`;
//! - per transcript stream (by stream id): asset `AssetId::derive([source_id,
//!   "transcript_stream", stream_id])`, kind `subtitle`, content `{uri: source_url, hash: null,
//!   media_type: null, byte_length: null}`, metadata `processor`, `processor_version`,
//!   `transcript_source`, `transcript_stream_id`, `language` (if any); representation
//!   `RepresentationId::derive([asset_id, "transcript"])`, owner the asset, kind `transcript`,
//!   `content_hash` = sha256 of the segment texts joined by `"\n"`, content null, metadata `{}`,
//!   provenance `{operation: "assemble_transcript", producer: "youtube-corpus/corpus-export",
//!   producer_version: <rev>, inputs: [asset], parameters_hash: null}`; then one segment per
//!   transcript segment: `SegmentId::derive([asset_id, segment_id])`, `character_range` =
//!   Unicode-scalar range of the segment inside the representation text, `time_range_ms` =
//!   `round(seconds * 1000)` when both start and end exist (else null), other locator ranges
//!   and `page_index` null, selectors `{character_unit: "unicode_scalar", representation_id}`,
//!   provenance like the representation with operation `segment_transcript` and inputs
//!   `[representation]`, metadata `segment_index`, `text_sha256` (bare hex),
//!   `transcript_segment_id`, `language` (segment's, else stream's, if any);
//! - per OCR run (by run id): asset `AssetId::derive([source_id, "visual_text_run", run_id])`,
//!   kind `video`, same content shape, metadata `config_hash`, `input_hash`, `model`,
//!   `model_version`, `processor`, `processor_version`, `visual_text_run_id`; representation
//!   `RepresentationId::derive([asset_id, "ocr_text"])`, kind `ocr_text`, operation
//!   `assemble_visual_text`; segments `SegmentId::derive([asset_id, track_id])` in track
//!   order, operation `segment_visual_text`, metadata `text_sha256`, `visual_text_role`,
//!   `visual_text_track_id`, `language` (if any). No frame ranges (track frame bounds are
//!   inclusive; corpus-core spans are half-open).
//!
//! This is what philosophy-extractor's `source_spans_from_corpus_core_jsonl` already reads:
//! every segment has a scalar character range into one text representation per asset whose
//! text travels as a payload.
//!
//! ## Format 3: `belief_evidence_interchange@1`
//!
//! `serde_json::to_string_pretty + "\n"` of `{schema, schemaVersion: 1, exporter: {name:
//! "youtube-corpus", revision: <exporter rev>}, revision, evidence}`; `revision` =
//! `"sha256:" + sha256(canonical JSON of the evidence array)`. One record per transcript
//! segment, ordered by `id`: `{id: "evidence:youtube-corpus:transcript-segment:<segment id>",
//! class: "transcript", familyId: "family:transcript-segment:<segment id>", source:
//! {repository: "youtube-corpus", scopeId: "video:<video id>", recordId:
//! "transcript-segment:<segment id>", revision: "sha256:" + sha256(segment revision
//! material)}, producer: {name: stream.processor, revision: stream.processor_version,
//! configHash: stream.processing_config_hash (omitted when None)}}`. No `subject`, `score`,
//! `model` or `parentEvidenceIds` keys are emitted, and no text is copied.
//!
//! ## CLI
//!
//! `youtube-corpus export --format <source-span|corpus-core|belief-evidence>
//! --producer-revision <rev> [--video-id <uuid>] [--output <path>] [--snapshot <file.json>]
//! [--migrate]`, parsed into `Command::Export(cli::ExportArgs { format: cli::ExportFormat,
//! producer_revision: String, video_id: Option<Uuid>, output: Option<PathBuf>, snapshot:
//! Option<PathBuf>, migrate: bool })` with `ExportFormat::{SourceSpan, CorpusCore,
//! BeliefEvidence}`. Without `--snapshot` it loads the snapshot from Postgres; with
//! `--snapshot` it reads that JSON file and needs no database. Output goes to stdout, or
//! byte-for-byte to `--output`.
//!
//! ## Criterion 3: what a one-segment text edit changes
//!
//! - Format 1: that span's `text`/`contentHash` and its source's `revision`; the source's
//!   `contentHash` only when the stream has no `full_text` (stored full text stays the
//!   source content). Everything else is identical.
//! - Format 2: that segment's `text_sha256` metadata, its stream's representation
//!   `content_hash` and the stream's payload line. If the scalar length changes, the
//!   `character_range` of that segment's end and of the later segments of the same stream
//!   shifts too; nothing else in those records changes, and every other line is identical.
//! - Format 3: that record's `source.revision` and the batch `revision`.

mod support;

use clap::Parser;
use corpus_core::{AssetId, RepresentationId, SegmentId, SourceId};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use support::*;
use uuid::Uuid;
use youtube_corpus::cli::{Cli, Command, ExportFormat};
use youtube_corpus::corpus_export::{
    export_belief_evidence, export_corpus_core_jsonl, export_source_span, CorpusExportSnapshot,
    ExportTranscriptSegment, BELIEF_EVIDENCE_INTERCHANGE_SCHEMA,
    BELIEF_EVIDENCE_INTERCHANGE_VERSION_V1, CORPUS_CORE_BATCH_SCHEMA, CORPUS_CORE_BATCH_VERSION_V1,
};
use youtube_corpus::source_span_export::{
    SOURCE_SPAN_INTERCHANGE_SCHEMA, SOURCE_SPAN_INTERCHANGE_VERSION_V1,
};

type Exporter = fn(&CorpusExportSnapshot, &str) -> anyhow::Result<String>;

const VIDEO_A: &str = "11111111-1111-4111-8111-111111111111";
const STREAM_ASR: &str = "0a200000-0000-4000-8000-000000000002";
const STREAM_MANUAL: &str = "a1000000-0000-4000-8000-000000000001";
const SEGMENT_ASR_0: &str = "5e000000-0000-4000-8000-0000000000c0";
const SEGMENT_ASR_1: &str = "5e000000-0000-4000-8000-0000000000c1";
const SEGMENT_ASR_2: &str = "5e000000-0000-4000-8000-0000000000c2";
const SEGMENT_MANUAL_0: &str = "5e000000-0000-4000-8000-0000000000a0";

fn exporters() -> [(&'static str, Exporter, &'static str); 3] {
    [
        (
            "source-span",
            export_source_span as Exporter,
            "expected/source-span.json",
        ),
        (
            "corpus-core",
            export_corpus_core_jsonl as Exporter,
            "expected/corpus-core.jsonl",
        ),
        (
            "belief-evidence",
            export_belief_evidence as Exporter,
            "expected/belief-evidence.json",
        ),
    ]
}

fn snapshot() -> CorpusExportSnapshot {
    serde_json::from_str(&read_fixture("corpus-snapshot.json")).expect("fixture snapshot parses")
}

/// The same corpus with every vector reversed.
fn reversed(mut snapshot: CorpusExportSnapshot) -> CorpusExportSnapshot {
    snapshot.videos.reverse();
    snapshot.transcript_streams.reverse();
    for stream in &mut snapshot.transcript_streams {
        stream.segments.reverse();
    }
    snapshot.visual_text_runs.reverse();
    for run in &mut snapshot.visual_text_runs {
        run.tracks.reverse();
    }
    snapshot
}

fn segment_mut<'a>(
    snapshot: &'a mut CorpusExportSnapshot,
    id: &str,
) -> &'a mut ExportTranscriptSegment {
    snapshot
        .transcript_streams
        .iter_mut()
        .flat_map(|stream| stream.segments.iter_mut())
        .find(|segment| segment.id.to_string() == id)
        .unwrap_or_else(|| panic!("fixture segment {id}"))
}

fn error_text(result: anyhow::Result<String>) -> String {
    format!("{:#}", result.expect_err("export must fail"))
}

// --- Criterion 1: determinism --------------------------------------------------------------

#[test]
fn each_format_is_byte_identical_across_runs_and_input_order() {
    let snapshot = snapshot();
    let round_tripped: CorpusExportSnapshot =
        serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
    assert_eq!(round_tripped, snapshot);
    for (name, export, _) in exporters() {
        let first = export(&snapshot, FIXTURE_REVISION).unwrap();
        let second = export(&snapshot, FIXTURE_REVISION).unwrap();
        assert_eq!(first, second, "{name}: two runs differ");
        let shuffled = export(&reversed(snapshot.clone()), FIXTURE_REVISION).unwrap();
        assert_eq!(first, shuffled, "{name}: output depends on input order");
        assert_eq!(
            first,
            export(&round_tripped, FIXTURE_REVISION).unwrap(),
            "{name}"
        );
        assert_no_wall_clock_timestamps(&first);
    }
}

#[test]
fn only_the_revision_argument_changes_the_envelope() {
    let snapshot = snapshot();
    for (name, export, _) in exporters() {
        let first = export(&snapshot, "git:one").unwrap();
        let second = export(&snapshot, "git:two").unwrap();
        assert_ne!(first, second, "{name}: revision must be recorded");
        assert_eq!(
            first.replace("git:one", "git:two"),
            second,
            "{name}: the revision may only appear as itself"
        );
    }
}

// --- Criterion 2: committed expected fixtures and schema versions --------------------------

#[test]
fn source_span_export_matches_committed_fixture() {
    let output = export_source_span(&snapshot(), FIXTURE_REVISION).unwrap();
    assert_eq!(output, read_fixture("expected/source-span.json"));
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], SOURCE_SPAN_INTERCHANGE_SCHEMA);
    assert_eq!(value["schemaVersion"], SOURCE_SPAN_INTERCHANGE_VERSION_V1);
    assert_source_span_matches_snapshot(&output, &snapshot_json());
}

#[test]
fn corpus_core_export_matches_committed_fixture() {
    let output = export_corpus_core_jsonl(&snapshot(), FIXTURE_REVISION).unwrap();
    assert_eq!(output, read_fixture("expected/corpus-core.jsonl"));
    assert_eq!(CORPUS_CORE_BATCH_SCHEMA, "moenarch_corpus_core_batch");
    assert_eq!(CORPUS_CORE_BATCH_VERSION_V1, 1);
    let header: Value = serde_json::from_str(output.lines().next().unwrap()).unwrap();
    assert_eq!(header["schema"], CORPUS_CORE_BATCH_SCHEMA);
    assert_eq!(header["schema_version"], CORPUS_CORE_BATCH_VERSION_V1);
}

#[test]
fn belief_evidence_export_matches_committed_fixture() {
    let output = export_belief_evidence(&snapshot(), FIXTURE_REVISION).unwrap();
    assert_eq!(output, read_fixture("expected/belief-evidence.json"));
    assert_eq!(
        BELIEF_EVIDENCE_INTERCHANGE_SCHEMA,
        "belief_evidence_interchange"
    );
    assert_eq!(BELIEF_EVIDENCE_INTERCHANGE_VERSION_V1, 1);
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["schema"], BELIEF_EVIDENCE_INTERCHANGE_SCHEMA);
    assert_eq!(
        value["schemaVersion"],
        BELIEF_EVIDENCE_INTERCHANGE_VERSION_V1
    );
}

// --- Criterion 3: one segment edit changes only dependent records --------------------------

fn by_id(items: &Value) -> BTreeMap<String, Value> {
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|item| (item["id"].as_str().unwrap().to_string(), item.clone()))
        .collect()
}

fn without(mut value: Value, keys: &[&str]) -> Value {
    for key in keys {
        value.as_object_mut().unwrap().remove(*key);
    }
    value
}

#[test]
fn segment_edit_changes_only_its_source_span_and_source() {
    let before_snapshot = snapshot();
    let mut after_snapshot = before_snapshot.clone();
    segment_mut(&mut after_snapshot, SEGMENT_ASR_0).text =
        "so the question is what essence means".to_string();

    let before: Value =
        serde_json::from_str(&export_source_span(&before_snapshot, FIXTURE_REVISION).unwrap())
            .unwrap();
    let after: Value =
        serde_json::from_str(&export_source_span(&after_snapshot, FIXTURE_REVISION).unwrap())
            .unwrap();
    assert_eq!(
        without(before.clone(), &["sources", "spans"]),
        without(after.clone(), &["sources", "spans"])
    );

    let (sources_before, sources_after) = (by_id(&before["sources"]), by_id(&after["sources"]));
    assert_eq!(
        sources_before.keys().collect::<Vec<_>>(),
        sources_after.keys().collect::<Vec<_>>()
    );
    for (id, source) in &sources_before {
        if id == STREAM_ASR {
            // The ASR stream has no stored full text: its content hash follows the segments.
            assert_ne!(source["revision"], sources_after[id]["revision"]);
            assert_ne!(source["contentHash"], sources_after[id]["contentHash"]);
            assert_eq!(
                without(source.clone(), &["revision", "contentHash"]),
                without(sources_after[id].clone(), &["revision", "contentHash"])
            );
        } else {
            assert_eq!(source, &sources_after[id], "source {id} must not change");
        }
    }

    let (spans_before, spans_after) = (by_id(&before["spans"]), by_id(&after["spans"]));
    assert_eq!(spans_before.len(), spans_after.len());
    for (id, span) in &spans_before {
        if id == SEGMENT_ASR_0 {
            assert_eq!(sources_after[STREAM_ASR]["id"], spans_after[id]["sourceId"]);
            assert_eq!(
                spans_after[id]["contentHash"],
                sha256_prefixed("so the question is what essence means")
            );
            assert_eq!(
                without(span.clone(), &["text", "contentHash"]),
                without(spans_after[id].clone(), &["text", "contentHash"])
            );
        } else {
            assert_eq!(span, &spans_after[id], "span {id} must not change");
        }
    }
}

#[test]
fn segment_edit_with_stored_full_text_keeps_the_source_content_hash() {
    let before_snapshot = snapshot();
    let mut after_snapshot = before_snapshot.clone();
    segment_mut(&mut after_snapshot, SEGMENT_MANUAL_0).text = "First claim about esse.".into();

    let before: Value =
        serde_json::from_str(&export_source_span(&before_snapshot, FIXTURE_REVISION).unwrap())
            .unwrap();
    let after: Value =
        serde_json::from_str(&export_source_span(&after_snapshot, FIXTURE_REVISION).unwrap())
            .unwrap();
    let (before, after) = (by_id(&before["sources"]), by_id(&after["sources"]));
    assert_eq!(
        before[STREAM_MANUAL]["contentHash"],
        after[STREAM_MANUAL]["contentHash"]
    );
    assert_ne!(
        before[STREAM_MANUAL]["revision"],
        after[STREAM_MANUAL]["revision"]
    );
}

struct CoreIds {
    asset: AssetId,
    representation: RepresentationId,
}

fn asr_stream_ids() -> CoreIds {
    let source = SourceId::derive(&["youtube-corpus", VIDEO_A]);
    let asset = AssetId::derive(&[source.as_str(), "transcript_stream", STREAM_ASR]);
    let representation = RepresentationId::derive(&[asset.as_str(), "transcript"]);
    CoreIds {
        asset,
        representation,
    }
}

fn segment_id(ids: &CoreIds, segment: &str) -> String {
    SegmentId::derive(&[ids.asset.as_str(), segment]).to_string()
}

/// Record lines keyed by record id, in file order.
fn record_lines(jsonl: &str) -> Vec<(String, Value)> {
    jsonl
        .lines()
        .skip(1)
        .filter(|line| line.starts_with("{\"record\":"))
        .map(|line| {
            let value: Value = serde_json::from_str(line).unwrap();
            (
                value["record"]["record"]["id"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                value,
            )
        })
        .collect()
}

fn payload_lines(jsonl: &str) -> BTreeSet<String> {
    jsonl
        .lines()
        .filter(|line| line.starts_with("{\"payload\":"))
        .map(str::to_string)
        .collect()
}

fn core_after_edit(text: &str) -> (String, String) {
    let before_snapshot = snapshot();
    let mut after_snapshot = before_snapshot.clone();
    segment_mut(&mut after_snapshot, SEGMENT_ASR_0).text = text.to_string();
    (
        export_corpus_core_jsonl(&before_snapshot, FIXTURE_REVISION).unwrap(),
        export_corpus_core_jsonl(&after_snapshot, FIXTURE_REVISION).unwrap(),
    )
}

fn assert_payload_swap(before: &str, after: &str, old_text: &str, new_text: &str) {
    let (before, after) = (payload_lines(before), payload_lines(after));
    let removed = before.difference(&after).collect::<Vec<_>>();
    let added = after.difference(&before).collect::<Vec<_>>();
    assert_eq!(removed.len(), 1, "exactly the stream payload is replaced");
    assert_eq!(added.len(), 1);
    let removed: Value = serde_json::from_str(removed[0]).unwrap();
    let added: Value = serde_json::from_str(added[0]).unwrap();
    assert!(removed["payload"]["text"]
        .as_str()
        .unwrap()
        .contains(old_text));
    assert!(added["payload"]["text"]
        .as_str()
        .unwrap()
        .contains(new_text));
}

#[test]
fn same_length_segment_edit_changes_only_its_segment_representation_and_payload() {
    let old_text = "so the question is what being means";
    let new_text = "SO the question is what being means";
    let (before, after) = core_after_edit(new_text);
    let ids = asr_stream_ids();
    let target = segment_id(&ids, SEGMENT_ASR_0);

    let (before_records, after_records) = (record_lines(&before), record_lines(&after));
    assert_eq!(
        before_records.iter().map(|(id, _)| id).collect::<Vec<_>>(),
        after_records.iter().map(|(id, _)| id).collect::<Vec<_>>(),
        "identities and order are stable"
    );
    let mut changed = BTreeSet::new();
    for ((id, old), (_, new)) in before_records.iter().zip(&after_records) {
        if old != new {
            changed.insert(id.clone());
        }
        if *id == target {
            let mut old = old.clone();
            old["record"]["record"]["metadata"]["text_sha256"] =
                Value::String(sha256_hex(new_text));
            assert_eq!(&old, new, "only text_sha256 changes on the edited segment");
        }
        if *id == ids.representation.as_str() {
            let mut old = old.clone();
            old["record"]["record"]["content_hash"] =
                new["record"]["record"]["content_hash"].clone();
            assert_eq!(&old, new, "only content_hash changes on the representation");
        }
    }
    assert_eq!(
        changed,
        BTreeSet::from([target, ids.representation.to_string()])
    );
    assert_payload_swap(&before, &after, old_text, new_text);
}

#[test]
fn length_changing_segment_edit_only_shifts_later_ranges_in_its_stream() {
    let old_text = "so the question is what being means";
    let new_text = "so the question is what being really means";
    let shift = (new_text.chars().count() - old_text.chars().count()) as u64;
    let (before, after) = core_after_edit(new_text);
    let ids = asr_stream_ids();
    let target = segment_id(&ids, SEGMENT_ASR_0);
    let later = [
        segment_id(&ids, SEGMENT_ASR_1),
        segment_id(&ids, SEGMENT_ASR_2),
    ];

    let mut changed = BTreeSet::new();
    for ((id, old), (_, new)) in record_lines(&before).iter().zip(&record_lines(&after)) {
        if old != new {
            changed.insert(id.clone());
        }
        let range = |value: &Value, end: &str| {
            value["record"]["record"]["locator"]["character_range"][end]
                .as_u64()
                .unwrap()
        };
        if later.contains(id) {
            assert_eq!(range(new, "start"), range(old, "start") + shift);
            assert_eq!(range(new, "end"), range(old, "end") + shift);
            let mut old = old.clone();
            old["record"]["record"]["locator"]["character_range"] =
                new["record"]["record"]["locator"]["character_range"].clone();
            assert_eq!(&old, new, "later segments only shift");
        }
        if *id == target {
            assert_eq!(range(new, "start"), range(old, "start"));
            assert_eq!(range(new, "end"), range(old, "end") + shift);
        }
    }
    let mut expected = BTreeSet::from([target, ids.representation.to_string()]);
    expected.extend(later);
    assert_eq!(changed, expected);
    assert_payload_swap(&before, &after, old_text, new_text);
    parse_corpus_core_jsonl(&after).expect("edited export stays consumable");
}

#[test]
fn segment_edit_changes_only_its_belief_evidence_record() {
    let before_snapshot = snapshot();
    let mut after_snapshot = before_snapshot.clone();
    segment_mut(&mut after_snapshot, SEGMENT_ASR_0).text =
        "so the question is what essence means".to_string();
    let before: Value =
        serde_json::from_str(&export_belief_evidence(&before_snapshot, FIXTURE_REVISION).unwrap())
            .unwrap();
    let after: Value =
        serde_json::from_str(&export_belief_evidence(&after_snapshot, FIXTURE_REVISION).unwrap())
            .unwrap();
    assert_ne!(before["revision"], after["revision"]);
    assert_eq!(before["exporter"], after["exporter"]);

    let target = format!("evidence:youtube-corpus:transcript-segment:{SEGMENT_ASR_0}");
    let (before, after) = (by_id(&before["evidence"]), by_id(&after["evidence"]));
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    for (id, record) in &before {
        if *id == target {
            assert_ne!(
                record["source"]["revision"],
                after[id]["source"]["revision"]
            );
            let mut record = record.clone();
            record["source"]["revision"] = after[id]["source"]["revision"].clone();
            assert_eq!(&record, &after[id]);
        } else {
            assert_eq!(record, &after[id], "evidence {id} must not change");
        }
    }
}

// --- Criterion 4: corpus-core output parses through the real corpus_core types -------------

#[test]
fn corpus_core_export_parses_into_real_corpus_core_records() {
    let output = export_corpus_core_jsonl(&snapshot(), FIXTURE_REVISION).unwrap();
    let batch = parse_corpus_core_jsonl(&output).expect("every line is a corpus_core record");
    assert_eq!(batch.header.schema, CORPUS_CORE_BATCH_SCHEMA);
    assert_eq!(batch.header.producer.revision, FIXTURE_REVISION);
    assert_eq!(
        batch.record_lines.len() + batch.payload_lines.len() + 1,
        output.lines().count()
    );
    for line in output.lines().skip(1) {
        let value: Value = serde_json::from_str(line).unwrap();
        if let Some(record) = value.get("record") {
            let _: corpus_core::CorpusRecord = serde_json::from_value(record.clone()).unwrap();
        }
    }
    assert_core_matches_snapshot(&batch, &snapshot_json());
}

#[test]
fn corpus_core_envelope_matches_document_search_exporter() {
    let output = export_corpus_core_jsonl(&snapshot(), FIXTURE_REVISION).unwrap();
    assert_core_envelope_matches_document_search(&output);
}

// --- Criterion 5: belief-lab import rules ----------------------------------------------------

#[test]
fn belief_evidence_export_satisfies_belief_lab_import_rules() {
    let output = export_belief_evidence(&snapshot(), "git:exporter-revision").unwrap();
    assert_belief_rules(&output, &snapshot_json());
    let batch = validate_belief_batch(&output).unwrap();
    assert_eq!(batch.exporter.revision, "git:exporter-revision");
    let ids = batch
        .evidence
        .iter()
        .map(|evidence| evidence.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), batch.evidence.len(), "evidence ids are unique");
}

#[test]
fn belief_evidence_references_the_exported_transcript_segment_records() {
    let snapshot = snapshot();
    let spans: Value =
        serde_json::from_str(&export_source_span(&snapshot, FIXTURE_REVISION).unwrap()).unwrap();
    let spans = by_id(&spans["spans"]);
    let batch =
        validate_belief_batch(&export_belief_evidence(&snapshot, FIXTURE_REVISION).unwrap())
            .unwrap();
    for evidence in &batch.evidence {
        let segment_id = evidence
            .source
            .record_id
            .strip_prefix("transcript-segment:")
            .unwrap();
        let span = &spans[segment_id];
        assert_eq!(
            evidence.source.scope_id,
            format!("video:{}", span["metadata"]["videoId"].as_str().unwrap())
        );
    }
}

// --- Criterion 6: input validation -------------------------------------------------------------

#[test]
fn empty_producer_revision_is_rejected_by_every_format() {
    let snapshot = snapshot();
    for (name, export, _) in exporters() {
        for revision in ["", "   "] {
            let message = error_text(export(&snapshot, revision));
            assert!(message.contains("revision"), "{name}: {message}");
        }
    }
}

#[test]
fn reversed_segment_timestamps_are_rejected_by_every_format() {
    let mut snapshot = snapshot();
    let segment = segment_mut(&mut snapshot, SEGMENT_ASR_1);
    segment.start_seconds = Some(6.0);
    segment.end_seconds = Some(3.0);
    for (name, export, _) in exporters() {
        let message = error_text(export(&snapshot, FIXTURE_REVISION));
        assert!(message.contains("reversed timestamps"), "{name}: {message}");
    }
}

#[test]
fn invalid_segment_times_are_rejected_by_every_format() {
    for (start, end, expected) in [
        (Some(-1.0), Some(2.0), "invalid start time"),
        (Some(f64::NAN), Some(2.0), "invalid start time"),
        (Some(0.0), Some(f64::INFINITY), "invalid end time"),
        (None, Some(-0.5), "invalid end time"),
    ] {
        let mut snapshot = snapshot();
        let segment = segment_mut(&mut snapshot, SEGMENT_ASR_1);
        segment.start_seconds = start;
        segment.end_seconds = end;
        for (name, export, _) in exporters() {
            let message = error_text(export(&snapshot, FIXTURE_REVISION));
            assert!(message.contains(expected), "{name}: {message}");
        }
    }
}

#[test]
fn reversed_visual_text_timestamps_are_rejected() {
    let mut snapshot = snapshot();
    let track = &mut snapshot.visual_text_runs[0].tracks[0];
    track.start_seconds = Some(20.0);
    track.end_seconds = Some(10.0);
    for export in [export_source_span as Exporter, export_corpus_core_jsonl] {
        let message = error_text(export(&snapshot, FIXTURE_REVISION));
        assert!(message.contains("reversed timestamps"), "{message}");
    }
}

#[test]
fn streams_of_unknown_videos_are_rejected_by_every_format() {
    let mut snapshot = snapshot();
    snapshot.transcript_streams[0].video_id = Uuid::nil();
    for (name, export, _) in exporters() {
        let message = error_text(export(&snapshot, FIXTURE_REVISION));
        assert!(message.contains("unknown video"), "{name}: {message}");
    }
}

#[test]
fn duplicate_segment_indexes_are_rejected_by_every_format() {
    let mut snapshot = snapshot();
    segment_mut(&mut snapshot, SEGMENT_ASR_1).segment_index = 0;
    for (name, export, _) in exporters() {
        let message = error_text(export(&snapshot, FIXTURE_REVISION));
        assert!(
            message.contains("duplicate segment index"),
            "{name}: {message}"
        );
    }
}

// --- CLI -----------------------------------------------------------------------------------

#[test]
fn export_command_parses_format_revision_and_options() {
    let cli = Cli::try_parse_from([
        "youtube-corpus",
        "export",
        "--format",
        "corpus-core",
        "--producer-revision",
        "git:abc",
        "--video-id",
        VIDEO_A,
        "--output",
        "out.jsonl",
    ])
    .unwrap();
    let Some(Command::Export(args)) = cli.command else {
        panic!("expected the export command");
    };
    assert_eq!(args.format, ExportFormat::CorpusCore);
    assert_eq!(args.producer_revision, "git:abc");
    assert_eq!(args.video_id, Some(Uuid::parse_str(VIDEO_A).unwrap()));
    assert_eq!(args.output, Some(PathBuf::from("out.jsonl")));
    assert_eq!(args.snapshot, None);
    assert!(!args.migrate);

    for (value, format) in [
        ("source-span", ExportFormat::SourceSpan),
        ("corpus-core", ExportFormat::CorpusCore),
        ("belief-evidence", ExportFormat::BeliefEvidence),
    ] {
        let cli = Cli::try_parse_from([
            "youtube-corpus",
            "export",
            "--format",
            value,
            "--producer-revision",
            "git:abc",
            "--snapshot",
            "corpus.json",
            "--migrate",
        ])
        .unwrap();
        let Some(Command::Export(args)) = cli.command else {
            panic!("expected the export command");
        };
        assert_eq!(args.format, format);
        assert_eq!(args.snapshot, Some(PathBuf::from("corpus.json")));
        assert_eq!(args.video_id, None);
        assert_eq!(args.output, None);
        assert!(args.migrate);
    }
}

#[test]
fn export_command_requires_a_known_format_and_a_revision() {
    for args in [
        vec!["youtube-corpus", "export", "--producer-revision", "git:abc"],
        vec!["youtube-corpus", "export", "--format", "corpus-core"],
        vec![
            "youtube-corpus",
            "export",
            "--format",
            "csv",
            "--producer-revision",
            "git:abc",
        ],
        vec![
            "youtube-corpus",
            "export",
            "--format",
            "source-span",
            "--producer-revision",
            "git:abc",
            "--video-id",
            "not-a-uuid",
        ],
    ] {
        assert!(
            Cli::try_parse_from(&args).is_err(),
            "{args:?} must not parse"
        );
    }
}

fn run_cli(args: &[&str], dir: &std::path::Path) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_youtube-corpus"))
        .args(args)
        .current_dir(dir)
        .env_remove("DATABASE_URL")
        .output()
        .expect("run youtube-corpus")
}

#[test]
fn export_command_writes_fixture_exports_from_a_snapshot_without_a_database() {
    let dir = tempfile::tempdir().unwrap();
    let snapshot_path = fixture_path("corpus-snapshot.json");
    let snapshot_path = snapshot_path.to_str().unwrap();
    for (format, _, expected) in exporters() {
        let output = run_cli(
            &[
                "export",
                "--format",
                format,
                "--producer-revision",
                FIXTURE_REVISION,
                "--snapshot",
                snapshot_path,
            ],
            dir.path(),
        );
        assert!(
            output.status.success(),
            "{format}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            read_fixture(expected),
            "{format} stdout"
        );

        let target = dir.path().join(format!("{format}.out"));
        let output = run_cli(
            &[
                "export",
                "--format",
                format,
                "--producer-revision",
                FIXTURE_REVISION,
                "--snapshot",
                snapshot_path,
                "--output",
                target.to_str().unwrap(),
            ],
            dir.path(),
        );
        assert!(output.status.success());
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            read_fixture(expected),
            "{format} --output"
        );
    }
}

#[test]
fn export_command_rejects_an_empty_revision() {
    let dir = tempfile::tempdir().unwrap();
    let snapshot_path = fixture_path("corpus-snapshot.json");
    let output = run_cli(
        &[
            "export",
            "--format",
            "corpus-core",
            "--producer-revision",
            " ",
            "--snapshot",
            snapshot_path.to_str().unwrap(),
        ],
        dir.path(),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("revision"));
    assert!(output.stdout.is_empty());
}

// --- Postgres: the snapshot loader feeds the same source-span batch as today --------------

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn snapshot_loader_matches_the_database_source_span_export() {
    use youtube_corpus::{
        begin_visual_processing_run, export_source_span_batch, upsert_visual_text_track,
        BeginVisualProcessingRunRequest, ProcessingProvenance, UpsertVisualTextTrackRequest,
        VisualProcessingKind, VisualTextRole,
    };

    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();

    let video_id = Uuid::new_v4();
    let stream_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO videos (id, youtube_id, source_url, title, channel, channel_id, uploader)
         VALUES ($1, 'export42', 'https://www.youtube.com/watch?v=export42', 'Export lecture',
                 'Philosophy Channel', 'UC42', 'Lecturer')",
    )
    .bind(video_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status, full_text)
         VALUES ($1, $2, 'asr', 'en', 'completed', NULL)",
    )
    .bind(stream_id)
    .bind(video_id)
    .execute(&pool)
    .await
    .unwrap();
    for (index, (start, end, text)) in [(0.0, 2.5, "Café first."), (2.5, 4.0, "Second 😀 segment.")]
        .into_iter()
        .enumerate()
    {
        sqlx::query(
            "INSERT INTO transcript_segments
               (id, stream_id, video_id, segment_index, start_seconds, end_seconds, text, language)
             VALUES ($1, $2, $3, $4, $5, $6, $7, NULL)",
        )
        .bind(Uuid::new_v4())
        .bind(stream_id)
        .bind(video_id)
        .bind(index as i64)
        .bind(start)
        .bind(end)
        .bind(text)
        .execute(&pool)
        .await
        .unwrap();
    }
    let run_id = begin_visual_processing_run(
        &pool,
        BeginVisualProcessingRunRequest {
            video_id,
            kind: VisualProcessingKind::Ocr,
            provenance: ProcessingProvenance {
                processor: "visual-analysis".to_string(),
                processor_version: "ocr-fixture".to_string(),
                model: "trocr".to_string(),
                model_version: "fixture".to_string(),
                input_hash: "sha256:video".to_string(),
                config_hash: "sha256:ocr-config".to_string(),
                processing_config: serde_json::json!({"sampling": "fixed"}),
            },
        },
    )
    .await
    .unwrap();
    upsert_visual_text_track(
        &pool,
        UpsertVisualTextTrackRequest {
            run_id,
            video_id,
            track_key: "slide".to_string(),
            text: "Ens et essentia".to_string(),
            role: VisualTextRole::PresentationSlide,
            language: Some("la".to_string()),
            sample_count: 1,
            start_frame: Some(30),
            end_frame: Some(30),
            start_seconds: Some(1.0),
            end_seconds: Some(1.0),
            region: None,
            metadata: serde_json::json!({}),
            observation_ids: Vec::new(),
            scene_ids: Vec::new(),
        },
    )
    .await
    .unwrap();

    let snapshot = youtube_corpus::corpus_export::load_export_snapshot(&pool, Some(video_id))
        .await
        .unwrap();
    assert_eq!(snapshot.videos.len(), 1);
    assert_eq!(snapshot.transcript_streams.len(), 1);
    assert_eq!(snapshot.transcript_streams[0].segments.len(), 2);
    assert_eq!(snapshot.visual_text_runs.len(), 1);
    assert_eq!(snapshot.visual_text_runs[0].tracks.len(), 1);

    let from_snapshot = export_source_span(&snapshot, "git:db-parity").unwrap();
    let from_database = export_source_span_batch(&pool, "git:db-parity", Some(video_id))
        .await
        .unwrap();
    assert_eq!(
        from_snapshot,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&from_database).unwrap()
        )
    );
    parse_corpus_core_jsonl(&export_corpus_core_jsonl(&snapshot, "git:db-parity").unwrap())
        .expect("database corpus exports as consumable corpus-core");
    validate_belief_batch(&export_belief_evidence(&snapshot, "git:db-parity").unwrap())
        .expect("database corpus exports as importable belief evidence");

    sqlx::query("DELETE FROM videos WHERE id = $1")
        .bind(video_id)
        .execute(&pool)
        .await
        .unwrap();
}
