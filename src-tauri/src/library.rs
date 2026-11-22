//! The shared library: `<root>/YYYY-MM/<id>.<ext>` + `<id>.json` sidecar. Plain files, atomic writes.
use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{Local, SecondsFormat};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::winutil::{MonitorDesc, SourceInfo};

const SCHEMA: u32 = 1;

pub struct ImageMeta {
    pub method: &'static str,
    pub source: SourceInfo,
    pub monitor: MonitorDesc,
    /// Capture rect in virtual-desktop pixels: [x, y, w, h].
    pub rect: [i32; 4],
}

/// Small RGBA thumbnail (straight alpha) for native feedback windows.
pub struct Thumb {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

/// What the tag radial / toast shows for a fresh capture.
#[derive(Clone)]
pub enum Preview {
    Image(std::sync::Arc<Thumb>),
    Text(String),
}

pub struct Saved {
    pub sidecar: PathBuf,
    pub preview: Preview,
    /// One-line description, e.g. "Region · 1115×628 · chrome".
    pub meta: String,
}

fn method_label(m: &str) -> &str {
    match m {
        "snip" => "Region",
        "window" => "Window",
        "screen" => "Screen",
        "selection" => "Selection",
        "clipboard" => "Clipboard",
        "note" => "Note",
        "drop" => "Dropped",
        "record" => "Clip",
        "board" => "Board",
        o => o,
    }
}

fn via(source: &SourceInfo) -> String {
    if source.process.is_empty() {
        String::new()
    } else {
        format!(" · {}", source.process.trim_end_matches(".exe"))
    }
}

fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(Local::now().timestamp_nanos_opt().unwrap_or(0) as u128);
    format!("{}-{:04x}", Local::now().format("%Y%m%d-%H%M%S"), h.finish() & 0xFFFF)
}

/// Reads a sidecar, tolerating a UTF-8 BOM written by other tools.
pub fn read_json(path: &Path) -> Result<Value> {
    let raw = std::fs::read(path)?;
    Ok(serde_json::from_slice(raw.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&raw))?)
}

pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(p: &Path) -> Result<String> {
    let mut h = Sha256::new();
    std::io::copy(&mut std::fs::File::open(p)?, &mut h)?;
    Ok(hex(&h.finalize()))
}

fn encode_png(rgb: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    PngEncoder::new_with_quality(Cursor::new(&mut out), CompressionType::Fast, FilterType::Adaptive)
        .write_image(rgb, w, h, ExtendedColorType::Rgb8)?;
    Ok(out)
}

struct Item<'a> {
    kind: &'a str,
    ext: &'a str,
    mime: &'a str,
    method: &'a str,
    source: &'a SourceInfo,
    original_path: Option<String>,
    /// Extra top-level sidecar sections (image / text).
    extra: Value,
}

/// Writes the content file (via `write`), then the sidecar. Content first, sidecar last.
fn put(root: &Path, it: Item, write: impl FnOnce(&Path) -> Result<()>) -> Result<PathBuf> {
    let now = Local::now();
    let id = new_id();
    let dir = root.join(now.format("%Y-%m").to_string());
    std::fs::create_dir_all(&dir).context("create month dir")?;
    let file_name = format!("{id}.{}", it.ext);
    let path = dir.join(&file_name);
    write(&path)?;
    let bytes = std::fs::metadata(&path)?.len();
    let ts = now.to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut v = json!({
        "schema": SCHEMA, "id": id, "type": it.kind, "created": ts, "modified": ts,
        "method": it.method, "caption": "", "tags": [], "source": it.source,
        "file": { "name": file_name, "ext": it.ext, "bytes": bytes, "sha256": sha256_file(&path)?, "mime": it.mime },
        "palette": [],
    });
    if let Some(p) = it.original_path {
        v["file"]["original_path"] = Value::String(p);
    }
    if let Value::Object(extra) = it.extra {
        for (k, val) in extra {
            v[k] = val;
        }
    }
    let sidecar = dir.join(format!("{id}.json"));
    write_atomic(&sidecar, serde_json::to_string_pretty(&v)?.as_bytes())?;
    notify_changed();
    Ok(sidecar)
}

/// Saves an RGBA image (alpha ignored).
pub fn save_image(root: &Path, rgba: &[u8], w: u32, h: u32, meta: ImageMeta) -> Result<Saved> {
    let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let png = encode_png(&rgb, w, h)?;
    let (thumb, palette) = thumb_and_palette(&rgb, w, h)?;
    let sidecar = put(
        root,
        Item {
            kind: "image",
            ext: "png",
            mime: "image/png",
            method: meta.method,
            source: &meta.source,
            original_path: None,
            extra: json!({ "image": { "w": w, "h": h, "monitor": meta.monitor.name, "scale": meta.monitor.scale, "rect": meta.rect }, "palette": palette }),
        },
        |p| write_atomic(p, &png),
    )?;
    let meta = format!("{} · {w}×{h}{}", method_label(meta.method), via(&meta.source));
    Ok(Saved { sidecar, preview: Preview::Image(thumb), meta })
}

