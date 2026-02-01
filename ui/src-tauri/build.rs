use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::symlink;

fn copy_file(src: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create dir failed: {err}"))?;
    }
    let metadata = fs::symlink_metadata(src).map_err(|err| format!("stat failed: {err}"))?;
    if metadata.file_type().is_symlink() {
        #[cfg(unix)]
        {
            if dest.exists() {
                let _ = fs::remove_file(dest);
            }
            let target = fs::read_link(src).map_err(|err| format!("read_link failed: {err}"))?;
            symlink(target, dest).map_err(|err| format!("symlink failed: {err}"))?;
            return Ok(());
        }
    }
    fs::copy(src, dest).map_err(|err| format!("copy failed: {err}"))?;
    Ok(())
}

fn copy_dir_filtered(src: &Path, dest: &Path, filter: &dyn Fn(&Path) -> bool) -> Result<(), String> {
    if !src.exists() {
        return Err(format!("source path missing: {}", src.display()));
    }
    for entry in fs::read_dir(src).map_err(|err| format!("read_dir failed: {err}"))? {
        let entry = entry.map_err(|err| format!("read_dir entry failed: {err}"))?;
        let path = entry.path();
        let rel = path.strip_prefix(src).map_err(|_| "strip prefix failed".to_string())?;
        let out = dest.join(rel);
        if path.is_dir() {
            copy_dir_filtered(&path, &out, filter)?;
        } else if filter(&path) {
            copy_file(&path, &out)?;
        }
    }
    Ok(())
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root_dir = manifest_dir.join("..").join("..");
    let ffmpeg_src = root_dir.join("third_party").join("ffmpeg");
    let resources_dir = manifest_dir.join("resources");
    let ffmpeg_dest = resources_dir.join("ffmpeg");

    let bin_filter = |path: &Path| path.file_name().and_then(|n| n.to_str()) == Some("ffmpeg")
        || path.file_name().and_then(|n| n.to_str()) == Some("ffprobe");
    let lib_filter = |path: &Path| {
        path.extension().and_then(|e| e.to_str()) == Some("dylib")
            || path.file_name().and_then(|n| n.to_str()).map(|n| n.contains(".dylib")).unwrap_or(false)
    };

    let _ = fs::create_dir_all(&resources_dir);
    let _ = fs::create_dir_all(ffmpeg_dest.join("bin"));
    let _ = fs::create_dir_all(ffmpeg_dest.join("lib"));

    if ffmpeg_src.exists() {
        let _ = copy_dir_filtered(&ffmpeg_src.join("bin"), &ffmpeg_dest.join("bin"), &bin_filter);
        let _ = copy_dir_filtered(&ffmpeg_src.join("lib"), &ffmpeg_dest.join("lib"), &lib_filter);
    }

    tauri_build::build()
}
