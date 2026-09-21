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

## Philosophy pipeline integration

The intended architecture is:

```text
YouTube / local media
  -> youtube-corpus
  -> transcript spans + provenance
  -> philosophy-extractor
  -> Jev screening and typed assessments
  -> source-grounded statement candidates
  -> worldview-lab experiments
```

Jev integration belongs in `philosophy-extractor`, not here. `youtube-corpus` should make the source evidence dependable enough that downstream classifiers and extractors can always be audited against the original video/transcript.

## Acceptance principles

- Transcript ingestion, source selection, timestamps, metadata, and search remain authoritative in `youtube-corpus`.
- Export contracts are versioned and deterministic.
- No downstream philosophical model may rewrite transcript truth in place.
- Network-backed discovery/ingest stays separable from deterministic export and verification.
- Provenance must survive caption/ASR replacement and re-ingest.
