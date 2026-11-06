//! App config at `%APPDATA%\Stash\config.json`. Created with defaults on first run.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Root of the shared library folder.
    pub library_path: PathBuf,
    /// The single global hotkey.
    pub hotkey: String,
    /// Hold time (ms) before the radial wheel opens; shorter = tap = snip.
    pub hold_ms: u64,
    /// Read the browser address bar (UI Automation) to record the source URL. Note: querying UI Automation
    /// makes Chromium browsers enable their accessibility tree, which costs some page-load speed.
    pub capture_url: bool,
    /// Show the confirmation popup after captures.
    pub show_popup: bool,
    /// Start with Windows (applied for installed/release builds only).
    pub autostart: bool,
    /// system | light | dark | glass
    pub theme: String,
    /// Liquid Glass gradient colours (3 hex stops).
    pub glass: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
        Self {
            library_path: home.join("Pictures").join("Stash"),
            hotkey: "Ctrl+Alt+S".into(),
            hold_ms: 200,
            capture_url: true,
            show_popup: true,
            autostart: true,
            theme: "system".into(),
            glass: crate::theme::DEFAULT_GLASS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// `%APPDATA%\Stash` (config + logs).
pub fn app_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Stash")
}

impl Config {
    pub fn load_or_create() -> Result<Self> {
        let dir = app_dir();
        std::fs::create_dir_all(&dir).context("create app dir")?;
        let path = dir.join("config.json");
        let cfg = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).context("parse config.json")?,
            Err(_) => {
                let c = Config::default();
                c.save()?;
                c
            }
        };
        std::fs::create_dir_all(&cfg.library_path).context("create library dir")?;
        Ok(cfg)
    }

    /// Atomic save: temp file + rename.
    pub fn save(&self) -> Result<()> {
        let path = app_dir().join("config.json");
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}
