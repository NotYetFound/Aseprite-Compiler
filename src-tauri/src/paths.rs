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

/// Stable source-tree path. Deliberately not version-keyed: when build files
/// are kept, a version update syncs changed files into the same tree so an
/// existing build directory stays incremental (compiler paths never change).
pub fn src_dir() -> PathBuf {
    work_dir().join("src")
}

pub fn build_dir() -> PathBuf {
    work_dir().join("build")
}

/// Records which release tag the source tree currently holds.
pub fn src_version_marker() -> PathBuf {
    work_dir().join("src.version")
}

/// Downloaded archives (source zips, Skia packages).
pub fn cache_dir() -> PathBuf {
    data_dir().join("cache")
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

/// Compiler cache (when enabled in settings). Lives outside work/cache so the
/// post-build cleanup never deletes it; removed when the setting is turned off.
pub fn ccache_dir() -> PathBuf {
    data_dir().join("ccache")
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
