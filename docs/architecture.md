# Architecture

`youtube-corpus` is a local-first Rust CLI with an optional browser UI. The
same Rust library powers terminal commands and the HTTP API.

## Runtime Flow

```text
CLI -> web server/API -> core youtube_corpus library -> Postgres/yt-dlp/ASR
     -> embedded Vite app -> browser -> /api
```

Running `youtube-corpus` with no subcommand starts the local web server. The
server owns database configuration through `DATABASE_URL` or `--database-url`,
serves embedded Vite assets from `dist/`, and exposes JSON endpoints under
`/api`.

## Module Boundaries

- `src/main.rs`: process entrypoint and command dispatch
- `src/cli.rs`: Clap argument parsing for CLI commands and web server flags
- `src/api_surface.rs`: runtime-core API surface, examples, and execution-plan metadata
- `src/api_types.rs`: public HTTP API request/response DTOs
- `src/web.rs`: Axum HTTP API, static asset serving, and browser launch
- `src/config.rs`: shared source, caption, search, and external tool config
- `src/db.rs`: Postgres connection and migration entrypoints
- `src/ingest.rs`: corpus ingest orchestration and database writes
- `src/multimodal.rs`: face/voice processing provenance, observations, tracks, and persistence
- `src/search.rs`: full-text, semantic, and hybrid search queries (default provider)
- `src/search_index.rs`: opt-in Tantivy literal search pilot (#76) over a rebuildable projection, via Search Engine
- `src/status.rs`: corpus status and video listing queries
- `src/corpus_export.rs`: deterministic downstream exports (source-span, corpus-core JSONL, belief-lab evidence) from a loaded `CorpusExportSnapshot`
- `src/source_span_export.rs`: `source_span_interchange@1` types and builder shared by the database and snapshot exports
- `src/subscriptions.rs`: saved channel/playlist subscription management
- `src/youtube.rs`: `yt-dlp` discovery, metadata, and media helpers
- `src/captions.rs`: caption download and transcript parsing
- `src/asr.rs`: optional local ASR command integration
- `src/App.tsx`: browser UI
- `src/app/api.ts`: frontend HTTP client and TypeScript API types

## HTTP API

The browser talks to relative `/api` endpoints:

- `GET /api/database-status`
- `GET /api/schema`
- `GET /api/corpus-status`
- `POST /api/search`
- `POST /api/transcript-context`
- `GET /api/downloaded-files`
- `POST /api/sources`
- `GET /api/ingest-runs`
- `GET /api/ingest-runs/{id}`

Request and response bodies use camelCase JSON. Errors use this shape:

```json
{
  "error": {
    "message": "..."
  }
}
```

The API never accepts a database URL from the browser. This keeps database
configuration process-scoped and avoids exposing write access through arbitrary
client-supplied connection strings.

`GET /api/schema` returns the serialized `runtime_core::PackageSurface` used by
`youtube-corpus api-schema`. It is database-free and records public operation
ids such as `corpus.search`, `corpus.addSource`, and
`corpus.ingestRuns.get`, along with example requests and execution-plan
metadata.

## Ingest Flow

1. CLI or `/api/sources` builds an `IngestRequest`.
2. `src/ingest.rs` resolves the requested video, playlist, channel, or local
   file into video items.
3. YouTube sources use `yt-dlp` for discovery and metadata.
4. Caption files are downloaded and parsed when enabled.
5. ASR runs only when enabled and a transcriber command is available.
6. Transcript streams and segments are written to Postgres.
7. Deterministic text embeddings are generated for semantic search.

The web API can run ingest synchronously or as a background job. For async
requests, `/api/sources` inserts a running row in `ingest_runs`, returns the
`jobId`, and a spawned task updates the same run to `completed` or `failed`.
The database status vocabulary is kept for persistence compatibility. The
corpus web/API layer projects it directly into the only three job states it
actually exposes: `running`, `succeeded`, and `failed`. Scheduling, task
spawning, progress, and failure ownership stay with `youtube-corpus`; there is
no generic job-framework authority in this path.

Generated media, captions, metadata, and transcripts should stay under
`use-case-output/`.

## Multimodal Analysis Boundary

Face and voice results are derived evidence attached to corpus videos. They do
not change the source video, transcript stream, or transcript segment records.

`youtube-corpus` owns:

- idempotent processing-run provenance keyed by video, modality, model, input,
  and configuration
- persisted face and voice observations
- temporal face and voice tracks
- cross-video face/voice clusters and optional corpus-local `people` links
- transcript-segment links for voice observations
- later retrieval and browser inspection of those records

Detection and embedding implementations remain outside this repository.
`visual-analysis` owns face detection and face embeddings. `audio-analysis`
owns voice activity detection, speaker embeddings, diarization, and transcript
speaker assignment. This repository stores their typed outputs instead of
copying model/runtime implementations.

A face or voice cluster means only that observations are similar according to a
versioned algorithm. `people` records are optional corpus-local entities; a
cluster is never automatically treated as a verified real-world identity.

Embeddings are stored in dimension-neutral `pgvector` columns together with
model/version provenance. No approximate-nearest-neighbor index is created in
the initial schema because different backends can produce different dimensions.
Index-backed scaling should be introduced only when the active embedding model
and retrieval contract are explicit.

Face and voice embeddings are biometric-derived data. They should remain in the
local corpus database and should not be copied into committed fixtures,
repository history, or generated documentation.

## Search Flow

1. CLI or `/api/search` builds a `SearchRequest`.
2. `src/search.rs` applies source, language, timestamp, metadata, and count
   filters.
3. FTS mode uses Postgres `tsvector` ranking.
4. Semantic mode embeds the query deterministically and uses `pgvector`.
5. Hybrid mode combines FTS and semantic scores.
6. Results are returned with segment ids, timestamps, source URLs, text, and
   score components.

### Opt-in Tantivy literal pilot (#76)

Decision Atlas 3.14 keeps Postgres the default until a measured advantage. The
pilot reuses [Search Engine](https://github.com/moritzbrantner/search-engine)'s
`TantivyIndex`; it adds no ingest path and owns no corpus records.

- **Projection.** `search-index sync` (and `serve --tantivy-index-dir` at
  launch) reads `corpus_search_segments` joined with `videos` and rebuilds the
  index. Document IDs are the Postgres segment/track/translation IDs. An
  unchanged passage keeps its revision, an edited one gets the next revision,
  and a passage gone from Postgres becomes a revisioned tombstone, so re-syncing
  the same data changes nothing and restarts reopen the same directory.
- **Selection.** Chosen when the process starts (`search --tantivy-index-dir`,
  `serve --tantivy-index-dir`), never per query. `serve` keeps one index for
  its lifetime and answers `/api/search` from it; the corpus-scoped
  `/api/corpora/{id}/…search` routes stay on Postgres.
- **Semantics (owner question #76.Q1, option A).** Every whitespace-separated
  term must occur as a case-folded substring in the passage text. There is no
  tokenization, stemming or ranking: `planet` also matches `planetary`. Hits
  come back in video/timestamp order with zero scores and
  `provider: "tantivy_literal"`. Only `--mode fts` is accepted; semantic,
  hybrid and the raw `metadata` filter are rejected rather than approximated.
  Source kind, modality, video, language, timestamp, upload date, duration,
  view count, channel, title, category and tag filters are projected.
- **Provenance.** Translation and OCR evidence are attached from Postgres, as
  on the default path. Each video also gets a small `kind = video` document,
  so `ocrCoverage` counts the videos matching the video-level filters without
  scanning passages; a video without an OCR run is `unprocessedVideos`, never
  a visible-text negative.
- **Comparison.** `search-index compare` runs one query on both providers and
  prints shared hits, provider-only hits and latency.

#### Measurements (2026-10-29, copy of the local corpus)

Single runs, release build, x86_64, 16 cores, local Docker Postgres 16;
163 videos, 109,767 caption passages, no OCR tracks. Informational only.

| Step | Result |
| --- | --- |
| First `search-index sync` | 9.4 s, 120 MB index, ~0.9 GB peak RSS |
| Repeat sync, no changes | 13.0 s, 0 upserts |
| Opening the index (each CLI `search`) | ~4 s (Search Engine loads every lifecycle record), vs 0.04 s for Postgres |
| Query latency after open | 18–41 ms for selective terms; 155–254 ms for `god` (Postgres 11–61 ms) |

| Query (`--mode fts`) | Postgres hits | Tantivy hits | Shared | Note |
| --- | --- | --- | --- | --- |
| `consciousness` | 268 | 268 | 268 | same set |
| `free will` | 298 | 298 | 298 | same set |
| `resurrection of jesus` | 19 | 19 | 19 | same set |
| `wager` | 273 | 343 | 273 | +70 substring hits (`wagering`, `wagers`) |
| `evolution` | 428 | 560 | 428 | +132 (`evolutionary`, `evolutionist`, …) |
| `god` (top 1000) | 1000 | 1000 | 95 | both capped at 1000 of ~7,000; substring also matches `godless`, `gods`, `godhead` |

Postgres never found a passage the pilot missed. The pilot's top 10 differ
from the ranked top 10 because it has no ranking (0–6 of 10 shared). Missing
OCR was reported as 163 unprocessed videos. No measured advantage yet, so
Postgres stays the default (Decision Atlas 3.14).

## Migrations

Migrations live in `migrations/` and are applied by:

- `youtube-corpus migrate`
- any existing CLI command with its command-specific `--migrate` flag
- `youtube-corpus --migrate` for the default web UI
- `youtube-corpus serve --migrate` for the explicit web UI command

The web UI reports when the database is reachable but migrations are missing.

## Generated Data Policy

Do not commit:

- Postgres data
- downloaded videos or audio
- caption files
- generated transcripts
- face crops, voice clips, or biometric embeddings
- temporary JSON output
- build output in `dist/`, `target/`, or `node_modules/`

## Transcript processor identity for downstream evidence

Transcript writes record a separate immutable `processor_revision`; release labels
such as `0.1.0` remain in `processor_version`. The build derives a `sha256:` content
identity from sorted Rust source paths and bytes under `src/`, `build.rs`,
`Cargo.toml` and `Cargo.lock`, with length framing and the
`youtube-corpus-processor-source-v1` domain marker. This identifies the actual
corpus orchestration/normalization source, including locked dependencies. It does
not claim to identify an external ASR executable, its model, or source media.
Command/configuration provenance remains separately inspectable.

Ingest and stored-caption re-segmentation record this identity in the same
transaction as their actual transcript output. Metadata refreshes, unsuccessful
stages, unrelated streams and embedding-only processing do not restamp transcript
producer identity. Migration 0018 intentionally leaves historical revisions null.

Belief evidence export uses the recorded processor revision, or a genuinely
immutable legacy processor-version value. A release label alone fails with the
stream identity and guidance to reprocess or restore the original receipt. Export
never substitutes its own revision or hashes a release label to invent history.
The independent fixture generator and import-rule mirror enforce this distinction.

Since #115 the belief export also says where each segment is in the media. A segment with stored start and end times gets a half-open `mediaSpan` in rational media time (microsecond ticks, `timebase 1/1000000`, rounded to the nearest tick), and the export becomes `belief_evidence_interchange@2`. `sceneIds` lists the stored `video_scenes` records (visual-analysis scene detection) that overlap the span on the segment's video, from all scene runs, ordered by start and id. Untimed segments get neither field, and a corpus without timing still exports version 1. The corpus never invents timing or scenes.
