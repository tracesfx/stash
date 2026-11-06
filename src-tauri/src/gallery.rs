//! Gallery: scans the library (sidecars are the source of truth), serves thumbnails over a custom
//! `stash://` protocol, and edits metadata / trash / import. The window exists only while open.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use chrono::{Local, SecondsFormat};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

use crate::{library, winutil::SourceInfo, AppState};

#[derive(Clone)]
struct Entry {
    content: PathBuf,
    sidecar: PathBuf,
}

#[derive(Default)]
pub struct GalleryState {
    index: Mutex<HashMap<String, Entry>>,
}

#[derive(Serialize)]
pub struct ItemDto {
    id: String,
    kind: String,
    created: String,
    method: String,
    caption: String,
    tags: Vec<String>,
    source_title: String,
    process: String,
    url: Option<String>,
    ext: String,
    name: String,
    bytes: u64,
    mime: String,
    w: Option<u64>,
    h: Option<u64>,
    original_path: Option<String>,
    /// Text/link body preview (text and link items only).
    text: Option<String>,
    path: String,
    /// Image used as the OS drag icon.
    icon: String,
}

fn root(app: &AppHandle) -> PathBuf {
    app.state::<AppState>().cfg().library_path
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Walks `<root>/YYYY-MM/*.json`. Skips dot-folders (.trash, .thumbs).
fn scan(root: &Path) -> (Vec<ItemDto>, HashMap<String, Entry>) {
    let mut items = Vec::new();
    let mut map = HashMap::new();
    let generic_icon = ensure_generic_icon(root);
    let Ok(months) = std::fs::read_dir(root) else { return (items, map) };
    for m in months.flatten() {
        let mp = m.path();
        if !mp.is_dir() || m.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&mp) else { continue };
        for f in files.flatten() {
            let sp = f.path();
            if sp.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read(&sp) else { continue };
            let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&raw); // tolerate a UTF-8 BOM from other tools
            let Ok(v) = serde_json::from_slice::<Value>(raw) else { continue };
            let id = s(&v, "id");
            let name = v["file"]["name"].as_str().unwrap_or("").to_string();
            if id.is_empty() || name.is_empty() {
                continue;
            }
            let content = mp.join(&name);
            if !content.exists() {
                continue;
            }
            let kind = s(&v, "type");
            let text = matches!(kind.as_str(), "text" | "link")
                .then(|| std::fs::read_to_string(&content).ok())
                .flatten()
                .map(|t| t.replace("[InternetShortcut]\r\nURL=", "").chars().take(400).collect());
            let thumb = thumb_path(root, &id);
            let icon = if thumb.exists() { thumb } else { generic_icon.clone() };
            items.push(ItemDto {
                id: id.clone(),
                kind,
                created: s(&v, "created"),
                method: s(&v, "method"),
                caption: s(&v, "caption"),
                tags: v["tags"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default(),
                source_title: v["source"]["title"].as_str().unwrap_or("").to_string(),
                process: v["source"]["process"].as_str().unwrap_or("").to_string(),
                url: v["source"]["url"].as_str().map(String::from),
                ext: v["file"]["ext"].as_str().unwrap_or("").to_string(),
                name,
                bytes: v["file"]["bytes"].as_u64().unwrap_or(0),
                mime: v["file"]["mime"].as_str().unwrap_or("").to_string(),
                w: v["image"]["w"].as_u64(),
                h: v["image"]["h"].as_u64(),
                original_path: v["file"]["original_path"].as_str().map(String::from),
                text,
                path: content.to_string_lossy().into_owned(),
                icon: icon.to_string_lossy().into_owned(),
            });
            map.insert(id, Entry { content, sidecar: sp });
        }
    }
    items.sort_by(|a, b| b.id.cmp(&a.id)); // ids are timestamp-sortable
    (items, map)
}

fn thumbs_dir(root: &Path) -> PathBuf {
    root.join(".thumbs")
}
fn thumb_path(root: &Path, id: &str) -> PathBuf {
    thumbs_dir(root).join(format!("{id}.png"))
}

fn ensure_generic_icon(root: &Path) -> PathBuf {
    let p = thumbs_dir(root).join("_icon.png");
    if !p.exists() {
        let _ = std::fs::create_dir_all(thumbs_dir(root));
        let img = image::RgbImage::from_pixel(64, 64, image::Rgb([74, 125, 255]));
        let _ = img.save(&p);
    }
    p
}

/// Creates (once) a <=480px PNG thumbnail. Only PNG sources can be decoded; others return None.
fn make_thumb(root: &Path, id: &str, content: &Path) -> Option<PathBuf> {
    let out = thumb_path(root, id);
    if out.exists() {
        return Some(out);
    }
    if content.extension().and_then(|e| e.to_str()).map(|e| !e.eq_ignore_ascii_case("png")).unwrap_or(true) {
        return None;
    }
    let img = image::open(content).ok()?;
    let t = img.thumbnail(480, 480).to_rgb8();
    let _ = std::fs::create_dir_all(thumbs_dir(root));
    t.save(&out).ok()?;
    Some(out)
}

// ---------- window ----------

pub fn open(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(w) = app.get_webview_window("gallery") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
            return;
        }
        let built = WebviewWindowBuilder::new(&app, "gallery", WebviewUrl::App("gallery.html".into()))
            .title("Stash")
            .inner_size(1180.0, 780.0)
            .min_inner_size(640.0, 420.0)
            .transparent(true) // the backdrop (Mica/Acrylic) shows through the translucent page
            .effects(crate::theme::builder_effects(&app.state::<AppState>().cfg()))
            .build();
        if let Err(e) = built {
            tracing::error!("gallery window: {e}");
        }
    });
}

// ---------- protocol: stash://thumb/<id>, stash://full/<id> ----------

