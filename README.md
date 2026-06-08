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

## Subscriptions

Store channels or playlists and check them later for new videos:

```bash
cargo run -- subscribe \
  --channel-url "https://www.youtube.com/@Distinguo/videos" \
  --name "Distinguo" \
  --max-items 20 \
  --no-asr \
  --migrate

cargo run -- subscriptions add --playlist-url "https://www.youtube.com/playlist?list=..."
cargo run -- subscriptions list
cargo run -- subscriptions check
cargo run -- status
```

`subscriptions check` discovers videos with `yt-dlp`, remembers which videos
were already seen for each subscription, and ingests only newly discovered
items. To run continuously instead of from cron or systemd:

```bash
cargo run -- subscriptions check --watch --interval-seconds 3600
```

Use `status` to see both the channels/playlists being monitored and the videos
already known to the corpus. Each video reports whether media was downloaded,
whether caption files were downloaded, and whether transcript segments were
parsed and indexed:

```bash
cargo run -- status
cargo run -- videos --parsed
cargo run -- videos --downloaded --limit 25
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

For YouTube sources, ingest also downloads normalized video metadata with
`yt-dlp` into each item's `metadata/` work-dir folder. The corpus stores common
filter fields such as channel/uploader ids, thumbnails, view/like/comment
counts, categories, tags, live status, availability, and age limit alongside the
full normalized metadata JSON.

Pass extra `yt-dlp` arguments with repeated `--yt-dlp-arg` flags. Each flag is
passed as one argv entry to every `yt-dlp` call used by ingest, caption download,
metadata download, media download, benchmark ingest, and saved subscription
checks.

```bash
cargo run -- ingest --url "$URL" --caption-language de --transcriber-command whisper
cargo run -- ingest --url "$URL" --yt-dlp-arg=--cookies-from-browser --yt-dlp-arg=firefox
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
