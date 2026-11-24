//! Settings window + applying changes at runtime (hotkey re-bind, autostart, folder, look).
//! Model (SPEC §7): no Save button. Every change is saved as it is made (`settings_save`, debounced by the
//! page) and applies immediately; closing the window just closes it.
use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::config::Config;
use crate::{tray, AppState};

#[derive(Serialize)]
pub struct SettingsDto {
    config: Config,
    /// Autostart is only managed in release builds (a debug exe must not register itself at login).
    autostart_supported: bool,
    /// What Auto resolves to on this PC (small | medium | large), shown next to Auto.
    auto_scale: &'static str,
    /// The built-in look, for the Default theme tile.
    default_appearance: crate::config::Appearance,
    default_glass: Vec<String>,
}

pub fn open(app: &AppHandle) {
    crate::gallery::note_open("settings");
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(w) = app.get_webview_window("settings") {
            if let Ok(h) = w.hwnd() {
                crate::winutil::bring_to_front(h.0 as isize);
            }
            return;
        }
        // a snapshot of the desktop (before the window exists) for the Appearance preview
        *app.state::<PreviewBg>().0.lock().unwrap() = preview_snapshot();
        let f = scale_factor(&app.state::<AppState>().cfg());
        let b = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
            .title("Stash settings")
            .inner_size(640.0 * f, 760.0 * f)
            .min_inner_size(560.0 * f, 560.0 * f)
            .transparent(true) // the backdrop (theme::style_window) shows through the translucent page
            .theme(Some(tauri::Theme::Dark))
            .visible(false); // shown by `window_ready` after the first paint
        match b.build() {
            Ok(w) => {
                let _ = w.set_zoom(f);
                crate::theme::style_window(&w, &app.state::<AppState>().cfg());
                crate::gallery::show_if_not_ready(w)
            }
            Err(e) => tracing::error!("settings window: {e}"),
        }
    });
}

/// Settings > General > Window size: how much the gallery and settings windows (and their content) are scaled.
pub fn scale_factor(cfg: &Config) -> f64 {
    match cfg.ui_scale.as_str() {
        "small" => 0.8,
        "large" => 1.2,
        "medium" => 1.0,
        _ => match auto_scale() {
            "small" => 0.8,
            "large" => 1.2,
            _ => 1.0,
        },
    }
}

/// The size Auto picks, from the primary monitor's work area in logical px (the UI was designed on a 1440p screen
/// at 100 %: about 1400 px tall). 1080p-class screens get small, 4K at 100 % gets large.
pub fn auto_scale() -> &'static str {
    let (wa, s) = crate::winutil::primary_work_area();
    let h = (wa.bottom - wa.top) as f32 / s.max(0.5);
    if h < 1200.0 {
        "small"
    } else if h < 1800.0 {
        "medium"
    } else {
        "large"
    }
}

/// Window size changed in settings: zoom the open windows and resize them by the same ratio.
fn rescale_windows(app: &AppHandle, old: f64, new: f64) {
    for label in ["gallery", "settings"] {
        let Some(w) = app.get_webview_window(label) else { continue };
        let _ = w.set_zoom(new);
        if w.is_maximized().unwrap_or(false) {
            continue;
        }
        if let (Ok(sz), Ok(sf)) = (w.inner_size(), w.scale_factor()) {
            let k = new / old;
            let l = sz.to_logical::<f64>(sf);
            let (mw, mh) = if label == "gallery" { (640.0, 420.0) } else { (560.0, 560.0) };
            let _ = w.set_min_size(Some(tauri::LogicalSize::new(mw * new, mh * new)));
            let _ = w.set_size(tauri::LogicalSize::new(l.width * k, l.height * k));
        }
    }
}

/// On launch, make the OS autostart entry match the config (release builds only).
pub fn apply_autostart_on_start(app: &AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    let want = app.state::<AppState>().cfg().autostart;
    set_autostart(app, want);
}

