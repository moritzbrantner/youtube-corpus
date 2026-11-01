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
- Optional CUDA toolkit, to build ASR for the GPU (`--features cuda`)

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
./scripts/start.sh
```

This creates `.env` only when missing, starts or reuses Compose Postgres, installs
frontend dependencies (including wasm-pack when missing), builds the app,
applies migrations, then opens
`http://127.0.0.1:1420`. The server stays in the foreground; press Ctrl+C to stop it.
A repeated start preserves your configuration and data and reuses a valid frontend
build. The repository keeps Bun's isolated package links project-local because the
global virtual store hides companion React declaration types from TypeScript.
This project setting leaves your global Bun configuration unchanged. Starting the app downloads build dependencies when needed, but no media or
analysis models.

`bun run start` is an alias. The shell entrypoint can also diagnose a missing Bun
installation. Docker Compose or Podman Compose supplies local Postgres; set
`DATABASE_URL` in `.env` to use an existing database instead. Startup's database
setting overrides `youtube-corpus.conf`; explicit startup flags take precedence.

```bash
./scripts/start.sh --port 1421 --no-open
```

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
cargo run -- ingest --input ./lecture.mp4
```

Common `yt-dlp` reliability options have typed flags, including
`--yt-dlp-cookies-from-browser`, `--yt-dlp-retries`,
`--yt-dlp-fragment-retries`, `--yt-dlp-sleep-interval-seconds`,
`--yt-dlp-max-sleep-interval-seconds`, `--yt-dlp-socket-timeout-seconds`,
and `--yt-dlp-user-agent`. Pass emergency compatibility
arguments with repeated `--yt-dlp-arg` flags. Each raw flag is passed as one argv
entry after the typed options to every `yt-dlp` call used by ingest, caption
download, metadata download, media download, benchmark ingest, and saved
subscription checks.

```bash
cargo run -- ingest --url "$URL" --caption-language de --transcriber-command /opt/whisper/bin/whisper
cargo run -- ingest --url "$URL" --yt-dlp-cookies-from-browser brave
cargo run -- ingest --url "$URL" --yt-dlp-timeout-seconds 120 --transcriber-timeout-seconds 1800
cargo run -- ingest --url "$URL" --no-asr
```

By default the tool requests manual and auto captions in English and transcribes
the audio with [native-whisperx](https://github.com/moritzbrantner/native-whisperx),
linked as a library at the exact revision pinned in `Cargo.toml` (Whisper `small`,
no word alignment). It decodes in the first requested caption language, because
native language detection does not work yet (native-whisperx#434). One transcription session keeps
the model loaded across videos; it downloads into the Hugging Face cache on first
use. Build with `--features cuda` to run it on the GPU when one is present; a
failed GPU run is retried on the CPU. Every ASR stream records the producer
(`asrProducer`: provider, revision, model config) and the run (`asrRun`: model,
device, CPU fallback) in its `processing_config`.

`--transcriber-command` selects an explicit Whisper-CLI-compatible command
instead (called as `<command> <media> <args…> --output_dir <dir> --output_format json`).
Nothing is looked up on `PATH`. `--transcriber-timeout-seconds` applies to that
command only.

For YouTube sources, ingest downloads normalized video metadata with `yt-dlp`
into each item's `metadata/` work-dir folder. The corpus stores common filter
fields such as channel/uploader ids, thumbnails, counts, categories, tags, live
status, availability, and age limit alongside the full normalized metadata JSON.

Source media is acquired only for the enabled stages that still need to run.
Captions, metadata, entities and translations download no media blob. ASR and
voices fetch an audio-only stream (`ba/wa*`; yt-dlp's default `lang` sort picks
the original-language track over auto-dubs). Scenes request video capped at
360p, faces and OCR at 720p (`bv*[height<=H]+ba/b[height<=H]/wv*+ba/w`); one
video download serves every stage. Each acquisition is recorded in
`videos.media_acquisition` with the yt-dlp format ids, the audio track's
language, the file's byte size and its SHA-256. A later plan that needs more
(video after audio, or a higher cap) acquires again, as does a plan whose
recorded file was removed. Local `--input` files are used as they are and never
re-acquired.

Downloaded media is temporary by default. Once every enabled stage that needs it
(ASR, voices, scenes, faces, OCR) in every corpus plan containing the video has
committed current output, the file and its yt-dlp info JSON are deleted,
`local_video_path` is cleared and the acquisition record keeps `deletedAt`,
`deletionReason` and the scene fingerprint, so captions, transcripts, search,
exports and stage reuse keep working without the file. Media stays while a stage
is unavailable, failed, interrupted or still running, or while another run holds
it. Files a later acquisition superseded (audio after video was fetched) are
deleted at the same point. Only files proven to be this corpus's downloads are
touched: a recorded acquisition of a remote source inside its own
`<item>/media/` directory, not a symlink, with the recorded size and SHA-256.
Local `--input` files and media downloaded before acquisitions were recorded are
never deleted. Pass `ingest --keep-media` (or `keepMedia: true` when running a
plan through the API) to keep the media of that run. A later plan that needs the
media again acquires it again.

## Process a list of videos (workflow plan)

`process` takes a list of videos from ingest to citable export in one command. A
plan file names the corpus, the videos and the stages:

```bash
cargo run -- process --plan workflows/cc-by-sample.plan.json \
  --out-dir use-case-output/cc-by-sample --migrate
