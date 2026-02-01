use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::PathBuf,
    sync::mpsc,
    time::{SystemTime, UNIX_EPOCH},
    sync::Mutex,
    thread,
    process::Command,
    io::{BufRead, BufReader},
    collections::HashMap,
    sync::Arc,
    os::unix::fs::PermissionsExt,
};
use tauri::{AppHandle, State, Emitter, Manager};

// These are minimal Rust-side mirrors of the existing UI types.
// They intentionally mirror the TS shapes so we can return stub data now,
// and later evolve them into the real Job/Config models.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub initialized: bool,
    pub vault_path: String,
    pub output_subfolder: String,
    pub model_size: String,
    pub preload_model: bool,
    pub language: Option<String>,
    pub enable_summarization: bool,
    pub auto_summarize_after_transcription: bool,
    pub ollama_base_url: String,
    pub ollama_model: String,
    pub summary_prompt: String,
    pub include_timestamps: bool,
    pub watch_inbox_enabled: bool,
    pub inbox_poll_seconds: u32,
    pub whisper_binary_url: Option<String>,
    pub ffmpeg_binary_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub filename: String,
    pub status: String,
    pub progress: f32,
    pub stage: String,
    pub logs: Vec<String>,
    pub created_at: String,
    pub audio_path: String,
    pub transcript_txt_path: String,
    pub transcript_json_path: String,
    pub transcript_srt_path: String,
    pub md_preview: Option<String>,
    pub summary_status: Option<String>,
    pub summary_model: Option<String>,
    pub summary_error: Option<String>,
    pub summary_md: Option<String>,
    pub exported_to_obsidian: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub start: f32,
    pub end: f32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryResponse {
    pub summary_status: String,
    pub summary_model: String,
    pub summary_error: Option<String>,
    pub summary_md: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDownloadStatus {
    pub state: String,
    pub model_size: String,
    pub repo_id: String,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub message: Option<String>,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JobLogEvent {
    id: String,
    line: String,
}

pub struct ModelDownloadState {
    models_dir: PathBuf,
    whisper_dir: PathBuf,
    ffmpeg_dir: PathBuf,
    statuses: Arc<Mutex<HashMap<String, ModelDownloadStatus>>>,
}

impl ModelDownloadState {
    pub fn load(app: &AppHandle) -> Result<Self, String> {
        let base_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("app_data_dir unavailable: {err}"))?;
        let app_dir = base_dir.join("voicenote");
        fs::create_dir_all(&app_dir)
            .map_err(|err| format!("failed to create app data dir: {err}"))?;
        let models_dir = app_dir.join("models");
        fs::create_dir_all(&models_dir)
            .map_err(|err| format!("failed to create models dir: {err}"))?;
        let whisper_dir = app_dir.join("whisper");
        fs::create_dir_all(&whisper_dir)
            .map_err(|err| format!("failed to create whisper dir: {err}"))?;
        let ffmpeg_dir = app_dir.join("ffmpeg");
        fs::create_dir_all(&ffmpeg_dir)
            .map_err(|err| format!("failed to create ffmpeg dir: {err}"))?;
        Ok(Self {
            models_dir,
            whisper_dir,
            ffmpeg_dir,
            statuses: Arc::new(Mutex::new(HashMap::new())),
        })
    }
}

fn emit_job_updated(app: &AppHandle, job: &Job) {
    // Fire-and-forget so UI can update without polling in Tauri mode.
    let _ = app.emit("job:updated", job);
}

fn emit_job_log(app: &AppHandle, job_id: &str, line: &str) {
    // Small payload so UI can append to its log buffer.
    let payload = JobLogEvent {
        id: job_id.to_string(),
        line: line.to_string(),
    };
    let _ = app.emit("job:log", payload);
}

fn push_log(job: &mut Job, line: &str) {
    // Keep a bounded in-memory log buffer to avoid unbounded growth.
    job.logs.push(line.to_string());
    if job.logs.len() > 2000 {
        let excess = job.logs.len() - 2000;
        job.logs.drain(0..excess);
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            initialized: false,
            vault_path: String::new(),
            output_subfolder: "VoiceNote".to_string(),
            model_size: "small".to_string(),
            preload_model: false,
            language: Some("en".to_string()),
            enable_summarization: true,
            auto_summarize_after_transcription: true,
            ollama_base_url: "http://127.0.0.1:11434".to_string(),
            ollama_model: "qwen2.5:7b-instruct".to_string(),
            summary_prompt: "Summarize the transcript.".to_string(),
            include_timestamps: true,
            watch_inbox_enabled: false,
            inbox_poll_seconds: 10,
            whisper_binary_url: Some(
                "https://github.com/bizenlabs/whisper-cpp-macos-bin/releases/latest"
                    .to_string(),
            ),
            ffmpeg_binary_url: Some(
                "https://github.com/ravaru/voicenoteapp/releases/latest/download/ffmpeg-macos-arm64-lgpl.zip".to_string(),
            ),
        }
    }
}

pub struct ConfigState {
    path: PathBuf,
    config: Mutex<AppConfig>,
}

impl ConfigState {
    pub fn load(app: &AppHandle) -> Result<Self, String> {
        let base_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("app_data_dir unavailable: {err}"))?;
        let app_dir = base_dir.join("voicenote");
        fs::create_dir_all(&app_dir)
            .map_err(|err| format!("failed to create app data dir: {err}"))?;
        let path = app_dir.join("config.json");
        let config = load_config_from_disk(&path)?;
        Ok(Self {
            path,
            config: Mutex::new(config),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JobIndex {
    jobs: Vec<Job>,
}

pub struct JobIndexState {
    path: PathBuf,
    jobs_dir: PathBuf,
    index: Mutex<JobIndex>,
}

impl JobIndexState {
    pub fn load(app: &AppHandle) -> Result<Self, String> {
        let base_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| format!("app_data_dir unavailable: {err}"))?;
        let app_dir = base_dir.join("voicenote");
        fs::create_dir_all(&app_dir)
            .map_err(|err| format!("failed to create app data dir: {err}"))?;
        let path = app_dir.join("index.json");
        let jobs_dir = app_dir.join("jobs");
        fs::create_dir_all(&jobs_dir)
            .map_err(|err| format!("failed to create jobs dir: {err}"))?;
        let index = load_index_from_disk(&path)?;
        Ok(Self {
            path,
            jobs_dir,
            index: Mutex::new(index),
        })
    }
}

pub struct JobQueueState {
    sender: mpsc::Sender<String>,
}

impl JobQueueState {
    pub fn new(sender: mpsc::Sender<String>) -> Self {
        Self { sender }
    }

    pub fn enqueue(&self, job_id: String) -> Result<(), String> {
        self.sender
            .send(job_id)
            .map_err(|err| format!("failed to enqueue job: {err}"))
    }
}

fn load_config_from_disk(path: &PathBuf) -> Result<AppConfig, String> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let mut file =
        File::open(path).map_err(|err| format!("failed to open config.json: {err}"))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|err| format!("failed to read config.json: {err}"))?;
    if contents.trim().is_empty() {
        return Ok(AppConfig::default());
    }
    serde_json::from_str(&contents).map_err(|err| format!("invalid config.json: {err}"))
}

fn save_config_to_disk(path: &PathBuf, config: &AppConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(config)
        .map_err(|err| format!("failed to serialize config.json: {err}"))?;
    let mut file =
        File::create(path).map_err(|err| format!("failed to write config.json: {err}"))?;
    file.write_all(json.as_bytes())
        .map_err(|err| format!("failed to save config.json: {err}"))
}

fn load_index_from_disk(path: &PathBuf) -> Result<JobIndex, String> {
    if !path.exists() {
        return Ok(JobIndex { jobs: Vec::new() });
    }
    let mut file =
        File::open(path).map_err(|err| format!("failed to open index.json: {err}"))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|err| format!("failed to read index.json: {err}"))?;
    if contents.trim().is_empty() {
        return Ok(JobIndex { jobs: Vec::new() });
    }
    serde_json::from_str(&contents).map_err(|err| format!("invalid index.json: {err}"))
}

fn save_index_to_disk(path: &PathBuf, index: &JobIndex) -> Result<(), String> {
    let json = serde_json::to_string_pretty(index)
        .map_err(|err| format!("failed to serialize index.json: {err}"))?;
    let mut file =
        File::create(path).map_err(|err| format!("failed to write index.json: {err}"))?;
    file.write_all(json.as_bytes())
        .map_err(|err| format!("failed to save index.json: {err}"))
}

fn generate_job_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);
    let pid = std::process::id();
    format!("job_{now}_{pid}")
}

fn unix_timestamp_string() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now.to_string()
}

