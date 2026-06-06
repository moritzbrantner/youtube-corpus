# Agent Instructions

## Project Purpose

This repository is a standalone Rust CLI for building a searchable YouTube
transcript corpus in Postgres. It uses local `moritzbrantner` Rust crates for
transcript parsing and deterministic text embeddings.

## Local Services

This project uses Docker services for local development or tests: `postgres`.

## Commands

```bash
cargo fmt --check
cargo check
cargo test
docker compose up -d postgres
DATABASE_URL=postgres://postgres:postgres@localhost:5432/youtube_corpus cargo test -- --ignored
```

## Conventions

- Keep generated media and downloaded captions under `use-case-output/`.
- Do not commit local Postgres data, videos, audio, captions, or transcripts.
- Prefer deterministic tests that do not require network access. Mark real
  `yt-dlp`, Postgres, or ASR integration tests as ignored or skip when required
  tools/environment variables are missing.
