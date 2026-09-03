#!/usr/bin/env bash
set -euo pipefail

readonly DB_NAME="${VERIFY_POSTGRES_DB:-youtube_corpus_verify}"
readonly DEFAULT_DATABASE_URL="postgres://postgres:postgres@localhost:5432/${DB_NAME}"

if [[ -z "${DATABASE_URL:-}" ]]; then
  docker compose up -d --wait postgres
  docker compose exec -T postgres dropdb -U postgres --if-exists "${DB_NAME}"
  docker compose exec -T postgres createdb -U postgres "${DB_NAME}"
  export DATABASE_URL="${DEFAULT_DATABASE_URL}"
fi

cargo test --test db_schema -- --ignored
cargo test --test db_behavior -- --ignored
cargo test --test multimodal_db -- --ignored
