//! Gallery: scans the library (sidecars are the source of truth), serves thumbnails over a custom
//! `stash://` protocol, and edits metadata / trash / import. The window exists only while open.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use chrono::Local;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

use crate::{library, winutil::SourceInfo, AppState};

#[derive(Clone)]
struct Entry {
    content: PathBuf,
    sidecar: PathBuf,
    /// Image still needs a palette and/or OCR text.
    needs: bool,
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
    palette: Vec<String>,
    board: Option<String>,
    /// Text recognised inside the image (OCR), for search.
    ocr: Option<String>,
    /// Whiteboard with an editable scene next to the PNG.
    scene: bool,
}

fn root(app: &AppHandle) -> PathBuf {
    app.state::<AppState>().cfg().library_path
}

/// First `max` bytes of a file as lossy UTF-8 (never loads big files whole).
fn read_head(p: &Path, max: usize) -> Option<String> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(max);
    std::fs::File::open(p).ok()?.take(max as u64).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Walks `<root>/YYYY-MM/*.json`. Skips dot-folders (.trash, .thumbs).
fn scan(root: &Path, ocr_on: bool) -> (Vec<ItemDto>, HashMap<String, Entry>) {
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
            let needs = kind == "image" && (v["palette"].as_array().map(|a| a.is_empty()).unwrap_or(true) || (ocr_on && v.get("ocr").is_none()));
            let ext_s = v["file"]["ext"].as_str().unwrap_or("");
            let text = (matches!(kind.as_str(), "text" | "link") || (kind == "file" && is_text_ext(ext_s)))
                .then(|| read_head(&content, 4096))
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
                palette: v["palette"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default(),
                board: v["board"].as_str().filter(|b| !b.is_empty()).map(String::from),
                ocr: v["ocr"].as_str().filter(|t| !t.is_empty()).map(|t| t.chars().take(4000).collect()),
                scene: content.with_extension("board").exists(),
            });
            map.insert(id, Entry { content, sidecar: sp, needs });
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
    let ext = content.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp") {
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
            .disable_drag_drop_handler() // HTML5 drops (text, links, browser files) are handled by the page
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
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "m4a" => "audio/mp4",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        e if is_text_ext(e) => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Extensions previewed as plain text (also used to snippet them on cards).
pub fn is_text_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "txt" | "md" | "markdown" | "json" | "csv" | "tsv" | "log" | "xml" | "yaml" | "yml" | "toml" | "ini" | "html" | "htm" | "css"
            | "js" | "ts" | "tsx" | "jsx" | "rs" | "py" | "c" | "h" | "cpp" | "cs" | "java" | "go" | "sh" | "ps1" | "bat" | "sql" | "srt" | "vtt"
    )
}

pub struct Served {
    pub status: u16,
    pub mime: &'static str,
    pub body: Vec<u8>,
    pub content_range: Option<String>,
}

fn not_found() -> Served {
    Served { status: 404, mime: "text/plain", body: Vec::new(), content_range: None }
}

/// Serves `thumb/<id>` and `full/<id>`. Honours HTTP Range so <video>/<audio> can seek without loading the file.
pub fn serve(app: &AppHandle, uri_path: &str, range: Option<&str>) -> Served {
    use std::io::{Read, Seek, SeekFrom};
    let mut parts = uri_path.trim_start_matches('/').splitn(2, '/');
    let (kind, id) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let path = if kind == "media" {
        // library-relative media (whiteboards reference videos/images this way): no traversal, media types only
        let ext = id.rsplit('.').next().unwrap_or("");
        let ok_ext = matches!(mime_for_ext(ext), m if m.starts_with("video/") || m.starts_with("image/") || m.starts_with("audio/"));
        if id.contains("..") || id.contains(':') || id.contains('\\') || !ok_ext {
            return not_found();
        }
        root(app).join(id)
    } else {
        let entry = app.state::<GalleryState>().index.lock().unwrap().get(id).cloned();
        let Some(e) = entry else { return not_found() };
        if kind == "thumb" { make_thumb(&root(app), id, &e.content).unwrap_or(e.content.clone()) } else { e.content.clone() }
    };
    let mime = mime_for_ext(path.extension().and_then(|x| x.to_str()).unwrap_or(""));
    let Ok(mut f) = std::fs::File::open(&path) else { return not_found() };
    let total = f.metadata().map(|m| m.len()).unwrap_or(0);
    let parsed = range.and_then(|r| r.strip_prefix("bytes=")).and_then(|r| {
        let (a, b) = r.split_once('-')?;
        let start: u64 = if a.is_empty() { total.saturating_sub(b.parse().ok()?) } else { a.parse().ok()? };
        let end: u64 = if a.is_empty() || b.is_empty() { total.saturating_sub(1) } else { b.parse().ok()? };
        Some((start, end.min(total.saturating_sub(1))))
    });
    match parsed {
        Some((start, end)) if start <= end && start < total => {
            let end = end.min(start + 4 * 1024 * 1024 - 1); // 4 MB chunks keep memory flat
            let mut body = vec![0u8; (end - start + 1) as usize];
            if f.seek(SeekFrom::Start(start)).and_then(|_| f.read_exact(&mut body)).is_err() {
                return not_found();
            }
            Served { status: 206, mime, body, content_range: Some(format!("bytes {start}-{end}/{total}")) }
        }
        _ => {
            let mut body = Vec::with_capacity(total as usize);
            if f.read_to_end(&mut body).is_err() {
                return not_found();
            }
            Served { status: 200, mime, body, content_range: None }
        }
    }
}

