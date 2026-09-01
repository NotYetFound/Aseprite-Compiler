use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::github::ReleaseInfo;
use crate::paths;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PersistedState {
    pub installed_version: Option<String>,
    pub install_path: Option<String>,
    pub last_check: Option<u64>, // epoch millis
    pub latest: Option<ReleaseInfo>,
    /// Version the user was last notified about — never nag twice for the same release.
    pub last_notified_version: Option<String>,
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Exclusive cross-process lock on the state file. The GUI/tray process and
/// the `--run-aseprite` launcher shim are separate processes that both write
/// state, so an in-process Mutex alone cannot serialize them.
fn state_lock() -> Option<std::fs::File> {
    use fs4::fs_std::FileExt;
    std::fs::create_dir_all(paths::config_dir()).ok()?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths::config_dir().join("state.lock"))
        .ok()?;
    file.lock_exclusive().ok()?;
    Some(file) // dropping the handle releases the lock
}

impl PersistedState {
    pub fn load() -> Self {
        let path = paths::state_file();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str(&text) {
            Ok(st) => st,
            Err(_) => {
                // Preserve corrupt state for diagnosis instead of silently
                // replacing it with defaults.
                let quarantine =
                    paths::config_dir().join(format!("state.corrupt-{}.json", now_millis()));
                let _ = std::fs::rename(&path, quarantine);
                Self::default()
            }
        }
    }

    /// Serialized load-modify-save across threads AND processes: the whole
    /// cycle holds an OS file lock, and the write lands atomically via a
    /// temp-file rename.
    pub fn update(f: impl FnOnce(&mut PersistedState)) -> PersistedState {
        use std::sync::Mutex;
        static LOCAL: Mutex<()> = Mutex::new(());
        let _thread_guard = LOCAL.lock().unwrap();
        let _process_guard = state_lock();
        let mut st = Self::load();
        f(&mut st);
        let _ = st.save_atomic();
        st
    }

    fn save_atomic(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(paths::config_dir())?;
        let path = paths::state_file();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}
