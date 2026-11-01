# Development

## Setup

Capability crates come from exact public git revisions pinned in `Cargo.toml` (owner decision, #40), so a plain clone builds and tests without sibling repositories, credentials or source mode:

```bash
git clone https://github.com/moritzbrantner/youtube-corpus.git
cd youtube-corpus
```

Testing against an _unpushed_ change in a sibling repository is not supported by `scripts/source-deps` yet: it only generates `[patch.crates-io]`, which does not apply to git dependencies (moritzbrantner/coding-tooling#313). `.coding-tooling.source-deps.json` therefore declares no patches. Push the upstream commit and bump the `rev` instead.

```bash
bun install --frozen-lockfile
docker compose up -d postgres
cp .env.example .env
```

Binary release users only need the release archive; it includes the CLI binary and embedded browser UI.

The default local database URL is:

```text
postgres://postgres:postgres@localhost:5432/youtube_corpus
```

## Backend CLI Development

Run command-line workflows directly with Cargo after source-mode activation:

```bash
cargo run -- migrate
cargo run -- api-schema
cargo run -- search --query "first uploaded youtube video" --top-k 5
cargo run -- status
```

Most deterministic tests do not need external services:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

## Web UI Development

Production-style local run:

```bash
bun run build
DATABASE_URL=postgres://postgres:postgres@localhost:5432/youtube_corpus cargo run -- --migrate
```

Fast frontend iteration:

```bash
cargo run -- --no-open
bun run dev:frontend
```

Open `http://127.0.0.1:5173`. Vite proxies `/api` to the Rust server on
`http://127.0.0.1:1420`.

### Pages canonical NER

The Pages analyzer's opt-in "Canonical named entities" panel loads NLP Stack's `browser-ner`
module from jsDelivr at the commit pinned in `src/app/features/analyzer/canonical-ner.ts`.
Its browser tests use the en/de/es caption fixtures in `tests/fixtures/canonical-ner/`:

```bash
# Stub producer; no downloads. Covers opt-in, run, timestamps, provenance, cancel,
# a failed module load and stale results after a transcript change.
bun run test:e2e tests/e2e/canonical-ner.spec.ts

# Real pinned module and model (opt-in). Uses a persistent Chromium profile
# (CANONICAL_NER_PROFILE, default ~/.cache/youtube-corpus/canonical-ner-profile), so only
# the first run downloads the model. CANONICAL_NER_MEASURE=1 also measures from a fresh
# profile and writes test-results/canonical-ner-metrics.json.
CANONICAL_NER_MODEL=1 bun run test:e2e tests/e2e/canonical-ner-model.spec.ts --project=desktop-chromium
```

CI runs neither suite. CI does not run Playwright, and the real model needs about 140 MB of
network on a cold profile.

Measured on 2026-11-01 with nlp-stack `e1063c5b` and the model
`Xenova/distilbert-base-multilingual-cased-ner-hrl@c2a4dbf5` (q8). Hardware: Ryzen 7 5700X,
headless Playwright Chromium, Wasm CPU backend (no WebGPU), on a shared machine with load
average about 20. The figures are ranges over four runs.

|                                                  | Canonical NER                                                                                                                     | Lexical `proper-noun-like` guesses                                                                                                                                            |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Cold download                                    | 140.4 MB (model 135.4 MB, ORT Wasm 4.7 MB, tokenizer, Transformers.js)                                                            | 0 (ships in the page)                                                                                                                                                         |
| First run, cold profile (download, init, 4 cues) | 6.0–10.8 s                                                                                                                        | n/a                                                                                                                                                                           |
| First run after a reload, model cached           | 2.4–5.0 s                                                                                                                         | n/a                                                                                                                                                                           |
| Warm rerun, 4 cues                               | 0.14–0.6 s                                                                                                                        | under 1 ms                                                                                                                                                                    |
| Next transcript in the same page (worker reused) | 0.75 s (one run)                                                                                                                  | under 1 ms                                                                                                                                                                    |
| 240-cue transcript, warm                         | 5.7–15.3 s (420 entities)                                                                                                         | 3–21 ms (16 guesses, capped)                                                                                                                                                  |
| Peak browser RSS, all processes                  | 1.35–1.40 GB cold; 1.51–1.60 GB on 240 cues                                                                                       | 0.54–0.56 GB baseline                                                                                                                                                         |
| Labels                                           | PER / ORG / LOC with confidence                                                                                                   | none                                                                                                                                                                          |
| Fixtures (en/de/es, 4 cues each)                 | 21 entities: every person and place, plus Deutsche Bahn and Cambridge University Press. Misses "Roman Empire"; labels museums LOC | 6 exact, plus "Roman Empire" and 3 clipped spans ("Die Deutsche Bahn", "Johann Wolfgang", "El Museo"). Misses every one-word, single-mention name (London, Berlin, Madrid, …) |

Known gap: a cached model does not work offline yet. Transformers.js still probes
huggingface.co on the moving `main` revision at startup, and with no network it fails with
"this.tokenizer is not a function"
([nlp-stack#179](https://github.com/moritzbrantner/nlp-stack/issues/179)). A cold offline load
fails cleanly, and a retry fails the same way.

## Embedded Assets

The Rust web server embeds files from `dist/` using `rust-embed`. Run the
frontend build before `cargo check`, `cargo test`, or `cargo run` in a fresh
checkout:

```bash
bun run build
cargo check
```

Hosted CI keeps only repository-local checks. Exact cross-repository Rust build/test evidence comes from the local source workspace.

## Integration Tests

Postgres and `pgvector` are required for ignored integration tests:

```bash
docker compose up -d --wait postgres
DATABASE_URL=postgres://postgres:postgres@localhost:5432/youtube_corpus cargo test -- --ignored
docker compose down -v
```

The ignored database tests cover migrations, seeded search behavior, transcript
context, ingest-run job mapping, and subscription upserts.

Tests that require real `yt-dlp`, Postgres, or ASR should remain ignored or
skip when tools or environment variables are missing.

### Optional native face integration

The regular Postgres gate checks cache invalidation and failure retention without models
or network access. A separate ignored test exercises actual YuNet/SFace inference and
refresh against an operator-supplied short video containing a repeated face. Use a disposable
Postgres database, a configured model cache, and an ONNX Runtime API 24 shared library:

```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/youtube_corpus_verify \
ORT_DYLIB_PATH=/absolute/path/libonnxruntime.so \
CORPUS_FACE_TEST_MEDIA=/absolute/path/local-face-video.mp4 \
CORPUS_FACE_TEST_MODELS=/absolute/path/use-case-output/models \
cargo test --test detector_profiles_db native_faces -- --ignored
```

It checks model observations, 128-dimensional embeddings, within-video tracks, reuse and
refresh without duplicate rows. Real media and biometric vectors must stay ignored locally.

### Face identity review verification

`face_matching_db` covers the HTTP contract with explicitly synthetic, non-biometric vectors:
automatic compatible matches, negative/incompatible evidence, ambiguous chains/alternatives,
simultaneous contradictory faces, persistent split/unlink/link/confirm corrections, corpus
isolation, occurrence counts and compatible run replacement. The canonical Postgres gate and
CI both run it.

With the native face prerequisites above, run the actual producer-to-identity integration:

```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/youtube_corpus_verify \
ORT_DYLIB_PATH=/absolute/path/libonnxruntime.so \
CORPUS_FACE_TEST_MEDIA=/absolute/path/local-repeat-face-video.mp4 \
CORPUS_FACE_TEST_MODELS=/absolute/path/use-case-output/models \
cargo test --test face_matching_db native_face_output -- --ignored --nocapture
```

This creates two local corpus videos from the supplied repeated-face reference, executes
canonical YuNet/SFace, matches their tracks, unlinks one, refreshes the actual detector output,
and checks that the correction survives. It does not establish general real-world identity
accuracy or name attribution. Media and embeddings remain local.

### Optional native speaker integration

The regular database gate checks truthful failure without retained media. The native
integration uses a local speech recording (any FFmpeg-supported rate/format), verified
WeSpeaker model cache and ONNX Runtime API 24:

```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/youtube_corpus_verify \
ORT_DYLIB_PATH=/absolute/path/libonnxruntime.so \
CORPUS_VOICE_TEST_MEDIA=/absolute/path/local-speech.wav \
CORPUS_VOICE_TEST_MODELS=/absolute/path/use-case-output/models \
cargo test --test detector_profiles_db native_voices -- --ignored --nocapture
```

It exercises the effective plan, 256-dimensional observations, speaker-track links,
unchanged reuse, refresh without duplicates, and evidence retention after invalidated input.
The input is copied to a temporary path before testing a failed refresh. Real speech and
voice embeddings stay ignored locally.

## Dependency Updates

Capability crates are pinned to exact git revisions of `nlp-stack`, `moenarch-foundation`, `visual-analysis` and `audio-analysis` in `Cargo.toml`.

1. Land and push the reviewed upstream commit.
2. Update the matching `rev` values in `Cargo.toml` (all crates from one repository share one rev).
3. Keep `[patch.crates-io]` aligned: the foundation rev must be the one the pinned nlp-stack and visual-analysis declare, and the `moenarch-text-core` patch must match the nlp-stack rev. Cargo must report no unused patches.
4. Run `cargo update` for the changed crates, `bun run build`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`, and Postgres integration checks when affected.
5. Commit `Cargo.toml` and `Cargo.lock` together.

Do not publish upstream packages simply to unblock this workflow. Registry-only or release-binary proof is a separate explicit distribution concern.

The GitHub release workflow is intentionally separate from ordinary development. It uses exact release inputs and must keep every Cargo invocation locked; source-development credentials or source graph changes are not authorization to mutate release dependency resolution.

## Validation

Canonical local validation:

```bash
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

For a faster loop without Docker, use `bun run verify:fast`. Network-backed
`yt-dlp` checks stay outside the default verification path and can be run with
`bun run verify:yt-dlp` when `yt-dlp` and network access are available.

### Corpus navigation

The local app serves `/corpora` for corpus selection and management and `/search`
for unified spoken/OCR search. Both pages expose the current corpus. Their URLs
retain the corpus, submitted query, evidence selection and search filters through
reloads and browser history. Changing corpus retains the page and filters while
clearing the selected evidence. An unknown corpus requires an explicit replacement
selection. Search currently requires one corpus; there is no all-corpora option.
The root page remains the standalone analyzer; static Pages does not provide the
local database workspaces.
