//! Consumer-contract checks of the committed export fixtures (issue #42).
//!
//! These tests need no production export code: they validate the hand-derived expected files
//! under `tests/fixtures/exports/expected/` (see `derive_expected.py`) against the consumer
//! readers mirrored in `tests/support/mod.rs`, and validate those mirrors against the consumers'
//! own committed fixtures under `tests/fixtures/exports/consumers/`:
//!
//! - `document-search-corpus-core-batch.jsonl`: document-search
//!   `src/test/fixtures/corpus-core-batch.jsonl` at 5f6d16e405644392f2d2f75eb80f646ff2179a86,
//!   produced by its real `serializeCorpusCoreBatch` exporter (the format philosophy-extractor
//!   already reads);
//! - `belief-lab-youtube-multimodal-v1.json`: belief-lab
//!   `fixtures/evidence-interchange/youtube-multimodal-v1.json` at
//!   71780e0d6176b87acbde01298e49d03d6f50c5ff.
//!
//! `tests/corpus_exports.rs` holds the tests that run the exporters against these fixtures.

mod support;

use serde_json::Value;
use support::*;

#[test]
fn expected_corpus_core_fixture_is_consumable_and_matches_the_snapshot() {
    let jsonl = read_fixture("expected/corpus-core.jsonl");
    let batch = parse_corpus_core_jsonl(&jsonl).expect("philosophy-extractor intake accepts it");
    assert_eq!(batch.header.producer.revision, FIXTURE_REVISION);
    assert_core_matches_snapshot(&batch, &snapshot_json());
    assert_core_envelope_matches_document_search(&jsonl);
}

#[test]
fn document_search_fixture_passes_the_mirrored_intake_reader() {
    let jsonl = read_fixture("consumers/document-search-corpus-core-batch.jsonl");
    let batch = parse_corpus_core_jsonl(&jsonl).expect("document-search fixture is accepted");
    assert_eq!(batch.header.producer.name, "document-search");
    assert_eq!(batch.segment_texts.len(), 2);
    assert!(batch
        .segment_texts
        .values()
        .any(|text| text == "Alpha 😀 café one."));
}

#[test]
fn mirrored_intake_reader_rejects_what_philosophy_extractor_rejects() {
    let jsonl = read_fixture("expected/corpus-core.jsonl");
    let header_only = format!("{}\n", jsonl.lines().next().unwrap());
    assert!(parse_corpus_core_jsonl(&header_only).is_ok());

    let wrong_version = jsonl.replacen("\"schema_version\":1", "\"schema_version\":2", 1);
    assert!(parse_corpus_core_jsonl(&wrong_version).is_err());

    let extra_header_field = jsonl.replacen(
        "\"schema_version\":1,",
        "\"schema_version\":1,\"exported_at\":\"x\",",
        1,
    );
    assert!(parse_corpus_core_jsonl(&extra_header_field).is_err());

    let without_payloads = jsonl
        .lines()
        .filter(|line| !line.starts_with("{\"payload\":"))
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    assert!(parse_corpus_core_jsonl(&without_payloads)
        .unwrap_err()
        .contains("no payload"));

    let unit = jsonl.replace(
        "\"character_unit\":\"unicode_scalar\"",
        "\"character_unit\":\"utf16\"",
    );
    assert!(parse_corpus_core_jsonl(&unit).is_err());
}

#[test]
fn expected_belief_fixture_satisfies_belief_lab_import_rules() {
    let json = read_fixture("expected/belief-evidence.json");
    assert_belief_rules(&json, &snapshot_json());
    let value: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["exporter"]["revision"], FIXTURE_REVISION);
    assert!(json.ends_with("}\n"));
}

#[test]
fn belief_lab_fixture_passes_the_mirror_and_inference_scores_are_rejected() {
    let fixture = read_fixture("consumers/belief-lab-youtube-multimodal-v1.json");
    let batch = validate_belief_batch(&fixture).expect("belief-lab's own fixture is valid");
    assert_eq!(batch.evidence.len(), 6);

    for semantics in ["soft_truth", "posterior_probability"] {
        let mutated = fixture.replacen("\"model_confidence\"", &format!("\"{semantics}\""), 1);
        assert!(
            validate_belief_batch(&mutated).is_err(),
            "{semantics} must be rejected"
        );
    }
    let duplicate = fixture.replacen("evidence:entity:arch\"", "evidence:transcript:17\"", 1);
    assert!(validate_belief_batch(&duplicate).is_err());
    let missing_parent =
        fixture.replacen("\"evidence:scene:4\"\n", "\"evidence:scene:missing\"\n", 1);
    assert!(validate_belief_batch(&missing_parent).is_err());
    let unknown_field = fixture.replacen(
        "\"class\": \"transcript\"",
        "\"class\": \"transcript\", \"text\": \"copied\"",
        1,
    );
    assert!(validate_belief_batch(&unknown_field).is_err());
}

#[test]
fn expected_source_span_fixture_is_valid_and_matches_the_snapshot() {
    let json = read_fixture("expected/source-span.json");
    assert_source_span_matches_snapshot(&json, &snapshot_json());
    let batch = validate_source_span_batch(&json).unwrap();
    assert_eq!(batch.producer.revision, FIXTURE_REVISION);
    assert_eq!(
        batch
            .sources
            .iter()
            .map(|source| source.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "0a200000-0000-4000-8000-000000000002",
            "a1000000-0000-4000-8000-000000000001",
            "b1000000-0000-4000-8000-000000000003",
            "ocr:c0000000-0000-4000-8000-000000000004",
        ]
    );
}

#[test]
fn expected_fixtures_carry_no_wall_clock_timestamps() {
    for name in [
        "expected/source-span.json",
        "expected/corpus-core.jsonl",
        "expected/belief-evidence.json",
    ] {
        assert_no_wall_clock_timestamps(&read_fixture(name));
    }
}
