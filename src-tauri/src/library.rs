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
        "import" => "Imported",
        "upload" => "Added",
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

/// Extensions a folder import picks up (pictures, video, audio, PDF and text-like files); everything else in the
/// folder (system files, archives, project files) is skipped.
pub fn importable(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    matches!(e.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "mp4" | "m4v" | "webm" | "mov" | "mp3" | "wav" | "ogg" | "oga" | "m4a" | "flac" | "aac" | "opus" | "weba" | "pdf")
        || crate::gallery::is_text_ext(&e)
}

/// Copies a file into the library as part of a folder import: method `import`, the folder name(s) as tags and an
/// `import` section recording where it came from. Imported items stay out of the gallery's All view by default.
pub fn import_file(root: &Path, src: &Path, tags: &[String], folder: &str) -> Result<Saved> {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("bin").to_string();
    let kind = if matches!(ext.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp") { "image" } else { "file" };
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let sidecar = put(
        root,
        Item {
            kind,
            ext: &ext,
            mime: mime_for(&ext),
            method: "import",
            source: &SourceInfo::default(),
            original_path: Some(src.to_string_lossy().into_owned()),
            extra: json!({ "original_name": name, "tags": tags, "import": { "folder": folder } }),
        },
        |p| std::fs::copy(src, p).map(|_| ()).map_err(Into::into),
    )?;
    Ok(Saved { sidecar, preview: Preview::Text(name.clone()), meta: format!("Imported · {name}") })
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

/// While > 0, `notify_changed` is a no-op (a folder import adds hundreds of items: the gallery reloads once at the end).
static QUIET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Runs `f` without per-item change notifications, then notifies once.
pub fn quietly<T>(f: impl FnOnce() -> T) -> T {
    let r = quietly_no_notify(f);
    notify_changed();
    r
}

/// Runs `f` without change notifications (the caller notifies when its whole batch is done).
pub fn quietly_no_notify<T>(f: impl FnOnce() -> T) -> T {
    use std::sync::atomic::Ordering;
    struct Done;
    impl Drop for Done {
        fn drop(&mut self) {
            QUIET.fetch_sub(1, Ordering::SeqCst);
        }
    }
    QUIET.fetch_add(1, Ordering::SeqCst);
    let _done = Done; // also on panic
    f()
}

pub fn notify_changed() {
    if QUIET.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        return;
    }
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

/// Rewrites a text note (or a link) after it was edited in the gallery: content first, then the sidecar
/// (size, hash, character count). A link must stay a single web address.
pub fn set_text(sidecar: &Path, content: &Path, text: &str) -> Result<()> {
    let v = read_json(sidecar)?;
    let kind = v["type"].as_str().unwrap_or("");
    let text = text.trim();
    let body = match kind {
        "text" => text.to_string(),
        "link" if is_url(text) => format!("[InternetShortcut]\r\nURL={text}\r\n"),
        "link" => anyhow::bail!("A link must stay a single web address"),
        _ => anyhow::bail!("only notes and links can be edited"),
    };
    if text.is_empty() {
        anyhow::bail!("A note can't be empty (delete it instead)");
    }
    write_atomic(content, body.as_bytes())?;
    let bytes = std::fs::metadata(content)?.len();
    let hash = sha256_file(content)?;
    let chars = text.chars().count();
    update_sidecar(sidecar, |v| {
        v["file"]["bytes"] = json!(bytes);
        v["file"]["sha256"] = Value::String(hash);
        v["text"]["chars"] = json!(chars);
    })?;
    notify_changed();
    Ok(())
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

/// Removes tags (case-insensitive) from a sidecar.
pub fn remove_tags(sidecar: &Path, tags: &[String]) -> Result<()> {
    update_sidecar(sidecar, |v| {
        let cur: Vec<String> = v["tags"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default();
        let kept: Vec<String> = cur.into_iter().filter(|c| !tags.iter().any(|t| t.eq_ignore_ascii_case(c))).collect();
        v["tags"] = json!(kept);
    })?;
    notify_changed();
    Ok(())
}

/// Stars or un-stars an item (sidecar `favorite: true`; the field is absent when off).
pub fn set_favorite(sidecar: &Path, on: bool) -> Result<()> {
    update_sidecar(sidecar, |v| {
        if on {
            v["favorite"] = Value::Bool(true);
        } else if let Some(m) = v.as_object_mut() {
            m.remove("favorite");
        }
    })?;
    notify_changed();
    Ok(())
}

/// Hides or shows an item (sidecar `hidden: true`; absent when shown). Hidden items only appear in the gallery's
/// Hidden view.
pub fn set_hidden(sidecar: &Path, on: bool) -> Result<()> {
    update_sidecar(sidecar, |v| {
        if on {
            v["hidden"] = Value::Bool(true);
        } else if let Some(m) = v.as_object_mut() {
            m.remove("hidden");
        }
    })
}

/// Cancels a fresh capture (the tag radial's Discard): deletes its content file, sidecar and gallery thumbnail
/// for good. Holds the sidecar lock so a tag or caption still being written cannot bring the sidecar back.
pub fn discard(sidecar: &Path) -> Result<()> {
    {
        let _g = SIDECAR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let v = read_json(sidecar)?;
        let dir = sidecar.parent().context("sidecar dir")?;
        if let Some(name) = v["file"]["name"].as_str().filter(|n| !n.is_empty() && !n.contains(['/', '\\'])) {
            let _ = std::fs::remove_file(dir.join(name));
        }
        std::fs::remove_file(sidecar).context("remove sidecar")?;
        if let (Some(root), Some(id)) = (dir.parent(), v["id"].as_str()) {
            let _ = std::fs::remove_file(crate::gallery::thumb_path(root, id));
        }
    }
    notify_changed();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stash-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn favorite_sets_and_clears_the_field() {
        let root = temp_root("fav");
        let saved = save_text(&root, "a note", "note", &SourceInfo::default()).unwrap();
        set_favorite(&saved.sidecar, true).unwrap();
        assert_eq!(read_json(&saved.sidecar).unwrap()["favorite"], Value::Bool(true));
        set_favorite(&saved.sidecar, false).unwrap();
        assert!(read_json(&saved.sidecar).unwrap().get("favorite").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discard_removes_content_sidecar_and_thumb() {
        let root = temp_root("discard");
        let saved = save_text(&root, "throw me away", "note", &SourceInfo::default()).unwrap();
        let v = read_json(&saved.sidecar).unwrap();
        let content = saved.sidecar.parent().unwrap().join(v["file"]["name"].as_str().unwrap());
        let thumb = crate::gallery::thumb_path(&root, v["id"].as_str().unwrap());
        std::fs::create_dir_all(thumb.parent().unwrap()).unwrap();
        std::fs::write(&thumb, b"x").unwrap();
        assert!(content.exists());
        discard(&saved.sidecar).unwrap();
        assert!(!content.exists() && !saved.sidecar.exists() && !thumb.exists());
        // a tag arriving after the discard must not bring the sidecar back
        assert!(add_tags(&saved.sidecar, &["late".into()]).is_err());
        assert!(!saved.sidecar.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