fn build_job_audio_path(jobs_dir: &PathBuf, job_id: &str, source_path: &str) -> Result<PathBuf, String> {
    let job_dir = jobs_dir.join(job_id);
    fs::create_dir_all(&job_dir)
        .map_err(|err| format!("failed to create job dir: {err}"))?;
    let ext = std::path::Path::new(source_path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let filename = if ext.is_empty() {
        "audio.original".to_string()
    } else {
        format!("audio.original.{ext}")
    };
    Ok(job_dir.join(filename))
}

fn job_dir_from_audio_path(audio_path: &str) -> Option<PathBuf> {
    std::path::Path::new(audio_path).parent().map(|p| p.to_path_buf())
}

fn write_stub_artifacts(job_dir: &PathBuf) -> Result<(String, String, String), String> {
    fs::create_dir_all(job_dir)
        .map_err(|err| format!("failed to create job dir: {err}"))?;
    let transcript_path = job_dir.join("transcript.txt");
    let segments_path = job_dir.join("segments.json");
    let srt_path = job_dir.join("transcript.srt");
    let transcript = "Stub transcript from Rust core.\n";
    let segments = r#"[{"start":0.0,"end":1.5,"text":"Stub segment one."},{"start":1.6,"end":3.2,"text":"Stub segment two."}]"#;
    fs::write(&transcript_path, transcript)
        .map_err(|err| format!("failed to write transcript.txt: {err}"))?;
    fs::write(&segments_path, segments)
        .map_err(|err| format!("failed to write segments.json: {err}"))?;
    fs::write(&srt_path, "")
        .map_err(|err| format!("failed to write transcript.srt: {err}"))?;
    Ok((
        transcript_path.to_string_lossy().to_string(),
        segments_path.to_string_lossy().to_string(),
        srt_path.to_string_lossy().to_string(),
    ))
}

fn read_transcript_text(path: &str) -> Result<String, String> {
    let content =
        fs::read_to_string(path).map_err(|err| format!("failed to read transcript: {err}"))?;
    Ok(content)
}

fn build_summary_prompt(template: &str, transcript: &str) -> String {
    if template.contains("{text}") {
        template.replace("{text}", transcript)
    } else {
        format!("{template}\n\n{text}\n", template = template, text = transcript)
    }
}

fn write_summary_file(job_dir: &PathBuf, content: &str) -> Result<String, String> {
    let summary_path = job_dir.join("summary.md");
    fs::write(&summary_path, content)
        .map_err(|err| format!("failed to write summary.md: {err}"))?;
    Ok(summary_path.to_string_lossy().to_string())
}

fn summarize_with_ollama(
    base_url: &str,
    model: &str,
    prompt: &str,
) -> Result<String, String> {
    let url = format!("{}/api/generate", base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|err| format!("Failed to build HTTP client: {err}"))?;
    let payload = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": false
    });
    let resp = client
        .post(&url)
        .json(&payload)
        .send()
        .map_err(|err| {
            if err.is_timeout() {
                format!("Ollama timeout after 120s at {url}")
            } else if err.is_connect() {
                format!("Ollama not reachable at {url}. Is Ollama running?")
            } else {
                format!("Ollama request failed: {err}")
            }
        })?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("Ollama error: {status} {body}"));
    }
    let json: serde_json::Value = resp
        .json()
        .map_err(|err| format!("Invalid Ollama response: {err}"))?;
    let response = json
        .get("response")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if response.trim().is_empty() {
        return Err("Ollama returned empty response.".to_string());
    }
    Ok(response)
}

fn parse_progress_from_line(line: &str) -> Option<f32> {
    for token in line.split_whitespace() {
        if let Some(stripped) = token.strip_suffix('%') {
            if let Ok(value) = stripped.parse::<f32>() {
                return Some(value.max(0.0).min(100.0));
            }
        }
    }
    None
}

fn update_job_and_emit<F>(app: &AppHandle, job_id: &str, mutator: F) -> Result<(), String>
where
    F: FnOnce(&mut Job),
{
    let index_state = app.state::<JobIndexState>();
    let mut guard = index_state
        .index
        .lock()
        .map_err(|_| "job index mutex poisoned".to_string())?;
    let mut snapshot: Option<Job> = None;
    if let Some(job) = guard.jobs.iter_mut().find(|job| job.id == job_id) {
        mutator(job);
        snapshot = Some(job.clone());
        save_index_to_disk(&index_state.path, &guard)?;
    }
    if let Some(job) = snapshot {
        emit_job_updated(app, &job);
    }
    Ok(())
}

fn append_job_log(app: &AppHandle, job_id: &str, line: &str) -> Result<(), String> {
    update_job_and_emit(app, job_id, |job| {
        push_log(job, line);
    })?;
    emit_job_log(app, job_id, line);
    Ok(())
}

fn is_macho_binary(path: &PathBuf) -> bool {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut buf = [0u8; 4];
    if file.read_exact(&mut buf).is_err() {
        return false;
    }
    let be = u32::from_be_bytes(buf);
    let le = u32::from_le_bytes(buf);
    matches!(
        be,
        0xFEEDFACE | 0xFEEDFACF | 0xCAFEBABE | 0xBEBAFECA | 0xCEFAEDFE | 0xCFFAEDFE
    ) || matches!(
        le,
        0xFEEDFACE | 0xFEEDFACF | 0xCAFEBABE | 0xBEBAFECA | 0xCEFAEDFE | 0xCFFAEDFE
    )
}

fn resolve_whisper_paths(app: &AppHandle, model_size: &str) -> Result<(PathBuf, PathBuf), String> {
    if let (Ok(bin), Ok(model)) = (
        std::env::var("VOICENOTE_WHISPER_PATH"),
        std::env::var("VOICENOTE_WHISPER_MODEL"),
    ) {
        let bin_path = PathBuf::from(bin);
        let model_path = PathBuf::from(model);
        if bin_path.exists() && model_path.exists() {
            return Ok((bin_path, model_path));
        }
    }

    let mut bin_candidates: Vec<PathBuf> = Vec::new();
    let mut model_candidates: Vec<PathBuf> = Vec::new();
    let model_name = format!("ggml-{model_size}.bin");

    bin_candidates.push(PathBuf::from("third_party/whisper/bin/whisper"));
    bin_candidates.push(PathBuf::from("third_party/whisper/bin/main"));

    model_candidates.push(PathBuf::from(format!(
        "third_party/whisper/models/{model_name}"
    )));

    if let Ok(cwd) = std::env::current_dir() {
        bin_candidates.push(cwd.join("third_party/whisper/bin/whisper"));
        bin_candidates.push(cwd.join("third_party/whisper/bin/main"));
        model_candidates.push(cwd.join(format!(
            "third_party/whisper/models/{model_name}"
        )));
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        bin_candidates.push(resource_dir.join("whisper/bin/whisper"));
        bin_candidates.push(resource_dir.join("whisper/bin/main"));
        model_candidates.push(resource_dir.join(format!("whisper/models/{model_name}")));
    }

    if let Ok(app_data_dir) = app.path().app_data_dir() {
        model_candidates.push(app_data_dir.join(format!("voicenote/models/{model_name}")));
        bin_candidates.push(app_data_dir.join("voicenote/whisper/bin/whisper"));
        bin_candidates.push(app_data_dir.join("voicenote/whisper/bin/main"));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            bin_candidates.push(dir.join("whisper"));
            bin_candidates.push(dir.join("main"));
            model_candidates.push(dir.join(format!(
                "../Resources/whisper/models/{model_name}"
            )));
        }
    }

    let bin = bin_candidates
        .into_iter()
        .find(|p| p.exists() && is_macho_binary(p));
    let model = model_candidates.into_iter().find(|p| p.exists());

    if let (Some(bin), Some(model)) = (bin, model) {
        return Ok((bin, model));
    }

    Err(
        "Whisper binary/model not found. Provide whisper.cpp at third_party/whisper/bin/whisper \
and model at third_party/whisper/models/ggml-<size>.bin, or set VOICENOTE_WHISPER_PATH \
and VOICENOTE_WHISPER_MODEL."
            .to_string(),
    )
}

fn model_filename(model_size: &str) -> Result<String, String> {
    let filename = match model_size {
        "tiny" => "ggml-tiny.bin",
        "base" => "ggml-base.bin",
        "small" => "ggml-small.bin",
        "medium" => "ggml-medium.bin",
        "large-v3" => "ggml-large-v3.bin",
        other => {
            return Err(format!(
                "Unknown model size: {other}. Expected tiny/base/small/medium/large-v3."
            ));
        }
    };
    Ok(filename.to_string())
}

fn model_url(model_size: &str) -> Result<String, String> {
    let filename = model_filename(model_size)?;
    Ok(format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{filename}?download=true"
    ))
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn whisper_binary_status_key() -> String {
    "whisper-binary".to_string()
}

fn ffmpeg_status_key() -> String {
    "ffmpeg".to_string()
}

fn github_repo_from_api(url: &str) -> Option<String> {
    let marker = "api.github.com/repos/";
    let idx = url.find(marker)?;
    let rest = &url[idx + marker.len()..];
    let mut parts = rest.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    Some(format!("https://github.com/{owner}/{repo}"))
}