fn set_autostart(app: &AppHandle, on: bool) {
    let mgr = app.autolaunch();
    let now = mgr.is_enabled().unwrap_or(false);
    if on != now {
        let r = if on { mgr.enable() } else { mgr.disable() };
        if let Err(e) = r {
            tracing::error!("autostart {on}: {e}");
        }
    }
}

#[tauri::command]
pub fn settings_get(state: State<AppState>) -> SettingsDto {
    let d = Config::default();
    SettingsDto { config: state.cfg(), autostart_supported: !cfg!(debug_assertions), auto_scale: auto_scale(), default_appearance: d.appearance, default_glass: d.glass }
}

/// Valid wheel action ids in canonical order; at least two.
fn valid_wheel(actions: &[String]) -> Result<Vec<String>> {
    let v: Vec<String> = crate::wheel::ALL_ACTIONS.iter().filter(|a| actions.iter().any(|i| i == a.id())).map(|a| a.id().to_string()).collect();
    if v.len() < 2 {
        return Err(anyhow!("Keep at least two actions on the wheel."));
    }
    Ok(v)
}

fn valid_glass(colors: &[String]) -> Result<Vec<String>> {
    if colors.len() != 3 || colors.iter().any(|c| crate::theme::parse_hex(c).is_none()) {
        return Err(anyhow!("expected 3 colours like #8fa6ff"));
    }
    Ok(colors.to_vec())
}

fn apply(app: &AppHandle, new: Config) -> Result<Config> {
    let st = app.state::<AppState>();
    let old = st.cfg();
    let mut new = new;
    new.appearance.clamp();
    new.wheel_actions = valid_wheel(&new.wheel_actions)?;
    new.glass = valid_glass(&new.glass)?;
    new.style_rev = crate::config::STYLE_REV;
    // the tutorial flag is owned by the tutorial window: a settings page opened earlier must not reset it
    new.tutorial_done |= old.tutorial_done;
    if !crate::config::UI_SCALES.contains(&new.ui_scale.as_str()) {
        new.ui_scale = "auto".into();
    }
    for t in &mut new.themes {
        t.appearance.clamp();
        if valid_glass(&t.glass).is_err() {
            t.glass = new.glass.clone();
        }
        t.name = t.name.trim().chars().take(40).collect();
    }
    new.themes.retain(|t| !t.name.is_empty());
    new.rec_fps = new.rec_fps.clamp(5, 60);
    new.rec_max_secs = new.rec_max_secs.min(3600);
    new.hold_ms = new.hold_ms.clamp(80, 1000);
    if new.library_path.as_os_str().is_empty() {
        return Err(anyhow!("Choose a library folder."));
    }
    if new.library_path != old.library_path {
        std::fs::create_dir_all(&new.library_path).with_context(|| format!("can't use folder {}", new.library_path.display()))?;
    }

    if new.hotkey != old.hotkey {
        let sc: Shortcut = new.hotkey.parse().map_err(|e| anyhow!("invalid hotkey: {e}"))?;
        let gs = app.global_shortcut();
        if let Ok(o) = old.hotkey.parse::<Shortcut>() {
            let _ = gs.unregister(o);
        }
        if let Err(e) = gs.register(sc) {
            // restore the previous binding so the user is never left without a hotkey
            if let Ok(o) = old.hotkey.parse::<Shortcut>() {
                let _ = gs.register(o);
            }
            return Err(anyhow!("That shortcut is unavailable (used by another app?): {e}"));
        }
        tray::set_status(app, &format!("Capture: {}", new.hotkey));
    }
    if !cfg!(debug_assertions) && new.autostart != old.autostart {
        set_autostart(app, new.autostart);
    }
    new.save()?;
    *st.config.write().unwrap() = new.clone();
    if new.library_path != old.library_path {
        crate::tags::init(&new.library_path);
    }
    if new.keep_gallery_ready != old.keep_gallery_ready {
        if new.keep_gallery_ready { crate::gallery::prewarm(app) } else { crate::gallery::release(app) }
    }
    let (fo, fnew) = (scale_factor(&old), scale_factor(&new));
    if (fo - fnew).abs() > 1e-6 {
        let a = app.clone();
        let _ = app.run_on_main_thread(move || rescale_windows(&a, fo, fnew));
    }
    let look_changed = serde_json::to_value(&new.appearance).ok() != serde_json::to_value(&old.appearance).ok() || new.glass != old.glass;
    if look_changed {
        let a = app.clone();
        let _ = app.run_on_main_thread(move || crate::theme::apply(&a));
    }
    Ok(new)
}

