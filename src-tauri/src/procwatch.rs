//! Optional process watch: notices when the compiled Aseprite starts in a way
//! that bypasses the launcher entry (direct binary run, file association) and
//! triggers a throttled update check on that rising edge.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

use crate::pipeline::Engine;
use crate::settings::Settings;
use crate::{installer, status, updates};

#[cfg(target_os = "linux")]
const POLL: Duration = Duration::from_secs(5);
#[cfg(not(target_os = "linux"))]
const POLL: Duration = Duration::from_secs(15);

pub fn spawn(app: AppHandle, engine: Arc<Engine>) {
    std::thread::spawn(move || {
        let mut was_running = false;
        loop {
            std::thread::sleep(POLL);
            if !Settings::load().process_watch {
                was_running = false;
                continue;
            }
            let bin = installer::aseprite_bin(&updates::install_root());
            if !bin.is_file() {
                was_running = false;
                continue;
            }
            let running = aseprite_running(&bin);
            if running && !was_running && !engine.running() {
                updates::check_and_notify(true, &|t, b| crate::notify(&app, t, b));
                status::emit_status(&app, engine.running());
            }
            was_running = running;
        }
    });
}

/// Is a process running from our installed Aseprite binary?
#[cfg(target_os = "linux")]
fn aseprite_running(bin: &Path) -> bool {
    let canonical: PathBuf = std::fs::canonicalize(bin).unwrap_or_else(|_| bin.to_path_buf());
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Ok(exe) = std::fs::read_link(entry.path().join("exe")) {
            // A replaced binary shows as "<path> (deleted)" — still ours.
            let exe = exe.to_string_lossy();
            let exe = exe.strip_suffix(" (deleted)").unwrap_or(&exe);
            if Path::new(exe) == canonical {
                return true;
            }
        }
    }
    false
}

#[cfg(target_os = "windows")]
fn aseprite_running(bin: &Path) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-Process aseprite -ErrorAction SilentlyContinue).Path",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let Ok(out) = out else { return false };
    let want = bin.to_string_lossy().to_lowercase();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|line| line.trim().to_lowercase() == want)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn aseprite_running(_bin: &Path) -> bool {
    false
}
