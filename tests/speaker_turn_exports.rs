//! Consumer-facing speaker-turn export contract (youtube-corpus#119, belief-lab#63).
//! Philosophy-extractor keeps corpus-core segment ids and media seconds on sourceSegments;
//! Belief Lab can join them to source-local annotations without rerunning transcription.

mod support;

use corpus_core::{AssetId, CorpusRecord, SegmentId, SourceId};
use serde_json::Value;
use support::{parse_corpus_core_jsonl, read_fixture, snapshot_json};
use youtube_corpus::corpus_export::{
    export_belief_evidence, export_corpus_core_jsonl, export_source_span, CorpusExportSnapshot,
};

const VIDEO_WITH_TURNS: &str = "11111111-1111-4111-8111-111111111111";
const VIDEO_WITHOUT_TURNS: &str = "22222222-2222-4222-8222-222222222222";
const ASR_STREAM: &str = "0a200000-0000-4000-8000-000000000002";
const ASR_SEGMENT: &str = "5e000000-0000-4000-8000-0000000000c0";

fn snapshot(with_turns: bool) -> CorpusExportSnapshot {
    let mut input = snapshot_json();
    if with_turns {
        input["speakerTurns"] = serde_json::from_str(&read_fixture("speaker-turns.json")).unwrap();
    }
    serde_json::from_value(input).unwrap()
}

fn source_turns(batch: &support::CoreBatch, video_id: &str) -> Option<Vec<Value>> {
    let id = SourceId::derive(&["youtube-corpus", video_id]);
    let source = batch.records.iter().find_map(|record| match record {
        CorpusRecord::Source(source) if source.id == id => Some(source),
        _ => None,
    })?;
    source
        .metadata
        .get("speaker_turns")
        .map(|json| serde_json::from_str(json).expect("valid speaker-turn metadata"))
}

#[test]
fn only_persisted_source_local_turns_are_exported() {
    let with_turns = snapshot(true);
    let without_turns = snapshot(false);
    let annotated = export_corpus_core_jsonl(&with_turns, "git:fixture").unwrap();
    let baseline = export_corpus_core_jsonl(&without_turns, "git:fixture").unwrap();
    let mut shuffled = with_turns.clone();
    shuffled.speaker_turns.reverse();
    assert_eq!(
        annotated,
        export_corpus_core_jsonl(&shuffled, "git:fixture").unwrap(),
        "speaker-turn row order cannot change exported bytes"
    );
    let batch = parse_corpus_core_jsonl(&annotated).expect("v1 corpus-core consumer accepts it");
    let source_id = SourceId::derive(&["youtube-corpus", VIDEO_WITH_TURNS]);
    let source = batch
        .records
        .iter()
        .find_map(|record| match record {
            CorpusRecord::Source(source) if source.id == source_id => Some(source),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        source.metadata["speaker_turns_schema"],
        "youtube-corpus/speaker_turns@1"
    );

    let turns = source_turns(&batch, VIDEO_WITH_TURNS).expect("persisted turns are exported");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0]["speakerLabel"], "SPEAKER_00");
    assert_eq!(turns[0]["startSeconds"], 0.0);
    assert_eq!(turns[0]["endSeconds"], 3.125);
    assert_eq!(
        turns[0]["producerRevision"],
        "git:diarization-model-fixture"
    );
    assert_eq!(turns[0]["annotationRevision"], 1);
    assert_eq!(turns[0]["corpusId"], "c0000000-0000-4000-8000-000000000001");
    assert_eq!(turns[0]["videoId"], VIDEO_WITH_TURNS);
    assert_eq!(turns[1]["speakerLabel"], "SPEAKER_01");
    assert_eq!(turns[1]["startSeconds"], 3.125);
    assert_eq!(turns[1]["endSeconds"], 6.0);
    assert!(source_turns(&batch, VIDEO_WITHOUT_TURNS).is_none());

    let baseline_batch = parse_corpus_core_jsonl(&baseline).unwrap();
    assert!(source_turns(&baseline_batch, VIDEO_WITH_TURNS).is_none());
    assert!(source_turns(&baseline_batch, VIDEO_WITHOUT_TURNS).is_none());
    assert_eq!(batch.segment_texts, baseline_batch.segment_texts);
    assert_eq!(batch.payload_lines, baseline_batch.payload_lines);
    assert_eq!(
        batch.header.schema_version,
        baseline_batch.header.schema_version
    );
    assert_eq!(batch.records.len(), baseline_batch.records.len());
    assert_eq!(
        batch
            .records
            .iter()
            .map(CorpusRecord::item_ref)
            .collect::<Vec<_>>(),
        baseline_batch
            .records
            .iter()
            .map(CorpusRecord::item_ref)
            .collect::<Vec<_>>(),
        "adding diarization metadata cannot change source, asset or segment identities"
    );
    assert_eq!(
        export_source_span(&with_turns, "git:fixture").unwrap(),
        export_source_span(&without_turns, "git:fixture").unwrap(),
    );
    assert_eq!(
        export_belief_evidence(&with_turns, "git:fixture").unwrap(),
        export_belief_evidence(&without_turns, "git:fixture").unwrap(),
    );
    let mut scoped = with_turns;
    scoped.retain_video(VIDEO_WITHOUT_TURNS.parse().unwrap());
    assert!(scoped.speaker_turns.is_empty());
    assert!(source_turns(
        &parse_corpus_core_jsonl(&export_corpus_core_jsonl(&scoped, "git:fixture").unwrap())
            .unwrap(),
        VIDEO_WITHOUT_TURNS
    )
    .is_none());
}