/// Saves a recorded MP4 clip (kind "file"; the first frame is the feedback thumbnail).
pub fn save_video(root: &Path, src: &Path, rec: &crate::record::Recorded, source: &SourceInfo, monitor: &MonitorDesc) -> Result<Saved> {
    let (thumb, _) = thumb_and_palette(&rec.first, rec.w, rec.h)?;
    let sidecar = put(
        root,
        Item {
            kind: "file",
            ext: "mp4",
            mime: "video/mp4",
            method: "record",
            source,
            original_path: None,
            extra: json!({
                "image": { "w": rec.w, "h": rec.h, "monitor": monitor.name, "scale": monitor.scale },
                "video": { "secs": (rec.secs * 10.0).round() / 10.0 },
            }),
        },
        |p| {
            if std::fs::rename(src, p).is_err() {
                std::fs::copy(src, p)?; // different volume
            }
            Ok(())
        },
    )?;
    let meta = format!("Clip · {}×{} · {:.0}s{}", rec.w, rec.h, rec.secs, via(source));
    Ok(Saved { sidecar, preview: Preview::Image(thumb), meta })
}

fn is_url(s: &str) -> bool {
    let s = s.trim();
    (s.starts_with("http://") || s.starts_with("https://")) && !s.contains(char::is_whitespace)
}

/// Saves text (or a link if it is a bare URL).
pub fn save_text(root: &Path, text: &str, method: &str, source: &SourceInfo) -> Result<Saved> {
    let text = text.trim();
    let link = is_url(text);
    let (kind, ext, mime, body) = if link {
        ("link", "url", "application/internet-shortcut", format!("[InternetShortcut]\r\nURL={text}\r\n"))
    } else {
        ("text", "md", "text/markdown", text.to_string())
    };
    let sidecar = put(
        root,
        Item { kind, ext, mime, method, source, original_path: None, extra: json!({ "text": { "chars": text.chars().count() } }) },
        |p| write_atomic(p, body.as_bytes()),
    )?;
    let label = if link { "Link" } else { method_label(method) };
    let meta = format!("{label} · {} chars{}", text.chars().count(), via(source));
    Ok(Saved { sidecar, preview: Preview::Text(text.chars().take(240).collect()), meta })
}

fn mime_for(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Copies a file into the library, recording where it came from.
pub fn save_file(root: &Path, src: &Path, method: &str, source: &SourceInfo) -> Result<Saved> {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("bin").to_string();
    let kind = if matches!(ext.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp") { "image" } else { "file" };
    let sidecar = put(
        root,
        Item {
            kind,
            ext: &ext,
            mime: mime_for(&ext),
            method,
            source,
            original_path: Some(src.to_string_lossy().into_owned()),
            extra: json!({ "original_name": src.file_name().map(|n| n.to_string_lossy().into_owned()) }),
        },
        |p| std::fs::copy(src, p).map(|_| ()).map_err(Into::into),
    )?;
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let meta = format!("{} · {name}", method_label(method));
    Ok(Saved { sidecar, preview: Preview::Text(name), meta })
}

/// Feedback thumbnail and the dominant-colour palette, from one downscale.
fn thumb_and_palette(rgb: &[u8], w: u32, h: u32) -> Result<(std::sync::Arc<Thumb>, Vec<String>)> {
    let img = image::RgbImage::from_raw(w, h, rgb.to_vec()).context("thumb buffer")?;
    let t = image::imageops::thumbnail(&img, 320, 320);
    let palette = crate::palette::dominant(t.as_raw(), 5);
    let rgba: Vec<u8> = t.as_raw().chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
    Ok((std::sync::Arc::new(Thumb { rgba, w: t.width(), h: t.height() }), palette))
}

static SIDECAR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Called when items are added, tagged or captioned outside the gallery (lib.rs emits `library-changed`, so an
/// open gallery slides new captures in instead of reloading on focus). Background palette/OCR updates do not
/// notify: the gallery gets those from its own backfill.
static NOTIFIER: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

pub fn set_notifier(f: impl Fn() + Send + Sync + 'static) {
    let _ = NOTIFIER.set(Box::new(f));
}

pub fn notify_changed() {
    if let Some(f) = NOTIFIER.get() {
        f();
    }
}

/// Read-modify-write a sidecar under a global lock (captions, tags, OCR and palette updates can race).
/// Unknown fields are preserved;  is refreshed.
pub fn update_sidecar(sidecar: &Path, f: impl FnOnce(&mut Value)) -> Result<()> {
    let _g = SIDECAR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut v = read_json(sidecar)?;
    f(&mut v);
    v["modified"] = Value::String(Local::now().to_rfc3339_opts(SecondsFormat::Millis, true));
    write_atomic(sidecar, serde_json::to_string_pretty(&v)?.as_bytes())
}

/// Sets the caption in an existing sidecar, preserving all other (even unknown) fields.
pub fn set_caption(sidecar: &Path, caption: &str) -> Result<()> {
    update_sidecar(sidecar, |v| v["caption"] = Value::String(caption.to_string()))?;
    notify_changed();
    Ok(())
}

/// Adds tags (case-insensitive de-dupe, existing order kept) to a sidecar.
pub fn add_tags(sidecar: &Path, tags: &[String]) -> Result<()> {
    update_sidecar(sidecar, |v| {
        let mut cur: Vec<String> = v["tags"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default();
        for t in tags {
            if !cur.iter().any(|c| c.eq_ignore_ascii_case(t)) {
                cur.push(t.clone());
            }
        }
        v["tags"] = json!(cur);
    })?;
    notify_changed();
    Ok(())
}
