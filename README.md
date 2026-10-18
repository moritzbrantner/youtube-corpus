# youtube-corpus

`youtube-corpus` is a Rust CLI for building and searching a local YouTube
transcript corpus in Postgres. Running the program with no subcommand starts a
local browser UI backed by the same Rust library and HTTP API. It is not a
Tauri desktop app.

The corpus stores transcript rows in Postgres and uses Postgres full-text
search plus `pgvector` for semantic search.

## Prerequisites

- Rust stable
- Bun
- Docker, for the local Postgres service
- `yt-dlp`, for YouTube discovery, metadata, captions, and media downloads
- Optional `whisper` command, for local ASR fallback

Binary release users only need the downloadable CLI archive with its embedded
browser UI.

Source builds fetch the capability crates (`nlp-stack`, `moenarch-foundation`,
`visual-analysis`) from exact public git revisions pinned in `Cargo.toml`, so a
plain clone builds without sibling checkouts:

```bash
git clone https://github.com/moritzbrantner/youtube-corpus.git
cd youtube-corpus
bun install --frozen-lockfile
bun run build
cargo build --release --locked
```

This source graph does not depend on `rust-packages` or on publishing a new
crate version before ordinary development can proceed.

## Quick Start

```bash
cp .env.example .env
docker compose up -d postgres
bun install --frozen-lockfile
bun run build
cargo run -- --migrate
```

The last command starts the local web UI at `http://127.0.0.1:1420` and opens it
in your browser.

## Web UI

Start the local web UI with defaults:

```bash
cargo run
```

Use another port without opening a browser:

```bash
cargo run -- --port 1421 --no-open
```

Use the explicit serve command:

```bash
cargo run -- serve --host 0.0.0.0 --port 1420
```

The web server reads `DATABASE_URL` from the process environment, or from
`--database-url`. The browser UI cannot override the database URL per request.

If the current directory contains `youtube-corpus.conf`, the CLI reads it before
parsing command-line arguments. Put the same arguments in this file that you
would pass to `youtube-corpus`; quoting and `#` comments use shell-style syntax.
Arguments typed on the command line are applied after the config file, so scalar
flags such as `--port` or `--database-url` can be overridden per run.

```conf
--database-url postgres://postgres:postgres@localhost:5432/youtube_corpus
--port 1420
--no-open
--yt-dlp-cookies-from-browser brave
--yt-dlp-retries 5
--yt-dlp-fragment-retries 5
```

`bun dev` starts the Rust process the same way, so it also picks up
`youtube-corpus.conf` from the repository root.

For frontend development, run the Rust API and Vite separately:

```bash
cargo run -- --no-open
bun run dev:frontend
```

Vite runs on `http://127.0.0.1:5173` and proxies `/api` to the Rust server on
`http://127.0.0.1:1420`.

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

For local media:

```bash
cargo run -- ingest --input ./lecture.mp4 --transcriber-command whisper
```

Common `yt-dlp` reliability options have typed flags, including
`--yt-dlp-cookies-from-browser`, `--yt-dlp-retries`,
`--yt-dlp-fragment-retries`, `--yt-dlp-sleep-interval-seconds`,
`--yt-dlp-max-sleep-interval-seconds`, `--yt-dlp-socket-timeout-seconds`,
`--yt-dlp-format`, and `--yt-dlp-user-agent`. Pass emergency compatibility
arguments with repeated `--yt-dlp-arg` flags. Each raw flag is passed as one argv
entry after the typed options to every `yt-dlp` call used by ingest, caption
download, metadata download, media download, benchmark ingest, and saved
subscription checks.

```bash
cargo run -- ingest --url "$URL" --caption-language de --transcriber-command whisper
cargo run -- ingest --url "$URL" --yt-dlp-cookies-from-browser brave
cargo run -- ingest --url "$URL" --yt-dlp-timeout-seconds 120 --transcriber-timeout-seconds 1800
cargo run -- ingest --url "$URL" --no-asr
```

By default the tool requests manual and auto captions in English and also tries
to run a `whisper` command from `PATH`. If no ASR command is available, ASR is
reported as skipped while captions are still indexed.

For YouTube sources, ingest downloads normalized video metadata with `yt-dlp`
into each item's `metadata/` work-dir folder. The corpus stores common filter
fields such as channel/uploader ids, thumbnails, counts, categories, tags, live
status, availability, and age limit alongside the full normalized metadata JSON.

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

Use `status` to see both monitored sources and videos already known to the
corpus:

```bash
cargo run -- status
cargo run -- videos --parsed
cargo run -- videos --downloaded --limit 25
```

## Review-first sources

A tracked channel or playlist analyzes newly discovered videos automatically by default. Set it
to **Hold for review first** (`--processing-policy review-first` on `subscribe`, or in the Sources
panel) to only register new videos; approve them from the source's "Awaiting review" list. A
video that an automatic source also tracks is analyzed without approval, once.

## Detector profiles

The corpus workspace's Detectors panel sets which analyses run: a corpus default, per-source
on/off overrides, and per-video overrides that apply only inside that corpus. A video tracked
through several sources gets every detector any of them requests. Pick a video to see its
effective plan and who requested each detector, then **Run plan**: existing output is reused,
and detectors without an integrated producer (OCR, translation) are reported as unavailable.

