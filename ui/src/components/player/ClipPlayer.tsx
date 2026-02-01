import React, { useEffect, useMemo, useRef, useState } from "react";
import Button from "../ui/Button";
import { useI18n } from "../../i18n/I18nProvider";

function formatMMSS(seconds: number): string {
  if (!Number.isFinite(seconds)) return "00:00";
  const total = Math.max(0, Math.floor(seconds));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
}

type Props = {
  src: string | null;
  start?: number;
  end?: number;
  playRequestId?: number;
  onEnded?: () => void;
};

export default function ClipPlayer({ src, start, end, playRequestId, onEnded }: Props) {
  const { t } = useI18n();
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [fullDuration, setFullDuration] = useState(0);
  const lastPlayRequest = useRef<number | null>(null);
  const clipStart = Math.max(0, start ?? 0);
  const clipEnd =
    end && end > clipStart ? end : fullDuration > 0 ? fullDuration : clipStart;
  const clipDuration = Math.max(0, clipEnd - clipStart);
  const clipTime = Math.max(0, currentTime - clipStart);
  const progress = clipDuration > 0 ? (clipTime / clipDuration) * 100 : 0;

  useEffect(() => {
    if (!audioRef.current || !src || playRequestId === undefined) return;
    if (lastPlayRequest.current === playRequestId) return;
    lastPlayRequest.current = playRequestId;
    const el = audioRef.current;
    const playWhenReady = () => {
      el.currentTime = clipStart;
      el.play().catch(() => {
        // Autoplay can fail; we keep controls responsive.
      });
    };
    if (el.readyState >= 1) {
      playWhenReady();
    } else {
      const handler = () => {
        el.removeEventListener("loadedmetadata", handler);
        playWhenReady();
      };
      el.addEventListener("loadedmetadata", handler);
    }
  }, [src, clipStart, playRequestId]);

  const label = useMemo(
    () => `${formatMMSS(clipTime)} / ${formatMMSS(clipDuration)}`,
    [clipTime, clipDuration]
  );

  return (
    <div className="clip-player">
      <audio
        ref={audioRef}
        src={src ?? undefined}
        onPlay={() => setIsPlaying(true)}
        onPause={() => setIsPlaying(false)}
        onTimeUpdate={(event) => {
          const nextTime = event.currentTarget.currentTime;
          setCurrentTime(nextTime);
          if (clipDuration > 0 && nextTime >= clipEnd) {
            event.currentTarget.pause();
            setIsPlaying(false);
            onEnded?.();
          }
        }}
        onLoadedMetadata={(event) => setFullDuration(event.currentTarget.duration)}
        onEnded={() => {
          setIsPlaying(false);
          onEnded?.();
        }}
      />
      <Button
        variant="secondary"
        onClick={() => {
          if (!audioRef.current) return;
          if (isPlaying) {
            audioRef.current.pause();
          } else {
            audioRef.current.play().catch(() => {});
          }
        }}
        disabled={!src}
        aria-label={isPlaying ? t("player.pause") : t("player.play")}
      >
        <span className="clip-icon" aria-hidden="true">
          {isPlaying ? (
            <svg viewBox="0 0 24 24" width="16" height="16">
              <rect x="6" y="5" width="4" height="14" rx="1" />
              <rect x="14" y="5" width="4" height="14" rx="1" />
            </svg>
          ) : (
            <svg viewBox="0 0 24 24" width="16" height="16">
              <path d="M8 5l11 7-11 7z" />
            </svg>
          )}
        </span>
      </Button>
      <div className="clip-time">{src ? label : "00:00 / 00:00"}</div>
      <button
        type="button"
        className="clip-progress"
        aria-label={t("player.scrub")}
        onClick={(event) => {
          if (!audioRef.current || clipDuration <= 0) return;
          const rect = event.currentTarget.getBoundingClientRect();
          const ratio = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
          audioRef.current.currentTime = clipStart + ratio * clipDuration;
        }}
      >
        <span style={{ width: `${progress}%` }} />
      </button>
    </div>
  );
}
