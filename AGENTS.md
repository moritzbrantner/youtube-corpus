# Agent Instructions

## Project Purpose

This repository is a Rust CLI for building a searchable YouTube transcript
corpus in Postgres. Running the binary with no subcommand starts a local browser
UI served by the Rust process. It uses exact sibling source checkouts from `nlp-stack`, `moenarch-foundation`,
and `visual-analysis` for transcript parsing, embeddings, runtime, and ingest.

## Authority boundaries

Machine-readable in `.repository.toml` (`[architecture]`); keep both lists identical.

- Owns: `youtube-corpus/media-ingest-orchestration`, `youtube-corpus/corpus-persistence`, `youtube-corpus/corpus-identity`, `youtube-corpus/timeline-alignment`, `youtube-corpus/corpus-retrieval`, `youtube-corpus/subscriptions`
- Adapts: `audio-analysis/transcription`, `visual-analysis/video-ingest`, `visual-analysis/ocr`, `visual-analysis/scene-detection`, `nlp-stack/text-transcripts`, `nlp-stack/text-embeddings`, `nlp-stack/text-lexical`, `moenarch-foundation/runtime`
- Non-authoritative: `audio-analysis/transcription`, `audio-analysis/speakers`, `visual-analysis/ocr`, `visual-analysis/scene-detection`, `visual-analysis/faces`, `nlp-stack/text-transcripts`, `nlp-stack/text-embeddings`, `nlp-stack/text-lexical`

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
