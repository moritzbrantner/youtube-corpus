# Changelog

## 0.1.0 - 2026-06-09

Initial release.

- Local YouTube/channel/playlist ingest with `yt-dlp`.
- Caption parsing through `text-transcripts`.
- Optional Whisper CLI ASR fallback.
- Postgres + pgvector-backed transcript search.
- Local browser UI served by the Rust process.
- Subscription discovery and background ingest jobs.
