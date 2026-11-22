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
    /// Show the tag radial after captures (otherwise a small confirmation toast).
    pub tag_menu: bool,
    /// Start with Windows (applied for installed/release builds only).
    pub autostart: bool,
    /// Liquid Glass sheen colours (3 hex stops).
    pub glass: Vec<String>,
    /// Index text inside images with the built-in Windows OCR (local).
    pub ocr: bool,
    /// Look & feel knobs (glass, wheel, fonts, note/toast corner).
    pub appearance: Appearance,
    /// Record system audio / microphone into video clips.
    pub rec_system_audio: bool,
    pub rec_mic: bool,
    /// Video clip frame rate and max length (seconds, 0 = unlimited).
    pub rec_fps: u32,
    pub rec_max_secs: u32,
    /// Wheel wedges that are shown (ids from `wheel::Action::id`). The wheel lays them out in a fixed order.
    pub wheel_actions: Vec<String>,
    /// Close the settings window when it loses focus (changes are saved as they are made).
    pub settings_autoclose: bool,
    /// Days trashed items are kept before they are deleted for good (0 = never).
    pub trash_days: u32,
    /// Look revision. Missing in configs written before M6 (reads as 0), which then get the new glass defaults.
    #[serde(default)]
    pub style_rev: u32,
}

/// Current look revision (M6: neutral Liquid Glass only; 4: menu animation picker).
pub const STYLE_REV: u32 = 4;

/// Opening animations of the wheel and the tag menu (Appearance > Menu animation).
pub const MENU_ANIMS: [&str; 6] = ["petals", "fade", "zoom", "bounce", "swirl", "fan"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Native glass (wheel, tag menu, note, toast). Tint density: 0 = dense .. 100 = thin.
    pub glass_translucency: u8,
    /// Native glass backdrop blur, 0..60 (0 = the screen shows through sharp).
    pub glass_blur: u8,
    /// Native glass backdrop saturation %, 100..250.
    pub glass_saturation: u8,
    /// Window glass (gallery, settings): tint density, 0 = dense .. 100 = thin.
    pub win_translucency: u8,
    /// Window glass blur, 0..60 (0 = transparent; drawn by backdrop.rs, Acrylic if that is unavailable).
    pub win_blur: u8,
    /// Window glass saturation %, 100..250.
    pub win_saturation: u8,
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
    /// Screen corner for the quick note and toasts: br | bl | tr | tl.
    pub popup_corner: String,
    /// UI animations on/off.
    pub animations: bool,
    /// Soft glow of the glass colours that follows the cursor: on/off, strength 0..100, follow delay (ms).
    pub cursor_glow_on: bool,
    pub cursor_glow: u8,
    pub cursor_glow_lag: u16,
    /// How the wheel and tag menu open: petals | fade | zoom | bounce | swirl | fan.
    pub menu_anim: String,
    /// Length of that opening animation (ms), 100..600.
    pub menu_anim_ms: u16,
    /// Legacy (style_rev < 4) Slice pop-in switch; read once by `Config::migrate`, never written.
    #[serde(skip_serializing)]
    pub slice_pop: bool,
    /// Legacy (style_rev < 3) Transparent switch; read once by `Config::migrate`, never written.
    #[serde(skip_serializing)]
    pub glass_clear: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            glass_translucency: 70,
            glass_blur: 30,
            glass_saturation: 150,
            win_translucency: 70,
            win_blur: 30,
            win_saturation: 150,
            glass_tint: 20,
            hover_color: String::new(),
            wheel_size: 100,
            wheel_hole: 40,
            wheel_gap: 5,
            wheel_opacity: 100,
            ui_font: 13,
            ui_radius: 12,
            popup_corner: "br".into(),
            animations: true,
            cursor_glow_on: true,
            cursor_glow: 30,
            cursor_glow_lag: 90,
            menu_anim: "petals".into(),
            menu_anim_ms: 260,
            slice_pop: true,
            glass_clear: false,
        }
    }
}

