use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

use crate::pipeline::Engine;
use crate::settings::{Settings, WatcherMode};
use crate::state::{now_millis, PersistedState};
use crate::{github, status};

/// Background release watcher: periodically checks GitHub for a new Aseprite
/// release and notifies (or auto-builds) when one appears.
pub fn spawn(app: AppHandle, engine: Arc<Engine>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(60));

        let settings = Settings::load();
        if settings.watcher_mode == WatcherMode::Off {
            continue;
        }
        let interval_ms = u64::from(settings.watcher_interval_hours.max(1)) * 3_600_000;
        let st = PersistedState::load();
        let due = st
            .last_check
            .map(|t| now_millis().saturating_sub(t) >= interval_ms)
            .unwrap_or(true);
        if !due || engine.running() {
            continue;
        }

        let Ok(release) = github::fetch_latest(settings.channel) else {
            continue; // offline; try again next interval
        };

        let st = PersistedState::update(|st| {
            st.last_check = Some(now_millis());
            st.latest = Some(release.clone());
        });
        status::emit_status(&app, engine.running());

        // Only act when an *installed* build is outdated. If the user
        // uninstalled Aseprite, the watcher must not sneak it back in.
        let Some(installed) = st.installed_version.clone() else {
            continue;
        };
        if installed == release.version {
            continue;
        }

        match settings.watcher_mode {
            WatcherMode::Notify => {
                crate::notify(
                    &app,
                    "Aseprite update available",
                    &format!(
                        "Aseprite {} is out. Open Aseprite Compiler to build it.",
                        release.version
                    ),
                );
            }
            WatcherMode::Auto => {
                crate::notify(
                    &app,
                    "Aseprite update",
                    &format!("Building Aseprite {} in the background…", release.version),
                );
                let _ = engine.start(settings);
            }
            WatcherMode::Off => {}
        }
    });
}