// ---------- commands ----------

/// Opens a new whiteboard pre-filled with the selected items (images, videos, text).
#[tauri::command]
pub fn gallery_new_board(ids: Vec<String>, app: AppHandle, state: State<GalleryState>) -> Result<(), String> {
    let root = root(&app);
    let mut seeds = Vec::new();
    for id in ids {
        let Ok(e) = entry(&state, &id) else { continue };
        let Ok(v) = library::read_json(&e.sidecar) else { continue };
        let ext = v["file"]["ext"].as_str().unwrap_or("").to_ascii_lowercase();
        let kind = s(&v, "type");
        let rel = e.content.strip_prefix(&root).ok().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        let (w, h) = (v["image"]["w"].as_u64().unwrap_or(0), v["image"]["h"].as_u64().unwrap_or(0));
        let seed = if kind == "image" {
            crate::board::Seed { t: "image".into(), rel, text: String::new(), w, h }
        } else if matches!(ext.as_str(), "mp4" | "m4v" | "webm" | "mov") {
            crate::board::Seed { t: "video".into(), rel, text: String::new(), w, h }
        } else if matches!(kind.as_str(), "text" | "link") || (kind == "file" && is_text_ext(&ext)) {
            let text = read_head(&e.content, 1200).unwrap_or_default().replace("[InternetShortcut]\r\nURL=", "");
            crate::board::Seed { t: "note".into(), rel: String::new(), text, w: 0, h: 0 }
        } else {
            continue;
        };
        seeds.push(seed);
    }
    if seeds.is_empty() {
        return Err("Nothing here can go on a whiteboard (images, videos and text can)".into());
    }
    crate::board::open_with(&app, None, seeds);
    Ok(())
}

#[tauri::command]
pub fn gallery_edit_board(id: String, app: AppHandle, state: State<GalleryState>) -> Result<(), String> {
    let e = entry(&state, &id).map_err(|e| format!("{e:#}"))?;
    crate::board::open(&app, Some(e.sidecar));
    Ok(())
}

#[tauri::command]
pub async fn gallery_list(app: AppHandle, state: State<'_, GalleryState>) -> Result<Vec<ItemDto>, String> {
    let root = root(&app);
    let ocr_on = app.state::<AppState>().cfg().ocr;
    let (items, map) = tauri::async_runtime::spawn_blocking(move || scan(&root, ocr_on)).await.map_err(|e| e.to_string())?;
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
        let mut clean: Vec<String> = Vec::new();
        for t in tags {
            let t = t.trim().trim_start_matches('#').to_string();
            if !t.is_empty() && !clean.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                clean.push(t);
            }
        }
        library::update_sidecar(&e.sidecar, |v| {
            v["caption"] = Value::String(caption.trim().to_string());
            v["tags"] = serde_json::json!(clean);
        })
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


/// Ctrl+V in the gallery: stash whatever is on the clipboard (files, image or text/link).
#[tauri::command]
pub fn gallery_paste(app: AppHandle) -> Result<String, String> {
    err((|| {
        let got = crate::clip::read().ok_or_else(|| anyhow!("The clipboard is empty."))?;
        let (saved, n) = crate::clip::save(&root(&app), got, "paste", &SourceInfo::default())?;
        for s in &saved {
            crate::ocr::spawn(&app, s.sidecar.clone());
        }
        Ok(if n == 1 { "Pasted 1 item".to_string() } else { format!("Pasted {n} items") })
    })())
}