#[test]
fn proposition_source_segments_join_exported_turns_by_id_or_media_time() {
    let annotated = export_corpus_core_jsonl(&snapshot(true), "git:fixture").unwrap();
    let batch = parse_corpus_core_jsonl(&annotated).unwrap();
    let source_id = SourceId::derive(&["youtube-corpus", VIDEO_WITH_TURNS]);
    let asset_id = AssetId::derive(&[source_id.as_str(), "transcript_stream", ASR_STREAM]);
    let segment_id = SegmentId::derive(&[asset_id.as_str(), ASR_SEGMENT]);
    let segment = batch
        .records
        .iter()
        .find_map(|record| match record {
            CorpusRecord::Segment(segment) if segment.id == segment_id => Some(segment),
            _ => None,
        })
        .expect("corpus-core source segment remains stable");
    assert_eq!(segment.asset_id, asset_id);
    let media_time = segment.locator.time_range_ms.expect("timed segment");
    assert_eq!((media_time.start, media_time.end), (0, 3_125));

    // This is the sourceSegments shape emitted by philosophy-extractor PR #19. Those
    // source times are already stored, not inferred from the candidate's wording.
    let candidate = serde_json::json!({"sourceSegments": [{
        "segmentId": segment_id.to_string(), "startSeconds": 0.0, "endSeconds": 3.125
    }]});
    let source_segment = &candidate["sourceSegments"][0];
    assert_eq!(source_segment["segmentId"], segment_id.to_string());
    let turns = source_turns(&batch, VIDEO_WITH_TURNS).unwrap();
    let same_segment = turns
        .iter()
        .find(|turn| turn["segmentId"] == segment.metadata["transcript_segment_id"])
        .expect("annotation carries original transcript segment id");
    assert_eq!(same_segment["speakerLabel"], "SPEAKER_00");
    let overlapping = turns
        .iter()
        .filter(|turn| {
            turn["startSeconds"].as_f64().unwrap() < source_segment["endSeconds"].as_f64().unwrap()
                && source_segment["startSeconds"].as_f64().unwrap()
                    < turn["endSeconds"].as_f64().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(overlapping, vec![same_segment]);
    assert_ne!(same_segment["speakerLabel"], "Bertrand Russell");
    assert_ne!(same_segment["speakerLabel"], "Frederick Copleston");
}

#[test]
fn incomplete_provenance_is_not_reinvented_and_invalid_turns_are_rejected() {
    let mut sparse = snapshot(true);
    sparse.speaker_turns[0].producer_revision = None;
    let batch = parse_corpus_core_jsonl(&export_corpus_core_jsonl(&sparse, "git:fixture").unwrap())
        .unwrap();
    let turns = source_turns(&batch, VIDEO_WITH_TURNS).unwrap();
    assert!(turns[1].get("producerRevision").is_none());
    assert_eq!(turns[1]["annotationRevision"], 2);

    let mut invalid = snapshot(true);
    invalid.speaker_turns[0].start_seconds = Some(-1.0);
    assert!(export_corpus_core_jsonl(&invalid, "git:fixture")
        .unwrap_err()
        .to_string()
        .contains("invalid start time"));
    invalid = snapshot(true);
    invalid.speaker_turns[1].id = invalid.speaker_turns[0].id;
    assert!(export_corpus_core_jsonl(&invalid, "git:fixture")
        .unwrap_err()
        .to_string()
        .contains("duplicate speaker turn"));
}
