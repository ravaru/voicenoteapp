#!/usr/bin/env bash
set -euo pipefail

# Build an LGPL-only FFmpeg for macOS and install into third_party/ffmpeg.
# This script intentionally disables GPL and non-free components.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FFMPEG_VERSION="${FFMPEG_VERSION:-6.1.1}"
SOURCE_URL="${FFMPEG_SOURCE_URL:-https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz}"
BUILD_DIR="${BUILD_DIR:-/tmp/ffmpeg-build-${FFMPEG_VERSION}}"
PREFIX="${PREFIX:-${ROOT_DIR}/third_party/ffmpeg}"
PACKAGE_ZIP="${PACKAGE_ZIP:-0}"
OUTPUT_ZIP="${OUTPUT_ZIP:-${ROOT_DIR}/dist/ffmpeg-macos-arm64-lgpl.zip}"

mkdir -p "${BUILD_DIR}"
mkdir -p "${PREFIX}"

echo "[ffmpeg] Downloading ${SOURCE_URL}"
curl -L "${SOURCE_URL}" -o "${BUILD_DIR}/ffmpeg.tar.xz"
tar -xf "${BUILD_DIR}/ffmpeg.tar.xz" -C "${BUILD_DIR}"

SRC_DIR="${BUILD_DIR}/ffmpeg-${FFMPEG_VERSION}"
if [ ! -d "${SRC_DIR}" ]; then
  echo "[ffmpeg] Source dir not found: ${SRC_DIR}"
  exit 1
fi

cd "${SRC_DIR}"

echo "[ffmpeg] Configuring (LGPL-only, shared libs, no GPL/non-free)"
./configure \
  --prefix="${PREFIX}" \
  --disable-gpl \
  --disable-nonfree \
  --disable-autodetect \
  --enable-shared \
  --disable-static \
  --disable-debug \
  --disable-doc \
  --disable-ffplay

echo "[ffmpeg] Building"
make -j"$(sysctl -n hw.ncpu)"

echo "[ffmpeg] Installing to ${PREFIX}"
make install

echo "[ffmpeg] Done. Binaries at ${PREFIX}/bin"

if [ "${PACKAGE_ZIP}" = "1" ]; then
  echo "[ffmpeg] Packaging LGPL zip"
  STAGING_DIR="$(mktemp -d)"
  mkdir -p "${STAGING_DIR}/ffmpeg/bin" "${STAGING_DIR}/ffmpeg/lib" "${STAGING_DIR}/ffmpeg/licenses"
  cp "${PREFIX}/bin/ffmpeg" "${STAGING_DIR}/ffmpeg/bin/"
  if [ -f "${PREFIX}/bin/ffprobe" ]; then
    cp "${PREFIX}/bin/ffprobe" "${STAGING_DIR}/ffmpeg/bin/"
  fi
  if ls "${PREFIX}/lib/"*.dylib >/dev/null 2>&1; then
    cp "${PREFIX}/lib/"*.dylib "${STAGING_DIR}/ffmpeg/lib/"
  fi
  if [ -f "${SRC_DIR}/COPYING.LGPLv2.1" ]; then
    cp "${SRC_DIR}/COPYING.LGPLv2.1" "${STAGING_DIR}/ffmpeg/licenses/"
  elif [ -f "${SRC_DIR}/LICENSE.md" ]; then
    cp "${SRC_DIR}/LICENSE.md" "${STAGING_DIR}/ffmpeg/licenses/"
  fi
  mkdir -p "$(dirname "${OUTPUT_ZIP}")"
  (cd "${STAGING_DIR}" && zip -r "${OUTPUT_ZIP}" ffmpeg >/dev/null)
  echo "[ffmpeg] Packaged ${OUTPUT_ZIP}"
fi