/// A file dragged in from somewhere that has no path (a browser, a zip viewer): raw bytes in the request body,
/// the file name in the `x-name` header.
#[tauri::command]
pub fn gallery_import_data(request: tauri::ipc::Request<'_>, app: AppHandle) -> Result<(), String> {
    err((|| {
        let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else { return Err(anyhow!("expected raw bytes")) };
        let raw = request.headers().get("x-name").and_then(|v| v.to_str().ok()).unwrap_or("dropped");
        let name: String = urlencoding_decode(raw);
        // keep only the file name; never let a dropped name walk out of the temp folder
        let name = Path::new(&name).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "dropped".into());
        let dir = crate::config::app_dir().join("tmp");
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join(&name);
        std::fs::write(&tmp, bytes)?;
        let mut ext_fixed = tmp.clone();
        if ext_fixed.extension().is_none() {
            ext_fixed.set_extension("png"); // clipboard-style blobs arrive as "image.png" normally; be safe
            std::fs::rename(&tmp, &ext_fixed)?;
        }
        let r = library::save_file(&root(&app), &ext_fixed, "drop", &SourceInfo::default());
        let _ = std::fs::remove_file(&ext_fixed);
        let saved = r?;
        // the temp path is meaningless; do not record it as the original location
        let _ = library::update_sidecar(&saved.sidecar, |v| {
            if let Some(f) = v["file"].as_object_mut() {
                f.remove("original_path");
            }
        });
        crate::ocr::spawn(&app, saved.sidecar);
        Ok(())
    })())
}

/// Text, a URL or a link dragged in from a browser or editor.
#[tauri::command]
pub fn gallery_import_text(text: String, app: AppHandle) -> Result<(), String> {
    err((|| {
        if text.trim().is_empty() {
            return Err(anyhow!("nothing to add"));
        }
        library::save_text(&root(&app), &text, "drop", &SourceInfo::default())?;
        Ok(())
    })())
}

/// Minimal %XX decoder for the `x-name` header (HTTP headers must be ASCII; the page encodes with encodeURIComponent).
fn urlencoding_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
// ---------- palette + OCR backfill ----------

#[derive(Serialize)]
pub struct Enriched {
    id: String,
    palette: Vec<String>,
    ocr: Option<String>,
}

#[derive(Serialize)]
pub struct Backfill {
    done: Vec<Enriched>,
    remaining: usize,
}

/// Computes missing palettes / OCR text for a few images per call (the gallery loops until `remaining` is 0).
#[tauri::command]
pub async fn gallery_backfill(limit: usize, app: AppHandle, state: State<'_, GalleryState>) -> Result<Backfill, String> {
    let ocr_on = app.state::<AppState>().cfg().ocr;
    let batch: Vec<(String, Entry)> = {
        let idx = state.index.lock().unwrap();
        idx.iter().filter(|(_, e)| e.needs).take(limit).map(|(k, e)| (k.clone(), e.clone())).collect()
    };
    let finished: Vec<String> = batch.iter().map(|(k, _)| k.clone()).collect();
    let done = tauri::async_runtime::spawn_blocking(move || {
        let mut out = Vec::new();
        for (id, e) in batch {
            let palette = image::open(&e.content)
                .ok()
                .map(|img| {
                    let t = img.thumbnail(160, 160).to_rgb8();
                    crate::palette::dominant(t.as_raw(), 5)
                })
                .unwrap_or_default();
            let has_ocr_key = library::read_json(&e.sidecar).map(|v| v.get("ocr").is_some()).unwrap_or(true);
            let ocr = if ocr_on && !has_ocr_key {
                let t = crate::ocr::recognize(&e.content).unwrap_or_default();
                Some(t.split_whitespace().collect::<Vec<_>>().join(" "))
            } else {
                None
            };
            let (p2, o2) = (palette.clone(), ocr.clone());
            let _ = library::update_sidecar(&e.sidecar, |v| {
                if !p2.is_empty() {
                    v["palette"] = serde_json::json!(p2);
                }
                if let Some(t) = o2 {
                    v["ocr"] = Value::String(t);
                }
            });
            out.push(Enriched { id, palette, ocr: ocr.filter(|t| !t.is_empty()) });
        }
        out
    })
    .await
    .map_err(|e| e.to_string())?;
    let mut idx = state.index.lock().unwrap();
    for id in finished {
        if let Some(e) = idx.get_mut(&id) {
            e.needs = false; // never retry within this session, even if decoding failed
        }
    }
    Ok(Backfill { done, remaining: idx.values().filter(|e| e.needs).count() })
}