fn mime_for_ext(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

pub fn serve(app: &AppHandle, uri_path: &str) -> (u16, &'static str, Vec<u8>) {
    let mut parts = uri_path.trim_start_matches('/').splitn(2, '/');
    let (kind, id) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let entry = app.state::<GalleryState>().index.lock().unwrap().get(id).cloned();
    let Some(e) = entry else { return (404, "text/plain", Vec::new()) };
    let path = if kind == "thumb" { make_thumb(&root(app), id, &e.content).unwrap_or(e.content.clone()) } else { e.content.clone() };
    match std::fs::read(&path) {
        Ok(bytes) => (200, mime_for_ext(path.extension().and_then(|x| x.to_str()).unwrap_or("")), bytes),
        Err(_) => (404, "text/plain", Vec::new()),
    }
}

// ---------- commands ----------

#[tauri::command]
pub async fn gallery_list(app: AppHandle, state: State<'_, GalleryState>) -> Result<Vec<ItemDto>, String> {
    let root = root(&app);
    let (items, map) = tauri::async_runtime::spawn_blocking(move || scan(&root)).await.map_err(|e| e.to_string())?;
    *state.index.lock().unwrap() = map;
    Ok(items)
}

fn entry(state: &State<GalleryState>, id: &str) -> Result<Entry> {
    state.index.lock().unwrap().get(id).cloned().ok_or_else(|| anyhow!("unknown item {id}"))
}

fn err<T>(r: Result<T>) -> Result<T, String> {
    r.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn gallery_set_meta(id: String, caption: String, tags: Vec<String>, state: State<GalleryState>) -> Result<(), String> {
    err((|| {
        let e = entry(&state, &id)?;
        let mut v = library::read_json(&e.sidecar)?;
        let mut clean: Vec<String> = Vec::new();
        for t in tags {
            let t = t.trim().trim_start_matches('#').to_string();
            if !t.is_empty() && !clean.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                clean.push(t);
            }
        }
        v["caption"] = Value::String(caption.trim().to_string());
        v["tags"] = serde_json::json!(clean);
        v["modified"] = Value::String(Local::now().to_rfc3339_opts(SecondsFormat::Millis, true));
        library::write_atomic(&e.sidecar, serde_json::to_string_pretty(&v)?.as_bytes())
    })())
}

/// Moves items to `<root>/.trash/<month>/` (recoverable). Returns the ids moved.
#[tauri::command]
pub fn gallery_delete(ids: Vec<String>, app: AppHandle, state: State<GalleryState>) -> Result<Vec<String>, String> {
    err((|| {
        let root = root(&app);
        let mut moved = Vec::new();
        for id in ids {
            let Ok(e) = entry(&state, &id) else { continue };
            let month = e.sidecar.parent().and_then(|p| p.file_name()).map(|n| n.to_owned()).context("month dir")?;
            let dest = root.join(".trash").join(month);
            std::fs::create_dir_all(&dest)?;
            std::fs::rename(&e.content, dest.join(e.content.file_name().context("name")?))?;
            std::fs::rename(&e.sidecar, dest.join(e.sidecar.file_name().context("name")?))?;
            let _ = std::fs::remove_file(thumb_path(&root, &id));
            state.index.lock().unwrap().remove(&id);
            moved.push(id);
        }
        Ok(moved)
    })())
}

/// Undo of `gallery_delete`: moves items back out of `.trash`.
#[tauri::command]
pub fn gallery_restore(ids: Vec<String>, app: AppHandle) -> Result<(), String> {
    err((|| {
        let root = root(&app);
        let trash = root.join(".trash");
        for month in std::fs::read_dir(&trash)?.flatten() {
            for f in std::fs::read_dir(month.path())?.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                if ids.iter().any(|id| name.starts_with(&format!("{id}."))) {
                    let dest = root.join(month.file_name());
                    std::fs::create_dir_all(&dest)?;
                    std::fs::rename(f.path(), dest.join(&name))?;
                }
            }
        }
        Ok(())
    })())
}

#[tauri::command]
pub fn gallery_reveal(id: String, app: AppHandle, state: State<GalleryState>) -> Result<(), String> {
    err((|| {
        let e = entry(&state, &id)?;
        app.opener().reveal_item_in_dir(&e.content).map_err(|e| anyhow!("{e}"))
    })())
}

#[tauri::command]
pub fn gallery_open_url(url: String, app: AppHandle) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http(s) URLs can be opened".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Copies an item to the clipboard: PNG image as an image, text/link as text.
#[tauri::command]
pub fn gallery_copy(id: String, state: State<GalleryState>) -> Result<(), String> {
    err((|| {
        let e = entry(&state, &id)?;
        let mut cb = arboard::Clipboard::new()?;
        let ext = e.content.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "png" {
            let img = image::open(&e.content)?.to_rgba8();
            cb.set_image(arboard::ImageData { width: img.width() as usize, height: img.height() as usize, bytes: img.into_raw().into() })?;
        } else if ext == "md" || ext == "url" {
            let t = std::fs::read_to_string(&e.content)?;
            cb.set_text(t.replace("[InternetShortcut]\r\nURL=", "").trim().to_string())?;
        } else {
            return Err(anyhow!("copy is only supported for PNG, text and links"));
        }
        Ok(())
    })())
}

/// Imports dropped files into the library (method "drop").
#[tauri::command]
pub fn gallery_import(paths: Vec<String>, app: AppHandle) -> Result<usize, String> {
    err((|| {
        let root = root(&app);
        let mut n = 0;
        for p in paths {
            let p = PathBuf::from(p);
            if p.is_file() {
                library::save_file(&root, &p, "drop", &SourceInfo::default())?;
                n += 1;
            }
        }
        Ok(n)
    })())
}
