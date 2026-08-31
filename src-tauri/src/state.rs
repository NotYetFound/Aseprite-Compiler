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
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl PersistedState {
    pub fn load() -> Self {
        std::fs::read_to_string(paths::state_file())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Serialized load-modify-save: multiple threads (pipeline, watcher,
    /// commands) update state, so the whole cycle holds one lock and the
    /// write lands atomically via a temp-file rename.
    pub fn update(f: impl FnOnce(&mut PersistedState)) -> PersistedState {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();
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
