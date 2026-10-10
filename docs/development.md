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

## Dependency Updates

Capability crates are pinned to exact git revisions of `nlp-stack`, `moenarch-foundation` and `visual-analysis` in `Cargo.toml`.

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