fn is_macos_arm_asset(name: &str) -> bool {
    let name_lc = name.to_lowercase();
    let is_arm = name_lc.contains("arm64") || name_lc.contains("aarch64");
    let is_macos = name_lc.contains("macos")
        || name_lc.contains("osx")
        || name_lc.contains("darwin")
        || name_lc.contains("apple");
    let is_zip = name_lc.ends_with(".zip");
    is_arm && is_macos && is_zip
}

fn extract_latest_tag(html: &str) -> Option<String> {
    let needle = "/releases/tag/";
    let mut idx = 0usize;
    while let Some(pos) = html[idx..].find(needle) {
        let start = idx + pos + needle.len();
        let rest = &html[start..];
        let end = rest
            .find(['"', '\'', '?', '#', '<', ' '])
            .unwrap_or(rest.len());
        if end > 0 {
            return Some(rest[..end].to_string());
        }
        idx = start + end;
    }
    None
}

fn probe_download_url(client: &reqwest::blocking::Client, url: &str) -> bool {
    let resp = client
        .get(url)
        .header(reqwest::header::USER_AGENT, "voicenote")
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send();
    if let Ok(resp) = resp {
        let status = resp.status();
        return status.is_success() || status.is_redirection();
    }
    false
}

fn resolve_whisper_download_url(url: &str) -> Result<String, String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("Whisper download URL is empty.".to_string());
    }
    let url = trimmed.replace("http://", "https://");
    let normalized = if url.contains("github.com/") && url.contains("/releases") {
        let parts: Vec<&str> = url.split("github.com/").collect();
        if parts.len() == 2 {
            format!("https://api.github.com/repos/{}", parts[1])
                .replace("/releases/latest", "/releases/latest")
        } else {
            url.clone()
        }
    } else {
        url.clone()
    };
    let is_github_api = normalized.contains("api.github.com/repos/") && normalized.contains("/releases");
    if !is_github_api {
        return Ok(normalized);
    }
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(&normalized)
        .header(reqwest::header::USER_AGENT, "voicenote")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send();
    if let Ok(resp) = resp {
        if resp.status().is_success() {
            let json: serde_json::Value = resp
                .json()
                .map_err(|err| format!("Invalid GitHub response: {err}"))?;
            let assets = json
                .get("assets")
                .and_then(|v| v.as_array())
                .ok_or_else(|| "No assets in release.".to_string())?;
            for asset in assets {
                let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let url = asset
                    .get("browser_download_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if is_macos_arm_asset(name) {
                    return Ok(url.to_string());
                }
            }
        } else {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            if !normalized.ends_with("/releases/latest") {
                return Err(format!("GitHub API error: {status} {body}"));
            }
        }
    }

    let repo_url = if let Some(repo_url) = github_repo_from_api(&normalized) {
        repo_url
    } else {
        return Err("No macOS arm64 zip asset found in GitHub release.".to_string());
    };

    if !repo_url.is_empty() {
        let latest_url = format!("{}/releases/latest", repo_url);
        let resp = client
            .get(&latest_url)
            .header(reqwest::header::USER_AGENT, "voicenote")
            .send()
            .map_err(|err| format!("GitHub HTML request failed: {err}"))?;
        if resp.status().is_success() {
            let html = resp.text().unwrap_or_default();
            let tag = extract_latest_tag(&html);
            let mut best: Option<String> = None;
            let needle = "/releases/download/";
            let mut index = 0;
            while let Some(pos) = html[index..].find(needle) {
                let start = index + pos;
                let rest = &html[start..];
                if let Some(end) = rest.find(".zip") {
                    let end_pos = start + end + 4;
                    let url_path = &html[start..end_pos];
                    let url = format!("https://github.com{}", url_path);
                    if is_macos_arm_asset(&url) {
                        best = Some(url);
                        break;
                    }
                    if best.is_none() {
                        best = Some(url);
                    }
                    index = end_pos;
                } else {
                    break;
                }
            }
            if let Some(url) = best {
                return Ok(url);
            }

            if let Some(tag) = tag {
                let version = tag.trim_start_matches('v');
                let candidates = [
                    format!("whisper-cpp-{version}-macos-arm64-metal.zip"),
                    format!("whisper-cpp-v{version}-macos-arm64-metal.zip"),
                    format!("whisper-cpp-{version}-macos-arm64-accelerate.zip"),
                    format!("whisper-cpp-v{version}-macos-arm64-accelerate.zip"),
                    format!("whisper-cpp-{version}-macos-arm64.zip"),
                    format!("whisper-cpp-v{version}-macos-arm64.zip"),
                    "whisper-cpp-macos-arm64-metal.zip".to_string(),
                    "whisper-cpp-macos-arm64.zip".to_string(),
                ];
                for name in candidates {
                    let candidate = format!(
                        "{}/releases/download/{}/{}",
                        repo_url, tag, name
                    );
                    if probe_download_url(&client, &candidate) {
                        return Ok(candidate);
                    }
                }
            }
        }
    }

    Err(
        "No macOS arm64 zip asset found in GitHub release. Paste a direct .zip asset URL from the release."
            .to_string(),
    )
}

fn download_to_file(url: &str, dest: &PathBuf, status: &mut ModelDownloadStatus, status_map: &Arc<Mutex<HashMap<String, ModelDownloadStatus>>>) -> Result<(), String> {
    let client = reqwest::blocking::Client::new();
    let mut resp = client
        .get(url)
        .header(reqwest::header::USER_AGENT, "voicenote")
        .send()
        .map_err(|err| format!("Request failed: {err}"))?;
    if !resp.status().is_success() {
        let status_code = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("Download failed ({status_code}): {body}"));
    }
    if let Some(len) = resp.headers().get(reqwest::header::CONTENT_LENGTH) {
        if let Ok(len) = len.to_str() {
            if let Ok(bytes) = len.parse::<u64>() {
                status.total_bytes = bytes;
                let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
                guard.insert(status.model_size.clone(), status.clone());
            }
        }
    }
    let mut file = File::create(dest)
        .map_err(|err| format!("Failed to create file: {err}"))?;
    let mut downloaded = 0u64;
    let mut buffer = [0u8; 1024 * 64];
    loop {
        let read = match resp.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) => return Err(format!("Download error: {err}")),
        };
        file.write_all(&buffer[..read])
            .map_err(|err| format!("Write error: {err}"))?;
        downloaded += read as u64;
        status.downloaded_bytes = downloaded;
        let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(status.model_size.clone(), status.clone());
    }
    Ok(())
}

fn extract_whisper_zip(zip_path: &PathBuf, dest_path: &PathBuf) -> Result<(), String> {
    let file = File::open(zip_path)
        .map_err(|err| format!("Failed to open zip: {err}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("Invalid zip: {err}"))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|err| format!("Zip entry error: {err}"))?;
        let name = entry.name().to_string();
        if name.ends_with("/whisper")
            || name.ends_with("/main")
            || name.ends_with("/whisper-cli")
            || name == "whisper"
            || name == "main"
            || name == "whisper-cli"
        {
            let mut out = File::create(dest_path)
                .map_err(|err| format!("Failed to create binary: {err}"))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|err| format!("Failed to extract binary: {err}"))?;
            return Ok(());
        }
    }
    Err("Whisper binary not found in zip.".to_string())
}

fn extract_ffmpeg_zip(zip_path: &PathBuf, dest_dir: &PathBuf) -> Result<(), String> {
    let file = File::open(zip_path)
        .map_err(|err| format!("Failed to open zip: {err}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("Invalid zip: {err}"))?;
    let mut found_ffmpeg = false;
    let mut found_ffprobe = false;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|err| format!("Zip entry error: {err}"))?;
        let name = entry.name().to_string();
        if name.ends_with("/ffmpeg") || name == "ffmpeg" {
            let out_path = dest_dir.join("bin/ffmpeg");
            fs::create_dir_all(out_path.parent().unwrap())
                .map_err(|err| format!("Failed to create ffmpeg dir: {err}"))?;
            let mut out = File::create(&out_path)
                .map_err(|err| format!("Failed to create ffmpeg binary: {err}"))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|err| format!("Failed to extract ffmpeg: {err}"))?;
            found_ffmpeg = true;
        }
        if name.ends_with("/ffprobe") || name == "ffprobe" {
            let out_path = dest_dir.join("bin/ffprobe");
            fs::create_dir_all(out_path.parent().unwrap())
                .map_err(|err| format!("Failed to create ffprobe dir: {err}"))?;
            let mut out = File::create(&out_path)
                .map_err(|err| format!("Failed to create ffprobe binary: {err}"))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|err| format!("Failed to extract ffprobe: {err}"))?;
            found_ffprobe = true;
        }
    }
    if !found_ffmpeg {
        return Err("ffmpeg binary not found in zip.".to_string());
    }
    if !found_ffprobe {
        return Err("ffprobe binary not found in zip.".to_string());
    }
    Ok(())
}

