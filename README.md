# youtube-corpus

`youtube-corpus` builds a searchable Postgres corpus from YouTube videos,
playlists, channels, or local media files. It stores transcript rows in
Postgres and uses Postgres full-text search plus `pgvector` for semantic search.

## Setup

```bash
cp .env.example .env
docker compose up -d postgres
cargo run -- migrate
```

## Ingest

```bash
cargo run -- ingest \
  --url "https://www.youtube.com/watch?v=jNQXAC9IVRw" \
  --work-dir use-case-output/youtube-corpus \
  --migrate
```

For playlists:

```bash
cargo run -- ingest --playlist-url "https://www.youtube.com/playlist?list=..." --max-items 10
```

First example corpus and benchmark:

```bash
cargo run -- ingest \
  --channel-url "https://www.youtube.com/@Distinguo/videos" \
  --max-items 3 \
  --no-asr \
  --migrate

cargo run -- benchmark --max-items 3 --migrate
```

The benchmark preset uses Distinguo long-form videos from
`https://www.youtube.com/@Distinguo/videos`, indexes captions by default, skips
ASR unless `--with-asr` is passed, stores files under
`use-case-output/youtube-corpus-benchmarks/distinguo`, and returns a JSON report
with ingest and search timings.

By default the tool requests manual and auto captions in English and also tries
to run a `whisper` command from `PATH`. If no ASR command is available, ASR is
reported as skipped while captions are still indexed.

```bash
cargo run -- ingest --url "$URL" --caption-language de --transcriber-command whisper
cargo run -- ingest --url "$URL" --no-asr
cargo run -- ingest --input ./lecture.mp4 --transcriber-command whisper
```

## Search

```bash
cargo run -- search --query "first uploaded youtube video" --top-k 5
cargo run -- search --query "rust pipeline" --mode fts
cargo run -- search --query "semantic topic" --mode semantic
```

Search results include the video id, stream id, transcript source, timestamps,
source URL, title, snippet text, and scores.

## Verification

```bash
cargo fmt --check
cargo check
cargo test
DATABASE_URL=postgres://postgres:postgres@localhost:5432/youtube_corpus cargo test -- --ignored
```
