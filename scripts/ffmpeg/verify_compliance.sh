#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

forbidden_patterns=(
  "enable-gpl"
  "enable-nonfree"
  "x264"
  "x265"
  "fdk-aac"
)

search_cmd=""
if command -v rg >/dev/null 2>&1; then
  search_cmd="rg -n"
else
  search_cmd="grep -R -n"
fi

# Only scan build/config/docs to avoid false positives in runtime checks.
scan_paths=(
  scripts
  .github/workflows
  README.md
  ui/src-tauri/tauri.conf.json
)

for pattern in "${forbidden_patterns[@]}"; do
  if ${search_cmd} "${pattern}" "${scan_paths[@]}" 2>/dev/null; then
    echo "[compliance] Forbidden pattern found: ${pattern}"
    exit 1
  fi
done

if [ ! -f "THIRD_PARTY_NOTICES.md" ]; then
  echo "[compliance] THIRD_PARTY_NOTICES.md is missing."
  exit 1
fi

if ! ${search_cmd} -i "ffmpeg" THIRD_PARTY_NOTICES.md >/dev/null 2>&1; then
  echo "[compliance] THIRD_PARTY_NOTICES.md must mention FFmpeg."
  exit 1
fi

if ! ${search_cmd} -i "lgpl" THIRD_PARTY_NOTICES.md >/dev/null 2>&1; then
  echo "[compliance] THIRD_PARTY_NOTICES.md must mention LGPL."
  exit 1
fi

echo "[compliance] OK"
