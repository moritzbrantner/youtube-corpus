# Development

## Setup

```bash
bun install --frozen-lockfile
docker compose up -d postgres
cp .env.example .env
```

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

The `moritzbrantner` runtime, jobs, and text crates are pinned to a git revision
in `Cargo.toml`.
To update them:

1. Choose a new commit from `https://github.com/moritzbrantner/rust-packages.git`.
2. Update the pinned `rev` values in `Cargo.toml`.
3. Run `bun run build`.
4. Run `cargo update` for the affected `moritzbrantner-*` packages.
5. Run `cargo check` and `cargo test`.

If the dependency repository requires authentication, configure GitHub auth for
Cargo before running the update.

## Validation

Canonical validation:

```bash
bun run validate
```

Expanded validation:

```bash
bun run build
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
```
