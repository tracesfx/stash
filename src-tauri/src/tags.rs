//! Tag usage stats for the tag radial: which tags exist, how often and how recently they were used.
//! Built once from the sidecars on a background thread (tags only; cheap), then kept current by every write
//! that adds tags (radial, note #tags, gallery edits). Derived state only: the sidecars stay the truth.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Local};

#[derive(Clone, Copy)]
struct Stat {
    count: u32,
    last: i64, // unix seconds
}

struct Index {
    root: Option<PathBuf>,
    ready: bool,
    tags: HashMap<String, (String, Stat)>, // lowercase key -> (display name, stats)
}

static INDEX: std::sync::LazyLock<Mutex<Index>> = std::sync::LazyLock::new(|| Mutex::new(Index { root: None, ready: false, tags: HashMap::new() }));

fn lock() -> std::sync::MutexGuard<'static, Index> {
    INDEX.lock().unwrap_or_else(|e| e.into_inner())
}

/// (Re)builds the index for `root` in the background. Cheap no-op when already built for that root.
pub fn init(root: &Path) {
    {
        let mut ix = lock();
        if ix.root.as_deref() == Some(root) {
            return;
        }
        ix.root = Some(root.to_path_buf());
        ix.ready = false;
        ix.tags.clear();
    }
    let root = root.to_path_buf();
    std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        let found = scan(&root);
        let n = found.len();
        let mut ix = lock();
        if ix.root.as_deref() != Some(root.as_path()) {
            return; // library changed meanwhile
        }
        for (name, st) in found {
            merge(&mut ix.tags, &name, st);
        }
        ix.ready = true;
        tracing::info!("tag index: {} tags from {n} uses in {} ms", ix.tags.len(), t0.elapsed().as_millis());
    });
}

fn merge(map: &mut HashMap<String, (String, Stat)>, name: &str, st: Stat) {
    let e = map.entry(name.to_lowercase()).or_insert_with(|| (name.to_string(), Stat { count: 0, last: 0 }));
    e.1.count += st.count;
    e.1.last = e.1.last.max(st.last);
}

/// Every (tag, use) in the library. Reads each sidecar but only looks at `tags` and `modified`.
fn scan(root: &Path) -> Vec<(String, Stat)> {
    let mut out = Vec::new();
    let Ok(months) = std::fs::read_dir(root) else { return out };
    for m in months.flatten() {
        let mp = m.path();
        if !mp.is_dir() || m.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&mp) else { continue };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(v) = crate::library::read_json(&p) else { continue };
            let Some(tags) = v["tags"].as_array() else { continue };
            if tags.is_empty() {
                continue;
            }
            let when = v["modified"].as_str().or(v["created"].as_str()).and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.timestamp()).unwrap_or(0);
            for t in tags.iter().filter_map(|t| t.as_str()) {
                if !t.trim().is_empty() {
                    out.push((t.trim().to_string(), Stat { count: 1, last: when }));
                }
            }
        }
    }
    out
}

/// Records that these tags were just used.
pub fn bump(tags: &[String]) {
    let now = Local::now().timestamp();
    let mut ix = lock();
    for t in tags {
        merge(&mut ix.tags, t, Stat { count: 1, last: now });
    }
}

/// Top `n` tags: frequency, boosted strongly by recency (a tag used in the last hour outranks old favourites).
pub fn top(n: usize) -> Vec<String> {
    let now = Local::now().timestamp();
    let ix = lock();
    let mut v: Vec<(f64, &String)> = ix
        .tags
        .values()
        .map(|(name, st)| {
            let age_h = ((now - st.last).max(0) as f64) / 3600.0;
            let recency = 6.0 / (1.0 + age_h / 6.0); // ~6 now, ~3 after 6 h, ~0.25 after a week
            ((st.count as f64).ln_1p() * 2.0 + recency, name)
        })
        .collect();
    v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.1.cmp(b.1)));
    v.into_iter().take(n).map(|(_, s)| s.clone()).collect()
}

/// All known tags (display names), most used first, for type-to-filter.
pub fn all() -> Vec<String> {
    let ix = lock();
    let mut v: Vec<&(String, Stat)> = ix.tags.values().collect();
    v.sort_by(|a, b| b.1.count.cmp(&a.1.count).then_with(|| a.0.cmp(&b.0)));
    v.into_iter().map(|(n, _)| n.clone()).collect()
}

/// Normalises a typed tag: trims, drops a leading '#', collapses inner whitespace to '-'.
pub fn clean(t: &str) -> Option<String> {
    let t = t.trim().trim_start_matches('#').trim();
    if t.is_empty() {
        return None;
    }
    Some(t.split_whitespace().collect::<Vec<_>>().join("-").chars().take(40).collect())
}

/// `#word` tokens in a note (letters, digits, '-' and '_'), in order, without duplicates.
pub fn hashtags(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let starts_word = i == 0 || chars[i - 1].is_whitespace() || "([{,;".contains(chars[i - 1]);
        if chars[i] == '#' && starts_word {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '-' || chars[j] == '_') {
                j += 1;
            }
            // must contain a letter so "#1" or "#123" (issue numbers, ranks) stays plain text
            let word: String = chars[i + 1..j].iter().collect();
            let word = word.trim_end_matches(['-', '_']).to_string();
            if word.chars().any(|c| c.is_alphabetic()) && !out.iter().any(|x| x.eq_ignore_ascii_case(&word)) {
                out.push(word);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashtags_basic() {
        assert_eq!(hashtags("great hook #idea at 3:20 #YT-ref"), vec!["idea", "YT-ref"]);
        assert_eq!(hashtags("#a1 and #a1 again"), vec!["a1"]);
        assert!(hashtags("issue #123 and a#b").is_empty());
        assert_eq!(hashtags("(#colour) #motion_design."), vec!["colour", "motion_design"]);
    }

    #[test]
    fn clean_tags() {
        assert_eq!(clean("  #Mood board "), Some("Mood-board".into()));
        assert_eq!(clean("#"), None);
    }
}
