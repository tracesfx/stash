//! Post-capture confirmation popup (a small webview window). Click it to add a caption.
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::library::Preview;

// 12 px of transparent margin around the card holds its shadow.
const W: f64 = 324.0;
const H: f64 = 214.0;
const CAPTION_EXTRA: f64 = 46.0;

#[derive(Default)]
pub struct PopupState {
    inner: Mutex<Option<Pending>>,
}

struct Pending {
    sidecar: Option<PathBuf>, // None for notices (nothing saved)
    preview: Preview,
    meta: String,
}

#[derive(Serialize)]
pub struct PopupData {
    kind: &'static str, // image | text | notice
    data: String,
    captionable: bool,
    meta: String,
    show_ms: u32,
}

/// Shows (or replaces) the popup in the bottom-right corner of the primary monitor.
pub fn show(app: &AppHandle, sidecar: Option<PathBuf>, preview: Preview, meta: String) {
    if sidecar.is_some() && !app.state::<crate::AppState>().cfg().show_popup {
        return; // popup disabled in settings (notices still show)
    }
    *app.state::<PopupState>().inner.lock().unwrap() = Some(Pending { sidecar, preview, meta });
    // Building a webview can take ~1 s on first use; never hold up the capture state machine for it.
    let app = app.clone();
    std::thread::spawn(move || build(&app));
}

fn build(app: &AppHandle) {
    if let Some(old) = app.get_webview_window("popup") {
        let _ = old.destroy();
        // destroy is asynchronous; the label must be free before we can reuse it
        for _ in 0..50 {
            if app.get_webview_window("popup").is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    let (x, y) = corner(app, W, H, 68.0);
    let built = WebviewWindowBuilder::new(app, "popup", WebviewUrl::App("popup.html".into()))
        .title("Stash")
        .inner_size(W, H)
        .position(x, y)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .transparent(true)
        .shadow(false)
        .focusable(false)
        .focused(false)
        .build();
    if let Err(e) = built {
        tracing::error!("popup: {e}");
    }
}

pub fn notice(app: &AppHandle, msg: &str) {
    show(app, None, Preview::Notice(msg.to_string()), String::new());
}

/// Logical top-left for a window of (w, h) in the bottom-right of the primary monitor.
pub fn corner(app: &AppHandle, w: f64, h: f64, margin: f64) -> (f64, f64) {
    let corner = app.state::<crate::AppState>().cfg().appearance.popup_corner;
    match app.primary_monitor() {
        Ok(Some(m)) => {
            let s = m.scale_factor();
            let (p, sz) = (m.position(), m.size());
            let (left, top) = (p.x as f64 / s, p.y as f64 / s);
            let (right, bottom) = (left + sz.width as f64 / s, top + sz.height as f64 / s);
            let x = if corner.ends_with('l') { left + 16.0 } else { right - w - 16.0 };
            let y = if corner.starts_with('t') { top + 16.0 } else { bottom - h - margin };
            (x, y)
        }
        _ => (100.0, 100.0),
    }
}

#[tauri::command]
pub fn popup_data(state: State<PopupState>, app: AppHandle) -> Option<PopupData> {
    let show_ms = app.state::<crate::AppState>().cfg().appearance.popup_ms;
    state.inner.lock().unwrap().as_ref().map(|p| match &p.preview {
        Preview::Image(u) => PopupData { kind: "image", data: u.clone(), captionable: p.sidecar.is_some(), meta: p.meta.clone(), show_ms },
        Preview::Text(t) => PopupData { kind: "text", data: t.clone(), captionable: p.sidecar.is_some(), meta: p.meta.clone(), show_ms },
        Preview::Notice(t) => PopupData { kind: "notice", data: t.clone(), captionable: false, meta: String::new(), show_ms },
    })
}

/// Called when the popup is clicked: grow it for the caption field and take focus.
#[tauri::command]
pub fn popup_focus(app: AppHandle) {
    if let Some(w) = app.get_webview_window("popup") {
        let _ = w.set_focusable(true);
        let _ = w.set_size(tauri::LogicalSize::new(W, H + CAPTION_EXTRA));
        let _ = w.set_focus();
    }
}

#[tauri::command]
pub fn set_caption(caption: String, state: State<PopupState>) -> Result<(), String> {
    let path = state.inner.lock().unwrap().as_ref().and_then(|p| p.sidecar.clone());
    match path {
        Some(p) => crate::library::set_caption(&p, caption.trim()).map_err(|e| format!("{e:#}")),
        None => Err("no capture".into()),
    }
}

#[tauri::command]
pub fn close_popup(app: AppHandle) {
    if let Some(w) = app.get_webview_window("popup") {
        let _ = w.destroy();
    }
}
