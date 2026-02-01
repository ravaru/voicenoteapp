# VoiceNote

Локальное desktop-приложение для транскрибации MP3/M4A/WAV и экспорта в Obsidian. MVP ориентирован на macOS Apple Silicon. Всё работает локально: аудио, модели и результаты не уходят в сеть.

## Документация пользователя

См. `USER_GUIDE.md`.

## Архитектура

- **UI**: Tauri v2 + React + TypeScript (`ui/`).
- **Backend**: Rust core внутри Tauri (миграция в процессе).
- **Очередь**: один фоновой воркер обрабатывает jobs последовательно.
- **Данные**: app data каталог (`AppData`) + `index.json`/`config.json`.

## Pipeline

1. **Upload**: MP3/M4A/WAV попадает в очередь и сохраняется как `AppData/voicenote/jobs/<job_id>/audio.original.<ext>`.
2. **Convert**: `ffmpeg` -> `audio.wav` (16kHz, mono).
3. **Transcribe**: whisper.cpp с выбранной моделью и языком.
4. **Artifacts**: `whisper.txt`, `whisper.srt`, `whisper.json`.
5. **Summarize (optional)**: локальная суммаризация через Ollama (RU).
6. **Note**: сборка `note.md` (Transcript + Summary).
7. **Export**: по кнопке экспорт в Obsidian (Markdown, выбранные jobs).

## Структура выходных данных

`AppData/voicenote/jobs/<job_id>/`:
- `audio.original.<ext>`
- `audio.wav`
- `whisper.txt`
- `whisper.srt`
- `whisper.json`
- `summary.md`
- `note.md`

## Установка

### Требования
- Rust (для Tauri)
- Node.js (рекомендуется LTS)
- FFmpeg (только LGPL сборка, без GPL/non-free)
- whisper.cpp (бинарник + модель)

FFmpeg нужно положить в `third_party/ffmpeg/bin/ffmpeg` (и `ffprobe` при необходимости).
Для macOS есть скрипт сборки LGPL-версии:

```bash
scripts/ffmpeg/build_macos_lgpl.sh
```

FFmpeg используется как внешний CLI-бинарник; доступность форматов зависит от сборки и платформы.

whisper.cpp нужно положить в:
- бинарник: `third_party/whisper/bin/whisper` (или `.../bin/main`)
- модель: `third_party/whisper/models/ggml-<size>.bin` (например, `ggml-small.bin`)

Альтернатива: можно указать пути через переменные окружения
`VOICENOTE_WHISPER_PATH` и `VOICENOTE_WHISPER_MODEL`.

Модель можно скачать из UI (Settings → Download model). Файл сохраняется в
`AppData/voicenote/models/ggml-<size>.bin`.

Бинарник whisper.cpp можно скачать из UI (Settings → Download whisper) по указанному URL.
Файл сохраняется в `AppData/voicenote/whisper/bin/whisper`.
Поле URL можно заполнить автоматически кнопкой “Use latest release URL” (GitHub Releases).

### Setup

```bash
make setup
```

## Запуск

```bash
make ui
```

## Конфиг Obsidian

- При первом запуске открывается Wizard.
- Укажите путь к vault и подпапку (default: `Transcripts`).
- Настройки можно изменить в `Settings`.

## Локальная суммаризация (Ollama)

- По умолчанию включена и выполняется после транскрибации.
- Используется локальный Ollama API (`http://127.0.0.1:11434`).
- Модель по умолчанию: `qwen2.5:7b-instruct`.
- Если Ollama недоступен, job не падает, summary помечается как `skipped`.

## Troubleshooting

- **ffmpeg не найден**: положите LGPL-сборку в `third_party/ffmpeg/bin/ffmpeg` или установите в PATH.
- **Нет прав на запись в vault**: убедитесь, что приложению разрешён доступ к каталогу Obsidian.
- **Медленно/падает при первой загрузке silero-vad**: требуется загрузка модели (может занять время).
- **Summary не создаётся**: проверьте, что Ollama запущен и доступен по `ollama_base_url`.

## Безопасность и приватность

- Всё происходит локально: аудио, текст и файлы остаются на компьютере.
- Сеть не используется для отправки данных.

## Third-party licenses

См. `THIRD_PARTY_NOTICES.md`.
