use std::path::PathBuf;

const APP_DIR: &str = "aseprite-compiler";

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .expect("no data dir on this platform")
        .join(APP_DIR)
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .expect("no config dir on this platform")
        .join(APP_DIR)
}

pub fn settings_file() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn state_file() -> PathBuf {
    config_dir().join("state.json")
}

/// Portable tools live here (cmake, ninja) — never installed system-wide.
pub fn tools_dir() -> PathBuf {
    data_dir().join("tools")
}

/// Transient build workspace: source trees and build output.
pub fn work_dir() -> PathBuf {
    data_dir().join("work")
}

pub fn src_dir(version: &str) -> PathBuf {
    work_dir().join(format!("src-{version}"))
}

pub fn build_dir(version: &str) -> PathBuf {
    work_dir().join(format!("build-{version}"))
}

/// Downloaded archives (source zips, Skia packages).
pub fn cache_dir() -> PathBuf {
    data_dir().join("cache")
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

/// Default Aseprite install root; the live build lives in `<root>/current`
/// so the launcher entry keeps a stable path across updates.
pub fn default_install_root() -> PathBuf {
    data_dir().join("install")
}

/// The stable path of this application. For an AppImage that is the
/// .AppImage file itself ($APPIMAGE) — not the transient squashfs mount that
/// current_exe() points into.
pub fn app_executable() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("APPIMAGE") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    std::env::current_exe().ok()
}

pub fn ensure_dirs() -> std::io::Result<()> {
    for d in [
        data_dir(),
        config_dir(),
        tools_dir(),
        work_dir(),
        cache_dir(),
        logs_dir(),
    ] {
        std::fs::create_dir_all(d)?;
    }
    Ok(())
}