```

For every video it downloads the metadata, and drops the video unless its stated
YouTube licence contains `requireLicense` (default `Creative Commons`; `""`
accepts any). It then ingests the captions, adds the video to the corpus and runs
the corpus plan: the detectors listed under `detectors` become the corpus default,
media is acquired once for the stages that run, and it is deleted after they stored
their output. Finally it writes the corpus's exports (`source-span.json`,
`corpus-core.jsonl`, `belief-evidence.json`) and `workflow-report.json` into
`--out-dir`. The report lists each video's licence, every stage outcome, the
deleted or retained media, and each export's SHA-256. The command exits non-zero
when a video or an enabled stage failed. Unavailable producers, such as OCR, are
reported but are not failures.

| Plan field         | Default            | Meaning                                                                                         |
| ------------------ | ------------------ | ----------------------------------------------------------------------------------------------- |
| `corpus`           | required           | Corpus slug; created on first use                                                               |
| `corpusName`       | the slug           | Name of a newly created corpus                                                                  |
| `videos`           | required           | YouTube video URLs                                                                              |
| `detectors`        | required           | Enabled detectors, as ids or `{"detector": "translate_spoken", "targetLanguages": ["de"]}`      |
| `requireLicense`   | `Creative Commons` | Text the video's YouTube licence must contain                                                   |
| `captionLanguages` | `["en"]`           | Caption languages to fetch                                                                      |
| `transcriber`      | native-whisperx    | `{"command": "...", "args": [...], "timeoutSeconds": n}` for ASR of videos no source configures |
| `exports`          | all three          | `source-span`, `corpus-core`, `belief-evidence`                                                 |
| `keepMedia`        | `false`            | Keep downloaded media after processing                                                          |
| `extracts`         | none               | Media extracts to cut before the media is released (see below)                                  |

Running the same plan again reuses every stored stage, downloads nothing and
writes byte-identical exports. If a stage has to run again (for example after a
transcriber change) and the media must be downloaded again, every enabled media
stage runs on the new file, and that file is deleted afterwards. `--refresh`
re-ingests and reruns every stage. `export --corpus <slug>` writes one corpus's
exports later without a plan.

## Media extracts (clips, audio excerpts, keyframes)

Downstream tools can bundle small, verifiable extracts of a span instead of a whole
video:

```bash
cargo run -- extract --youtube-id BnY2JdBrvaI --start 10 --end 14 \
  --out-dir use-case-output/extracts
```

This writes `<out-dir>/<youtube id>/span-<start ms>-<end ms>/` (or
`scenes-<digest>/` for `--scene <id>` selections) with:

- `clip.mp4`: the stored scenes the span overlaps, cut at their boundaries
  (H.264 + AAC);
- `audio.m4a`: the same range, audio only (AAC);
- `keyframe-<scene index>.png`: the midpoint frame of each covering scene;
- `manifest.json` (`media_extract@1`): each file's SHA-256, size, range and scene
  ids; the source media's SHA-256 and yt-dlp format ids; the video's stated
  licence; the producer revision, the ffmpeg version, the cutter config and its
  hash; and the scene run's detector revision and config hash.

A span without scene evidence gets only the audio excerpt of exactly the span; the
manifest lists the clip and keyframes as skipped. Videos whose licence is not
Creative Commons are refused unless you pass `--allow-non-cc`. `--kind
clip,audio,keyframes` selects what to cut, and `--max-height` (default 360, the
height scenes are detected at) caps the source video. If the media was already
released, `extract` downloads it again and deletes it after cutting
(`--keep-media` keeps it). The same inputs give byte-identical files, and an
existing extract whose files still match its manifest is reused without media.

In a plan, `extracts` entries are cut from the media the stages used, before it is
released, into `<out-dir>/extracts/`, and `workflow-report.json` lists each
manifest's SHA-256:

```json
"extracts": [{ "video": "https://www.youtube.com/watch?v=BnY2JdBrvaI",
               "startSeconds": 10, "endSeconds": 14 }]
