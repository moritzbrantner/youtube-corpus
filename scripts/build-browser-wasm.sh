#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NLP_STACK_DIR="${NLP_STACK_DIR:-$ROOT_DIR/../nlp-stack}"

if [[ ! -d "$NLP_STACK_DIR/crates/bindings/text-analysis-wasm" ]]; then
  echo "nlp-stack source not found at $NLP_STACK_DIR" >&2
  echo "Set NLP_STACK_DIR or place nlp-stack next to youtube-corpus." >&2
  exit 1
fi

bash "$NLP_STACK_DIR/packages/text-analysis-wasm/scripts/build-wasm.sh"

rm -rf "$ROOT_DIR/public/wasm"
mkdir -p "$ROOT_DIR/public/wasm"
cp "$NLP_STACK_DIR/packages/text-analysis-wasm/pkg/moenarch_text_analysis_wasm.js" "$ROOT_DIR/public/wasm/"
cp "$NLP_STACK_DIR/packages/text-analysis-wasm/pkg/moenarch_text_analysis_wasm_bg.wasm" "$ROOT_DIR/public/wasm/"
