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
- `src/search.rs`: full-text, semantic, and hybrid search queries
- `src/status.rs`: corpus status and video listing queries
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
The database status vocabulary is kept for compatibility; API responses add a
nested `job` object using `jobs-core` lifecycle values such as `running`,
`succeeded`, and `failed`.

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