/// Saves and applies the whole settings object (called on every change).
#[tauri::command]
pub fn settings_save(new: Config, app: AppHandle) -> Result<Config, String> {
    apply(&app, new).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn settings_close(app: AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.destroy();
    }
}

// ---------- Appearance preview ----------
// The native glass (wheel, tag menu, note, toasts) has its own Blur/Saturation/Translucency. The Appearance tab
// shows that glass live (the window glass is visible in the settings window itself): the same
// renderer (gfx::glass_fill) over a snapshot of the desktop taken just before the settings window opened.

const PREVIEW_W: f32 = 560.0;
const PREVIEW_H: f32 = 210.0;

/// Desktop snapshot behind the Appearance preview.
#[derive(Default)]
pub struct PreviewBg(std::sync::Mutex<Option<crate::winutil::Frame>>);

fn preview_snapshot() -> Option<crate::winutil::Frame> {
    let (wa, s) = crate::winutil::primary_work_area();
    let (w, h) = ((PREVIEW_W * s) as i32, (PREVIEW_H * s) as i32);
    let (cx, cy) = ((wa.left + wa.right) / 2, (wa.top + wa.bottom) / 2);
    let r = windows::Win32::Foundation::RECT { left: cx - w / 2, top: cy - h / 2, right: cx - w / 2 + w, bottom: cy - h / 2 + h };
    crate::winutil::grab_rect(r, false).ok()
}

