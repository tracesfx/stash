//! Settings window + applying changes at runtime (hotkey re-bind, autostart, folder).
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
    version: &'static str,
}

pub fn open(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(w) = app.get_webview_window("settings") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
            return;
        }
        let built = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
            .title("Stash settings")
            .inner_size(580.0, 720.0)
            .resizable(false)
            .transparent(true) // the backdrop (Mica/Acrylic) shows through the translucent page
            .effects(crate::theme::builder_effects(&app.state::<AppState>().cfg()))
            .build();
        if let Err(e) = built {
            tracing::error!("settings window: {e}");
        }
    });
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
    SettingsDto { config: state.cfg(), autostart_supported: !cfg!(debug_assertions), version: env!("CARGO_PKG_VERSION") }
}

fn apply(app: &AppHandle, new: Config) -> Result<Config> {
    let st = app.state::<AppState>();
    let old = st.cfg();
    let mut new = new;
    new.theme = old.theme.clone(); // theme + glass colours are changed live via their own commands
    new.glass = old.glass.clone();
    new.appearance = old.appearance.clone(); // appearance has its own live command
    new.wheel_actions = old.wheel_actions.clone(); // wheel wedges have their own live command
    new.rec_fps = new.rec_fps.clamp(5, 60);
    new.rec_max_secs = new.rec_max_secs.min(3600);
    new.hold_ms = new.hold_ms.clamp(80, 1000);
    std::fs::create_dir_all(&new.library_path).with_context(|| format!("can't use folder {}", new.library_path.display()))?;

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
            return Err(anyhow!("that shortcut is unavailable (used by another app?): {e}"));
        }
        tray::set_status(app, &format!("Capture: {}", new.hotkey));
    }
    if !cfg!(debug_assertions) {
        set_autostart(app, new.autostart);
    }
    new.save()?;
    *st.config.write().unwrap() = new.clone();
    tracing::info!("settings saved");
    if new.theme != old.theme {
        let _ = app.run_on_main_thread({
            let app = app.clone();
            move || crate::theme::apply(&app)
        });
    }
    Ok(new)
}

#[tauri::command]
pub fn settings_save(new: Config, app: AppHandle) -> Result<Config, String> {
    apply(&app, new).map_err(|e| format!("{e:#}"))
}

/// Theme changes apply immediately (no Save needed).
#[tauri::command]
pub fn settings_set_theme(theme: String, app: AppHandle) -> Result<(), String> {
    if !matches!(theme.as_str(), "system" | "light" | "dark" | "glass") {
        return Err("unknown theme".into());
    }
    let st = app.state::<AppState>();
    let mut cfg = st.cfg();
    cfg.theme = theme;
    cfg.save().map_err(|e| format!("{e:#}"))?;
    *st.config.write().unwrap() = cfg;
    let a = app.clone();
    let _ = app.run_on_main_thread(move || crate::theme::apply(&a));
    Ok(())
}

/// Liquid Glass gradient colours (3 x #rrggbb); applies immediately.
#[tauri::command]
pub fn settings_set_glass(colors: Vec<String>, app: AppHandle) -> Result<(), String> {
    if colors.len() != 3 || colors.iter().any(|c| crate::theme::parse_hex(c).is_none()) {
        return Err("expected 3 colours like #5a78ff".into());
    }
    let st = app.state::<AppState>();
    let mut cfg = st.cfg();
    cfg.glass = colors;
    cfg.save().map_err(|e| format!("{e:#}"))?;
    *st.config.write().unwrap() = cfg;
    let a = app.clone();
    let _ = app.run_on_main_thread(move || crate::theme::apply(&a));
    Ok(())
}

/// Look & feel knobs (glass, wheel, popup, fonts); applies immediately.
#[tauri::command]
pub fn settings_set_appearance(appearance: crate::config::Appearance, app: AppHandle) -> Result<crate::config::Appearance, String> {
    let mut a = appearance;
    a.clamp();
    let st = app.state::<AppState>();
    let mut cfg = st.cfg();
    cfg.appearance = a.clone();
    cfg.save().map_err(|e| format!("{e:#}"))?;
    *st.config.write().unwrap() = cfg;
    let ap = app.clone();
    let _ = app.run_on_main_thread(move || crate::theme::apply(&ap));
    Ok(a)
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

/// Which wedges the radial wheel shows; applies immediately. At least two must stay enabled.
#[tauri::command]
pub fn settings_set_wheel(actions: Vec<String>, app: AppHandle) -> Result<Vec<String>, String> {
    let valid: Vec<String> = crate::wheel::ALL_ACTIONS.iter().filter(|a| actions.iter().any(|i| i == a.id())).map(|a| a.id().to_string()).collect();
    if valid.len() < 2 {
        return Err("Keep at least two actions on the wheel.".into());
    }
    let st = app.state::<AppState>();
    let mut cfg = st.cfg();
    cfg.wheel_actions = valid.clone();
    cfg.save().map_err(|e| format!("{e:#}"))?;
    *st.config.write().unwrap() = cfg;
    Ok(valid)
}
