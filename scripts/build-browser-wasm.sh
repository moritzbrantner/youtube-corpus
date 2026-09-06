#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NLP_STACK_DIR="${NLP_STACK_DIR:-$ROOT_DIR/../nlp-stack}"
AUDIO_ANALYSIS_DIR="${AUDIO_ANALYSIS_DIR:-$ROOT_DIR/../audio-analysis}"

if [[ ! -d "$NLP_STACK_DIR/crates/bindings/text-analysis-wasm" ]]; then
  echo "nlp-stack source not found at $NLP_STACK_DIR" >&2
  echo "Set NLP_STACK_DIR or place nlp-stack next to youtube-corpus." >&2
  exit 1
fi

if [[ ! -f "$AUDIO_ANALYSIS_DIR/packages/audio-analysis-transcription-wasm/index.js" ]]; then
  echo "audio-analysis browser transcription source not found at $AUDIO_ANALYSIS_DIR" >&2
  echo "Set AUDIO_ANALYSIS_DIR or place audio-analysis next to youtube-corpus." >&2
  exit 1
fi

bash "$NLP_STACK_DIR/packages/text-analysis-wasm/scripts/build-wasm.sh"

rm -rf "$ROOT_DIR/public/wasm" "$ROOT_DIR/public/audio-analysis"
mkdir -p "$ROOT_DIR/public/wasm" "$ROOT_DIR/public/audio-analysis"
cp "$NLP_STACK_DIR/packages/text-analysis-wasm/pkg/moenarch_text_analysis_wasm.js" "$ROOT_DIR/public/wasm/"
cp "$NLP_STACK_DIR/packages/text-analysis-wasm/pkg/moenarch_text_analysis_wasm_bg.wasm" "$ROOT_DIR/public/wasm/"
cp "$AUDIO_ANALYSIS_DIR/packages/audio-analysis-transcription-wasm/index.js" \
  "$ROOT_DIR/public/audio-analysis/transcription.js"
