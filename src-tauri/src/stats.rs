//! Library statistics for Settings > Statistics (advanced). Everything is derived from the sidecars on demand, so
//! there is no separate tracking file to keep in sync and nothing runs while the page is closed.
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::{DateTime, Datelike, Local, Timelike};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::{library, AppState};

#[derive(Serialize, Default)]
pub struct Stats {
    total: usize,
    bytes: u64,
    /// notes (text + links), images, videos, audio, other files
    notes: usize,
    links: usize,
    images: usize,
    videos: usize,
    audio: usize,
    files: usize,
    favorites: usize,
    hidden: usize,
    imported: usize,
    captioned: usize,
    tagged: usize,
    with_ocr: usize,
    trash: usize,
    trash_bytes: u64,
    canvases: usize,
    /// Seconds of recorded / imported video that the sidecars know the length of.
    video_secs: f64,
    /// Words written in text notes.
    words: usize,
    first: Option<String>,
    last: Option<String>,
    /// Items saved per local day ("YYYY-MM-DD").
    days: BTreeMap<String, usize>,
    /// Items saved per hour of the day (local) and per weekday (0 = Monday).
    hours: [usize; 24],
    weekdays: [usize; 7],
    /// How things were saved (method label, count), most used first.
    methods: Vec<(String, usize)>,
    tags: Vec<(String, usize)>,
    tag_count: usize,
    apps: Vec<(String, usize)>,
    sites: Vec<(String, usize)>,
}

const VIDEO: [&str; 4] = ["mp4", "m4v", "webm", "mov"];
const AUDIO: [&str; 6] = ["mp3", "wav", "ogg", "oga", "m4a", "flac"];

fn method_name(m: &str) -> &str {
    match m {
        "snip" => "Region snip",
        "window" => "Window",
        "screen" => "Screen",
        "selection" => "Selection",
        "clipboard" => "Clipboard",
        "note" => "Quick note",
        "record" => "Recording",
        "drop" => "Dropped in",
        "paste" => "Pasted",
        "upload" => "Added from files",
        "import" => "Folder import",
        "" => "Other",
        o => o,
    }
}

/// Host of a URL without "www." (no URL crate: sidecar URLs are plain http(s) strings).
fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let h = rest.split(['/', '?', '#']).next()?.split('@').next_back()?.split(':').next()?;
    let h = h.trim_start_matches("www.").to_ascii_lowercase();
    (!h.is_empty()).then_some(h)
}

fn top(m: HashMap<String, (String, usize)>, n: usize) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = m.into_values().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

/// Calls `f` with every readable sidecar under `<base>/<month>/` (dot-folders skipped), and its content's size.
fn each_sidecar(base: &Path, mut f: impl FnMut(&Value, u64)) {
    let Ok(months) = std::fs::read_dir(base) else { return };
    for m in months.flatten() {
        if !m.path().is_dir() || m.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(files) = std::fs::read_dir(m.path()) else { continue };
        for e in files.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Ok(v) = library::read_json(&p) else { continue };
            let Some(name) = v["file"]["name"].as_str().filter(|n| !n.is_empty()) else { continue };
            let Ok(meta) = std::fs::metadata(m.path().join(name)) else { continue }; // content gone: not an item
            f(&v, meta.len());
        }
    }
}

