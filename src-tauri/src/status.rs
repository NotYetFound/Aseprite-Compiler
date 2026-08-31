use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::settings::Settings;
use crate::state::{now_millis, PersistedState};
use crate::{installer, toolchain};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    pub installed_version: Option<String>,
    pub installed_path: Option<String>,
    pub latest_version: Option<String>,
    pub latest_name: Option<String>,
    pub last_check: Option<u64>,
    pub busy: bool,
}

/// Build the dashboard status. With `refresh`, asks GitHub for the latest
/// release first (and persists it); otherwise serves cached data.
pub fn build_status(refresh: bool, busy: bool) -> anyhow::Result<StatusInfo> {
    let settings = Settings::load();
    let st = if refresh {
        let release = crate::github::fetch_latest(settings.channel)?;
        PersistedState::update(|st| {
            st.latest = Some(release);
            st.last_check = Some(now_millis());
        })
    } else {
        PersistedState::load()
    };

    // Ask the installed binary itself for its version; fall back to state.
    let root = settings.install_root();
    let bin = installer::aseprite_bin(&root);
    let installed_version =
        toolchain::probe_aseprite_version(&bin).or_else(|| st.installed_version.clone());
    let installed_path = if bin.is_file() {
        Some(root.display().to_string())
    } else {
        None
    };

    Ok(StatusInfo {
        installed_version,
        installed_path,
        latest_version: st.latest.as_ref().map(|r| r.version.clone()),
        latest_name: st.latest.as_ref().map(|r| r.name.clone()),
        last_check: st.last_check,
        busy,
    })
}

pub fn emit_status(app: &AppHandle, busy: bool) {
    if let Ok(info) = build_status(false, busy) {
        let _ = app.emit("status://changed", &info);
    }
}
