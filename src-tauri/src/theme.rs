//! Liquid Glass is the only theme (M6). Web windows get their look from `theme_info`; native windows (wheel,
//! snip overlay, tag radial, quick note, toast) get the same values through `Style`. Gallery/settings
//! windows sit on our own blurred backdrop (backdrop.rs), or Acrylic when that is unavailable.
use serde::Serialize;
use tauri::window::{Color, Effect, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use windows::core::w;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

use crate::config::Config;
use crate::AppState;

/// Everything a native window needs to draw itself.
#[derive(Clone, Copy)]
pub struct Style {
    pub accent: [u8; 3],
    /// Liquid Glass sheen colours (3 stops).
    pub glass: [[u8; 3]; 3],
    /// Hover/selection colour (accent unless overridden).
    pub hover: [u8; 3],
    pub translucency: f32, // 0..1
    pub blur: u8,
    pub saturation: f32, // 1.0 = unchanged
    pub tint: f32,       // 0..1 colour sheen strength
    pub wheel_size: f32, // 1.0 = 100%
    pub wheel_hole: f32,
    pub wheel_gap: f32,
    pub wheel_opacity: f32,
    pub animations: bool,
    /// Cursor glow strength 0..1 (0 = off) and follow delay (time constant, ms).
    pub glow: f32,
    pub glow_lag: f32,
    /// How the wheel and tag menu open, and how long that takes (ms). Ignored when `animations` is off.
    pub anim: OpenAnim,
    pub anim_ms: f32,
}

/// Opening animation of the wheel and the tag menu (Appearance > Menu animation). Drawn by `gfx::open_window`
/// (whole menu) and `gfx::open_slice` (each slice); settings.ts mirrors the curves for its live preview.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpenAnim {
    /// Slices pop in one after another with a small overshoot (the M6 look).
    Petals,
    /// A soft fade.
    Fade,
    /// Grows from smaller with a long, smooth ease-out.
    Zoom,
    /// Springs in past full size and settles.
    Bounce,
    /// Turns into place while it grows.
    Swirl,
    /// Slices start stacked at the top and fan out clockwise.
    Fan,
}

impl OpenAnim {
    pub fn parse(s: &str) -> Self {
        match s {
            "fade" => Self::Fade,
            "zoom" => Self::Zoom,
            "bounce" => Self::Bounce,
            "swirl" => Self::Swirl,
            "fan" => Self::Fan,
            _ => Self::Petals,
        }
    }
}

/// Neutral, cool glass sheen (the reference picks are mostly colourless; colour comes from what is behind).
pub const DEFAULT_GLASS: [&str; 3] = ["#8fa6ff", "#b59cff", "#7fd6e6"];

fn reg_dword(sub: windows::core::PCWSTR, name: windows::core::PCWSTR) -> Option<u32> {
    unsafe {
        let mut v = 0u32;
        let mut n = 4u32;
        let r = RegGetValueW(HKEY_CURRENT_USER, sub, name, RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut n));
        r.is_ok().then_some(v)
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
    for (i, o) in out.iter_mut().enumerate() {
        *o = cfg.glass.get(i).and_then(|s| parse_hex(s)).or_else(|| parse_hex(DEFAULT_GLASS[i])).unwrap_or([128, 128, 160]);
    }
    out
}

pub fn style_for(cfg: &Config) -> Style {
    let a = &cfg.appearance;
    let acc = accent();
    Style {
        accent: acc,
        glass: glass_colors(cfg),
        hover: parse_hex(&a.hover_color).unwrap_or(acc),
        translucency: a.glass_translucency as f32 / 100.0,
        blur: a.glass_blur,
        saturation: a.glass_saturation as f32 / 100.0,
        tint: a.glass_tint as f32 / 100.0,
        wheel_size: a.wheel_size as f32 / 100.0,
        wheel_hole: a.wheel_hole as f32,
        wheel_gap: a.wheel_gap as f32,
        wheel_opacity: a.wheel_opacity as f32 / 100.0,
        animations: a.animations,
        glow: if a.cursor_glow_on { a.cursor_glow as f32 / 100.0 } else { 0.0 },
        glow_lag: a.cursor_glow_lag as f32,
        anim: OpenAnim::parse(&a.menu_anim),
        anim_ms: a.menu_anim_ms.clamp(100, 600) as f32,
    }
}

pub fn style(app: &AppHandle) -> Style {
    style_for(&app.state::<AppState>().cfg())
}

