#!/usr/bin/env bash
set -euo pipefail

# Build an LGPL-only FFmpeg for macOS and install into third_party/ffmpeg.
# This script intentionally disables GPL and non-free components.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FFMPEG_VERSION="${FFMPEG_VERSION:-6.1.1}"
SOURCE_URL="${FFMPEG_SOURCE_URL:-https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz}"
BUILD_DIR="${BUILD_DIR:-/tmp/ffmpeg-build-${FFMPEG_VERSION}}"
PREFIX="${PREFIX:-${ROOT_DIR}/third_party/ffmpeg}"

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
