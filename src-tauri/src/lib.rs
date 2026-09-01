mod archive;
mod commands;
pub mod github;
mod installer;
mod net;
pub mod paths;
pub mod pipeline;
mod procwatch;
pub mod settings;
mod state;
mod status;
pub mod toolchain;
mod tray;
mod updates;
mod watcher;

/// Entry point for `--run-aseprite`: launch Aseprite, check for updates, exit.
pub fn run_shim(forward_args: Vec<String>) {
    updates::run_shim(forward_args);
}

use tauri::{AppHandle, Emitter, Manager};

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}

/// Routes pipeline events into the Tauri event system and native notifications.
struct TauriSink(AppHandle);

impl pipeline::Sink for TauriSink {
    fn emit_state(&self, state: &pipeline::PipelineState) {
        let _ = self.0.emit("pipeline://state", state);
    }

    fn emit_log(&self, line: &str) {
        let _ = self.0.emit("pipeline://log", line);
    }

    fn emit_status(&self, busy: bool) {
        status::emit_status(&self.0, busy);
    }

    fn notify(&self, title: &str, body: &str) {
        notify(&self.0, title, body);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .setup(|app| {
            let _ = paths::ensure_dirs();
            let engine = pipeline::Engine::new(Box::new(TauriSink(app.handle().clone())));
            app.manage(engine.clone());
            app.manage(tray::TrayState::default());

            let s = settings::Settings::load();
            if s.tray_enabled {
                tray::apply(app.handle(), engine.clone(), true);
                let hidden = std::env::args().any(|a| a == "--hidden");
                if hidden {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.hide();
                    }
                }
            }

            watcher::spawn(app.handle().clone(), engine.clone());
            procwatch::spawn(app.handle().clone(), engine);

            // Self-repair Aseprite's launcher entry: if this app moved (e.g.
            // a relocated AppImage the shim Exec pointed at), rewrite it.
            std::thread::spawn(installer::repair_launcher_entry);

            // Silent self-update check on startup; the About tab has the
            // manual controls. Failures (e.g. offline) are ignored.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri_plugin_updater::UpdaterExt;
                if let Ok(updater) = handle.updater() {
                    if let Ok(Some(update)) = updater.check().await {
                        notify(
                            &handle,
                            "Aseprite Compiler update",
                            &format!(
                                "Version {} is available — install it from the About tab.",
                                update.version
                            ),
                        );
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // With the tray enabled, closing the window keeps the app
                // (and the release watcher) running in the background.
                if settings::Settings::load().tray_enabled {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    // The app is about to exit: kill any running build so its
                    // process group doesn't survive as an orphan.
                    let engine = window.app_handle().state::<std::sync::Arc<pipeline::Engine>>();
                    if engine.running() {
                        engine.cancel();
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::set_settings,
            commands::get_status,
            commands::check_tools,
            commands::provision_tools,
            commands::start_pipeline,
            commands::retry_pipeline,
            commands::cancel_pipeline,
            commands::get_pipeline_state,
            commands::get_log_tail,
            commands::launch_aseprite,
            commands::uninstall_aseprite,
            commands::open_path,
            commands::copy_to_clipboard,
            commands::get_app_version,
            commands::check_app_update,
            commands::install_app_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Aseprite Compiler");
}