fn convert_to_wav(ffmpeg_path: &PathBuf, input: &str, output: &PathBuf) -> Result<(), String> {
    let status = Command::new(ffmpeg_path)
        .args([
            "-y",
            "-i",
            input,
            "-vn",
            "-acodec",
            "pcm_s16le",
            "-ar",
            "16000",
            "-ac",
            "1",
            output.to_str().unwrap_or_default(),
        ])
        .status()
        .map_err(|err| format!("failed to run ffmpeg: {err}"))?;
    if !status.success() {
        return Err("ffmpeg convert failed".to_string());
    }
    Ok(())
}

fn run_whisper_cpp(
    app: &AppHandle,
    job_id: &str,
    bin: &PathBuf,
    model: &PathBuf,
    audio_path: &PathBuf,
    output_base: &PathBuf,
    language: Option<&str>,
) -> Result<(), String> {
    let mut args = vec![
        "-m".to_string(),
        model.to_str().unwrap_or_default().to_string(),
        "-f".to_string(),
        audio_path.to_str().unwrap_or_default().to_string(),
        "-oj".to_string(),
        "-osrt".to_string(),
        "-otxt".to_string(),
        "-of".to_string(),
        output_base.to_str().unwrap_or_default().to_string(),
    ];
    if let Some(lang) = language {
        let trimmed = lang.trim();
        if !trimmed.is_empty() {
            args.push("-l".to_string());
            args.push(trimmed.to_string());
        }
    }
    let mut child = Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to run whisper: {err}"))?;

    if let Some(stderr) = child.stderr.take() {
        let app_handle = app.clone();
        let job_id = job_id.to_string();
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                let _ = append_job_log(&app_handle, &job_id, &line);
            }
        });
    }

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().flatten() {
            if let Some(progress) = parse_progress_from_line(&line) {
                let mapped = 0.3 + (progress / 100.0) * 0.6;
                let _ = update_job_and_emit(app, job_id, |job| {
                    job.stage = "transcribe".to_string();
                    job.progress = mapped;
                });
            }
            let _ = append_job_log(app, job_id, &line);
        }
    }

    let status = child
        .wait()
        .map_err(|err| format!("failed to wait for whisper: {err}"))?;
    if !status.success() {
        return Err("whisper failed".to_string());
    }
    Ok(())
}

fn ensure_clip(
    ffmpeg_path: &PathBuf,
    audio_path: &str,
    job_dir: &PathBuf,
    start: f64,
    end: f64,
) -> Result<String, String> {
    // We try to create a real clipped file using ffmpeg if available.
    // If ffmpeg is not present, we fall back to the full audio file.
    let clips_dir = job_dir.join("clips");
    fs::create_dir_all(&clips_dir)
        .map_err(|err| format!("failed to create clips dir: {err}"))?;
    let start_ms = (start * 1000.0).max(0.0).round() as u64;
    let end_ms = (end * 1000.0).max(0.0).round() as u64;
    let clip_name = format!("clip_{start_ms}_{end_ms}.wav");
    let clip_path = clips_dir.join(clip_name);
    if clip_path.exists() {
        return Ok(clip_path.to_string_lossy().to_string());
    }

    let status = Command::new(ffmpeg_path)
        .args([
            "-y",
            "-i",
            audio_path,
            "-ss",
            &start.to_string(),
            "-to",
            &end.to_string(),
            "-vn",
            "-acodec",
            "pcm_s16le",
            "-ar",
            "16000",
            "-ac",
            "1",
            clip_path.to_str().unwrap_or_default(),
        ])
        .status()
        .map_err(|err| format!("failed to run ffmpeg: {err}"))?;

    if !status.success() {
        return Ok(audio_path.to_string());
    }

    Ok(clip_path.to_string_lossy().to_string())
}

fn resolve_ffmpeg_path(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(explicit) = std::env::var("VOICENOTE_FFMPEG_PATH") {
        let path = PathBuf::from(explicit);
        if path.exists() {
            return ensure_lgpl_ffmpeg(path);
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("third_party/ffmpeg/bin/ffmpeg"));
        let mut cursor = Some(cwd.as_path());
        for _ in 0..4 {
            if let Some(dir) = cursor {
                candidates.push(dir.join("third_party/ffmpeg/bin/ffmpeg"));
                cursor = dir.parent();
            }
        }
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("ffmpeg/bin/ffmpeg"));
        candidates.push(resource_dir.join("resources/ffmpeg/bin/ffmpeg"));
        candidates.push(resource_dir.join("third_party/ffmpeg/bin/ffmpeg"));
    }
    if let Ok(app_data_dir) = app.path().app_data_dir() {
        candidates.push(app_data_dir.join("voicenote/ffmpeg/bin/ffmpeg"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("../Resources/ffmpeg/bin/ffmpeg"));
            candidates.push(dir.join("../Resources/resources/ffmpeg/bin/ffmpeg"));
            candidates.push(dir.join("../Resources/third_party/ffmpeg/bin/ffmpeg"));
        }
    }

    for candidate in candidates {
        if candidate.exists() {
            return ensure_lgpl_ffmpeg(candidate);
        }
    }

    Err(
        "FFmpeg not found. Provide an LGPL build at ./third_party/ffmpeg/bin/ffmpeg \
(see scripts/ffmpeg/build_macos_lgpl.sh) or set VOICENOTE_FFMPEG_PATH."
            .to_string(),
    )
}