fn render_preview(frame: &crate::winutil::Frame, style: &crate::theme::Style) -> Option<Vec<u8>> {
    use image::ImageEncoder as _;
    use tiny_skia::*;
    let (w, h) = (frame.w, frame.h);
    let s = w as f32 / PREVIEW_W;
    let origin = windows::Win32::Foundation::POINT { x: frame.x, y: frame.y };
    let mut pm = Pixmap::new(w as u32, h as u32)?;
    // the sharp desktop underneath (what a layered window composites over)
    for (px, p) in pm.pixels_mut().iter_mut().zip(frame.bgra.chunks_exact(4)) {
        *px = ColorU8::from_rgba(p[2], p[1], p[0], 255).premultiply();
    }
    let bg = crate::gfx::blur_backdrop(frame, origin, w, h, style.blur, style.saturation);
    let white = [255u8, 255, 255];
    // a card with two tag chips, and a quick-note pill
    let (cx, cy, cw, ch) = (24.0 * s, 22.0 * s, 300.0 * s, 166.0 * s);
    let card = crate::gfx::rrect(cx, cy, cw, ch, 22.0 * s)?;
    let cb = Rect::from_xywh(cx, cy, cw, ch)?;
    crate::gfx::glass_fill(&mut pm, &card, cb, bg.as_ref(), style, 1.0);
    crate::gfx::glass_rim(&mut pm, &card, cb, 1.0 * s, 1.0);
    crate::gfx::draw_text_at(&mut pm, "Saved · Region", 15.0 * s, cx + 20.0 * s, cy + 34.0 * s, white, 250, crate::gfx::Weight::Semibold, None);
    crate::gfx::draw_text_at(&mut pm, "1115 × 628 · chrome", 12.0 * s, cx + 20.0 * s, cy + 58.0 * s, white, 160, crate::gfx::Weight::Regular, None);
    let mut x = cx + 20.0 * s;
    for (i, tag) in ["#moodboard", "#typography"].iter().enumerate() {
        let tw = crate::gfx::text_width(tag, 12.5 * s) + 26.0 * s;
        let chip = crate::gfx::rrect(x, cy + 92.0 * s, tw, 32.0 * s, 16.0 * s)?;
        let hv = style.hover;
        let fill = if i == 0 { [hv[0], hv[1], hv[2], 150] } else { [255, 255, 255, 22] };
        pm.fill_path(&chip, &crate::gfx::paint(fill), FillRule::Winding, Transform::identity(), None);
        crate::gfx::glass_rim(&mut pm, &chip, Rect::from_xywh(x, cy + 92.0 * s, tw, 32.0 * s)?, 1.0 * s, 0.8);
        crate::gfx::draw_text_at(&mut pm, tag, 12.5 * s, x + 13.0 * s, cy + 108.0 * s, white, 245, crate::gfx::Weight::Semibold, None);
        x += tw + 8.0 * s;
    }
    let (px_, py, pw, ph) = (340.0 * s, 78.0 * s, 196.0 * s, 48.0 * s);
    let pill = crate::gfx::rrect(px_, py, pw, ph, ph / 2.0)?;
    let pb = Rect::from_xywh(px_, py, pw, ph)?;
    crate::gfx::glass_fill(&mut pm, &pill, pb, bg.as_ref(), style, 1.15);
    crate::gfx::glass_rim(&mut pm, &pill, pb, 1.0 * s, 1.0);
    crate::gfx::draw_text_at(&mut pm, "Quick note…", 13.5 * s, px_ + 20.0 * s, py + ph / 2.0, white, 150, crate::gfx::Weight::Regular, None);
    // PNG (opaque)
    let rgb: Vec<u8> = pm.pixels().iter().flat_map(|p| [p.red(), p.green(), p.blue()]).collect();
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(std::io::Cursor::new(&mut out), image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Sub)
        .write_image(&rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(out)
}

/// PNG of the native glass with the given (unsaved or saved) look, over the desktop snapshot.
#[tauri::command]
pub fn appearance_preview(appearance: crate::config::Appearance, glass: Vec<String>, app: AppHandle, bg: State<PreviewBg>) -> Result<tauri::ipc::Response, String> {
    let mut cfg = app.state::<AppState>().cfg();
    cfg.appearance = appearance;
    cfg.appearance.clamp();
    if valid_glass(&glass).is_ok() {
        cfg.glass = glass;
    }
    let style = crate::theme::style_for(&cfg);
    let mut g = bg.0.lock().unwrap();
    if g.is_none() {
        *g = preview_snapshot();
    }
    let frame = g.as_ref().ok_or("no desktop snapshot")?;
    render_preview(frame, &style).map(tauri::ipc::Response::new).ok_or_else(|| "preview failed".to_string())
}

/// While the settings window records a new shortcut, the live hotkey must not fire.
#[tauri::command]
pub fn hotkey_suspend(suspend: bool, app: AppHandle) {
    let Ok(sc) = app.state::<AppState>().cfg().hotkey.parse::<Shortcut>() else { return };
    let gs = app.global_shortcut();
    if suspend {
        let _ = gs.unregister(sc);
    } else {
        let _ = gs.register(sc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders the Appearance preview over the preview backdrop (no window, no screen access).
    #[test]
    #[ignore]
    fn previews() {
        let frame = crate::gfx::preview::backdrop(840, 315); // 560 x 210 at 150 %
        let style = crate::gfx::preview::style();
        let png = render_preview(&frame, &style).expect("render");
        let out = std::path::PathBuf::from(std::env::var("STASH_PREVIEW_OUT").expect("STASH_PREVIEW_OUT"));
        std::fs::write(out.join("appearance_preview.png"), png).unwrap();
    }
}