In **Videos**, expand **Processing stages** to inspect the effective per-detector state,
retained output and source languages. Translation reports each requested target separately.
Run one stage to retry missing/failed work or reuse identical output; **Refresh** explicitly
reruns that stage. ASR retries retain the tracked source's configured transcriber command,
arguments and timeout. Failed refreshes retain earlier searchable evidence. Changed local
input bytes, executable content, producer revision or relevant settings prevent stale reuse.
Review-first videos must be approved in their source before processing. Interrupted requests
are shown as interrupted rather than remaining running indefinitely. Stages started by an
older binary have no attempt history; only their retained evidence can be reported.

The status API is `GET /api/corpora/{corpusId}/videos/{videoId}/processing-status`.
`POST …/detector-plan/run` accepts `{"detectors":["asr"],"refresh":false}` to target
one enabled stage; omitting `detectors` runs the enabled plan.

## Entities

Extract people, places and other named entities from each corpus video's transcript and
visible-text (OCR) tracks with the nlp-stack `bert-base-NER` model (downloaded once from
Hugging Face into `use-case-output/models`), then explore them in the corpus workspace's
Entities panel:

```bash
cargo run --release -- entities extract --corpus <corpus-id-or-slug>
```

Mentions keep their modality (spoken or on-screen text), timestamp, source segment/track and
producer run. Labeled corpus people from face/voice matching are listed separately as shown or
heard, never merged with name mentions.

## Diagnostics

Check local prerequisites without requiring a configured database:

```bash
cargo run -- diagnostics
cargo run -- diagnostics --transcriber-command whisper
```

The report includes database reachability and migration readiness when
`DATABASE_URL` is configured, plus availability/version checks for `yt-dlp`,
`ffmpeg`, `ffprobe`, and the transcriber command.

## API Schema

The HTTP API surface is exposed without requiring a database connection:

```bash
cargo run -- api-schema
cargo run -- api-schema --typescript
```

The same contract is available from the web server at `GET /api/schema`. Treat
the schema output as the source of truth for operation ids, example requests,
and execution-plan metadata.

## Search

```bash
cargo run -- search --query "first uploaded youtube video" --top-k 5
cargo run -- search --query "rust pipeline" --mode fts
cargo run -- search --query "semantic topic" --mode semantic
```

Search results include the video id, stream id, transcript source, timestamps,
source URL, title, snippet text, and scores.

## Downstream Exports

`export` writes a deterministic, versioned export of the corpus for downstream
consumers. The same corpus state gives byte-identical output: records have a
stable order, and the only revision in the envelope is the one you pass.

```bash
cargo run -- export --format source-span --producer-revision "git:$(git rev-parse HEAD)"
cargo run -- export --format corpus-core --producer-revision git:abc123 \
  --video-id 11111111-1111-4111-8111-111111111111 --output corpus-core.jsonl
cargo run -- export --format belief-evidence --producer-revision git:abc123 \
  --snapshot tests/fixtures/exports/corpus-snapshot.json
```

| `--format`        | Schema                          | Consumer                                                                         |
| ----------------- | ------------------------------- | -------------------------------------------------------------------------------- |
| `source-span`     | `source_span_interchange@1`     | Generic source/span records with stable ids, timed locators, hashes, revisions   |
| `corpus-core`     | `moenarch_corpus_core_batch@1`  | philosophy-extractor corpus-core intake (document-search's JSONL envelope)       |
| `belief-evidence` | `belief_evidence_interchange@1` | belief-lab evidence import: one reference per transcript segment, no text copied |

Without `--snapshot`, the export reads Postgres (`--migrate` applies migrations
first). With `--snapshot <file.json>` it reads a `CorpusExportSnapshot` JSON
file and needs no database. Output goes to stdout, or byte-for-byte to
`--output`. An empty `--producer-revision` is rejected. The committed consumer
fixtures and the contract are in `tests/fixtures/exports/` and
`tests/corpus_exports.rs`.

## Benchmark

```bash
cargo run -- ingest \
  --channel-url "https://www.youtube.com/@Distinguo/videos" \
  --max-items 3 \
  --no-asr \
  --migrate

cargo run -- benchmark --max-items 3 --migrate
```

The benchmark preset uses Distinguo long-form videos, indexes captions by
default, skips ASR unless `--with-asr` is passed, stores files under
`use-case-output/youtube-corpus-benchmarks/distinguo`, and returns a JSON report
with ingest and search timings.

## Web API Ingest Jobs

The browser UI submits ingest work as a background job by default. `POST
/api/sources` accepts `"async": true` and returns `202 Accepted` with `jobId`
and the initial `ingestRun`. Poll `GET /api/ingest-runs/{id}` for status and
`GET /api/ingest-runs?limit=20` for recent runs. Omitting `"async"` keeps the
older synchronous response behavior.

The persistent `ingest_runs.status` values remain `running`, `completed`, and
`failed`. Responses also include the corpus-owned `ingestRun.job.status`
projection: `running`, `succeeded`, or `failed`. The web/API layer owns this
small lifecycle directly instead of depending on a generic job framework.

## Verification

Canonical validation:

```bash
bun install --frozen-lockfile
bun run verify
```

Expanded validation:

```bash
bun run build
cargo fmt --check
cargo check --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
bun run verify:postgres
```

`bun run verify:fast` runs the deterministic checks without Docker. `bun run
verify:postgres` starts the local Postgres service when `DATABASE_URL` is unset
and uses a disposable `youtube_corpus_verify` database. Network-backed `yt-dlp`
tests are intentionally separate under `bun run verify:yt-dlp`.

See [docs/architecture.md](docs/architecture.md) and
[docs/development.md](docs/development.md) for implementation details.
