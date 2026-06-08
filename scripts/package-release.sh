#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: scripts/package-release.sh <version> <target>" >&2
}

if [[ $# -ne 2 ]]; then
  usage
  exit 2
fi

version="$1"
target="$2"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="$repo_root/target/release-artifacts"
package_name="youtube-corpus-v${version}-${target}"
stage_dir="$artifact_dir/$package_name"

cd "$repo_root"

if [[ ! -f dist/index.html ]]; then
  echo "dist/index.html is missing; run 'bun run build' before packaging." >&2
  exit 1
fi

if [[ "$target" == *windows* ]]; then
  binary_path="target/release/youtube-corpus.exe"
  binary_name="youtube-corpus.exe"
  archive_name="${package_name}.zip"
else
  binary_path="target/release/youtube-corpus"
  binary_name="youtube-corpus"
  archive_name="${package_name}.tar.gz"
fi

if [[ ! -f "$binary_path" ]]; then
  echo "$binary_path is missing; run 'cargo build --release --locked' before packaging." >&2
  exit 1
fi

for file in README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE; do
  if [[ ! -f "$file" ]]; then
    echo "$file is missing." >&2
    exit 1
  fi
done

rm -rf "$stage_dir"
mkdir -p "$stage_dir"
cp "$binary_path" "$stage_dir/$binary_name"
cp README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE "$stage_dir/"

if [[ "$binary_name" == "youtube-corpus" ]]; then
  chmod 755 "$stage_dir/$binary_name"
fi

archive_path="$artifact_dir/$archive_name"
rm -f "$archive_path"

if [[ "$archive_name" == *.zip ]]; then
  if command -v zip >/dev/null 2>&1; then
    (cd "$stage_dir" && zip -qr "$archive_path" .)
  elif command -v 7z >/dev/null 2>&1; then
    (cd "$stage_dir" && 7z a -tzip "$archive_path" . >/dev/null)
  elif command -v powershell.exe >/dev/null 2>&1; then
    powershell.exe -NoProfile -Command "Compress-Archive -Path '${stage_dir}/*' -DestinationPath '${archive_path}' -Force" >/dev/null
  else
    echo "zip, 7z, or powershell.exe is required to create $archive_name." >&2
    exit 1
  fi
else
  tar -czf "$archive_path" -C "$stage_dir" .
fi

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$artifact_dir" && sha256sum "$archive_name" > SHA256SUMS)
elif command -v shasum >/dev/null 2>&1; then
  (cd "$artifact_dir" && shasum -a 256 "$archive_name" > SHA256SUMS)
elif command -v powershell.exe >/dev/null 2>&1; then
  hash="$(powershell.exe -NoProfile -Command "(Get-FileHash -Algorithm SHA256 '${archive_path}').Hash.ToLowerInvariant()" | tr -d '\r')"
  printf "%s  %s\n" "$hash" "$archive_name" > "$artifact_dir/SHA256SUMS"
else
  echo "sha256sum, shasum, or powershell.exe is required to create SHA256SUMS." >&2
  exit 1
fi

echo "$archive_path"
