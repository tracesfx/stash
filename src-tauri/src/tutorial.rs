//! First-run tutorial: a small glass card that teaches the three things worth knowing by having the user do
//! them (snip, open the wheel, open the gallery). Each step advances on its own when the app sees it happen.
//! Shown once on a fresh install (config `tutorial_done`), and again from Settings > General.
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::AppState;

pub const LABEL: &str = "welcome";

pub fn open(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(w) = app.get_webview_window(LABEL) {
            if let Ok(h) = w.hwnd() {
                crate::winutil::bring_to_front(h.0 as isize);
            }
            return;
        }
        let cfg = app.state::<AppState>().cfg();
        let f = crate::settings::scale_factor(&cfg);
        let (w, h) = (470.0 * f, 400.0 * f);
        // top centre of the primary screen: out of the way of what the user is about to snip
        let (wa, s) = crate::winutil::primary_work_area();
        let s = s as f64;
        let x = (wa.left + wa.right) as f64 / 2.0 / s - w / 2.0;
        let y = wa.top as f64 / s + 72.0 * f;
        let b = WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::App("welcome.html".into()))
            .title("Welcome to Stash")
            .inner_size(w, h)
            .position(x, y)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(false)
            .transparent(true)
            .visible(false); // shown by `window_ready` after the first paint
        match b.build() {
            Ok(win) => {
                let _ = win.set_zoom(f);
                crate::theme::style_window(&win, &cfg);
                crate::gallery::show_if_not_ready(win);
            }
            Err(e) => tracing::error!("tutorial window: {e}"),
        }
    });
}

/// Something the tutorial waits for happened: "saved", "wheel" or "gallery".
pub fn step(app: &AppHandle, what: &str) {
    if app.get_webview_window(LABEL).is_some() {
        let _ = app.emit_to(LABEL, "tutorial", what);
    }
}

/// The page's data: the shortcut to show and where the library lives.
#[tauri::command]
pub fn tutorial_info(app: AppHandle) -> serde_json::Value {
    let c = app.state::<AppState>().cfg();
    serde_json::json!({ "hotkey": c.hotkey, "library": c.library_path, "hold_ms": c.hold_ms })
}

/// Done or skipped: remember it and close the card.
#[tauri::command]
pub fn tutorial_finish(app: AppHandle) {
    let st = app.state::<AppState>();
    let mut c = st.cfg();
    if !c.tutorial_done {
        c.tutorial_done = true;
        if let Err(e) = c.save() {
            tracing::error!("save config: {e:#}");
        }
        *st.config.write().unwrap() = c;
    }
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.destroy();
    }
}

/// Settings > General > Show the tutorial again.
#[tauri::command]
pub fn tutorial_open(app: AppHandle) {
    open(&app);
}
