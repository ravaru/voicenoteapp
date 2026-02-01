SHELL := /bin/bash

UI_DIR := ui

# Ensure make doesn't run targets in parallel by default for setup.
.NOTPARALLEL:

.PHONY: setup ui

setup:
	@echo "[setup] Checking ffmpeg..."
	@[ -x "third_party/ffmpeg/bin/ffmpeg" ] || command -v ffmpeg >/dev/null 2>&1 || (echo "ffmpeg not found. Provide LGPL build in third_party/ffmpeg/bin/ffmpeg or add to PATH." && exit 1)
	@echo "[setup] Installing UI deps..."
	@cd $(UI_DIR) && npm install
	@echo "[setup] Done."

ui:
	@echo "[ui] Starting Tauri dev..."
	@cd $(UI_DIR) && npm run tauri dev
