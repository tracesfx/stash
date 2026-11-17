//! Themes: System / Light / Dark / Glass. Web windows get their theme from `theme_info`; this module also
//! resolves the theme for native windows (wheel, snip overlay) and sets window backdrops (Mica / Acrylic).
use serde::Serialize;
use tauri::window::{Color, Effect, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use windows::core::w;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

use crate::config::Config;
use crate::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    Dark,
    Light,
    Glass,
}

/// Everything a native window needs to draw itself in the current theme.
#[derive(Clone, Copy)]
pub struct Style {
    pub theme: Resolved,
    pub accent: [u8; 3],
    /// Liquid Glass gradient stops.
    pub glass: [[u8; 3]; 3],
    /// Hover/selection colour (accent unless overridden).
    pub hover: [u8; 3],
    pub translucency: f32, // 0..1
    pub blur: u8,
    pub saturation: f32, // 1.0 = unchanged
    pub tint: f32,       // 0..1
    pub wheel_size: f32, // 1.0 = 100%
    pub wheel_hole: f32,
    pub wheel_gap: f32,
    pub wheel_opacity: f32,
}

pub const DEFAULT_GLASS: [&str; 3] = ["#5a78ff", "#be5aff", "#3cdcc8"];

fn reg_dword(sub: windows::core::PCWSTR, name: windows::core::PCWSTR) -> Option<u32> {
    unsafe {
        let mut v = 0u32;
        let mut n = 4u32;
        let r = RegGetValueW(HKEY_CURRENT_USER, sub, name, RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut n));
        r.is_ok().then_some(v)
    }
}

/// Windows "app mode" light/dark setting.
pub fn system_is_light() -> bool {
    reg_dword(w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), w!("AppsUseLightTheme")) == Some(1)
}

pub fn resolve(theme: &str) -> Resolved {
    match theme {
        "light" => Resolved::Light,
        "dark" => Resolved::Dark,
        "glass" => Resolved::Glass,
        _ => {
            if system_is_light() {
                Resolved::Light
            } else {
                Resolved::Dark
            }
        }
    }
}

/// Windows accent colour (RGB); a neutral grey accent would make hover/selected states look disabled, so the
/// brand blue is used instead.
pub fn accent() -> [u8; 3] {
    if let Some(v) = reg_dword(w!("Software\\Microsoft\\Windows\\DWM"), w!("AccentColor")) {
        let rgb = [v as u8, (v >> 8) as u8, (v >> 16) as u8]; // stored as 0xAABBGGRR
        let spread = rgb.iter().max().unwrap() - rgb.iter().min().unwrap();
        if spread >= 30 {
            return rgb;
        }
    }
    [0x4a, 0x7d, 0xff]
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn glass_colors(cfg: &Config) -> [[u8; 3]; 3] {
    let mut out = [[0u8; 3]; 3];
    for i in 0..3 {
        out[i] = cfg.glass.get(i).and_then(|s| parse_hex(s)).or_else(|| parse_hex(DEFAULT_GLASS[i])).unwrap();
    }
    out
}

pub fn style_for(cfg: &Config) -> Style {
    let a = &cfg.appearance;
    let acc = accent();
    Style {
        theme: resolve(&cfg.theme),
        accent: acc,
        glass: glass_colors(cfg),
        hover: parse_hex(&a.hover_color).unwrap_or(acc),
        translucency: a.glass_translucency as f32 / 100.0,
        blur: a.glass_blur,
        saturation: a.glass_saturation as f32 / 100.0,
        tint: a.glass_tint as f32 / 50.0, // 50 = the original look
        wheel_size: a.wheel_size as f32 / 100.0,
        wheel_hole: a.wheel_hole as f32,
        wheel_gap: a.wheel_gap as f32,
        wheel_opacity: a.wheel_opacity as f32 / 100.0,
    }
}

pub fn style(app: &AppHandle) -> Style {
    style_for(&app.state::<AppState>().cfg())
}

#[derive(Serialize)]
pub struct ThemeInfo {
    theme: String,
    accent: String,
    glass: Vec<String>,
    hover: String,
    appearance: crate::config::Appearance,
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Theme name, accent and glass colours for the web windows (single source of truth with native windows).
#[tauri::command]
pub fn theme_info(state: tauri::State<AppState>) -> ThemeInfo {
    let cfg = state.cfg();
    let st = style_for(&cfg);
    ThemeInfo { theme: cfg.theme, accent: hex(st.accent), glass: st.glass.iter().map(|c| hex(*c)).collect(), hover: hex(st.hover), appearance: cfg.appearance }
}

pub fn builder_effects(cfg: &Config) -> tauri::utils::config::WindowEffectsConfig {
    let st = style_for(cfg);
    if st.theme == Resolved::Glass {
        // Acrylic tinted faintly with the first glass colour
        let g = st.glass[0];
        let mix = |c: u8| (12.0 + c as f32 * 0.16) as u8;
        // translucency 0 -> strong tint (frosted), 100 -> barely tinted (clear)
        let alpha = (120.0 - st.translucency * 112.0) as u8;
        EffectsBuilder::new().effect(Effect::Acrylic).color(Color(mix(g[0]), mix(g[1]), mix(g[2]), alpha)).build()
    } else {
        EffectsBuilder::new().effect(Effect::Mica).build()
    }
}

/// Backdrop + frame theme for a Stash window (gallery / settings).
pub fn style_window(win: &WebviewWindow, cfg: &Config) {
    let _ = win.set_effects(builder_effects(cfg));
    let t = match cfg.theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" | "glass" => Some(tauri::Theme::Dark),
        _ => None,
    };
    let _ = win.set_theme(t);
}

/// Re-style open windows and tell every web view to refresh its theme.
pub fn apply(app: &AppHandle) {
    let cfg = app.state::<AppState>().cfg();
    for label in ["gallery", "settings"] {
        if let Some(w) = app.get_webview_window(label) {
            style_window(&w, &cfg);
        }
    }
    let _ = app.emit("theme-changed", cfg.theme);
}
