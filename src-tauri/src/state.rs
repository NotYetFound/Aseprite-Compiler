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

/// Epoch millis → sortable UTC stamp "YYYYMMDD-HHMMSS" (for file names).
pub fn utc_stamp(millis: u64) -> String {
    let secs = millis / 1000;
    let (y, m, d) = civil_from_days((secs / 86400) as i64);
    let rem = secs % 86400;
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

// Days-since-epoch → (year, month, day), Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
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
