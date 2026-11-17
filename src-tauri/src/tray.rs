//! System tray: the only always-present UI.
use std::sync::atomic::Ordering;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::AppState;

/// Managed handle so other code (the wheel) can keep the tray checkbox in sync.
#[allow(dead_code)]
pub struct PauseItem(pub CheckMenuItem<tauri::Wry>);

/// Status line at the top of the tray menu (shows the current hotkey).
pub struct StatusItem(pub MenuItem<tauri::Wry>);

pub fn set_status(app: &AppHandle, text: &str) {
    if let Some(i) = app.try_state::<StatusItem>() {
        let _ = i.0.set_text(text);
    }
    if let Some(t) = app.tray_by_id("main") {
        let _ = t.set_tooltip(Some(format!("Stash — {text}")));
    }
}

/// Tray icon states: normal, paused, hotkey unavailable.
fn tray_icon(name: &str) -> Option<tauri::image::Image<'static>> {
    let bytes: &[u8] = match name {
        "paused" => include_bytes!("../icons/tray-paused.png"),
        "warn" => include_bytes!("../icons/tray-warn.png"),
        _ => include_bytes!("../icons/tray-normal.png"),
    };
    tauri::image::Image::from_bytes(bytes).ok()
}

fn set_tray_icon(app: &AppHandle, name: &str) {
    if let (Some(t), Some(i)) = (app.tray_by_id("main"), tray_icon(name)) {
        let _ = t.set_icon(Some(i));
    }
}

#[allow(dead_code)]
pub fn sync_pause(app: &AppHandle, paused: bool) {
    set_tray_icon(app, if paused { "paused" } else { "normal" });
    if let Some(i) = app.try_state::<PauseItem>() {
        let _ = i.0.set_checked(paused);
    }
}

pub fn build(app: &AppHandle, hotkey_ok: bool) -> tauri::Result<()> {
    let hotkey = app.state::<AppState>().cfg().hotkey;
    let status_text = if hotkey_ok {
        format!("Capture: {hotkey}")
    } else {
        format!("⚠ Hotkey {hotkey} unavailable")
    };
    let status = MenuItem::with_id(app, "status", status_text.clone(), false, None::<&str>)?;
    app.manage(StatusItem(status.clone()));
    let gallery = MenuItem::with_id(app, "gallery", "Open gallery", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let board = MenuItem::with_id(app, "board", "New whiteboard", true, None::<&str>)?;
    let folder = MenuItem::with_id(app, "folder", "Open library folder", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pause capture", true, false, None::<&str>)?;
    app.manage(PauseItem(pause.clone()));
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&status, &sep1, &gallery, &board, &folder, &settings, &pause, &sep2, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip(format!("Stash — {status_text}"))
        .menu(&menu)
        .show_menu_on_left_click(false) // left click opens the gallery, right click shows the menu
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click { button: tauri::tray::MouseButton::Left, button_state: tauri::tray::MouseButtonState::Up, .. } = event {
                crate::gallery::open(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "gallery" => crate::gallery::open(app),
            "settings" => crate::settings::open(app),
            "board" => crate::board::open(app, None),
            "folder" => {
                let path = app.state::<AppState>().cfg().library_path;
                if let Err(e) = app.opener().open_path(path.to_string_lossy(), None::<&str>) {
                    tracing::error!("open library folder: {e}");
                }
            }
            "pause" => {
                let st = app.state::<AppState>();
                let now = !st.paused.load(Ordering::Relaxed);
                st.paused.store(now, Ordering::Relaxed);
                set_tray_icon(app, if now { "paused" } else { "normal" });
                tracing::info!("paused = {now}");
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = tray_icon(if hotkey_ok { "normal" } else { "warn" }).or_else(|| app.default_window_icon().cloned()) {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}
