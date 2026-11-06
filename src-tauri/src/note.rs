//! Quick note: a tiny single-line input in the corner. Enter saves a text item, Esc discards.
//! The one intentional window that takes focus.
use std::sync::Mutex;

use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::winutil::SourceInfo;
use crate::{library, popup, AppState};

#[derive(Default)]
pub struct NoteState(Mutex<Option<SourceInfo>>);

pub fn open(app: &AppHandle, source: SourceInfo) {
    *app.state::<NoteState>().0.lock().unwrap() = Some(source);
    let app = app.clone();
    std::thread::spawn(move || build(&app));
}

fn build(app: &AppHandle) {
    if let Some(old) = app.get_webview_window("note") {
        let _ = old.destroy();
        // destroy is asynchronous; the label must be free before we can reuse it
        for _ in 0..50 {
            if app.get_webview_window("note").is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    let (w, h) = (404.0, 78.0); // 12 px margin around the card for its shadow
    let (x, y) = popup::corner(app, w, h, 80.0);
    let built = WebviewWindowBuilder::new(app, "note", WebviewUrl::App("note.html".into()))
        .title("Stash note")
        .inner_size(w, h)
        .position(x, y)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .transparent(true)
        .shadow(false)
        .focused(true)
        .build();
    match built {
        Ok(win) => {
            let _ = win.set_focus();
        }
        Err(e) => tracing::error!("note window: {e}"),
    }
}

#[tauri::command]
pub fn note_save(text: String, app: AppHandle, state: State<NoteState>) -> Result<(), String> {
    let text = text.trim().to_string();
    if let Some(w) = app.get_webview_window("note") {
        let _ = w.destroy();
    }
    if text.is_empty() {
        return Ok(());
    }
    let source = state.0.lock().unwrap().clone().unwrap_or_default();
    let root = app.state::<AppState>().cfg().library_path;
    let saved = library::save_text(&root, &text, "note", &source).map_err(|e| format!("{e:#}"))?;
    popup::show(&app, Some(saved.sidecar), saved.preview, saved.meta);
    Ok(())
}

#[tauri::command]
pub fn note_close(app: AppHandle) {
    if let Some(w) = app.get_webview_window("note") {
        let _ = w.destroy();
    }
}
