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
    /// Index text inside images with the built-in Windows OCR (local).
    pub ocr: bool,
    /// Look & feel knobs (glass, wheel, popup, fonts).
    pub appearance: Appearance,
    /// Record system audio / microphone into video clips.
    pub rec_system_audio: bool,
    pub rec_mic: bool,
    /// Video clip frame rate and max length (seconds, 0 = unlimited).
    pub rec_fps: u32,
    pub rec_max_secs: u32,
    /// Wheel wedges that are shown (ids from `wheel::Action::id`). The wheel lays them out in a fixed order.
    pub wheel_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// 0 = frosted/opaque .. 100 = nearly clear (Liquid Glass).
    pub glass_translucency: u8,
    /// Backdrop blur strength, 0..60.
    pub glass_blur: u8,
    /// Backdrop saturation %, 100..250.
    pub glass_saturation: u8,
    /// Strength of the gradient colour wash, 0..100.
    pub glass_tint: u8,
    /// Hover/selection colour (#rrggbb); empty = Windows accent.
    pub hover_color: String,
    /// Wheel diameter %, 60..160.
    pub wheel_size: u8,
    /// Wheel centre hole radius (px at 100%), 20..80.
    pub wheel_hole: u8,
    /// Gap between slices (px), 0..14.
    pub wheel_gap: u8,
    /// Wheel opacity %, 30..100.
    pub wheel_opacity: u8,
    /// Base UI font size (px), 11..18.
    pub ui_font: u8,
    /// Corner roundness (px for medium radius), 0..20.
    pub ui_radius: u8,
    /// Confirmation popup corner: br | bl | tr | tl.
    pub popup_corner: String,
    /// How long the popup stays (ms).
    pub popup_ms: u32,
    /// UI animations on/off.
    pub animations: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            glass_translucency: 75,
            glass_blur: 26,
            glass_saturation: 170,
            glass_tint: 50,
            hover_color: String::new(),
            wheel_size: 100,
            wheel_hole: 40,
            wheel_gap: 5,
            wheel_opacity: 100,
            ui_font: 13,
            ui_radius: 10,
            popup_corner: "br".into(),
            popup_ms: 1500,
            animations: true,
        }
    }
}

impl Appearance {
    pub fn clamp(&mut self) {
        self.glass_translucency = self.glass_translucency.min(100);
        self.glass_blur = self.glass_blur.min(60);
        self.glass_saturation = self.glass_saturation.clamp(100, 250);
        self.glass_tint = self.glass_tint.min(100);
        self.wheel_size = self.wheel_size.clamp(60, 160);
        self.wheel_hole = self.wheel_hole.clamp(20, 80);
        self.wheel_gap = self.wheel_gap.min(14);
        self.wheel_opacity = self.wheel_opacity.clamp(30, 100);
        self.ui_font = self.ui_font.clamp(11, 18);
        self.ui_radius = self.ui_radius.min(20);
        self.popup_ms = self.popup_ms.clamp(600, 15000);
        if !matches!(self.popup_corner.as_str(), "br" | "bl" | "tr" | "tl") {
            self.popup_corner = "br".into();
        }
        if !self.hover_color.is_empty() && crate::theme::parse_hex(&self.hover_color).is_none() {
            self.hover_color.clear();
        }
    }
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
            ocr: true,
            appearance: Appearance::default(),
            rec_system_audio: false,
            rec_mic: false,
            rec_fps: 30,
            rec_max_secs: 300,
            wheel_actions: crate::wheel::default_ids(),
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