```

Fields: `video` (one of the plan's URLs), `startSeconds`/`endSeconds` or
`sceneIds`, `kinds` (default all three) and `maxHeight` (default 360). The plan's
`requireLicense` applies.

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
items.

While the local server runs (`cargo run` without a subcommand), every enabled
monitored source is checked automatically on its own interval (default 6 hours,
set with `subscriptions add --check-interval-seconds` or edited per source in the
Sources view, which also shows the last check result and the next or overdue
check). After a restart, each overdue source gets one prompt catch-up check and
then resumes its interval. Check now remains available and joins a check of the
same source that is already running. Checks while the server is stopped do not
happen. The CLI watch mode remains for use without the server:

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
and detectors without an integrated producer (OCR) are reported as unavailable.

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

## Translated text

Translation is opt-in per detector profile, separately for **spoken transcripts**
(`translate_spoken`) and **on-screen text** (`translate_ocr`), with one or more target languages
per corpus default, source and video override. Running the stage translates the video's
preferred transcript stream or its OCR tracks with the native-whisperx planner and its
Marian/OPUS-MT provider (en, de, fr, es, it, pt, nl, pl; other pairs pivot through English).
The pinned free models download into `use-case-output/models/translation` on first use.

Translations are stored beside the original spans with source/target language, modality,
timestamps and provider/model/configuration provenance, and are indexed for corpus search: a
translated hit is marked **Translated** with its languages and original text, and links to the
original time in the video. Unchanged translations are reused per span and target; adding a
target generates only that language. Unsupported languages, missing source evidence and failed
targets are reported per target and never appear translated. Search does not translate queries.

## Faces

Enable **Faces and within-video tracks** in a detector profile, retain the video locally,
and run its plan. The detector samples once per second using visual-analysis YuNet and
SFace. It stores frame/time/box/score observations, 128-dimensional embeddings, and inferred
tracks inside one video. Tracks are similarity-based observations, not verified identities
or named people. After face processing, the corpus also checks compatible face evidence
for repeated inferred people; review these associations in **Entities → Face matches**.

The free, revision-pinned YuNet (MIT) and SFace (Apache-2.0) models download on the first
requested run into the configured model cache under `faces/`; opening the app does not
fetch them. Their ONNX files must match recorded SHA-256 checksums. Install ONNX Runtime
1.24 or newer with API 24 support and set `ORT_DYLIB_PATH` to its shared library when it
is outside the normal loader path. Missing media, models or runtime produce a failed stage.
No media is downloaded by this detector. Run provenance records the producer, models,
input bytes, sample interval and tracking settings. Identical successful runs, including
videos with no detected faces, are reused; **Refresh** replaces only the same run's output
atomically. Earlier evidence remains available if a refresh fails.

Face images and embeddings remain local corpus data. Do not commit real biometric fixtures.

## Face matching and corrections

High-confidence, compatible face tracks from separate corpus videos are linked automatically
into an **inferred corpus-local person** after face processing. Existing evidence can be
checked with **Recheck face matches** in Entities. This rechecks stored evidence without
downloading models or processing media. Opening the review only reads existing associations.

Automatic matches require multiple sampled observations, strong detections and mutually
consistent model-compatible embeddings. Ambiguous alternatives, incompatible model versions
and contradictory simultaneous faces stay unresolved. Each association exposes its cosine
similarity, producer/model/input/configuration and matching revision, with links to the exact
video times. Generated “Inferred person …” labels are local handles, not real-world names.

In **Entities → Face matches**, select tracks to **Unlink**, **Split selected into new
person**, or **Link selected** to another corpus person. **Confirm current group** records a
researcher's confirmation of its current associations; it does not verify a real-world name.
Unlink permanently rejects the former association while allowing a different, uniquely supported
automatic match; an explicit researcher link can override that rejection. Corrections persist across
unchanged detector and matcher reruns, update the existing person occurrence counts, and
retain the original observations. Face-to-voice bridges and evidence-based naming remain
separate work.

## Voices

Enable **Voices and within-video speaker tracks** in a detector profile and run its plan
on retained media with an audio stream. The canonical audio-analysis pipeline decodes and
resamples audio, finds energy-based speech spans, produces 256-dimensional WeSpeaker
embeddings for speech windows of at most 3 s, and clusters them per video (average-linkage,
with clusters under 2 s or 5 % of the speech folded into their nearest speaker) into
inferred speaker tracks. Each window is one stored voice observation. Energy activity scores
are kept separately from identity confidence; speaker labels are not verified people.
Short/noisy spans, overlapping speakers and music can make these inferences unreliable.
Cross-video voice matching (#134) and naming from spoken self-introductions (#135) build on
these tracks: a person mention right after "I'm", "my name is" or "Hi, this is" (or "ich bin",
"mein Name ist", "hier ist") in a transcript segment that one voice track clearly speaks names that
voice's corpus person, as an inferred name to confirm in **Entities → Person names**. A host's
"joining me is …" never names the speaker. Voice matching only forms a person once a voice is
heard in two videos; a guest heard in one video who clearly introduces themselves gets a
single-video person for their track (#164), suggested and unverified like any other match, and
a later appearance in another video joins it. Unnamed single-video voices form no person.

The free [WeSpeaker model](https://huggingface.co/Wespeaker/wespeaker-voxceleb-resnet34-LM)
(CC-BY-4.0, attribution: WeSpeaker) downloads into the configured cache under `voices/`
only on a requested run, at a pinned revision and SHA-256. As with faces, install ONNX
Runtime with API 24 support and set `ORT_DYLIB_PATH` when needed. No fallback spectral
embeddings or pretend successful results are substituted when inference fails. Identical
runs, including no-speech results, are reused; failed refreshes retain prior observations.
Voice embeddings remain local corpus data and must not be committed as real fixtures.

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
cargo run -- diagnostics --transcriber-command /opt/whisper/bin/whisper
```