fn ensure_lgpl_ffmpeg(path: PathBuf) -> Result<PathBuf, String> {
    let output = Command::new(&path)
        .arg("-version")
        .output()
        .map_err(|err| format!("Failed to run ffmpeg: {err}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    if text.contains("--enable-gpl") || text.contains("--enable-nonfree") {
        return Err("FFmpeg build contains GPL/nonfree flags; please use LGPL build.".to_string());
    }
    Ok(path)
}

fn process_job(app: &AppHandle, job_id: &str) -> Result<(), String> {
    let index_state = app.state::<JobIndexState>();
    let config_state = app.state::<ConfigState>();
    let (model_size, language, enable_summarization, auto_summarize, ollama_base, ollama_model, summary_prompt) = {
        let guard = config_state
            .config
            .lock()
            .map_err(|_| "config mutex poisoned".to_string())?;
        (
            guard.model_size.clone(),
            guard.language.clone(),
            guard.enable_summarization,
            guard.auto_summarize_after_transcription,
            guard.ollama_base_url.clone(),
            guard.ollama_model.clone(),
            guard.summary_prompt.clone(),
        )
    };
    let mut job_snapshot: Option<Job> = None;
    let mut job_dir: Option<PathBuf> = None;
    let mut audio_path: Option<String> = None;
    {
        let mut guard = index_state
            .index
            .lock()
            .map_err(|_| "job index mutex poisoned".to_string())?;
        if let Some(job) = guard.jobs.iter_mut().find(|job| job.id == job_id) {
            job.status = "running".to_string();
            job.stage = "convert".to_string();
            job.progress = 0.1;
            push_log(job, "Worker started.");
            job_snapshot = Some(job.clone());
            job_dir = job_dir_from_audio_path(&job.audio_path);
            audio_path = Some(job.audio_path.clone());
        }
        if job_snapshot.is_some() {
            save_index_to_disk(&index_state.path, &guard)?;
        }
    }
    if let Some(job) = job_snapshot.as_ref() {
        emit_job_updated(app, job);
        emit_job_log(app, &job.id, "Worker started.");
    }

    let mark_error = |message: &str| -> Result<(), String> {
        update_job_and_emit(app, job_id, |job| {
            job.status = "error".to_string();
            job.stage = "error".to_string();
            push_log(job, message);
        })?;
        emit_job_log(app, job_id, message);
        Ok(())
    };

    let job_dir = job_dir.ok_or_else(|| "missing job directory".to_string())?;
    let audio_path = audio_path.ok_or_else(|| "missing audio path".to_string())?;

    let ffmpeg_path = match resolve_ffmpeg_path(app) {
        Ok(path) => path,
        Err(err) => {
            mark_error(&err)?;
            return Ok(());
        }
    };
    let wav_path = job_dir.join("audio.wav");
    if !wav_path.exists() {
        emit_job_log(app, job_id, "Converting audio to 16k mono WAV...");
        if let Err(err) = convert_to_wav(&ffmpeg_path, &audio_path, &wav_path) {
            mark_error(&err)?;
            return Ok(());
        }
    }

    let _ = update_job_and_emit(app, job_id, |job| {
        job.stage = "transcribe".to_string();
        job.progress = 0.3;
    });

    emit_job_log(app, job_id, "Running whisper.cpp...");
    let (whisper_bin, whisper_model) = match resolve_whisper_paths(app, &model_size) {
        Ok(paths) => paths,
        Err(err) => {
            mark_error(&err)?;
            return Ok(());
        }
    };
    let output_base = job_dir.join("whisper");
    if let Err(err) = run_whisper_cpp(
        app,
        job_id,
        &whisper_bin,
        &whisper_model,
        &wav_path,
        &output_base,
        language.as_deref(),
    ) {
        mark_error(&err)?;
        return Ok(());
    }

    let transcript_txt_path = output_base
        .with_extension("txt")
        .to_string_lossy()
        .to_string();
    let transcript_json_path = output_base
        .with_extension("json")
        .to_string_lossy()
        .to_string();
    let transcript_srt_path = output_base
        .with_extension("srt")
        .to_string_lossy()
        .to_string();

    if !std::path::Path::new(&transcript_txt_path).exists()
        || !std::path::Path::new(&transcript_json_path).exists()
    {
        emit_job_log(app, job_id, "Whisper output missing; falling back to stub.");
        let (txt, json, srt) = write_stub_artifacts(&job_dir)?;
        let mut completed_snapshot: Option<Job> = None;
        {
            let mut guard = index_state
                .index
                .lock()
                .map_err(|_| "job index mutex poisoned".to_string())?;
            if let Some(job) = guard.jobs.iter_mut().find(|job| job.id == job_id) {
                job.progress = 1.0;
                job.status = "done".to_string();
                job.stage = "done".to_string();
                job.transcript_txt_path = txt;
                job.transcript_json_path = json;
                job.transcript_srt_path = srt;
                job.md_preview = Some("Stub transcript from Rust core.".to_string());
                job.summary_status = Some("skipped".to_string());
                push_log(job, "Worker finished (stub).");
                completed_snapshot = Some(job.clone());
            }
            if completed_snapshot.is_some() {
                save_index_to_disk(&index_state.path, &guard)?;
            }
        }
        if let Some(job) = completed_snapshot {
            emit_job_updated(app, &job);
            emit_job_log(app, &job.id, "Worker finished (stub).");
        }
        return Ok(());
    }

    let mut completed_snapshot: Option<Job> = None;
    {
        let mut guard = index_state
            .index
            .lock()
            .map_err(|_| "job index mutex poisoned".to_string())?;
        if let Some(job) = guard.jobs.iter_mut().find(|job| job.id == job_id) {
            job.progress = 1.0;
            job.status = "done".to_string();
            job.stage = "done".to_string();
            job.transcript_txt_path = transcript_txt_path;
            job.transcript_json_path = transcript_json_path;
            job.transcript_srt_path = transcript_srt_path;
            job.md_preview = Some("Transcript ready.".to_string());
            job.summary_status = Some(
                if enable_summarization {
                    "not_started"
                } else {
                    "skipped"
                }
                .to_string(),
            );
            push_log(job, "Whisper finished.");
            completed_snapshot = Some(job.clone());
        }
        if completed_snapshot.is_some() {
            save_index_to_disk(&index_state.path, &guard)?;
        }
    }

    if let Some(job) = completed_snapshot {
        emit_job_updated(app, &job);
        emit_job_log(app, &job.id, "Whisper finished.");
    }

    if enable_summarization && auto_summarize {
        emit_job_log(app, job_id, "Summarization queued.");
        let app_handle = app.clone();
        let job_id = job_id.to_string();
        let base_url = ollama_base.clone();
        let model = ollama_model.clone();
        let prompt = summary_prompt.clone();
        thread::spawn(move || {
            let _ = summarize_job_internal(
                &app_handle,
                &job_id,
                &base_url,
                &model,
                &prompt,
                false,
            );
        });
    } else {
        emit_job_log(app, job_id, "Summarization skipped.");
    }

    Ok(())
}

pub fn spawn_worker(app: &AppHandle) -> JobQueueState {
    let (sender, receiver) = mpsc::channel::<String>();
    let handle = app.clone();
    thread::spawn(move || {
        for job_id in receiver {
            if let Err(err) = process_job(&handle, &job_id) {
                let _ = handle.emit("job:log", JobLogEvent {
                    id: job_id.clone(),
                    line: format!("Worker error: {err}"),
                });
            }
        }
    });
    JobQueueState::new(sender)
}

#[tauri::command]
pub fn get_health() -> bool {
    // Tauri is running; no external backend is required in this mode.
    true
}

#[tauri::command]
pub fn get_config(state: State<ConfigState>) -> Result<AppConfig, String> {
    let guard = state
        .config
        .lock()
        .map_err(|_| "config mutex poisoned".to_string())?;
    Ok(guard.clone())
}

#[tauri::command]
pub fn update_config(state: State<ConfigState>, cfg: AppConfig) -> Result<AppConfig, String> {
    let mut guard = state
        .config
        .lock()
        .map_err(|_| "config mutex poisoned".to_string())?;
    *guard = cfg;
    save_config_to_disk(&state.path, &guard)?;
    Ok(guard.clone())
}

#[tauri::command]
pub fn initialize_config(state: State<ConfigState>, mut cfg: AppConfig) -> Result<AppConfig, String> {
    cfg.initialized = true;
    let mut guard = state
        .config
        .lock()
        .map_err(|_| "config mutex poisoned".to_string())?;
    *guard = cfg;
    save_config_to_disk(&state.path, &guard)?;
    Ok(guard.clone())
}

#[tauri::command]
pub fn get_config_initialized(state: State<ConfigState>) -> Result<bool, String> {
    let guard = state
        .config
        .lock()
        .map_err(|_| "config mutex poisoned".to_string())?;
    Ok(guard.initialized)
}

#[tauri::command]
pub fn list_jobs(state: State<JobIndexState>) -> Result<Vec<Job>, String> {
    let guard = state
        .index
        .lock()
        .map_err(|_| "job index mutex poisoned".to_string())?;
    Ok(guard.jobs.clone())
}

#[tauri::command]
pub fn get_job(state: State<JobIndexState>, id: String) -> Result<Job, String> {
    let guard = state
        .index
        .lock()
        .map_err(|_| "job index mutex poisoned".to_string())?;
    guard
        .jobs
        .iter()
        .find(|job| job.id == id)
        .cloned()
        .ok_or_else(|| "job not found".to_string())
}

#[tauri::command]
fn create_job_from_path_inner(
    app: &AppHandle,
    state: &JobIndexState,
    path: String,
) -> Result<Job, String> {
    let filename = std::path::Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown-audio");
    let job_id = generate_job_id();
    let dest_path = build_job_audio_path(&state.jobs_dir, &job_id, &path)?;
    fs::copy(&path, &dest_path)
        .map_err(|err| format!("failed to copy audio into job folder: {err}"))?;
    let mut job = Job {
        id: job_id,
        filename: filename.to_string(),
        status: "queued".to_string(),
        progress: 0.0,
        stage: "import".to_string(),
        logs: Vec::new(),
        created_at: unix_timestamp_string(),
        audio_path: dest_path.to_string_lossy().to_string(),
        transcript_txt_path: String::new(),
        transcript_json_path: String::new(),
        transcript_srt_path: String::new(),
        md_preview: None,
        summary_status: Some("not_started".to_string()),
        summary_model: None,
        summary_error: None,
        summary_md: None,
        exported_to_obsidian: false,
    };
    push_log(&mut job, "Queued for processing.");
    let mut guard = state
        .index
        .lock()
        .map_err(|_| "job index mutex poisoned".to_string())?;
    guard.jobs.insert(0, job.clone());
    save_index_to_disk(&state.path, &guard)?;
    emit_job_updated(app, &job);
    emit_job_log(app, &job.id, "Queued for processing.");
    Ok(job)
}

#[tauri::command]
pub fn create_job_from_path(
    app: AppHandle,
    state: State<JobIndexState>,
    queue: State<JobQueueState>,
    path: String,
) -> Result<Job, String> {
    let job = create_job_from_path_inner(&app, state.inner(), path)?;
    queue.enqueue(job.id.clone())?;
    Ok(job)
}

#[tauri::command]
pub fn add_files(
    app: AppHandle,
    state: State<JobIndexState>,
    queue: State<JobQueueState>,
    paths: Vec<String>,
) -> Result<Vec<Job>, String> {
    let mut created = Vec::new();
    for path in paths {
        let job = create_job_from_path_inner(&app, state.inner(), path)?;
        queue.enqueue(job.id.clone())?;
        created.push(job);
    }
    Ok(created)
}

#[tauri::command]
pub fn cancel_job(
    app: AppHandle,
    state: State<JobIndexState>,
    id: String,
) -> Result<bool, String> {
    let mut guard = state
        .index
        .lock()
        .map_err(|_| "job index mutex poisoned".to_string())?;
    let mut updated_job: Option<Job> = None;
    if let Some(job) = guard.jobs.iter_mut().find(|job| job.id == id) {
        job.status = "cancelled".to_string();
        job.stage = "cancelled".to_string();
        push_log(job, "Job cancelled.");
        updated_job = Some(job.clone());
    }
    if updated_job.is_none() {
        return Ok(false);
    }
    save_index_to_disk(&state.path, &guard)?;
    if let Some(job) = updated_job {
        emit_job_updated(&app, &job);
        emit_job_log(&app, &job.id, "Job cancelled.");
    }
    Ok(true)
}

#[tauri::command]
pub fn delete_job(state: State<JobIndexState>, id: String) -> Result<bool, String> {
    let mut guard = state
        .index
        .lock()
        .map_err(|_| "job index mutex poisoned".to_string())?;
    let before = guard.jobs.len();
    guard.jobs.retain(|job| job.id != id);
    if guard.jobs.len() != before {
        save_index_to_disk(&state.path, &guard)?;
        return Ok(true);
    }
    Ok(false)
}

#[tauri::command]
pub fn export_to_obsidian(_id: String) -> bool {
    true
}

#[tauri::command]
pub fn get_segments(state: State<JobIndexState>, id: String) -> Result<Vec<Segment>, String> {
    let guard = state
        .index
        .lock()
        .map_err(|_| "job index mutex poisoned".to_string())?;
    let job = guard
        .jobs
        .iter()
        .find(|job| job.id == id)
        .cloned()
        .ok_or_else(|| "job not found".to_string())?;
    if job.transcript_json_path.is_empty() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(&job.transcript_json_path)
        .map_err(|err| format!("failed to read transcript json: {err}"))?;

    if let Ok(segments) = serde_json::from_str::<Vec<Segment>>(&contents) {
        return Ok(segments);
    }

    let value: serde_json::Value = serde_json::from_str(&contents)
        .map_err(|err| format!("invalid transcript json: {err}"))?;
    if let Some(segments_val) = value.get("segments").and_then(|v| v.as_array()) {
        let mut segments = Vec::new();
        for seg in segments_val {
            let text = seg
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let start = seg
                .get("start")
                .and_then(|v| v.as_f64())
                .or_else(|| seg.get("t0").and_then(|v| v.as_f64()).map(|v| v / 100.0))
                .unwrap_or(0.0);
            let end = seg
                .get("end")
                .and_then(|v| v.as_f64())
                .or_else(|| seg.get("t1").and_then(|v| v.as_f64()).map(|v| v / 100.0))
                .unwrap_or(start);
            if !text.is_empty() {
                segments.push(Segment {
                    start: start as f32,
                    end: end as f32,
                    text,
                });
            }
        }
        return Ok(segments);
    }

    if let Some(transcription_val) = value.get("transcription").and_then(|v| v.as_array()) {
        let mut segments = Vec::new();
        for seg in transcription_val {
            let text = seg
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let offsets = seg.get("offsets").and_then(|v| v.as_object());
            let start_ms = offsets
                .and_then(|o| o.get("from"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let end_ms = offsets
                .and_then(|o| o.get("to"))
                .and_then(|v| v.as_f64())
                .unwrap_or(start_ms);
            if !text.is_empty() {
                segments.push(Segment {
                    start: (start_ms / 1000.0) as f32,
                    end: (end_ms / 1000.0) as f32,
                    text,
                });
            }
        }
        return Ok(segments);
    }

    Err("segments not found in transcript json".to_string())
}

#[tauri::command]
pub fn get_clip_path(
    app: AppHandle,
    state: State<JobIndexState>,
    id: String,
    start: f64,
    end: f64,
) -> Result<String, String> {
    let guard = state
        .index
        .lock()
        .map_err(|_| "job index mutex poisoned".to_string())?;
    let job = guard
        .jobs
        .iter()
        .find(|job| job.id == id)
        .cloned()
        .ok_or_else(|| "job not found".to_string())?;
    let job_dir = job_dir_from_audio_path(&job.audio_path)
        .ok_or_else(|| "missing job directory".to_string())?;
    let ffmpeg_path = match resolve_ffmpeg_path(&app) {
        Ok(path) => path,
        Err(message) => {
            emit_job_log(&app, &id, &message);
            return Ok(job.audio_path);
        }
    };
    ensure_clip(&ffmpeg_path, &job.audio_path, &job_dir, start, end)
}

#[tauri::command]
pub fn get_summary(state: State<JobIndexState>, id: String) -> Result<SummaryResponse, String> {
    let guard = state
        .index
        .lock()
        .map_err(|_| "job index mutex poisoned".to_string())?;
    let job = guard
        .jobs
        .iter()
        .find(|job| job.id == id)
        .cloned()
        .ok_or_else(|| "job not found".to_string())?;

    if let Some(summary) = job.summary_md.clone() {
        if !summary.trim().is_empty() {
            return Ok(SummaryResponse {
                summary_status: job.summary_status.unwrap_or_else(|| "done".to_string()),
                summary_model: job.summary_model.unwrap_or_else(|| "".to_string()),
                summary_error: job.summary_error,
                summary_md: summary,
            });
        }
    }

    let job_dir = job_dir_from_audio_path(&job.audio_path)
        .ok_or_else(|| "missing job directory".to_string())?;
    let summary_path = job_dir.join("summary.md");
    if summary_path.exists() {
        let content = fs::read_to_string(&summary_path)
            .map_err(|err| format!("failed to read summary.md: {err}"))?;
        return Ok(SummaryResponse {
            summary_status: job.summary_status.unwrap_or_else(|| "done".to_string()),
            summary_model: job.summary_model.unwrap_or_else(|| "".to_string()),
            summary_error: job.summary_error,
            summary_md: content,
        });
    }

    Ok(SummaryResponse {
        summary_status: job.summary_status.unwrap_or_else(|| "not_started".to_string()),
        summary_model: job.summary_model.unwrap_or_else(|| "".to_string()),
        summary_error: job.summary_error,
        summary_md: "".to_string(),
    })
}

#[tauri::command]
pub fn summarize_job(app: AppHandle, id: String) -> Result<SummaryResponse, String> {
    let config_state = app.state::<ConfigState>();
    let (enable, base_url, model, prompt) = {
        let guard = config_state
            .config
            .lock()
            .map_err(|_| "config mutex poisoned".to_string())?;
        (
            guard.enable_summarization,
            guard.ollama_base_url.clone(),
            guard.ollama_model.clone(),
            guard.summary_prompt.clone(),
        )
    };
    if !enable {
        return Ok(SummaryResponse {
            summary_status: "skipped".to_string(),
            summary_model: model,
            summary_error: None,
            summary_md: "".to_string(),
        });
    }
    let index_state = app.state::<JobIndexState>();
    if let Ok(guard) = index_state.index.lock() {
        if let Some(job) = guard.jobs.iter().find(|job| job.id == id) {
            let status = job.summary_status.clone().unwrap_or_else(|| "not_started".to_string());
            if status == "running" {
                return Ok(SummaryResponse {
                    summary_status: status,
                    summary_model: job.summary_model.clone().unwrap_or_else(|| model.clone()),
                    summary_error: job.summary_error.clone(),
                    summary_md: job.summary_md.clone().unwrap_or_default(),
                });
            }
        }
    }

    update_job_and_emit(&app, &id, |job| {
        job.summary_status = Some("running".to_string());
        job.summary_model = Some(model.clone());
        job.summary_error = None;
    })?;
    emit_job_log(&app, &id, "Summarization started.");

    let app_handle = app.clone();
    let id_clone = id.clone();
    let base_url_clone = base_url.clone();
    let model_clone = model.clone();
    let prompt_clone = prompt.clone();
    thread::spawn(move || {
        let _ = summarize_job_internal(
            &app_handle,
            &id_clone,
            &base_url_clone,
            &model_clone,
            &prompt_clone,
            true,
        );
    });

    Ok(SummaryResponse {
        summary_status: "running".to_string(),
        summary_model: model,
        summary_error: None,
        summary_md: "".to_string(),
    })
}

fn summarize_job_internal(
    app: &AppHandle,
    job_id: &str,
    base_url: &str,
    model: &str,
    prompt_template: &str,
    force: bool,
) -> Result<SummaryResponse, String> {
    let index_state = app.state::<JobIndexState>();
    let mut transcript_path: Option<String> = None;
    let mut job_dir: Option<PathBuf> = None;

    if !force {
        let guard = index_state
            .index
            .lock()
            .map_err(|_| "job index mutex poisoned".to_string())?;
        if let Some(job) = guard.jobs.iter().find(|job| job.id == job_id) {
            let status = job.summary_status.clone().unwrap_or_else(|| "not_started".to_string());
            if status == "running" {
                return Ok(SummaryResponse {
                    summary_status: status,
                    summary_model: job.summary_model.clone().unwrap_or_else(|| model.to_string()),
                    summary_error: job.summary_error.clone(),
                    summary_md: job.summary_md.clone().unwrap_or_default(),
                });
            }
            if status == "done" {
                if let Some(summary) = job.summary_md.clone() {
                    if !summary.trim().is_empty() {
                        return Ok(SummaryResponse {
                            summary_status: status,
                            summary_model: job.summary_model.clone().unwrap_or_else(|| model.to_string()),
                            summary_error: job.summary_error.clone(),
                            summary_md: summary,
                        });
                    }
                }
            }
        }
    }

    update_job_and_emit(app, job_id, |job| {
        job.summary_status = Some("running".to_string());
        job.summary_model = Some(model.to_string());
    })?;
    emit_job_log(app, job_id, "Summarization started.");

    {
        let guard = index_state
            .index
            .lock()
            .map_err(|_| "job index mutex poisoned".to_string())?;
        if let Some(job) = guard.jobs.iter().find(|job| job.id == job_id) {
            transcript_path = Some(job.transcript_txt_path.clone());
            job_dir = job_dir_from_audio_path(&job.audio_path);
        }
    }

    let transcript_path = transcript_path.ok_or_else(|| "Transcript not found.".to_string())?;
    if transcript_path.is_empty() {
        return Err("Transcript path missing.".to_string());
    }
    let job_dir = job_dir.ok_or_else(|| "Job directory missing.".to_string())?;
    let result = (|| -> Result<String, String> {
        let transcript = read_transcript_text(&transcript_path)?;
        let prompt = build_summary_prompt(prompt_template, &transcript);
        let summary = summarize_with_ollama(base_url, model, &prompt)?;
        let _summary_path = write_summary_file(&job_dir, &summary)?;
        Ok(summary)
    })();

    match result {
        Ok(summary) => {
            update_job_and_emit(app, job_id, |job| {
                job.summary_status = Some("done".to_string());
                job.summary_md = Some(summary.clone());
                job.summary_error = None;
                job.summary_model = Some(model.to_string());
                job.md_preview = Some(summary.clone());
            })?;
            emit_job_log(app, job_id, "Summarization finished.");
            Ok(SummaryResponse {
                summary_status: "done".to_string(),
                summary_model: model.to_string(),
                summary_error: None,
                summary_md: summary,
            })
        }
        Err(err) => {
            update_job_and_emit(app, job_id, |job| {
                job.summary_status = Some("error".to_string());
                job.summary_error = Some(err.clone());
                job.summary_model = Some(model.to_string());
            })?;
            emit_job_log(app, job_id, &format!("Summarization failed: {err}"));
            Err(err)
        }
    }
}

#[tauri::command]
pub fn get_model_size(model_size: String) -> u64 {
    let url = match model_url(&model_size) {
        Ok(url) => url,
        Err(_) => return 0,
    };
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(_) => return 0,
    };
    if let Ok(resp) = client.head(url).send() {
        if let Some(len) = resp.headers().get(reqwest::header::CONTENT_LENGTH) {
            if let Ok(len) = len.to_str() {
                if let Ok(bytes) = len.parse::<u64>() {
                    return bytes;
                }
            }
        }
    }
    0
}

#[tauri::command]
pub fn get_model_download_status(
    state: State<ModelDownloadState>,
    model_size: String,
) -> ModelDownloadStatus {
    let guard = state.statuses.lock().ok();
    if let Some(guard) = guard {
        if let Some(status) = guard.get(&model_size) {
            return status.clone();
        }
    }
    ModelDownloadStatus {
        state: "idle".to_string(),
        model_size,
        repo_id: "whisper.cpp".to_string(),
        total_bytes: 0,
        downloaded_bytes: 0,
        message: None,
        started_at: None,
        finished_at: None,
    }
}

#[tauri::command]
pub fn get_model_installed(state: State<ModelDownloadState>, model_size: String) -> bool {
    if let Ok(filename) = model_filename(&model_size) {
        let in_app_data = state.models_dir.join(&filename);
        if in_app_data.exists() {
            return true;
        }
        let in_third_party = PathBuf::from("third_party/whisper/models").join(&filename);
        if in_third_party.exists() {
            return true;
        }
        if let Ok(cwd) = std::env::current_dir() {
            if cwd.join("third_party/whisper/models").join(&filename).exists() {
                return true;
            }
        }
        if let Some(resource_dir) = state
            .models_dir
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("Resources/whisper/models").join(&filename))
        {
            if resource_dir.exists() {
                return true;
            }
        }
    }
    false
}

#[tauri::command]
pub fn start_model_download(
    state: State<ModelDownloadState>,
    model_size: String,
) -> Result<ModelDownloadStatus, String> {
    let filename = model_filename(&model_size)?;
    let url = model_url(&model_size)?;
    let dest_path = state.models_dir.join(&filename);
    let tmp_path = state.models_dir.join(format!("{filename}.part"));

    let mut guard = state
        .statuses
        .lock()
        .map_err(|_| "model download mutex poisoned".to_string())?;
    if let Some(existing) = guard.get(&model_size) {
        if existing.state == "downloading" {
            return Ok(existing.clone());
        }
    }
    let status = ModelDownloadStatus {
        state: "downloading".to_string(),
        model_size: model_size.clone(),
        repo_id: "whisper.cpp".to_string(),
        total_bytes: 0,
        downloaded_bytes: 0,
        message: Some(format!("Downloading {filename}")),
        started_at: Some(now_ts()),
        finished_at: None,
    };
    guard.insert(model_size.clone(), status.clone());
    drop(guard);

    let status_map = Arc::clone(&state.inner().statuses);
    let status_for_thread = status.clone();
    thread::spawn(move || {
        let mut result_status = status_for_thread.clone();
        let download_result =
            download_to_file(&url, &tmp_path, &mut result_status, &status_map);
        if let Err(err) = download_result {
            result_status.state = "error".to_string();
            result_status.message = Some(err);
            let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
            guard.insert(model_size.clone(), result_status);
            return;
        }

        if let Err(err) = fs::rename(&tmp_path, &dest_path) {
            result_status.state = "error".to_string();
            result_status.message = Some(format!("Finalize error: {err}"));
            let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
            guard.insert(model_size.clone(), result_status);
            return;
        }

        result_status.state = "done".to_string();
        result_status.finished_at = Some(now_ts());
        result_status.message = Some("Download complete".to_string());
        let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(model_size.clone(), result_status);
    });

    Ok(status)
}

#[tauri::command]
pub fn get_whisper_download_status(state: State<ModelDownloadState>) -> ModelDownloadStatus {
    let key = whisper_binary_status_key();
    let guard = state.statuses.lock().ok();
    if let Some(guard) = guard {
        if let Some(status) = guard.get(&key) {
            return status.clone();
        }
    }
    ModelDownloadStatus {
        state: "idle".to_string(),
        model_size: key,
        repo_id: "whisper.cpp".to_string(),
        total_bytes: 0,
        downloaded_bytes: 0,
        message: None,
        started_at: None,
        finished_at: None,
    }
}

#[tauri::command]
pub fn get_whisper_installed(state: State<ModelDownloadState>) -> bool {
    let bin = state.whisper_dir.join("bin/whisper");
    let alt = state.whisper_dir.join("bin/main");
    if bin.exists() || alt.exists() {
        return true;
    }
    let third_party = PathBuf::from("third_party/whisper/bin/whisper");
    let third_party_alt = PathBuf::from("third_party/whisper/bin/main");
    if third_party.exists() || third_party_alt.exists() {
        return true;
    }
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("third_party/whisper/bin/whisper").exists()
            || cwd.join("third_party/whisper/bin/main").exists()
        {
            return true;
        }
    }
    false
}