// ---------- boards ----------

/// Puts items on a board (a named collection); an empty name removes them from their board.
#[tauri::command]
pub fn gallery_set_board(ids: Vec<String>, board: String, state: State<GalleryState>) -> Result<(), String> {
    err((|| {
        let board = board.trim().to_string();
        for id in ids {
            let e = entry(&state, &id)?;
            library::update_sidecar(&e.sidecar, |v| {
                if board.is_empty() {
                    if let Value::Object(m) = v {
                        m.remove("board");
                    }
                } else {
                    v["board"] = Value::String(board.clone());
                }
            })?;
        }
        Ok(())
    })())
}

// ---------- export ----------

fn csv(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\"").replace(['\r', '\n'], " "))
}

/// Exports items to a new folder: the files, an index.md (images embedded, captions, tags, sources) and a
/// notion.csv that Notion can import as a database. Miro/Figma: drop the files folder onto the board.
#[tauri::command]
pub fn gallery_export(ids: Vec<String>, dest: String, state: State<GalleryState>) -> Result<String, String> {
    err((|| {
        let folder = PathBuf::from(&dest).join(format!("Stash export {}", Local::now().format("%Y-%m-%d %H-%M-%S")));
        let files = folder.join("files");
        std::fs::create_dir_all(&files)?;
        let mut md = format!("# Stash export — {} items\n\n", ids.len());
        let mut rows = String::from("Name,Caption,Type,Tags,Board,Source URL,Source App,Saved,File,Text\r\n");
        for id in &ids {
            let e = entry(&state, id)?;
            let v = library::read_json(&e.sidecar)?;
            let kind = s(&v, "type");
            let caption = s(&v, "caption");
            let tags: Vec<String> = v["tags"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(|t| format!("#{t}"))).collect()).unwrap_or_default();
            let board = s(&v, "board");
            let url = v["source"]["url"].as_str().unwrap_or("").to_string();
            let app_name = v["source"]["process"].as_str().unwrap_or("").to_string();
            let created = s(&v, "created");
            let ext = e.content.extension().and_then(|x| x.to_str()).unwrap_or("bin");
            let title = if caption.is_empty() { id.clone() } else { caption.clone() };
            let (rel, text) = if matches!(kind.as_str(), "text" | "link") {
                (String::new(), std::fs::read_to_string(&e.content).unwrap_or_default().replace("[InternetShortcut]\r\nURL=", "").trim().to_string())
            } else {
                let name = format!("{id}.{ext}");
                std::fs::copy(&e.content, files.join(&name))?;
                (format!("files/{name}"), String::new())
            };
            md.push_str(&format!("## {title}\n\n"));
            if kind == "image" {
                md.push_str(&format!("![{title}]({rel})\n\n"));
            } else if !rel.is_empty() {
                md.push_str(&format!("[{rel}]({rel})\n\n"));
            } else {
                md.push_str(&format!("> {}\n\n", text.replace('\n', "\n> ")));
            }
            if !tags.is_empty() {
                md.push_str(&format!("- Tags: {}\n", tags.join(" ")));
            }
            if !board.is_empty() {
                md.push_str(&format!("- Board: {board}\n"));
            }
            if !url.is_empty() {
                md.push_str(&format!("- Source: <{url}>\n"));
            }
            md.push_str(&format!("- Saved: {created}\n\n"));
            rows.push_str(&[csv(&title), csv(&caption), csv(&kind), csv(&tags.join(" ")), csv(&board), csv(&url), csv(&app_name), csv(&created), csv(&rel), csv(&text)].join(","));
            rows.push_str("\r\n");
        }
        std::fs::write(folder.join("index.md"), md)?;
        std::fs::write(folder.join("notion.csv"), format!("\u{feff}{rows}"))?; // BOM so Excel/Notion read UTF-8
        Ok(folder.to_string_lossy().into_owned())
    })())
}

/// Opens a folder in Explorer (used for the export result).
#[tauri::command]
pub fn gallery_open_path(path: String, app: AppHandle) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.is_dir() {
        return Err("not a folder".into());
    }
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}
