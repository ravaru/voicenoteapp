//! Tauri entrypoint for the Rust core.
//!
//! Step 2 of the migration: expose stub commands so the UI can switch
//! from HTTP to `invoke()` without a separate backend.

mod commands;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle();
            let config_state = commands::ConfigState::load(&handle).map_err(|err| {
                std::io::Error::new(std::io::ErrorKind::Other, err)
            })?;
            app.manage(config_state);
            let job_index_state = commands::JobIndexState::load(&handle).map_err(|err| {
                std::io::Error::new(std::io::ErrorKind::Other, err)
            })?;
            app.manage(job_index_state);
            let model_state = commands::ModelDownloadState::load(&handle).map_err(|err| {
                std::io::Error::new(std::io::ErrorKind::Other, err)
            })?;
            app.manage(model_state);
            let queue_state = commands::spawn_worker(&handle);
            app.manage(queue_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_health,
            commands::get_config,
            commands::update_config,
            commands::initialize_config,
            commands::get_config_initialized,
            commands::list_jobs,
            commands::get_job,
            commands::add_files,
            commands::create_job_from_path,
            commands::cancel_job,
            commands::delete_job,
            commands::export_to_obsidian,
            commands::get_segments,
            commands::get_clip_path,
            commands::get_summary,
            commands::summarize_job,
            commands::get_model_size,
            commands::get_model_download_status,
            commands::get_model_installed,
            commands::start_model_download,
            commands::get_whisper_download_status,
            commands::get_whisper_installed,
            commands::start_whisper_download,
            commands::get_latest_whisper_release_url,
            commands::get_ffmpeg_download_status,
            commands::get_ffmpeg_installed,
            commands::start_ffmpeg_download,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
