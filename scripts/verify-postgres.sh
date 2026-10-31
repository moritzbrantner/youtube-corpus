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
cargo test --test discovery_db -- --ignored
cargo test --test visual_timeline_db -- --ignored
cargo test --test media_evidence_db -- --ignored
cargo test --test visual_provenance_db -- --ignored
cargo test --test corpus_exports -- --ignored
cargo test --test corpus_export_db -- --ignored
cargo test --test unified_evidence_search_db -- --ignored
cargo test --test tantivy_search_pilot_db -- --ignored
cargo test --test review_regressions_db -- --ignored
cargo test --test user_labels_db -- --ignored
cargo test --test channel_directory_db -- --ignored
cargo test --test video_inspection_db -- --ignored
cargo test --test processing_runs_db -- --ignored
cargo test --test entity_explorer_db -- --ignored
cargo test --test detector_profiles_db -- --ignored
cargo test --test face_matching_db -- --ignored
cargo test --test face_matching_acceptance_db -- --ignored
cargo test --test person_names_db -- --ignored
cargo test --test identities_db -- --ignored
cargo test --test events_db -- --ignored
cargo test --test event_extraction_db -- --ignored
cargo test --test review_first_db -- --ignored
cargo test --test translated_text_db -- --ignored --skip native_opus_mt
cargo test --test backfill_db -- --ignored
cargo test --test source_monitor_db -- --ignored
