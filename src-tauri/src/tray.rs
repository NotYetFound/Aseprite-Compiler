use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager};

use crate::pipeline::Engine;
use crate::settings::Settings;
use crate::status;

/// Holds the live tray icon so it can be added/removed at runtime.
#[derive(Default)]
pub struct TrayState(pub Mutex<Option<TrayIcon>>);

pub fn apply(app: &AppHandle, engine: Arc<Engine>, enabled: bool) {
    let state = app.state::<TrayState>();
    let mut slot = state.0.lock().unwrap();
    if enabled {
        if slot.is_none() {
            match create(app, engine) {
                Ok(tray) => *slot = Some(tray),
                Err(e) => eprintln!("tray: {e}"),
            }
        }
    } else {
        // Dropping the TrayIcon removes it from the tray.
        *slot = None;
    }
}

fn create(app: &AppHandle, engine: Arc<Engine>) -> tauri::Result<TrayIcon> {
    let open = MenuItem::with_id(app, "open", "Open Aseprite Compiler", true, None::<&str>)?;
    let check = MenuItem::with_id(app, "check", "Check for updates now", true, None::<&str>)?;
    let build = MenuItem::with_id(app, "build", "Build latest Aseprite", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &check, &build, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Aseprite Compiler")
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "check" => {
                let app = app.clone();
                let engine = engine.clone();
                std::thread::spawn(move || {
                    let busy = engine.running();
                    match status::build_status(true, busy) {
                        Ok(info) => {
                            status::emit_status(&app, busy);
                            let latest = info.latest_version.unwrap_or_default();
                            let msg = if info.installed_version.as_deref() == Some(latest.as_str())
                            {
                                format!("Aseprite {latest} is installed and up to date.")
                            } else {
                                format!("Aseprite {latest} is available.")
                            };
                            crate::notify(&app, "Aseprite Compiler", &msg);
                        }
                        Err(e) => crate::notify(&app, "Aseprite Compiler", &format!("Check failed: {e}")),
                    }
                });
            }
            "build" => {
                let _ = engine.start(Settings::load());
                show_main(app);
            }
            "quit" => {
                // Build children live in their own process group: kill them
                // before exiting so no orphaned compile keeps burning CPU.
                engine.cancel();
                app.exit(0);
            }
            _ => {}
        })
        .build(app)
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