The report includes database reachability and migration readiness when
`DATABASE_URL` is configured, plus availability/version checks for `yt-dlp`,
`ffmpeg`, `ffprobe`, the native-whisperx revision and device, and a configured
transcriber command.

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
source URL, title, snippet text, and scores. Each CLI hit also carries a
`citation`: the YouTube id, channel, upload date, `transcriptSource`
(`caption_manual`, `caption_auto` or `asr`), a timestamp link, and the raw
labels of stored speaker turns that overlap it (labels, not identities).

### Citation search (#140)

```bash
# Hits of one recording, two segments of context on each side, as plain text
cargo run -- search --query "free will" --youtube-id dQw4w9WgXcQ --context 2 --format text
# Hits within 30 s of context, limited to a corpus (id or slug) and a diarization label
cargo run -- search --query "free will" --corpus debates --speaker-label SPEAKER_01 --context 30s
# Pin a citation: the segments overlapping [start, end) and their span_sha256
cargo run -- span --youtube-id dQw4w9WgXcQ --start 754 --end 781 \
  --transcript-source caption_manual --language en --producer-revision "git:$(git rev-parse HEAD)"
```

`--context` returns the neighbouring transcript segments of each hit in time
order with the hit marked (`isHit`, or `>` in text output); a translated hit
shows its original-language context. `--format text` prints
`[mm:ss–mm:ss] speaker: text` lines.

`span` prints, per matching transcript stream, the overlapping segments, the
`segmentCount`, and the `spanSha256` truth-core pins a `media_span` passage
with: the SHA-256 of the canonical JSON list of each segment's `segmentIndex`,
`startSeconds`, `endSeconds` and `textHash`, in time order. It is computed over
the same `source_span_interchange@1` batch `export --format source-span`
writes, so the hash equals the one a whole-corpus export yields; no export file
is needed. `--snapshot FILE` reads a corpus snapshot instead of Postgres.

### Automatic-caption passages (#92)

YouTube automatic captions roll: every cue redisplays the previous line and
~10 ms refresh cues repeat it again, so indexing cues gave stuttering,
duplicated hits. Automatic-caption streams therefore index **passages**: the
cues go through Foundation's `media_core::project_rolling_captions` (each
spoken word once, timed from the caption's inline word timestamps, else
interpolated in its cue), and the words are grouped into passages that close
at a sentence end after 12 words, at 40 words, 30 s, a pause over 2 s, or a
`>>` speaker change. Each passage's `metadata.passage` lists its words with
their timing source and source cue. Manual captions and ASR are unchanged.

The raw cues stay untouched: the caption file on disk, and the normalized cues
in `transcript_source_cues`, which are exactly what the stream indexed before
passages. The stream's `processing_config.segmentation` records the
representation (`rolling-caption-passages@1`) and projection
(`media-core/rolling-captions@1`). Existing corpora get passages by
reprocessing the `segments` stage from the stored caption files
(`POST /api/corpora/{corpus}/videos/{video}/reprocess` with
`{"stage":"segments"}`).

