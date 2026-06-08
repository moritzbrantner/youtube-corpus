# Development

## Setup

Source builds currently expect `youtube-corpus` and `rust-packages` to be
checked out as sibling repositories:

```bash
mkdir youtube-corpus-release-src
cd youtube-corpus-release-src
git clone https://github.com/moritzbrantner/rust-packages.git
git clone https://github.com/moritzbrantner/youtube-corpus.git
cd youtube-corpus
```

```bash
bun install --frozen-lockfile
docker compose up -d postgres
cp .env.example .env
```

Binary release users do not need to clone `rust-packages`; release archives
include the CLI binary and embedded browser UI.

The default local database URL is:

```text
postgres://postgres:postgres@localhost:5432/youtube_corpus
```

## Backend CLI Development

Run command-line workflows directly with Cargo:

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

CI enforces this order.

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

The `moritzbrantner` runtime, jobs, text, and video crates are local path
dependencies from the sibling `rust-packages` checkout. To update them:

1. Update the sibling `rust-packages` checkout to the intended commit.
2. Run `bun run build`.
3. Run `cargo update` for the affected `moritzbrantner-*` packages when lockfile
   metadata changes.
4. Run `cargo check` and `cargo test`.

For release builds, the GitHub workflow checks out `youtube-corpus` and
`rust-packages` as siblings and pins `rust-packages` through the workflow input
or `RUST_PACKAGES_RELEASE_REF` repository variable.

## Validation

Canonical validation:

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
