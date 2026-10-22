//! Consumer-side readers shared by the downstream export tests (issue #42).
//!
//! These mirror what the real consumers accept, so a fixture or an export that passes here is
//! one the consumer would take:
//!
//! - [`parse_corpus_core_jsonl`] mirrors philosophy-extractor's
//!   `packages/source-ingestion/src/corpus_core_intake.rs` (the `moenarch_corpus_core_batch`
//!   v1 reader), deserializing every record into the real `corpus_core` types;
//! - [`validate_belief_batch`] mirrors belief-lab's `crates/evidence-interchange` v1 parser
//!   (`deny_unknown_fields` envelope, unique ids, self-contained acyclic parents, known classes,
//!   import-allowed score semantics only);
//! - [`validate_source_span_batch`] checks the `source_span_interchange@1` invariants.
#![allow(dead_code)]

use corpus_core::{ContentHash, CorpusItemRef, CorpusRecord, RepresentationKind, SegmentId, Span};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const FIXTURE_REVISION: &str = "git:fixture";

pub fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/exports")
        .join(relative)
}

pub fn read_fixture(relative: &str) -> String {
    let path = fixture_path(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

/// The committed fixture corpus as untyped JSON (independent of the snapshot struct).
pub fn snapshot_json() -> Value {
    serde_json::from_str(&read_fixture("corpus-snapshot.json")).unwrap()
}

pub fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn sha256_prefixed(text: &str) -> String {
    format!("sha256:{}", sha256_hex(text))
}

/// Every transcript segment text and visual-text track text in the fixture corpus.
pub fn snapshot_texts(snapshot: &Value) -> Vec<String> {
    let mut texts = Vec::new();
    for stream in snapshot["transcriptStreams"].as_array().unwrap() {
        for segment in stream["segments"].as_array().unwrap() {
            texts.push(segment["text"].as_str().unwrap().to_string());
        }
    }
    for run in snapshot["visualTextRuns"].as_array().unwrap() {
        for track in run["tracks"].as_array().unwrap() {
            texts.push(track["text"].as_str().unwrap().to_string());
        }
    }
    texts
}

/// Fails when the output contains anything shaped like an RFC 3339 wall-clock timestamp
/// (`YYYY-MM-DDTHH`). No exported record carries one.
pub fn assert_no_wall_clock_timestamps(output: &str) {
    let bytes = output.as_bytes();
    for start in 0..bytes.len().saturating_sub(12) {
        let window = &bytes[start..start + 13];
        let digits = |range: std::ops::Range<usize>| window[range].iter().all(u8::is_ascii_digit);
        if digits(0..4)
            && window[4] == b'-'
            && digits(5..7)
            && window[7] == b'-'
            && digits(8..10)
            && window[10] == b'T'
            && digits(11..13)
        {
            panic!(
                "export contains a timestamp-like value near byte {start}: {}",
                String::from_utf8_lossy(window)
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// moenarch_corpus_core_batch v1 (philosophy-extractor intake mirror)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreBatchHeader {
    pub schema: String,
    pub schema_version: u64,
    pub producer: CoreBatchProducer,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreBatchProducer {
    pub name: String,
    pub revision: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum CoreBatchLine {
    Record(Box<CorpusRecord>),
    Payload(CorePayload),
}

#[derive(Serialize)]
struct RecordLine<'a> {
    record: &'a CorpusRecord,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorePayload {
    pub hash: String,
    pub media_type: Option<String>,
    pub text: String,
}

#[derive(Debug)]
pub struct CoreBatch {
    pub header: CoreBatchHeader,
    /// Records in file order.
    pub records: Vec<CorpusRecord>,
    /// Raw record lines in file order (parallel to `records`).
    pub record_lines: Vec<String>,
    /// Payloads in file order.
    pub payloads: Vec<CorePayload>,
    /// Raw payload lines in file order (parallel to `payloads`).
    pub payload_lines: Vec<String>,
    /// Text of every segment, recovered from its representation payload by character range.
    pub segment_texts: BTreeMap<SegmentId, String>,
}

/// Parses and validates a `moenarch_corpus_core_batch` v1 JSONL document the way
/// philosophy-extractor's intake does, and additionally requires every record line to be the
/// byte-exact canonical serde serialization of the parsed `corpus_core` record.
pub fn parse_corpus_core_jsonl(jsonl: &str) -> Result<CoreBatch, String> {
    if !jsonl.ends_with('\n') {
        return Err("batch must end with a newline".into());
    }
    let mut lines = jsonl.lines();
    let header_line = lines.next().ok_or("batch is empty")?;
    let header: CoreBatchHeader = serde_json::from_str(header_line)
        .map_err(|error| format!("invalid batch header: {error}"))?;
    if header.schema != "moenarch_corpus_core_batch" || header.schema_version != 1 {
        return Err(format!(
            "unsupported batch {}@{}",
            header.schema, header.schema_version
        ));
    }
    if header.producer.name.trim().is_empty() || header.producer.revision.trim().is_empty() {
        return Err("producer name and revision are required".into());
    }

    let mut records = Vec::new();
    let mut record_lines = Vec::new();
    let mut payloads = Vec::new();
    let mut payload_lines = Vec::new();
    let mut payload_text = BTreeMap::<ContentHash, String>::new();
    let mut seen_payload = false;
    for (index, line) in lines.enumerate() {
        let number = index + 2;
        if line.trim().is_empty() {
            return Err(format!("line {number} is blank"));
        }
        let parsed: CoreBatchLine = serde_json::from_str(line)
            .map_err(|error| format!("line {number} is not a record or payload: {error}"))?;
        match parsed {
            CoreBatchLine::Record(record) => {
                if seen_payload {
                    return Err(format!("record on line {number} follows a payload line"));
                }
                let canonical = serde_json::to_string(&RecordLine { record: &record })
                    .map_err(|error| error.to_string())?;
                if canonical != line {
                    return Err(format!(
                        "line {number} does not round-trip through corpus_core types"
                    ));
                }
                records.push(*record);
                record_lines.push(line.to_string());
            }
            CoreBatchLine::Payload(payload) => {
                seen_payload = true;
                let hash = ContentHash::parse(payload.hash.clone())
                    .map_err(|error| format!("payload on line {number}: {error}"))?;
                if ContentHash::from_bytes(payload.text.as_bytes()) != hash {
                    return Err(format!("payload on line {number} does not match its hash"));
                }
                if payload_text.insert(hash, payload.text.clone()).is_some() {
                    return Err(format!("duplicate payload on line {number}"));
                }
                payloads.push(payload);
                payload_lines.push(line.to_string());
            }
        }
    }

    let segment_texts = check_corpus_core_records(&records, &payload_text)?;
    Ok(CoreBatch {
        header,
        records,
        record_lines,
        payloads,
        payload_lines,
        segment_texts,
    })
}

fn item_id(item: &CorpusItemRef) -> String {
    match item {
        CorpusItemRef::Source(id) => id.to_string(),
        CorpusItemRef::Asset(id) => id.to_string(),
        CorpusItemRef::Segment(id) => id.to_string(),
        CorpusItemRef::Representation(id) => id.to_string(),
    }
}

fn check_corpus_core_records(
    records: &[CorpusRecord],
    payloads: &BTreeMap<ContentHash, String>,
) -> Result<BTreeMap<SegmentId, String>, String> {
    let mut known = BTreeSet::new();
    for record in records {
        if !known.insert(record.item_ref()) {
            return Err(format!("duplicate record {}", item_id(&record.item_ref())));
        }
    }
    let mut representations = BTreeMap::new();
    for record in records {
        match record {
            CorpusRecord::Source(source) => {
                if source.id.as_str().trim().is_empty() || source.uri.trim().is_empty() {
                    return Err("source id and uri are required".into());
                }
            }
            CorpusRecord::Asset(asset) => {
                if !known.contains(&CorpusItemRef::Source(asset.source_id.clone())) {
                    return Err(format!("asset {} has unknown source", asset.id));
                }
                if let Some(hash) = &asset.content.hash {
                    ContentHash::parse(hash.as_str()).map_err(|error| error.to_string())?;
                }
            }
            CorpusRecord::Representation(representation) => {
                ContentHash::parse(representation.content_hash.as_str())
                    .map_err(|error| error.to_string())?;
                if !known.contains(&representation.owner) {
                    return Err(format!(
                        "representation {} has unknown owner",
                        representation.id
                    ));
                }
                representations.insert(representation.id.to_string(), representation);
            }
            CorpusRecord::Segment(segment) => {
                if !known.contains(&CorpusItemRef::Asset(segment.asset_id.clone())) {
                    return Err(format!("segment {} has unknown asset", segment.id));
                }
                let locator = &segment.locator;
                for span in [
                    locator.byte_range,
                    locator.character_range,
                    locator.time_range_ms,
                    locator.frame_range,
                ]
                .into_iter()
                .flatten()
                {
                    Span::new(span.start, span.end).map_err(|error| error.to_string())?;
                }
            }
        }
        let provenance = match record {
            CorpusRecord::Representation(representation) => Some(&representation.provenance),
            CorpusRecord::Segment(segment) => segment.provenance.as_ref(),
            CorpusRecord::Source(_) | CorpusRecord::Asset(_) => None,
        };
        for input in provenance
            .into_iter()
            .flat_map(|provenance| &provenance.inputs)
        {
            if !known.contains(input) {
                return Err(format!(
                    "{} provenance references unknown input {}",
                    item_id(&record.item_ref()),
                    item_id(input)
                ));
            }
        }
    }

    // Text segments: philosophy-extractor maps only segments with a Unicode-scalar character
    // range into a text representation owned by the segment's asset, one representation per
    // asset, with its text carried as a payload.
    let mut texts = BTreeMap::new();
    let mut asset_representation = BTreeMap::<String, String>::new();
    for record in records {
        let CorpusRecord::Segment(segment) = record else {
            continue;
        };
        let range = segment
            .locator
            .character_range
            .ok_or_else(|| format!("segment {} has no character range", segment.id))?;
        let selectors = &segment.locator.selectors;
        if selectors.get("character_unit").map(String::as_str) != Some("unicode_scalar") {
            return Err(format!(
                "segment {} must declare character_unit unicode_scalar",
                segment.id
            ));
        }
        let representation_id = selectors
            .get("representation_id")
            .ok_or_else(|| format!("segment {} names no representation", segment.id))?;
        let representation = representations
            .get(representation_id)
            .ok_or_else(|| format!("segment {} has unknown representation", segment.id))?;
        if representation.owner != CorpusItemRef::Asset(segment.asset_id.clone()) {
            return Err(format!(
                "segment {} indexes a representation not owned by its asset",
                segment.id
            ));
        }
        if !matches!(
            representation.kind,
            RepresentationKind::ExtractedText
                | RepresentationKind::OcrText
                | RepresentationKind::Transcript
        ) {
            return Err(format!(
                "segment {} indexes a non-text representation",
                segment.id
            ));
        }
        if let Some(previous) =
            asset_representation.insert(segment.asset_id.to_string(), representation.id.to_string())
        {
            if previous != representation.id.as_str() {
                return Err(format!(
                    "asset {} has segments over more than one representation",
                    segment.asset_id
                ));
            }
        }
        let text = payloads
            .get(&representation.content_hash)
            .ok_or_else(|| format!("representation {} has no payload", representation.id))?;
        let scalars = text.chars().collect::<Vec<_>>();
        let (start, end) = (range.start as usize, range.end as usize);
        if range.is_empty() || end > scalars.len() {
            return Err(format!(
                "segment {} character range {start}..{end} is empty or out of bounds",
                segment.id
            ));
        }
        texts.insert(
            segment.id.clone(),
            scalars[start..end].iter().collect::<String>(),
        );
    }
    Ok(texts)
}

// ---------------------------------------------------------------------------------------------
// belief_evidence_interchange v1 (belief-lab importer mirror)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeliefBatch {
    pub schema: String,
    pub schema_version: u32,
    pub exporter: BeliefExporter,
    pub revision: String,
    #[serde(default)]
    pub evidence: Vec<BeliefEvidence>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeliefExporter {
    pub name: String,
    pub revision: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeliefEvidence {
    pub id: String,
    #[serde(default)]
    pub subject: Option<String>,
    pub class: String,
    pub family_id: String,
    pub source: BeliefSource,
    pub producer: BeliefProducer,
    #[serde(default)]
    pub score: Option<BeliefScore>,
    #[serde(default)]
    pub parent_evidence_ids: Vec<String>,
    #[serde(default)]
    pub media_span: Option<BeliefMediaSpan>,
    #[serde(default)]
    pub scene_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeliefMediaSpan {
    pub start: BeliefMediaTime,
    pub end: BeliefMediaTime,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeliefMediaTime {
    pub pts: i64,
    pub timebase: BeliefTimebase,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeliefTimebase {
    pub num: i64,
    pub den: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeliefSource {
    pub repository: String,
    pub scope_id: String,
    pub record_id: String,
    pub revision: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeliefProducer {
    pub name: String,
    pub revision: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub config_hash: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeliefScore {
    pub value: f64,
    pub semantics: String,
}

const BELIEF_CLASSES: [&str; 9] = [
    "user_assertion",
    "metadata",
    "transcript",
    "ocr",
    "object_detection",
    "scene_detection",
    "named_entity",
    "face_track_reference",
    "voice_track_reference",
];
const IMPORTABLE_SCORE_SEMANTICS: [&str; 3] =
    ["detector_confidence", "model_confidence", "similarity"];

fn non_empty(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    Ok(())
}

fn immutable_revision(field: &str, value: &str) -> Result<(), String> {
    non_empty(field, value)?;
    let prefixed = ["sha256:", "git:", "commit:"]
        .iter()
        .any(|prefix| value.starts_with(prefix) && value.len() > prefix.len());
    let hex = matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    if prefixed || hex {
        Ok(())
    } else {
        Err(format!("{field} must be immutable"))
    }
}

/// Parses and validates a `belief_evidence_interchange@1`/`@2` batch like belief-lab's importer.
pub fn validate_belief_batch(json: &str) -> Result<BeliefBatch, String> {
    let batch: BeliefBatch =
        serde_json::from_str(json).map_err(|error| format!("invalid JSON: {error}"))?;
    if batch.schema != "belief_evidence_interchange" || !matches!(batch.schema_version, 1 | 2) {
        return Err(format!(
            "unsupported schema {}@{}",
            batch.schema, batch.schema_version
        ));
    }
    non_empty("exporter.name", &batch.exporter.name)?;
    immutable_revision("exporter.revision", &batch.exporter.revision)?;
    non_empty("revision", &batch.revision)?;

    let mut parents = BTreeMap::<String, BTreeSet<String>>::new();
    for evidence in &batch.evidence {
        non_empty("evidence.id", &evidence.id)?;
        if parents
            .insert(evidence.id.clone(), BTreeSet::new())
            .is_some()
        {
            return Err(format!("duplicate evidence id {}", evidence.id));
        }
    }
    for evidence in &batch.evidence {
        if let Some(subject) = &evidence.subject {
            non_empty("subject", subject)?;
        }
        if !BELIEF_CLASSES.contains(&evidence.class.as_str()) {
            return Err(format!("unknown evidence class {}", evidence.class));
        }
        non_empty("familyId", &evidence.family_id)?;
        non_empty("source.repository", &evidence.source.repository)?;
        non_empty("source.scopeId", &evidence.source.scope_id)?;
        non_empty("source.recordId", &evidence.source.record_id)?;
        immutable_revision("source.revision", &evidence.source.revision)?;
        non_empty("producer.name", &evidence.producer.name)?;
        immutable_revision("producer.revision", &evidence.producer.revision)?;
        if let Some(model) = &evidence.producer.model {
            non_empty("producer.model", model)?;
        }
        if let Some(config_hash) = &evidence.producer.config_hash {
            non_empty("producer.configHash", config_hash)?;
        }
        if let Some(score) = &evidence.score {
            if !IMPORTABLE_SCORE_SEMANTICS.contains(&score.semantics.as_str()) {
                return Err(format!(
                    "evidence {} carries non-importable score semantics {}",
                    evidence.id, score.semantics
                ));
            }
            if !score.value.is_finite() || !(0.0..=1.0).contains(&score.value) {
                return Err(format!("evidence {} has an invalid score", evidence.id));
            }
        }
        validate_media(batch.schema_version, evidence)?;
        for parent in &evidence.parent_evidence_ids {
            if parent == &evidence.id {
                return Err(format!("evidence {} is its own parent", evidence.id));
            }
            if !parents.contains_key(parent) {
                return Err(format!(
                    "evidence {} has missing parent {parent}",
                    evidence.id
                ));
            }
        }
        parents.insert(
            evidence.id.clone(),
            evidence.parent_evidence_ids.iter().cloned().collect(),
        );
    }
    // Kahn-style cycle check, as belief-lab's topological ordering does.
    let mut emitted = BTreeSet::new();
    while emitted.len() < parents.len() {
        let ready = parents
            .iter()
            .filter(|(id, deps)| !emitted.contains(*id) && deps.is_subset(&emitted))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Err("evidence parents form a cycle".into());
        }
        emitted.extend(ready);
    }
    Ok(batch)
}

// ---------------------------------------------------------------------------------------------
// source_span_interchange v1
// ---------------------------------------------------------------------------------------------

pub fn validate_source_span_batch(json: &str) -> Result<youtube_corpus::SourceSpanBatchV1, String> {
    let batch: youtube_corpus::SourceSpanBatchV1 =
        serde_json::from_str(json).map_err(|error| format!("invalid JSON: {error}"))?;
    if batch.schema != "source_span_interchange" || batch.schema_version != 1 {
        return Err(format!(
            "unsupported schema {}@{}",
            batch.schema, batch.schema_version
        ));
    }
    if batch.producer.revision.trim().is_empty() {
        return Err("producer revision is required".into());
    }
    let mut source_ids = BTreeSet::new();
    for source in &batch.sources {
        if !source_ids.insert(source.id.clone()) {
            return Err(format!("duplicate source {}", source.id));
        }
        if !source.revision.starts_with("sha256:") || !source.content_hash.starts_with("sha256:") {
            return Err(format!(
                "source {} hashes must be sha256-prefixed",
                source.id
            ));
        }
    }
    let mut span_ids = BTreeSet::new();
    for span in &batch.spans {
        if !span_ids.insert(span.id.clone()) {
            return Err(format!("duplicate span {}", span.id));
        }
        if !source_ids.contains(&span.source_id) {
            return Err(format!("span {} has unknown source", span.id));
        }
        if span.content_hash != sha256_prefixed(&span.text) {
            return Err(format!(
                "span {} content hash does not match its text",
                span.id
            ));
        }
    }
    Ok(batch)
}

// ---------------------------------------------------------------------------------------------
// Shared contract assertions
// ---------------------------------------------------------------------------------------------

/// Compact JSON with object keys sorted recursively (independent of serde_json's
/// `preserve_order` feature). This is the canonical form every exported revision hashes.
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let sorted = map.iter().collect::<BTreeMap<_, _>>();
            let body = sorted
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap(),
                        canonical_json(value)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        scalar => serde_json::to_string(scalar).unwrap(),
    }
}

/// The per-segment revision material shared by the source-span stream revision and the
/// belief-evidence record revision.
pub fn segment_revision(segment: &Value) -> String {
    let material = serde_json::json!({
        "id": segment["id"],
        "sequence": segment["segmentIndex"],
        "textHash": sha256_prefixed(segment["text"].as_str().unwrap()),
        "startSeconds": segment.get("startSeconds").cloned().unwrap_or(Value::Null),
        "endSeconds": segment.get("endSeconds").cloned().unwrap_or(Value::Null),
        "language": segment.get("language").cloned().unwrap_or(Value::Null),
    });
    sha256_prefixed(&canonical_json(&material))
}

fn key_order(line: &str) -> Vec<String> {
    // serde_json::Value may sort keys, so read the top-level key order from the raw text.
    let value: Value = serde_json::from_str(line).unwrap();
    let object = value.as_object().unwrap();
    let mut keys = object.keys().cloned().collect::<Vec<_>>();
    keys.sort_by_key(|key| line.find(&format!("\"{key}\":")).unwrap());
    keys
}

/// Asserts that a corpus-core JSONL document uses exactly document-search's batch envelope:
/// the same header keys in the same order, one-key `record`/`payload` body lines, records before
/// payloads, payload objects keyed `hash`/`media_type`/`text` and sorted by hash, and a trailing
/// newline.
pub fn assert_core_envelope_matches_document_search(ours: &str) {
    let theirs = read_fixture("consumers/document-search-corpus-core-batch.jsonl");
    let our_lines = ours.lines().collect::<Vec<_>>();
    let their_lines = theirs.lines().collect::<Vec<_>>();
    assert!(ours.ends_with('\n') && !ours.ends_with("\n\n"));

    assert_eq!(key_order(our_lines[0]), key_order(their_lines[0]));
    let our_header: Value = serde_json::from_str(our_lines[0]).unwrap();
    let their_header: Value = serde_json::from_str(their_lines[0]).unwrap();
    assert_eq!(our_header["schema"], their_header["schema"]);
    assert_eq!(our_header["schema_version"], their_header["schema_version"]);
    assert!(their_lines[0].starts_with(
        "{\"schema\":\"moenarch_corpus_core_batch\",\"schema_version\":1,\"producer\":{\"name\":"
    ));
    assert!(our_lines[0].starts_with(
        "{\"schema\":\"moenarch_corpus_core_batch\",\"schema_version\":1,\"producer\":{\"name\":\"youtube-corpus\",\"revision\":"
    ));
    assert!(our_lines[0].ends_with("\"}}"));

    let kinds = |lines: &[&str]| {
        lines[1..]
            .iter()
            .map(|line| {
                let keys = key_order(line);
                assert_eq!(keys.len(), 1, "body line must hold one key: {line}");
                keys[0].clone()
            })
            .collect::<Vec<_>>()
    };
    for body in [kinds(&our_lines), kinds(&their_lines)] {
        let first_payload = body.iter().position(|kind| kind == "payload");
        if let Some(first_payload) = first_payload {
            assert!(body[first_payload..].iter().all(|kind| kind == "payload"));
        }
        assert!(body
            .iter()
            .all(|kind| kind == "record" || kind == "payload"));
    }

    let mut hashes = Vec::new();
    for line in our_lines
        .iter()
        .filter(|line| line.starts_with("{\"payload\":"))
    {
        let value: Value = serde_json::from_str(line).unwrap();
        let payload = &value["payload"];
        assert_eq!(
            payload.as_object().unwrap().len(),
            3,
            "payload keys: hash, media_type, text"
        );
        assert!(line.starts_with("{\"payload\":{\"hash\":"));
        assert!(line.contains("\",\"media_type\":\"text/plain; charset=utf-8\",\"text\":"));
        hashes.push(payload["hash"].as_str().unwrap().to_string());
    }
    let mut sorted = hashes.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(hashes, sorted, "payloads are unique and sorted by hash");
}

/// Asserts belief-lab's import rules plus youtube-corpus' reference contract against the
/// fixture corpus.
pub fn assert_belief_rules(json: &str, snapshot: &Value) {
    let batch = validate_belief_batch(json).expect("belief-lab would accept the batch");
    assert_eq!(batch.exporter.name, "youtube-corpus");

    // Exporter revision is separate from every source and producer revision.
    for evidence in &batch.evidence {
        assert_ne!(batch.exporter.revision, evidence.source.revision);
        assert_ne!(batch.exporter.revision, evidence.producer.revision);
        assert!(evidence.source.revision.starts_with("sha256:"));
        assert!(evidence.parent_evidence_ids.is_empty());
        assert!(evidence
            .score
            .iter()
            .all(|score| IMPORTABLE_SCORE_SEMANTICS.contains(&score.semantics.as_str())));
    }

    // No copied transcript or visual text, and no `text` field anywhere.
    for text in snapshot_texts(snapshot) {
        assert!(!json.contains(&text), "belief export copies text {text:?}");
        let escaped = serde_json::to_string(&text).unwrap();
        assert!(!json.contains(escaped.trim_matches('"')));
    }
    assert!(!json.contains("\"text\""));
    assert!(!json.contains("soft_truth") && !json.contains("posterior_probability"));

    // Batch revision is the hash of the canonical evidence array.
    let value: Value = serde_json::from_str(json).unwrap();
    assert_eq!(
        batch.revision,
        sha256_prefixed(&canonical_json(&value["evidence"]))
    );

    // One evidence record per transcript segment, ordered by evidence id.
    let mut expected = BTreeMap::new();
    for stream in snapshot["transcriptStreams"].as_array().unwrap() {
        for segment in stream["segments"].as_array().unwrap() {
            let id = segment["id"].as_str().unwrap();
            expected.insert(
                format!("evidence:youtube-corpus:transcript-segment:{id}"),
                (stream, segment),
            );
        }
    }
    let ids = batch
        .evidence
        .iter()
        .map(|evidence| evidence.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(ids, expected.keys().cloned().collect::<Vec<_>>());
    for evidence in &batch.evidence {
        let (stream, segment) = expected[&evidence.id];
        let segment_id = segment["id"].as_str().unwrap();
        assert_eq!(evidence.class, "transcript");
        assert_eq!(evidence.subject, None);
        assert_eq!(
            evidence.family_id,
            format!("family:transcript-segment:{segment_id}")
        );
        assert_eq!(evidence.source.repository, "youtube-corpus");
        assert_eq!(
            evidence.source.scope_id,
            format!("video:{}", stream["videoId"].as_str().unwrap())
        );
        assert_eq!(
            evidence.source.record_id,
            format!("transcript-segment:{segment_id}")
        );
        assert_eq!(evidence.source.revision, segment_revision(segment));
        assert_eq!(
            evidence.producer.name,
            stream["processor"].as_str().unwrap()
        );
        assert_eq!(
            evidence.producer.revision,
            stream
                .get("processorRevision")
                .unwrap_or(&stream["processorVersion"])
                .as_str()
                .unwrap()
        );
        assert_eq!(evidence.producer.model, None);
        assert_eq!(
            evidence.producer.config_hash.as_deref(),
            stream["processingConfigHash"].as_str()
        );
    }
}

/// Asserts the corpus-core export's identities, ordering, text and time locators against the
/// fixture corpus, using corpus-core's own `derive`.
pub fn assert_core_matches_snapshot(batch: &CoreBatch, snapshot: &Value) {
    use corpus_core::{AssetId, AssetKind, RepresentationId, SourceId, SourceKind};

    assert_eq!(batch.header.producer.name, "youtube-corpus");
    let mut videos = snapshot["videos"].as_array().unwrap().clone();
    videos.sort_by_key(|video| video["id"].as_str().unwrap().to_string());
    let mut expected_order = Vec::new();

    for video in &videos {
        let video_id = video["id"].as_str().unwrap();
        let url = video["sourceUrl"].as_str().unwrap();
        let source_id = SourceId::derive(&["youtube-corpus", video_id]);
        expected_order.push(CorpusItemRef::Source(source_id.clone()));
        let source = batch
            .records
            .iter()
            .find_map(|record| match record {
                CorpusRecord::Source(source) if source.id == source_id => Some(source),
                _ => None,
            })
            .expect("one source per video");
        assert_eq!(source.uri, url);
        let expected_kind = if url.starts_with("https://") || url.starts_with("http://") {
            SourceKind::Url
        } else {
            SourceKind::FileSystem
        };
        assert_eq!(source.kind, expected_kind);
        assert_eq!(
            source.metadata.get("video_id").map(String::as_str),
            Some(video_id)
        );

        let mut streams = snapshot["transcriptStreams"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|stream| stream["videoId"] == video["id"])
            .collect::<Vec<_>>();
        streams.sort_by_key(|stream| stream["id"].as_str().unwrap().to_string());
        for stream in streams {
            let stream_id = stream["id"].as_str().unwrap();
            let asset_id = AssetId::derive(&[source_id.as_str(), "transcript_stream", stream_id]);
            let representation_id = RepresentationId::derive(&[asset_id.as_str(), "transcript"]);
            expected_order.push(CorpusItemRef::Asset(asset_id.clone()));
            expected_order.push(CorpusItemRef::Representation(representation_id.clone()));
            let asset = find_asset(batch, &asset_id);
            assert_eq!(asset.kind, AssetKind::Subtitle);
            assert_eq!(asset.content.uri, url);
            assert_eq!(asset.content.hash, None);
            let representation = find_representation(batch, &representation_id);
            assert_eq!(representation.kind, RepresentationKind::Transcript);
            assert_eq!(
                representation.provenance.producer_version.as_deref(),
                Some(batch.header.producer.revision.as_str())
            );

            let mut segments = stream["segments"].as_array().unwrap().clone();
            segments.sort_by_key(|segment| segment["segmentIndex"].as_u64().unwrap());
            let joined = segments
                .iter()
                .map(|segment| segment["text"].as_str().unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(
                representation.content_hash,
                ContentHash::from_bytes(joined.as_bytes())
            );
            for segment in &segments {
                let segment_id =
                    SegmentId::derive(&[asset_id.as_str(), segment["id"].as_str().unwrap()]);
                expected_order.push(CorpusItemRef::Segment(segment_id.clone()));
                assert_segment(batch, &segment_id, segment, &representation_id);
                let record = find_segment(batch, &segment_id);
                assert_eq!(
                    record
                        .metadata
                        .get("transcript_segment_id")
                        .map(String::as_str),
                    segment["id"].as_str()
                );
                assert_eq!(
                    record.metadata.get("segment_index"),
                    Some(&segment["segmentIndex"].as_u64().unwrap().to_string())
                );
            }
        }

        let mut runs = snapshot["visualTextRuns"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|run| run["videoId"] == video["id"])
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| run["id"].as_str().unwrap().to_string());
        for run in runs {
            let run_id = run["id"].as_str().unwrap();
            let asset_id = AssetId::derive(&[source_id.as_str(), "visual_text_run", run_id]);
            let representation_id = RepresentationId::derive(&[asset_id.as_str(), "ocr_text"]);
            expected_order.push(CorpusItemRef::Asset(asset_id.clone()));
            expected_order.push(CorpusItemRef::Representation(representation_id.clone()));
            assert_eq!(find_asset(batch, &asset_id).kind, AssetKind::Video);
            assert_eq!(
                find_representation(batch, &representation_id).kind,
                RepresentationKind::OcrText
            );
            let mut tracks = run["tracks"].as_array().unwrap().clone();
            tracks.sort_by(|left, right| {
                let key = |track: &Value| {
                    (
                        track["startSeconds"].is_null(),
                        track["startSeconds"].as_f64().unwrap_or(0.0),
                        track["trackKey"].as_str().unwrap().to_string(),
                        track["id"].as_str().unwrap().to_string(),
                    )
                };
                key(left).partial_cmp(&key(right)).unwrap()
            });
            for track in &tracks {
                let segment_id =
                    SegmentId::derive(&[asset_id.as_str(), track["id"].as_str().unwrap()]);
                expected_order.push(CorpusItemRef::Segment(segment_id.clone()));
                assert_segment(batch, &segment_id, track, &representation_id);
                assert_eq!(
                    find_segment(batch, &segment_id)
                        .metadata
                        .get("visual_text_role")
                        .map(String::as_str),
                    track["role"].as_str()
                );
            }
        }
    }

    let actual_order = batch
        .records
        .iter()
        .map(CorpusRecord::item_ref)
        .collect::<Vec<_>>();
    assert_eq!(actual_order, expected_order, "record order");
}

fn find_asset<'a>(batch: &'a CoreBatch, id: &corpus_core::AssetId) -> &'a corpus_core::Asset {
    batch
        .records
        .iter()
        .find_map(|record| match record {
            CorpusRecord::Asset(asset) if &asset.id == id => Some(asset),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing asset {id}"))
}

fn find_representation<'a>(
    batch: &'a CoreBatch,
    id: &corpus_core::RepresentationId,
) -> &'a corpus_core::Representation {
    batch
        .records
        .iter()
        .find_map(|record| match record {
            CorpusRecord::Representation(representation) if &representation.id == id => {
                Some(representation)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing representation {id}"))
}

fn find_segment<'a>(batch: &'a CoreBatch, id: &SegmentId) -> &'a corpus_core::Segment {
    batch
        .records
        .iter()
        .find_map(|record| match record {
            CorpusRecord::Segment(segment) if &segment.id == id => Some(segment),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing segment {id}"))
}

fn assert_segment(
    batch: &CoreBatch,
    segment_id: &SegmentId,
    item: &Value,
    representation_id: &corpus_core::RepresentationId,
) {
    let text = item["text"].as_str().unwrap();
    assert_eq!(
        batch.segment_texts.get(segment_id).map(String::as_str),
        Some(text)
    );
    let segment = find_segment(batch, segment_id);
    assert_eq!(
        segment
            .locator
            .selectors
            .get("representation_id")
            .map(String::as_str),
        Some(representation_id.as_str())
    );
    assert_eq!(segment.metadata.get("text_sha256"), Some(&sha256_hex(text)));
    let expected_time = match (item["startSeconds"].as_f64(), item["endSeconds"].as_f64()) {
        (Some(start), Some(end)) => Some(
            Span::new(
                (start * 1000.0).round() as u64,
                (end * 1000.0).round() as u64,
            )
            .unwrap(),
        ),
        _ => None,
    };
    assert_eq!(segment.locator.time_range_ms, expected_time);
    assert_eq!(segment.locator.byte_range, None);
    assert_eq!(segment.locator.frame_range, None);
    assert_eq!(segment.locator.page_index, None);
    let provenance = segment.provenance.as_ref().expect("segment provenance");
    assert_eq!(provenance.producer, "youtube-corpus/corpus-export");
    assert_eq!(
        provenance.inputs,
        vec![CorpusItemRef::Representation(representation_id.clone())]
    );
}

/// Asserts the source-span export's text and identities against the fixture corpus.
pub fn assert_source_span_matches_snapshot(json: &str, snapshot: &Value) {
    let batch = validate_source_span_batch(json).expect("valid source-span batch");
    assert_eq!(batch.producer.name, "youtube-corpus");
    let mut expected_spans = BTreeMap::new();
    for stream in snapshot["transcriptStreams"].as_array().unwrap() {
        for segment in stream["segments"].as_array().unwrap() {
            expected_spans.insert(
                segment["id"].as_str().unwrap().to_string(),
                (
                    stream["id"].as_str().unwrap().to_string(),
                    segment["text"].as_str().unwrap().to_string(),
                ),
            );
        }
    }
    for run in snapshot["visualTextRuns"].as_array().unwrap() {
        for track in run["tracks"].as_array().unwrap() {
            expected_spans.insert(
                track["id"].as_str().unwrap().to_string(),
                (
                    format!("ocr:{}", run["id"].as_str().unwrap()),
                    track["text"].as_str().unwrap().to_string(),
                ),
            );
        }
    }
    assert_eq!(batch.spans.len(), expected_spans.len());
    for span in &batch.spans {
        let (source_id, text) = &expected_spans[&span.id];
        assert_eq!(&span.source_id, source_id);
        assert_eq!(&span.text, text);
    }
}

/// belief-lab #60: version 2 only; positive timebases, non-negative start strictly before end
/// (exactly, across timebases); unique non-empty scene ids that require a span.
fn validate_media(version: u32, evidence: &BeliefEvidence) -> Result<(), String> {
    if evidence.media_span.is_none() && evidence.scene_ids.is_none() {
        return Ok(());
    }
    if version < 2 {
        return Err(format!(
            "evidence {} uses media fields under version 1",
            evidence.id
        ));
    }
    let Some(span) = &evidence.media_span else {
        return Err(format!(
            "evidence {} has scene ids without a media span",
            evidence.id
        ));
    };
    for time in [span.start, span.end] {
        if time.timebase.num <= 0 || time.timebase.den <= 0 {
            return Err(format!(
                "evidence {} has a non-positive timebase",
                evidence.id
            ));
        }
    }
    let ticks = |time: BeliefMediaTime, other: BeliefMediaTime| {
        i128::from(time.pts) * i128::from(time.timebase.num) * i128::from(other.timebase.den)
    };
    if span.start.pts < 0 || ticks(span.start, span.end) >= ticks(span.end, span.start) {
        return Err(format!(
            "evidence {} has an invalid media span",
            evidence.id
        ));
    }
    if let Some(ids) = &evidence.scene_ids {
        let unique = ids.iter().collect::<BTreeSet<_>>();
        if ids.is_empty() || unique.len() != ids.len() || ids.iter().any(|id| id.trim().is_empty())
        {
            return Err(format!("evidence {} has invalid scene ids", evidence.id));
        }
    }
    Ok(())
}
