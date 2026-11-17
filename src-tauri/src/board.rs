//! Whiteboard: an infinite-canvas editor in its own window (created on open, destroyed on close). A board is a
//! normal library image (PNG render, so it shows up everywhere) plus `<id>.board`, the editable scene JSON.
use std::path::PathBuf;
use std::sync::Mutex;

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::{library, popup, AppState};

/// Something to place on a board when it opens (from a gallery selection).
#[derive(Serialize, Clone)]
pub struct Seed {
    /// image | video | note
    pub t: String,
    /// Library-relative path (image/video).
    pub rel: String,
    pub text: String,
    pub w: u64,
    pub h: u64,
}

#[derive(Default)]
pub struct BoardState {
    seeds: Mutex<Vec<Seed>>,
    /// Sidecar of the board being edited (set after the first save, or when editing an existing board).
    sidecar: Mutex<Option<PathBuf>>,
}

#[derive(Serialize)]
pub struct BoardInit {
    scene: Option<String>,
    existing: bool,
    seeds: Vec<Seed>,
}

/// Opens the whiteboard window: a new board, or `existing` (the sidecar of a saved board).
pub fn open(app: &AppHandle, existing: Option<PathBuf>) {
    open_with(app, existing, Vec::new());
}

/// Like `open`, and places `seeds` on the board (sent as an event when the board is already open).
pub fn open_with(app: &AppHandle, existing: Option<PathBuf>, seeds: Vec<Seed>) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(w) = app.get_webview_window("board") {
            if !seeds.is_empty() {
                let _ = tauri::Emitter::emit(&w, "board-seed", seeds);
            }
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
            return;
        }
        *app.state::<BoardState>().sidecar.lock().unwrap() = existing;
        *app.state::<BoardState>().seeds.lock().unwrap() = seeds;
        let cfg = app.state::<AppState>().cfg();
        let built = WebviewWindowBuilder::new(&app, "board", WebviewUrl::App("board.html".into()))
            .title("Stash whiteboard")
            .inner_size(1180.0, 780.0)
            .min_inner_size(520.0, 360.0)
            .transparent(true)
            .effects(crate::theme::builder_effects(&cfg))
            .build();
        match built {
            Ok(w) => crate::theme::style_window(&w, &cfg),
            Err(e) => tracing::error!("board window: {e}"),
        }
    });
}

#[tauri::command]
pub fn board_init(state: State<BoardState>) -> BoardInit {
    let sc = state.sidecar.lock().unwrap().clone();
    let scene = sc.as_ref().and_then(|sc| {
        let v = library::read_json(sc).ok()?;
        let name = v["file"]["name"].as_str()?.to_string();
        std::fs::read_to_string(sc.with_file_name(name).with_extension("board")).ok()
    });
    BoardInit { scene, existing: sc.is_some(), seeds: std::mem::take(&mut *state.seeds.lock().unwrap()) }
}

/// Saves the PNG render + scene. The first save creates the library item; later saves overwrite it.
#[tauri::command]
pub fn board_save(png: String, scene: String, notify: bool, app: AppHandle, state: State<BoardState>) -> Result<(), String> {
    let b64 = png.split_once(',').map(|x| x.1).unwrap_or(&png);
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| e.to_string())?;
    let root = app.state::<AppState>().cfg().library_path;
    let existing = state.sidecar.lock().unwrap().clone();
    let saved = library::save_board(&root, existing.as_deref(), &bytes, &scene).map_err(|e| format!("{e:#}"))?;
    if let Ok(v) = library::read_json(&saved.sidecar) {
        if let Some(id) = v["id"].as_str() {
            let _ = std::fs::remove_file(root.join(".thumbs").join(format!("{id}.png"))); // stale gallery thumbnail
        }
    }
    *state.sidecar.lock().unwrap() = Some(saved.sidecar.clone());
    if notify {
        popup::show(&app, Some(saved.sidecar), saved.preview, saved.meta);
    }
    Ok(())
}

#[tauri::command]
pub fn board_close(app: AppHandle) {
    if let Some(w) = app.get_webview_window("board") {
        let _ = w.destroy();
    }
}

/// Library-relative path for a video file dropped on a board (imports it into the library if it lives elsewhere).
#[tauri::command]
pub fn board_add_video(path: String, app: AppHandle) -> Result<String, String> {
    let root = app.state::<AppState>().cfg().library_path;
    let p = PathBuf::from(&path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "mp4" | "m4v" | "webm" | "mov") {
        return Err("not a video".into());
    }
    let (rootc, pc) = (root.canonicalize().map_err(|e| e.to_string())?, p.canonicalize().map_err(|e| e.to_string())?);
    if let Ok(rel) = pc.strip_prefix(&rootc) {
        return Ok(rel.to_string_lossy().replace('\\', "/"));
    }
    let saved = library::save_file(&root, &p, "drop", &Default::default()).map_err(|e| format!("{e:#}"))?;
    let v = library::read_json(&saved.sidecar).map_err(|e| e.to_string())?;
    let name = v["file"]["name"].as_str().ok_or("bad sidecar")?;
    let month = saved.sidecar.parent().and_then(|m| m.file_name()).map(|m| m.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(format!("{month}/{name}"))
}

/// Reads an image file (dropped onto the board) as a data URL.
#[tauri::command]
pub fn board_read_image(path: String) -> Result<String, String> {
    let p = PathBuf::from(&path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => return Err("not an image".into()),
    };
    let len = std::fs::metadata(&p).map_err(|e| e.to_string())?.len();
    if len > 30 * 1024 * 1024 {
        return Err("image too large".into());
    }
    let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}
