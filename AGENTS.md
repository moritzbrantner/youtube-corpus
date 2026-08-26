# Agent Instructions

## Project Purpose

This repository is a Rust CLI for building a searchable YouTube transcript
corpus in Postgres. Running the binary with no subcommand starts a local browser
UI served by the Rust process. It uses exact sibling source checkouts from `nlp-stack`, `moenarch-foundation`,
and `visual-analysis` for transcript parsing, embeddings, runtime, and ingest.

## Local Services

This project uses Docker services for local development or tests: `postgres`.

## Commands

```bash
bun install --frozen-lockfile
bun run build
bun run verify
bun run verify:fast
cargo fmt --check
cargo check
cargo test
bun run verify:postgres
```

## Conventions

- Keep generated media and downloaded captions under `use-case-output/`.
- Do not commit local Postgres data, videos, audio, captions, or transcripts.
- Prefer deterministic tests that do not require network access. Mark real
  `yt-dlp`, Postgres, or ASR integration tests as ignored or skip when required
  tools/environment variables are missing.
