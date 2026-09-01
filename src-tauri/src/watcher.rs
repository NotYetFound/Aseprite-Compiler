use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

use crate::pipeline::Engine;
use crate::settings::{Settings, WatcherMode};
use crate::state::{now_millis, PersistedState};
use crate::{status, updates};

/// Timed release watcher: periodically checks GitHub for a new Aseprite
/// release and notifies when one appears. Building is always user-initiated.
pub fn spawn(app: AppHandle, engine: Arc<Engine>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(60));

        let settings = Settings::load();
        if settings.watcher_mode == WatcherMode::Off {
            continue;
        }
        let interval_ms = u64::from(settings.watcher_interval_hours.max(1)) * 3_600_000;
        let due = PersistedState::load()
            .last_check
            .map(|t| now_millis().saturating_sub(t) >= interval_ms)
            .unwrap_or(true);
        if !due || engine.running() {
            continue;
        }

        updates::check_and_notify(false, &|t, b| crate::notify(&app, t, b));
        status::emit_status(&app, engine.running());
    });
}
