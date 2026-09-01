use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

use crate::pipeline::{Engine, PipelineState};
use crate::settings::Settings;
use crate::state::PersistedState;
use crate::status::{self, StatusInfo};
use crate::toolchain::{self, ToolReport};
use crate::{installer, tray};

type CmdResult<T> = Result<T, String>;

fn err_str(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

#[tauri::command]
pub fn get_settings() -> Settings {
    Settings::load()
}

#[tauri::command]
pub fn set_settings(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    settings: Settings,
) -> CmdResult<()> {
    settings.save().map_err(err_str)?;
    tray::apply(&app, engine.inner().clone(), settings.tray_enabled);

    // Autostart follows "start minimized to tray": register at login only
    // when the user wants the tray running from the start.
    use tauri_plugin_autostart::ManagerExt;
    let autostart = app.autolaunch();
    if settings.tray_enabled && settings.start_minimized {
        let _ = autostart.enable();
    } else {
        let _ = autostart.disable();
    }

    // The launch-check setting decides what Aseprite's launcher entry points
    // at (shim vs. binary) — keep it in sync.
    std::thread::spawn(installer::repair_launcher_entry);
    Ok(())
}

#[tauri::command]
pub async fn get_status(engine: State<'_, Arc<Engine>>, refresh: bool) -> CmdResult<StatusInfo> {
    let busy = engine.running();
    tauri::async_runtime::spawn_blocking(move || status::build_status(refresh, busy))
        .await
        .map_err(err_str)?
        .map_err(err_str)
}

#[tauri::command]
pub async fn check_tools() -> CmdResult<ToolReport> {
    tauri::async_runtime::spawn_blocking(toolchain::check)
        .await
        .map_err(err_str)
}

#[tauri::command]
pub async fn provision_tools(engine: State<'_, Arc<Engine>>) -> CmdResult<()> {
    if engine.running() {
        return Err("a build is running — the pipeline manages tools itself".into());
    }
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cancel = AtomicBool::new(false);
        toolchain::provision(&cancel, |m| engine.log_line(m))
    })
    .await
    .map_err(err_str)?
    .map_err(err_str)
}

#[tauri::command]
pub fn start_pipeline(engine: State<'_, Arc<Engine>>) -> CmdResult<()> {
    engine.start(Settings::load()).map_err(err_str)
}

#[tauri::command]
pub fn retry_pipeline(engine: State<'_, Arc<Engine>>) -> CmdResult<()> {
    // Stages skip work that is already complete, so a plain restart resumes
    // from the failed stage.
    engine.start(Settings::load()).map_err(err_str)
}

#[tauri::command]
pub fn cancel_pipeline(engine: State<'_, Arc<Engine>>) -> CmdResult<()> {
    engine.cancel();
    Ok(())
}

#[tauri::command]
pub fn get_pipeline_state(engine: State<'_, Arc<Engine>>) -> PipelineState {
    engine.snapshot()
}

#[tauri::command]
pub fn get_log_tail(engine: State<'_, Arc<Engine>>) -> Vec<String> {
    engine.log_tail()
}

#[tauri::command]
pub fn launch_aseprite() -> CmdResult<()> {
    let settings = Settings::load();
    let bin = installer::aseprite_bin(&settings.install_root());
    if !bin.is_file() {
        return Err("Aseprite is not installed yet".into());
    }
    std::process::Command::new(&bin)
        .current_dir(bin.parent().unwrap())
        .spawn()
        .map_err(err_str)?;
    Ok(())
}

#[tauri::command]
pub async fn uninstall_aseprite(app: AppHandle, engine: State<'_, Arc<Engine>>) -> CmdResult<()> {
    if engine.running() {
        return Err("a build is running — cancel it first".into());
    }
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<()> {
        // Prefer the recorded install location: the setting may have changed
        // since the build was installed.
        let root = PersistedState::load()
            .install_path
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Settings::load().install_root());
        installer::uninstall(&root).map_err(err_str)?;
        PersistedState::update(|st| {
            st.installed_version = None;
            st.install_path = None;
        });
        status::emit_status(&app, false);
        Ok(())
    })
    .await
    .map_err(err_str)?
}

#[tauri::command]
pub fn open_path(app: AppHandle, path: String) -> CmdResult<()> {
    if path.starts_with("http://") || path.starts_with("https://") {
        app.opener().open_url(&path, None::<&str>).map_err(err_str)
    } else {
        app.opener().open_path(&path, None::<&str>).map_err(err_str)
    }
}

#[tauri::command]
pub fn copy_to_clipboard(app: AppHandle, text: String) -> CmdResult<()> {
    app.clipboard().write_text(text).map_err(err_str)
}

#[tauri::command]
pub fn get_app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Check for an update of this app itself. Returns the new version, if any.
#[tauri::command]
pub async fn check_app_update(app: AppHandle) -> CmdResult<Option<String>> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(err_str)?;
    match updater.check().await {
        Ok(Some(update)) => Ok(Some(update.version.clone())),
        Ok(None) => Ok(None),
        Err(e) => Err(err_str(e)),
    }
}

/// Download and install the app's own update, then restart into it.
#[tauri::command]
pub async fn install_app_update(app: AppHandle, engine: State<'_, Arc<Engine>>) -> CmdResult<()> {
    use tauri_plugin_updater::UpdaterExt;
    if engine.running() {
        return Err("a build is running — finish or cancel it first".into());
    }
    let updater = app.updater().map_err(err_str)?;
    let Some(update) = updater.check().await.map_err(err_str)? else {
        return Err("no update available".into());
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(err_str)?;
    app.restart();
}
