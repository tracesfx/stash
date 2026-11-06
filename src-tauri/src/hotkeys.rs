//! The single global hotkey. M0: press/release plumbing only (logs tap vs hold).
use std::sync::atomic::Ordering;
use std::time::Instant;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::AppState;

/// Registers the hotkey. Returns false (and logs) on failure so the tray can surface it.
pub fn register(app: &AppHandle) -> bool {
    let hotkey = app.state::<AppState>().cfg().hotkey;
    let shortcut: Shortcut = match hotkey.parse() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("invalid hotkey {hotkey:?}: {e}");
            return false;
        }
    };
    let plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _sc, event| on_event(app, event.state()))
        .build();
    if let Err(e) = app.plugin(plugin) {
        tracing::error!("global-shortcut plugin: {e}");
        return false;
    }
    match app.global_shortcut().register(shortcut) {
        Ok(()) => {
            tracing::info!("hotkey registered: {hotkey}");
            true
        }
        Err(e) => {
            tracing::error!("hotkey {hotkey} unavailable: {e}");
            false
        }
    }
}

fn on_event(app: &AppHandle, state: ShortcutState) {
    let st = app.state::<AppState>();
    if st.paused.load(Ordering::Relaxed) {
        return;
    }
    let mut down = st.hotkey_down.lock().unwrap();
    match state {
        ShortcutState::Pressed => {
            if down.is_none() {
                *down = Some(Instant::now()); // ignore auto-repeat presses
                tracing::info!("hotkey down");
                crate::snip::begin(app);
            }
        }
        ShortcutState::Released => {
            if let Some(t0) = down.take() {
                let held = t0.elapsed().as_millis() as u64;
                let kind = if held >= st.cfg().hold_ms { "HOLD (wheel)" } else { "TAP (snip)" };
                tracing::info!("hotkey up after {held} ms -> {kind}");
                crate::snip::finish(app); // M1: hold also snips; the wheel arrives in M1.5
            }
        }
    }
}
