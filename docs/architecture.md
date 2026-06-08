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
- `src/web.rs`: Axum HTTP API, static asset serving, and browser launch
- `src/config.rs`: shared source, caption, search, and external tool config
- `src/db.rs`: Postgres connection and migration entrypoints
- `src/ingest.rs`: corpus ingest orchestration and database writes
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
- `GET /api/corpus-status`
- `POST /api/search`
- `POST /api/transcript-context`
- `GET /api/downloaded-files`
- `POST /api/sources`

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

## Ingest Flow

1. CLI or `/api/sources` builds an `IngestRequest`.
2. `src/ingest.rs` resolves the requested video, playlist, channel, or local
   file into video items.
3. YouTube sources use `yt-dlp` for discovery and metadata.
4. Caption files are downloaded and parsed when enabled.
5. ASR runs only when enabled and a transcriber command is available.
6. Transcript streams and segments are written to Postgres.
7. Deterministic text embeddings are generated for semantic search.

Generated media, captions, metadata, and transcripts should stay under
`use-case-output/`.

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
- temporary JSON output
- build output in `dist/`, `target/`, or `node_modules/`
