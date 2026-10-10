# Agent Instructions

## Project Purpose

This repository is a Rust CLI for building a searchable YouTube transcript
corpus in Postgres. Running the binary with no subcommand starts a local browser
UI served by the Rust process. It consumes `nlp-stack`, `moenarch-foundation` and `visual-analysis` through
exact-revision git dependencies on their public repositories (owner decision,
#40) for transcript parsing, embeddings, runtime, and ingest. Bump a `rev` only
to a pushed, reviewed commit; keep the `[patch.crates-io]` foundation/text-core
revisions aligned with what the pinned nlp-stack and visual-analysis declare;
never add sibling `path` dependencies or publish crates to unblock work.

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