pub fn compute(root: &Path) -> Stats {
    let mut s = Stats::default();
    let hidden_tags: Vec<String> = crate::gallery::tag_list(root, "hidden").into_iter().filter(|t| !t.contains(':')).map(|t| t.to_lowercase()).collect();
    let (mut methods, mut tags, mut apps, mut sites) = (HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());
    let bump = |m: &mut HashMap<String, (String, usize)>, label: &str| {
        let e = m.entry(label.to_lowercase()).or_insert_with(|| (label.to_string(), 0));
        e.1 += 1;
    };
    each_sidecar(root, |v, size| {
        s.total += 1;
        s.bytes += size;
        let kind = v["type"].as_str().unwrap_or("");
        let ext = v["file"]["ext"].as_str().unwrap_or("").to_ascii_lowercase();
        match kind {
            "text" => s.notes += 1,
            "link" => s.links += 1,
            "image" => s.images += 1,
            _ if VIDEO.contains(&ext.as_str()) => s.videos += 1,
            _ if AUDIO.contains(&ext.as_str()) => s.audio += 1,
            _ => s.files += 1,
        }
        if let Some(secs) = v["video"]["secs"].as_f64() {
            s.video_secs += secs;
        }
        let item_tags: Vec<&str> = v["tags"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        if v["favorite"].as_bool() == Some(true) {
            s.favorites += 1;
        }
        if v["hidden"].as_bool() == Some(true) || item_tags.iter().any(|t| hidden_tags.contains(&t.to_lowercase())) {
            s.hidden += 1;
        }
        let method = v["method"].as_str().unwrap_or("");
        if method == "import" {
            s.imported += 1;
        }
        bump(&mut methods, method_name(method));
        if !v["caption"].as_str().unwrap_or("").trim().is_empty() {
            s.captioned += 1;
        }
        if !item_tags.is_empty() {
            s.tagged += 1;
        }
        for t in &item_tags {
            bump(&mut tags, t);
        }
        if v["ocr"].as_str().is_some_and(|t| !t.is_empty()) {
            s.with_ocr += 1;
        }
        if let Some(p) = v["source"]["process"].as_str().filter(|p| !p.is_empty()) {
            bump(&mut apps, p.trim_end_matches(".exe").trim_end_matches(".EXE"));
        }
        if let Some(h) = v["source"]["url"].as_str().and_then(host) {
            bump(&mut sites, &h);
        }
        let created = v["created"].as_str().unwrap_or("");
        if let Ok(t) = DateTime::parse_from_rfc3339(created) {
            let t = t.with_timezone(&Local);
            *s.days.entry(t.format("%Y-%m-%d").to_string()).or_default() += 1;
            s.hours[t.hour() as usize] += 1;
            s.weekdays[t.weekday().num_days_from_monday() as usize] += 1;
            let stamp = t.to_rfc3339();
            if s.first.as_ref().is_none_or(|f| stamp < *f) {
                s.first = Some(stamp.clone());
            }
            if s.last.as_ref().is_none_or(|l| stamp > *l) {
                s.last = Some(stamp);
            }
        }
    });
    // words in notes: read the (small) note files themselves
    each_note_text(root, |t| s.words += t.split_whitespace().count());
    each_sidecar(&root.join(".trash"), |_, size| {
        s.trash += 1;
        s.trash_bytes += size;
    });
    s.canvases = std::fs::read_dir(root.join(".canvases"))
        .map(|d| d.flatten().filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json")).count())
        .unwrap_or(0);
    s.tag_count = tags.len();
    s.methods = top(methods, 12);
    s.tags = top(tags, 12);
    s.apps = top(apps, 8);
    s.sites = top(sites, 8);
    s
}

/// Text of every note (`.md` content files of kind "text"), at most 64 KB each.
fn each_note_text(root: &Path, mut f: impl FnMut(&str)) {
    let Ok(months) = std::fs::read_dir(root) else { return };
    for m in months.flatten() {
        if !m.path().is_dir() || m.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(files) = std::fs::read_dir(m.path()) else { continue };
        for e in files.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("md") || e.metadata().map(|m| m.len() > 64 * 1024).unwrap_or(true) {
                continue;
            }
            if let Ok(t) = std::fs::read_to_string(&p) {
                f(&t);
            }
        }
    }
}

#[tauri::command]
pub async fn library_stats(app: AppHandle) -> Result<Stats, String> {
    let root = app.state::<AppState>().cfg().library_path;
    tauri::async_runtime::spawn_blocking(move || compute(&root)).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::winutil::SourceInfo;

    #[test]
    fn host_strips_scheme_www_port_and_path() {
        assert_eq!(host("https://www.Example.com:8080/a?b#c").as_deref(), Some("example.com"));
        assert_eq!(host("http://user@site.org/x").as_deref(), Some("site.org"));
        assert_eq!(host("not a url"), None);
    }

    #[test]
    fn counts_items_words_and_days() {
        let root = std::env::temp_dir().join(format!("stash-test-stats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let src = SourceInfo { process: "chrome.exe".into(), url: Some("https://www.example.com/page".into()), ..Default::default() };
        library::save_text(&root, "three little words", "note", &src).unwrap();
        let link = library::save_text(&root, "https://example.com/x", "clipboard", &SourceInfo::default()).unwrap();
        library::add_tags(&link.sidecar, &["Blue".into()]).unwrap();
        library::set_favorite(&link.sidecar, true).unwrap();
        let s = compute(&root);
        assert_eq!((s.total, s.notes, s.links, s.favorites, s.tagged), (2, 1, 1, 1, 1));
        assert_eq!(s.words, 3);
        assert_eq!(s.days.values().sum::<usize>(), 2);
        assert_eq!(s.apps, vec![("chrome".to_string(), 1)]);
        assert_eq!(s.sites, vec![("example.com".to_string(), 1)]);
        assert_eq!(s.tags, vec![("Blue".to_string(), 1)]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
