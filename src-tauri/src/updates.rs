//! Shared "is there a new Aseprite?" check, used by the timed watcher, the
//! launcher shim (`--run-aseprite`), and the process watch. It notifies only
//! when a genuinely newer release exists, and never twice for the same one.

use std::path::PathBuf;

use crate::settings::Settings;
use crate::state::{now_millis, PersistedState};
use crate::{github, installer, toolchain};

/// Event-driven checks (launch / process watch) skip when a check already
/// happened recently; the timed watcher runs on its own schedule instead.
const EVENT_THROTTLE_MS: u64 = 15 * 60 * 1000;

#[derive(Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    UpdateAvailable(String),
    UpToDate,
    Skipped,
    Failed,
}

pub fn install_root() -> PathBuf {
    PersistedState::load()
        .install_path
        .map(PathBuf::from)
        .unwrap_or_else(|| Settings::load().install_root())
}

/// Check GitHub for a newer release than the installed build and notify once
/// per new version. `respect_throttle` is set by event-driven callers.
pub fn check_and_notify(respect_throttle: bool, notify: &dyn Fn(&str, &str)) -> CheckOutcome {
    let settings = Settings::load();

    if respect_throttle {
        if let Some(t) = PersistedState::load().last_check {
            if now_millis().saturating_sub(t) < EVENT_THROTTLE_MS {
                return CheckOutcome::Skipped;
            }
        }
    }

    let Ok(release) = github::fetch_latest(settings.channel) else {
        return CheckOutcome::Failed; // offline — try again another time
    };
    let st = PersistedState::update(|st| {
        st.last_check = Some(now_millis());
        st.latest = Some(release.clone());
    });

    // The binary itself is the authority on what's installed.
    let root = st
        .install_path
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| settings.install_root());
    let installed = toolchain::probe_aseprite_version(&installer::aseprite_bin(&root))
        .or_else(|| st.installed_version.clone());
    let Some(installed) = installed else {
        return CheckOutcome::Skipped; // nothing installed — nothing to update
    };

    if installed == release.version {
        return CheckOutcome::UpToDate;
    }

    if st.last_notified_version.as_deref() != Some(release.version.as_str()) {
        PersistedState::update(|st| {
            st.last_notified_version = Some(release.version.clone());
        });
        notify(
            "Aseprite update available",
            &format!(
                "Aseprite {} is out. Open Aseprite Compiler to build it.",
                release.version
            ),
        );
    }
    CheckOutcome::UpdateAvailable(release.version)
}

/// Notification that works without a running Tauri app (the shim path).
#[cfg(target_os = "linux")]
pub fn notify_headless(title: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .appname("Aseprite Compiler")
        .summary(title)
        .body(body)
        .icon(installer::ICON_NAME)
        .show();
}

#[cfg(target_os = "windows")]
pub fn notify_headless(title: &str, body: &str) {
    use tauri_winrt_notification::Toast;
    let _ = Toast::new(Toast::POWERSHELL_APP_ID)
        .title(title)
        .text1(body)
        .show();
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn notify_headless(_title: &str, _body: &str) {}

/// `aseprite-compiler --run-aseprite [args…]`: launch the installed Aseprite
/// immediately, then quietly check for updates in the background. This is what
/// the launcher entry points at while "check when Aseprite starts" is on.
pub fn run_shim(forward_args: Vec<String>) {
    let bin = installer::aseprite_bin(&install_root());
    if bin.is_file() {
        let mut cmd = std::process::Command::new(&bin);
        // Forward real arguments (opened files); drop stray %U-style
        // placeholders a launcher might pass through literally.
        cmd.args(forward_args.iter().filter(|a| !a.starts_with('%')));
        if let Some(dir) = bin.parent() {
            cmd.current_dir(dir);
        }
        let _ = cmd.spawn();
    }

    if Settings::load().check_on_launch {
        let _ = check_and_notify(true, &notify_headless);
    }
}