#[tauri::command]
pub fn get_ffmpeg_download_status(state: State<ModelDownloadState>) -> ModelDownloadStatus {
    let key = ffmpeg_status_key();
    let guard = state.statuses.lock().ok();
    if let Some(guard) = guard {
        if let Some(status) = guard.get(&key) {
            return status.clone();
        }
    }
    ModelDownloadStatus {
        state: "idle".to_string(),
        model_size: key,
        repo_id: "ffmpeg".to_string(),
        total_bytes: 0,
        downloaded_bytes: 0,
        message: None,
        started_at: None,
        finished_at: None,
    }
}

#[tauri::command]
pub fn get_ffmpeg_installed(state: State<ModelDownloadState>) -> bool {
    let bin = state.ffmpeg_dir.join("bin/ffmpeg");
    let probe = state.ffmpeg_dir.join("bin/ffprobe");
    if bin.exists() && probe.exists() {
        return true;
    }
    let third_party = PathBuf::from("third_party/ffmpeg/bin/ffmpeg");
    if third_party.exists() {
        return true;
    }
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("third_party/ffmpeg/bin/ffmpeg").exists() {
            return true;
        }
    }
    false
}

#[tauri::command]
pub fn start_ffmpeg_download(
    app: AppHandle,
    state: State<ModelDownloadState>,
    url: String,
) -> Result<ModelDownloadStatus, String> {
    if url.trim().is_empty() {
        return Err("FFmpeg download URL is empty.".to_string());
    }
    let url = url.replace("http://", "https://");
    let key = ffmpeg_status_key();
    let bin_dir = state.ffmpeg_dir.join("bin");
    fs::create_dir_all(&bin_dir)
        .map_err(|err| format!("failed to create ffmpeg bin dir: {err}"))?;
    let tmp_path = state.ffmpeg_dir.join("ffmpeg.part");
    let _ = fs::remove_file(&tmp_path);

    let mut guard = state
        .statuses
        .lock()
        .map_err(|_| "model download mutex poisoned".to_string())?;
    if let Some(existing) = guard.get(&key) {
        if existing.state == "downloading" {
            return Ok(existing.clone());
        }
    }
    let status = ModelDownloadStatus {
        state: "downloading".to_string(),
        model_size: key.clone(),
        repo_id: "ffmpeg".to_string(),
        total_bytes: 0,
        downloaded_bytes: 0,
        message: Some("Downloading FFmpeg".to_string()),
        started_at: Some(now_ts()),
        finished_at: None,
    };
    guard.insert(key.clone(), status.clone());
    drop(guard);

    let status_map = Arc::clone(&state.inner().statuses);
    let app_handle = app.clone();
    let status_for_thread = status.clone();
    let ffmpeg_dir = state.ffmpeg_dir.clone();
    let key_for_thread = key.clone();
    thread::spawn(move || {
        let mut result_status = status_for_thread.clone();
        let download_result = download_to_file(&url, &tmp_path, &mut result_status, &status_map);
        if let Err(err) = download_result {
            result_status.state = "error".to_string();
            result_status.message = Some(err);
            let _ = fs::remove_file(&tmp_path);
            let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
            guard.insert(key_for_thread.clone(), result_status);
            let _ = app_handle.emit("job:log", JobLogEvent {
                id: "ffmpeg-download".to_string(),
                line: "FFmpeg download failed.".to_string(),
            });
            return;
        }

        let is_zip = url.to_lowercase().ends_with(".zip");
        if is_zip {
            if let Err(err) = extract_ffmpeg_zip(&tmp_path, &ffmpeg_dir) {
                result_status.state = "error".to_string();
                result_status.message = Some(err);
                let _ = fs::remove_file(&tmp_path);
                let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
                guard.insert(key_for_thread.clone(), result_status);
                return;
            }
            let _ = fs::remove_file(&tmp_path);
        } else {
            let dest_path = ffmpeg_dir.join("bin/ffmpeg");
            if let Err(err) = fs::rename(&tmp_path, &dest_path) {
                result_status.state = "error".to_string();
                result_status.message = Some(format!("Finalize error: {err}"));
                let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
                guard.insert(key_for_thread.clone(), result_status);
                return;
            }
        }

        let ffmpeg_path = ffmpeg_dir.join("bin/ffmpeg");
        let ffprobe_path = ffmpeg_dir.join("bin/ffprobe");
        if let Ok(mut perms) = fs::metadata(&ffmpeg_path).map(|meta| meta.permissions()) {
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&ffmpeg_path, perms);
        }
        if ffprobe_path.exists() {
            if let Ok(mut perms) = fs::metadata(&ffprobe_path).map(|meta| meta.permissions()) {
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&ffprobe_path, perms);
            }
        }

        if let Err(err) = ensure_lgpl_ffmpeg(ffmpeg_path.clone()) {
            result_status.state = "error".to_string();
            result_status.message = Some(err);
            let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
            guard.insert(key_for_thread.clone(), result_status);
            return;
        }

        result_status.state = "done".to_string();
        result_status.finished_at = Some(now_ts());
        result_status.message = Some("Download complete".to_string());
        let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(key_for_thread.clone(), result_status);
    });

    Ok(status)
}