Passage boundaries change span hashes. To verify a citation made before
passages, cut the span from the raw cues:

```bash
cargo run -- span --youtube-id SVJiCL3K0j4 --start 265 --end 275 \
  --producer-revision "git:$(git rev-parse HEAD)" --representation source-cues
```

### Opt-in Tantivy literal search pilot

Postgres stays the default. The pilot (#76) answers `--mode fts` queries with
literal, case-folded substring terms from a local, rebuildable Tantivy index
(no ranking; `planet` also matches `planetary`). See
[docs/architecture.md](docs/architecture.md#opt-in-tantivy-literal-pilot-76)
for semantics and measurements.

```bash
cargo run -- search-index sync --index-dir use-case-output/tantivy-index
cargo run -- search --mode fts --query "free will" --tantivy-index-dir use-case-output/tantivy-index
cargo run -- search-index compare --mode fts --query "free will" --tantivy-index-dir use-case-output/tantivy-index
cargo run -- serve --tantivy-index-dir use-case-output/tantivy-index  # /api/search uses the pilot
```

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

| `--format`        | Schema                                                             | Consumer                                                                                                                                                                    |
| ----------------- | ------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `source-span`     | `source_span_interchange@1`                                        | Generic source/span records with stable ids, timed locators, hashes, revisions                                                                                              |
| `corpus-core`     | `moenarch_corpus_core_batch@1`                                     | philosophy-extractor corpus-core intake (document-search's JSONL envelope)                                                                                                  |
| `belief-evidence` | `belief_evidence_interchange@1`, or `@2` once a segment has timing | belief-lab evidence import: one reference per transcript segment, no text copied; `@2` adds each segment's media span (microsecond ticks) and the stored scenes covering it |

Without `--snapshot`, the export reads Postgres (`--migrate` applies migrations
first). With `--snapshot <file.json>` it reads a `CorpusExportSnapshot` JSON
file and needs no database. Output goes to stdout, or byte-for-byte to
`--output`. An empty `--producer-revision` is rejected. The committed consumer
fixtures and the contract are in `tests/fixtures/exports/` and
`tests/corpus_exports.rs`.

### Speaker turns in corpus-core exports

`--format corpus-core` also includes **persisted** `research_annotations` of kind
`speaker_turn`. The existing `moenarch_corpus_core_batch@1` envelope and transcript
segment IDs are unchanged. Each video's `source` record gains two string metadata
fields only when that video has turns:

- `speaker_turns_schema`: `youtube-corpus/speaker_turns@1`
- `speaker_turns`: a compact JSON array of records with `id`, `corpusId`, `videoId`,
  optional original `streamId`/`segmentId`, `speakerLabel`, `startSeconds`,
  `endSeconds`, `sourceKind`, `producer`, `producerRevision` and
  `annotationRevision`. Fields absent in persistence remain absent; in particular,
  `producerRevision` is the stored annotation `processor_version`, not a
  synthesized immutable model commit. `annotationRevision` is the database row
  revision, **not** a producer/model revision.

Turns are ordered by video, start/end media seconds, corpus and annotation ID;
no speaker turns are inferred from transcript text or invented for videos without
annotations. `speakerLabel` is a raw, source-local diarization label (for example
`SPEAKER_00`), not a confirmed person's identity. Corpus IDs prevent annotations
from different research corpora being silently conflated.

To join a proposition candidate's philosophy-extractor `sourceSegments`, look
up the corpus-core `segmentId`, follow its asset to the video's source record,
and use the source's stored turns. For a direct segment match, the original
transcript UUID is also retained in the segment's `transcript_segment_id`
metadata; otherwise intersect the already-stored `startSeconds`/`endSeconds`
intervals. No audio timestamp inference is necessary. Naming people and deciding
their stance remain Belief Lab responsibilities; the corpus does not perform
either operation. See [the compatibility decision](docs/adr/0002-speaker-turn-corpus-core-compatibility.md).

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

`bun run verify:startup` exercises the one-command launch in a temporary clone
against a uniquely named disposable database on Compose Postgres. It checks
missing-tool diagnostics, HTTP/schema readiness, preserved configuration and a
second start without rebuilding unchanged frontend output. It leaves existing
corpus databases and the running local server alone.

See [docs/architecture.md](docs/architecture.md) and
[docs/development.md](docs/development.md) for implementation details.
