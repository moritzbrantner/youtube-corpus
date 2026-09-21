# YouTube Corpus Roadmap

`youtube-corpus` should remain the authoritative ingestion, transcript, metadata, and search layer for YouTube-derived material. Domain-specific interpretation belongs in downstream consumers.

## Corpus interoperability

1. [x] **Define a stable transcript-span export contract.**
   - Export video/source identity separately from transcript spans.
   - Preserve video id, source URL, channel/uploader identity, title, language, transcript source, start/end timestamps, verbatim text, and content hash.
   - Include enough revision/provenance information to distinguish caption, ASR, parser, and re-ingest changes.

2. [ ] **Add deterministic batch export.**
   - Provide a versioned JSON/JSONL-oriented CLI or API export suitable for high-volume downstream processing.
   - Allow filtering by video, channel/subscription, transcript source, language, and ingest range without re-running discovery.
   - Keep exports reproducible for a fixed database snapshot and options.

3. [ ] **Integrate with `philosophy-extractor` as a consumer boundary.**
   - Feed source-grounded transcript spans to `philosophy-extractor`.
   - Preserve timestamps so every extracted philosophical statement can link back to the exact moment in the video.
   - Do not add philosophical relevance, claim classification, or worldview semantics to the corpus core.

4. [ ] **Make revision behavior explicit.**
   - A refreshed transcript must create distinguishable provenance rather than silently invalidating old extraction evidence.
   - Downstream results should be able to record which exact transcript revision/span hash they consumed.
   - Define how stale derived artifacts are detected when source spans change.

5. [ ] **Support corpus-scale downstream processing.**
   - Make it practical to stream thousands of transcript spans without loading the entire corpus into memory.
   - Expose stable cursors/batching where necessary.
   - Record export counts and rejected/missing transcript cases for observability.

## Multimodal video evidence

1. [ ] **Reconcile the existing scene/OCR evidence stack onto the current corpus foundation.**
   - Preserve the direction already explored by PR #6: first-class scene spans, raw OCR observations, derived visual-text tracks, and links between tracks, observations, and scenes.
   - Preserve the direction already explored by PR #7: run canonical scene analysis through the visual-analysis adapter backed by `scenedetect-rs`; do not copy scene-detection algorithms into this repository.
   - Rebase/reconcile those older stacked branches instead of creating a second scene/OCR authority.

2. [ ] **Run scene-aware OCR for retained media.**
   - Use representative scene frames from `visual-analysis` rather than blindly OCRing every decoded frame.
   - Keep raw OCR observations distinct from deduplicated/derived visual-text tracks.
   - Preserve timestamp/frame, scene identity, bounding box, language, confidence, processor/model revision, input hash, and configuration hash.
   - Treat presentation slides, quotations, diagrams with labels, end credits, subtitles, and incidental scene text as distinguishable roles rather than one undifferentiated transcript.

3. [ ] **Ingest SponsorBlock as external temporal annotations.**
   - Fetch segment annotations for a video without mutating or deleting the underlying transcript/media.
   - Preserve segment UUID, raw category/action vocabulary, start/end times, video-duration context, fetch time, API/source revision where available, and the exact request policy.
   - Treat SponsorBlock as community annotation evidence: useful for identifying sponsors, intros/outros, self-promotion, interaction reminders, and other non-core intervals, but never authoritative proof that content is philosophically irrelevant.
   - Make network-backed refresh explicit and keep deterministic fixtures for tests.

4. [ ] **Export one aligned media-evidence companion contract.**
   - Keep `source_span_interchange@1` focused on textual evidence.
   - Export scene boundaries, SponsorBlock segments, OCR provenance/geometry, and other non-text timeline facts in a separate versioned evidence bundle keyed to the same video/source revisions.
   - Allow OCR-derived visual text to also appear as timed source spans while retaining links to its raw observations and scene evidence.
   - Preserve raw versus derived evidence and all producing revisions so downstream staleness checks remain deterministic.

5. [ ] **Use the aligned timeline for downstream research without collapsing evidence channels.**
   - Give `philosophy-extractor` transcript spans plus aligned scene/OCR/SponsorBlock evidence.
   - Let scene boundaries guide coherent context windows.
   - Let OCR enrich slide-heavy lectures and recover quotations/names/formulas absent from speech transcripts.
   - Let SponsorBlock inform relevance/routing policy while keeping the original material available for audit.
   - Never silently merge OCR text into a transcript or silently remove SponsorBlock-marked intervals.

## Philosophy pipeline integration

The intended architecture is:

```text
YouTube / local media
  -> youtube-corpus
  -> transcript spans + OCR text + aligned scene/SponsorBlock evidence
  -> philosophy-extractor
  -> Jev screening and typed assessments
  -> source-grounded statement candidates
  -> worldview-lab experiments
```

Jev integration belongs in `philosophy-extractor`, not here. `youtube-corpus` should make the source evidence dependable enough that downstream classifiers and extractors can always be audited against the original video/transcript.

## Acceptance principles

- Transcript ingestion, source selection, timestamps, metadata, cross-modal alignment, and search remain authoritative in `youtube-corpus`.
- Scene algorithms remain authoritative in `scenedetect-rs`; OCR/visual algorithms remain authoritative in `visual-analysis`.
- SponsorBlock data remains externally sourced community evidence and must retain its own provenance.
- Export contracts are versioned and deterministic.
- No downstream philosophical model may rewrite transcript truth in place.
- Network-backed discovery/ingest stays separable from deterministic export and verification.
- Provenance must survive caption/ASR replacement and re-ingest.