#[tauri::command]
pub fn start_whisper_download(
    app: AppHandle,
    state: State<ModelDownloadState>,
    url: String,
) -> Result<ModelDownloadStatus, String> {
    let url = resolve_whisper_download_url(&url)?;
    let key = whisper_binary_status_key();
    let bin_dir = state.whisper_dir.join("bin");
    fs::create_dir_all(&bin_dir)
        .map_err(|err| format!("failed to create whisper bin dir: {err}"))?;
    let dest_path = bin_dir.join("whisper");
    let tmp_path = bin_dir.join("whisper.part");
    let _ = fs::remove_file(&dest_path);
    let _ = fs::remove_file(&tmp_path);

    let mut guard = state
        .statuses
        .lock()
        .map_err(|_| "model download mutex poisoned".to_string())?;
    if let Some(existing) = guard.get(&key) {
        if existing.state == "downloading" {
            return Ok(existing.clone());
        }
    }
    let status = ModelDownloadStatus {
        state: "downloading".to_string(),
        model_size: key.clone(),
        repo_id: "whisper.cpp".to_string(),
        total_bytes: 0,
        downloaded_bytes: 0,
        message: Some("Downloading whisper.cpp binary".to_string()),
        started_at: Some(now_ts()),
        finished_at: None,
    };
    guard.insert(key.clone(), status.clone());
    drop(guard);

    let status_map = Arc::clone(&state.inner().statuses);
    let app_handle = app.clone();
    let status_for_thread = status.clone();
    thread::spawn(move || {
        let mut result_status = status_for_thread.clone();
        let download_result = download_to_file(&url, &tmp_path, &mut result_status, &status_map);
        if let Err(err) = download_result {
            result_status.state = "error".to_string();
            result_status.message = Some(err);
            let _ = fs::remove_file(&tmp_path);
            let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
            guard.insert(key.clone(), result_status);
            let _ = app_handle.emit("job:log", JobLogEvent {
                id: "whisper-download".to_string(),
                line: "Whisper download failed.".to_string(),
            });
            return;
        }

        let is_zip = url.to_lowercase().ends_with(".zip");
        if is_zip {
            if let Err(err) = extract_whisper_zip(&tmp_path, &dest_path) {
                result_status.state = "error".to_string();
                result_status.message = Some(err);
                let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
                guard.insert(key.clone(), result_status);
                return;
            }
            let _ = fs::remove_file(&tmp_path);
        } else if let Err(err) = fs::rename(&tmp_path, &dest_path) {
            result_status.state = "error".to_string();
            result_status.message = Some(format!("Finalize error: {err}"));
            let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
            guard.insert(key.clone(), result_status);
            return;
        }

        if let Ok(mut perms) = fs::metadata(&dest_path).map(|meta| meta.permissions()) {
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&dest_path, perms);
        }

        result_status.state = "done".to_string();
        result_status.finished_at = Some(now_ts());
        result_status.message = Some("Download complete".to_string());
        let mut guard = status_map.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(key.clone(), result_status);
    });

    Ok(status)
}

