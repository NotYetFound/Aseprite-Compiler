use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Beta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WatcherMode {
    Off,
    // `other` also absorbs the removed "auto" mode from old settings files.
    #[serde(other)]
    Notify,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub channel: Channel,
    pub install_dir: String,
    pub cleanup_after_build: bool,
    pub tray_enabled: bool,
    pub start_minimized: bool,
    pub watcher_mode: WatcherMode,
    pub watcher_interval_hours: u32,
    pub parallel_jobs: u32,
    /// Launcher shim: Aseprite's launcher entry starts it through this app,
    /// which quietly checks for updates in the background.
    pub check_on_launch: bool,
    /// Watch the process table for a running compiled Aseprite (catches
    /// launches that bypass the launcher entry). Needs the app or tray open.
    pub process_watch: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            channel: Channel::Stable,
            install_dir: String::new(),
            // The build cleans up after itself by default.
            cleanup_after_build: true,
            tray_enabled: false,
            start_minimized: false,
            watcher_mode: WatcherMode::Notify,
            watcher_interval_hours: 12,
            parallel_jobs: 0,
            check_on_launch: true,
            process_watch: false,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read_to_string(paths::settings_file())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(paths::config_dir())?;
        // Atomic replace so a crash mid-write can't corrupt the file.
        let path = paths::settings_file();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn install_root(&self) -> PathBuf {
        if self.install_dir.trim().is_empty() {
            paths::default_install_root()
        } else {
            PathBuf::from(self.install_dir.trim())
        }
    }
}
