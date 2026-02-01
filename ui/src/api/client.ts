import type { AppConfig, Job, Segment, ModelDownloadStatus, SummaryResponse } from "./types";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";

const IS_TAURI_RUNTIME = typeof window !== "undefined" && "__TAURI__" in window;

async function invokeCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!IS_TAURI_RUNTIME) {
    throw new Error("Tauri runtime is required for this app.");
  }
  return invoke<T>(command, args);
}

export async function getHealth(): Promise<boolean> {
  return invokeCommand<boolean>("get_health");
}

export async function getConfig(): Promise<AppConfig> {
  return invokeCommand<AppConfig>("get_config");
}

export async function updateConfig(cfg: AppConfig): Promise<AppConfig> {
  return invokeCommand<AppConfig>("update_config", { cfg });
}

export async function initializeConfig(cfg: AppConfig): Promise<AppConfig> {
  return invokeCommand<AppConfig>("initialize_config", { cfg });
}

export async function getConfigInitialized(): Promise<boolean> {
  return invokeCommand<boolean>("get_config_initialized");
}

export async function getJobs(): Promise<Job[]> {
  return invokeCommand<Job[]>("list_jobs");
}

export async function getJob(id: string): Promise<Job> {
  return invokeCommand<Job>("get_job", { id });
}

export async function createJob(file: File): Promise<Job> {
  const path = (file as File & { path?: string }).path;
  if (!path) {
    throw new Error("Tauri mode requires a file path (use drag & drop or picker).");
  }
  return invokeCommand<Job>("create_job_from_path", { path });
}

export async function createJobFromPath(path: string): Promise<Job> {
  return invokeCommand<Job>("create_job_from_path", { path });
}

export async function cancelJob(id: string): Promise<boolean> {
  return invokeCommand<boolean>("cancel_job", { id });
}

export async function deleteJob(id: string): Promise<boolean> {
  return invokeCommand<boolean>("delete_job", { id });
}

export async function exportJobToObsidian(id: string): Promise<boolean> {
  return invokeCommand<boolean>("export_to_obsidian", { id });
}

export async function getSegments(id: string): Promise<Segment[]> {
  return invokeCommand<Segment[]>("get_segments", { id });
}

export async function getSummary(id: string): Promise<SummaryResponse> {
  return invokeCommand<SummaryResponse>("get_summary", { id });
}

export async function summarizeJob(id: string): Promise<SummaryResponse> {
  return invokeCommand<SummaryResponse>("summarize_job", { id });
}

export async function getModelSize(modelSize: string): Promise<number> {
  return invokeCommand<number>("get_model_size", { modelSize });
}

export async function getModelDownloadStatus(modelSize: string): Promise<ModelDownloadStatus> {
  return invokeCommand<ModelDownloadStatus>("get_model_download_status", { modelSize });
}

export async function startModelDownload(modelSize: string): Promise<ModelDownloadStatus> {
  return invokeCommand<ModelDownloadStatus>("start_model_download", { modelSize });
}

export async function getModelInstalled(modelSize: string): Promise<boolean> {
  return invokeCommand<boolean>("get_model_installed", { modelSize });
}

export async function startWhisperDownload(url: string): Promise<ModelDownloadStatus> {
  return invokeCommand<ModelDownloadStatus>("start_whisper_download", { url });
}

export async function getLatestWhisperReleaseUrl(): Promise<string> {
  return invokeCommand<string>("get_latest_whisper_release_url");
}

export async function getWhisperDownloadStatus(): Promise<ModelDownloadStatus> {
  return invokeCommand<ModelDownloadStatus>("get_whisper_download_status");
}

export async function getWhisperInstalled(): Promise<boolean> {
  return invokeCommand<boolean>("get_whisper_installed");
}

export async function getClipUrl(id: string, start: number, end: number): Promise<string> {
  const path = await invokeCommand<string>("get_clip_path", { id, start, end });
  if (!path) {
    throw new Error("Clip path missing for job.");
  }
  return convertFileSrc(path);
}