#[tauri::command]
pub fn get_latest_whisper_release_url() -> Result<String, String> {
    let bizenlabs_latest =
        "https://github.com/bizenlabs/whisper-cpp-macos-bin/releases/latest";
    let ggml_latest = "https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest";
    let ggml_backup = "https://api.github.com/repos/ggml-org/whisper.cpp/releases";
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(bizenlabs_latest)
        .header(reqwest::header::USER_AGENT, "voicenote")
        .send();

    let assets = if let Ok(resp) = resp {
        if resp.status().is_success() {
            let json: serde_json::Value = resp
                .json()
                .map_err(|err| format!("Invalid GitHub response: {err}"))?;
            json.get("assets")
                .and_then(|v| v.as_array())
                .ok_or_else(|| "No assets in release.".to_string())?
                .to_vec()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let assets = if assets.is_empty() {
        let resp = client
            .get(ggml_latest)
            .header(reqwest::header::USER_AGENT, "voicenote")
            .send();
        if let Ok(resp) = resp {
            if resp.status().is_success() {
                let json: serde_json::Value = resp
                    .json()
                    .map_err(|err| format!("Invalid GitHub response: {err}"))?;
                json.get("assets")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| "No assets in release.".to_string())?
                    .to_vec()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    } else {
        assets
    };

    let assets = if assets.is_empty() {
        let resp = client
            .get(ggml_backup)
            .header(reqwest::header::USER_AGENT, "voicenote")
            .send()
            .map_err(|err| format!("Failed to fetch releases: {err}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(format!("GitHub API error: {status} {body}"));
        }
        let json: serde_json::Value = resp
            .json()
            .map_err(|err| format!("Invalid GitHub response: {err}"))?;
        json.as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.get("assets"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| "No assets in release list.".to_string())?
            .to_vec()
    } else {
        assets
    };
    for asset in assets {
        let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let url = asset
            .get("browser_download_url")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let name_lc = name.to_lowercase();
        let is_arm = name_lc.contains("arm64") || name_lc.contains("aarch64");
        let is_macos = name_lc.contains("macos")
            || name_lc.contains("osx")
            || name_lc.contains("darwin")
            || name_lc.contains("apple");
        let is_zip = name_lc.ends_with(".zip");
        if is_arm && is_zip && (is_macos || name_lc.contains("whisper")) {
            return Ok(url.to_string());
        }
    }
    Err("No macOS arm64 zip asset found in latest release.".to_string())
}

#[cfg(test)]
mod tests;
