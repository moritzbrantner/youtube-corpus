# Development

## Setup

Cross-repository Rust development uses exact sibling source repositories. Keep the repositories next to one another so `scripts/source-deps` can validate the declared local-only source graph:

```bash
mkdir youtube-corpus-src
cd youtube-corpus-src
git clone https://github.com/moritzbrantner/youtube-corpus.git
git clone https://github.com/moritzbrantner/nlp-stack.git
git clone https://github.com/moritzbrantner/moenarch-foundation.git
git clone https://github.com/moritzbrantner/visual-analysis.git
git clone https://github.com/moritzbrantner/coding-tooling.git
cd youtube-corpus
```

The outer coding workspace or agent loop should place each sibling repository at the exact revision declared in `.coding-tooling.source-deps.json`. Activate the graph with:

```bash
bash scripts/source-deps activate
bash scripts/source-deps status
```

Local-only source mode fails if a required sibling checkout is missing or at a different revision. It never falls back to authenticated Git and ordinary development does not require package publication or `GH_PACKAGES_TOKEN`.

```bash
bun install --frozen-lockfile
docker compose up -d postgres
cp .env.example .env
```

Binary release users do not need sibling source repositories; release archives include the CLI binary and embedded browser UI.

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

The exact source-development owners are `nlp-stack`, `moenarch-foundation`, and `visual-analysis`.

1. Move the relevant sibling repository to the intended reviewed commit.
2. Update its exact `rev` in `.coding-tooling.source-deps.json`.
3. Run `bash scripts/source-deps activate`; it verifies every local checkout before writing the Cargo patch config.
4. Run `bun run build`, the relevant Cargo checks/tests, and Postgres integration checks when affected.
5. Commit application changes and the reviewed source revision pin, but never the generated `.cargo/config.toml`.

Do not publish upstream packages simply to unblock this workflow. Registry-only or release-binary proof is a separate explicit distribution concern.

The GitHub release workflow is intentionally separate from ordinary development. It uses exact release inputs and must keep every Cargo invocation locked; source-development credentials or source graph changes are not authorization to mutate release dependency resolution.

## Validation

Canonical local source validation:

```bash
bash scripts/source-deps activate
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

When finished with source development:

```bash
bash scripts/source-deps deactivate
```

For a faster loop without Docker, use `bun run verify:fast`. Network-backed
`yt-dlp` checks stay outside the default verification path and can be run with
`bun run verify:yt-dlp` when `yt-dlp` and network access are available.