impl Appearance {
    pub fn clamp(&mut self) {
        self.glass_translucency = self.glass_translucency.min(100);
        self.glass_blur = self.glass_blur.min(60);
        self.glass_saturation = self.glass_saturation.clamp(100, 250);
        self.win_translucency = self.win_translucency.min(100);
        self.win_blur = self.win_blur.min(60);
        self.win_saturation = self.win_saturation.clamp(100, 250);
        self.glass_tint = self.glass_tint.min(100);
        self.wheel_size = self.wheel_size.clamp(60, 160);
        self.wheel_hole = self.wheel_hole.clamp(20, 80);
        self.wheel_gap = self.wheel_gap.min(14);
        self.wheel_opacity = self.wheel_opacity.clamp(30, 100);
        self.ui_font = self.ui_font.clamp(11, 18);
        self.ui_radius = self.ui_radius.min(20);
        self.cursor_glow = self.cursor_glow.min(100);
        self.cursor_glow_lag = self.cursor_glow_lag.min(400);
        self.menu_anim_ms = self.menu_anim_ms.clamp(100, 600);
        if !MENU_ANIMS.contains(&self.menu_anim.as_str()) {
            self.menu_anim = "petals".into();
        }
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
            tag_menu: true,
            autostart: true,
            ocr: true,
            appearance: Appearance::default(),
            rec_system_audio: false,
            rec_mic: false,
            rec_fps: 30,
            rec_max_secs: 300,
            wheel_actions: crate::wheel::default_ids(),
            settings_autoclose: true,
            trash_days: 30,
            glass: crate::theme::DEFAULT_GLASS.iter().map(|s| s.to_string()).collect(),
            style_rev: STYLE_REV,
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
            Ok(s) => {
                // tolerate a UTF-8 BOM (some editors add one)
                let mut c: Config = serde_json::from_str(s.trim_start_matches('\u{feff}')).context("parse config.json")?;
                if c.migrate() {
                    c.save()?;
                }
                c
            }
            Err(_) => {
                let c = Config::default();
                c.save()?;
                c
            }
        };
        std::fs::create_dir_all(&cfg.library_path).context("create library dir")?;
        Ok(cfg)
    }

    /// Upgrades older configs. Returns true when something changed.
    fn migrate(&mut self) -> bool {
        let mut changed = false;
        if self.style_rev < 2 {
            // M6: the colourful glass wash and System/Light/Dark themes are gone; start from the new neutral look
            let d = Appearance::default();
            let a = &mut self.appearance;
            (a.glass_translucency, a.glass_blur, a.glass_saturation, a.glass_tint, a.ui_radius) =
                (d.glass_translucency, d.glass_blur, d.glass_saturation, d.glass_tint, d.ui_radius);
            self.glass = crate::theme::DEFAULT_GLASS.iter().map(|s| s.to_string()).collect();
            self.style_rev = 2;
            changed = true;
        }
        if self.style_rev < 3 {
            // window and native glass got separate knobs; blur 0 replaces the Transparent switch
            let a = &mut self.appearance;
            a.win_translucency = a.glass_translucency;
            a.win_saturation = a.glass_saturation;
            a.win_blur = if a.glass_clear { 0 } else { 30 };
            if a.glass_clear {
                a.glass_blur = 0;
            }
            a.glass_clear = false;
            self.style_rev = 3;
            changed = true;
        }
        if self.style_rev < 4 {
            // the Slice pop-in switch became the Menu animation picker (pop-in = petals)
            let a = &mut self.appearance;
            a.menu_anim = if a.slice_pop { "petals" } else { "fade" }.into();
            self.style_rev = STYLE_REV;
            changed = true;
        }
        let before = self.wheel_actions.len();
        self.wheel_actions.retain(|a| a != "board"); // whiteboard removed in M6
        if self.wheel_actions.len() != before {
            if self.wheel_actions.len() < 2 {
                self.wheel_actions = crate::wheel::default_ids();
            }
            changed = true;
        }
        changed
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
