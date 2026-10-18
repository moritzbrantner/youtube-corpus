#!/usr/bin/env bash
set -euo pipefail

cd -- "${BASH_SOURCE[0]%/*}/.."
if ! command -v bun >/dev/null 2>&1; then
  echo "Bun is required. Install Bun, then rerun ./scripts/start.sh." >&2
  exit 1
fi
if [[ ! -e .env ]]; then
  cp -n .env.example .env
fi
exec bun scripts/start.ts "$@"