#[derive(Serialize)]
pub struct ThemeInfo {
    accent: String,
    glass: Vec<String>,
    hover: String,
    appearance: crate::config::Appearance,
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Accent and glass values for the web windows (single source of truth with native windows).
#[tauri::command]
pub fn theme_info(state: tauri::State<AppState>) -> ThemeInfo {
    let cfg = state.cfg();
    let st = style_for(&cfg);
    ThemeInfo { accent: hex(st.accent), glass: st.glass.iter().map(|c| hex(*c)).collect(), hover: hex(st.hover), appearance: cfg.appearance }
}

/// How frosted the native glass is: 1 = frosted, 0 = transparent (blur 0). Low blurs fade between the two,
/// so the in-between look (a light veil over a barely softened screen) is reachable.
pub fn frost(style: &Style) -> f32 {
    frost_of(style.blur)
}

pub fn frost_of(blur: u8) -> f32 {
    (blur as f32 / 12.0).min(1.0)
}

/// Acrylic fallback for when our own backdrop cannot be created: a neutral smoky tint, or none (a clear
/// window) at blur 0, since Acrylic's own blur cannot be turned down.
fn acrylic(cfg: &Config) -> Option<tauri::utils::config::WindowEffectsConfig> {
    let a = &cfg.appearance;
    if a.win_blur == 0 {
        return None;
    }
    let tr = a.win_translucency as f32 / 100.0;
    let alpha = (150.0 - tr * 120.0).clamp(20.0, 220.0) as u8;
    Some(EffectsBuilder::new().effect(Effect::Acrylic).color(Color(14, 15, 20, alpha)).build())
}

/// Backdrop + dark frame for a new Stash window (gallery / settings); call right after it is built (it is
/// still hidden then).
pub fn style_window(win: &WebviewWindow, cfg: &Config) {
    let _ = win.set_theme(Some(tauri::Theme::Dark));
    restyle_window(win, cfg);
    if let Ok(h) = win.hwnd() {
        let hwnd = h.0 as isize;
        // Windows' open/close animation draws the window from a snapshot without the live backdrop, so the
        // blur only appeared once it finished; the page plays its own entrance animation instead.
        if !(cfg!(debug_assertions) && std::env::var_os("STASH_DWM_TRANSITIONS").is_some()) {
            use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};
            let on: windows::core::BOOL = true.into();
            unsafe {
                let _ = DwmSetWindowAttribute(windows::Win32::Foundation::HWND(hwnd as *mut _), DWMWA_TRANSITIONS_FORCEDISABLED, &on as *const _ as *const _, 4);
            }
        }
        // window events arrive on the main thread, where the composition objects live
        win.on_window_event(move |ev| {
            if matches!(ev, tauri::WindowEvent::Destroyed) {
                crate::backdrop::detach(hwnd);
            }
        });
    }
}

/// Applies the current window-glass settings. Composition must run on the main thread, so this queues it.
fn restyle_window(win: &WebviewWindow, cfg: &Config) {
    let Ok(h) = win.hwnd() else { return };
    let hwnd = h.0 as isize;
    let a = &cfg.appearance;
    let params = crate::backdrop::Params { blur: a.win_blur as f32, saturation: a.win_saturation as f32 / 100.0 };
    let fallback = acrylic(cfg);
    let w = win.clone();
    let _ = win.run_on_main_thread(move || {
        let t0 = std::time::Instant::now();
        if !crate::backdrop::attached(hwnd) {
            // clear any system backdrop first (the Acrylic fallback): it resets what backdrop.rs sets on the window
            let _ = w.set_effects(None);
        }
        let r = if crate::backdrop::transparency_enabled() {
            crate::backdrop::apply(hwnd, params).map_err(|e| e.to_string())
        } else {
            Err("Windows transparency effects are off".to_string())
        };
        match r {
            Ok(()) => {
                tracing::debug!("{}: backdrop {params:?} applied in {} ms", w.label(), t0.elapsed().as_millis());
            }
            Err(e) => {
                tracing::warn!("own backdrop unavailable ({e}); using Acrylic");
                crate::backdrop::detach(hwnd);
                let _ = w.set_effects(fallback);
            }
        }
    });
}

/// Re-style open windows and tell every web view to refresh its look.
pub fn apply(app: &AppHandle) {
    let cfg = app.state::<AppState>().cfg();
    crate::gfx::set_fps_cap(cfg.appearance.anim_fps);
    for label in ["gallery", "settings"] {
        if let Some(w) = app.get_webview_window(label) {
            restyle_window(&w, &cfg);
        }
    }
    let _ = app.emit("theme-changed", ());
}
