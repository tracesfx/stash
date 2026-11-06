//! The shared library: `<root>/YYYY-MM/<id>.<ext>` + `<id>.json` sidecar. Plain files, atomic writes.
use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::Engine;
use chrono::{Local, SecondsFormat};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::clip::Clip;
use crate::winutil::{MonitorDesc, SourceInfo};

const SCHEMA: u32 = 1;

pub struct ImageMeta {
    pub method: &'static str,
    pub source: SourceInfo,
    pub monitor: MonitorDesc,
    /// Capture rect in virtual-desktop pixels: [x, y, w, h].
    pub rect: [i32; 4],
}

/// What the confirmation popup shows.
pub enum Preview {
    Image(String), // PNG data URL
    Text(String),
    Notice(String),
}

pub struct Saved {
    pub sidecar: PathBuf,
    pub preview: Preview,
    /// One-line description for the popup, e.g. "Region · 1115×628 · chrome".
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
    Ok(sidecar)
}

/// Saves an RGBA image (alpha ignored).
pub fn save_image(root: &Path, rgba: &[u8], w: u32, h: u32, meta: ImageMeta) -> Result<Saved> {
    let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let png = encode_png(&rgb, w, h)?;
    let sidecar = put(
        root,
        Item {
            kind: "image",
            ext: "png",
            mime: "image/png",
            method: meta.method,
            source: &meta.source,
            original_path: None,
            extra: json!({ "image": { "w": w, "h": h, "monitor": meta.monitor.name, "scale": meta.monitor.scale, "rect": meta.rect } }),
        },
        |p| write_atomic(p, &png),
    )?;
    let meta = format!("{} · {w}×{h}{}", method_label(meta.method), via(&meta.source));
    Ok(Saved { sidecar, preview: Preview::Image(thumbnail_data_url(&rgb, w, h)?), meta })
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
    Ok(Saved { sidecar, preview: Preview::Text(format!("📄 {name}")), meta })
}

/// Saves clipboard content of any supported kind. Returns the last item saved and how many were saved.
pub fn save_clip(root: &Path, clip: Clip, method: &'static str, source: &SourceInfo, monitor: MonitorDesc) -> Result<(Saved, usize)> {
    match clip {
        Clip::Text(t) => Ok((save_text(root, &t, method, source)?, 1)),
        Clip::Image { rgba, w, h } => {
            let meta = ImageMeta { method, source: source.clone(), monitor, rect: [0, 0, w as i32, h as i32] };
            Ok((save_image(root, &rgba, w, h, meta)?, 1))
        }
        Clip::Files(files) => {
            let mut last = None;
            for f in &files {
                last = Some(save_file(root, f, method, source)?);
            }
            let n = files.len();
            Ok((last.context("no files")?, n))
        }
    }
}

fn thumbnail_data_url(rgb: &[u8], w: u32, h: u32) -> Result<String> {
    let img = image::RgbImage::from_raw(w, h, rgb.to_vec()).context("thumb buffer")?;
    let t = image::imageops::thumbnail(&img, 560, 340);
    let png = encode_png(t.as_raw(), t.width(), t.height())?;
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}

/// Sets the caption in an existing sidecar, preserving all other (even unknown) fields.
pub fn set_caption(sidecar: &Path, caption: &str) -> Result<()> {
    let mut v = read_json(sidecar)?;
    v["caption"] = Value::String(caption.to_string());
    v["modified"] = Value::String(Local::now().to_rfc3339_opts(SecondsFormat::Millis, true));
    write_atomic(sidecar, serde_json::to_string_pretty(&v)?.as_bytes())
}
