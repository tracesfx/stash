mod backdrop;
mod clip;
mod record;
mod config;
mod gfx;
mod settings;
mod stats;
mod theme;
mod gallery;
mod hotkeys;
mod library;
mod note;
mod ocr;
mod palette;
mod poster;
mod overlay;
mod radial;
mod snip;
mod tags;
mod textedit;
mod toast;
mod tray;
mod tutorial;
mod uia;
mod wheel;
mod winutil;

use std::sync::{atomic::AtomicBool, Mutex, RwLock};
use std::time::Instant;

use config::Config;

/// Shared app state (managed by Tauri).
pub struct AppState {
    pub config: RwLock<Config>,
    pub paused: AtomicBool,
    /// Set while the hotkey is physically held (ignores key auto-repeat).
    pub hotkey_down: Mutex<Option<Instant>>,
}

impl AppState {
    /// Snapshot of the current settings (they can change at runtime).
    pub fn cfg(&self) -> Config {
        self.config.read().unwrap().clone()
    }
}

fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    let dir = config::app_dir().join("logs");
    let (writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(dir, "stash.log"));
    tracing_subscriber::fmt().with_writer(writer).with_ansi(false).init();
    guard
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _log_guard = init_logging();
    let config = match Config::load_or_create() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("config: {e:#}");
            Config::default()
        }
    };
    tracing::info!("starting, library = {}", config.library_path.display());

    let app = tauri::Builder::default()
        // A second launch just exits; the running instance keeps going.
        .plugin(tauri_plugin_single_instance::init(|_app, args, _cwd| {
            tracing::info!("second instance launched; opening gallery");
            if cfg!(debug_assertions) && args.iter().any(|a| a == "--demo") {
                demo(_app, &args); // testing aid: show native windows without simulated input
            } else if args.iter().any(|a| a == "--settings") {
                settings::open(_app);
            } else {
                gallery::open(_app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .register_asynchronous_uri_scheme_protocol("stash", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_string();
            let range = request.headers().get("range").and_then(|v| v.to_str().ok()).map(String::from);
            std::thread::spawn(move || {
                let s = gallery::serve(&app, &path, range.as_deref());
                let mut b = tauri::http::Response::builder()
                    .status(s.status)
                    .header("Content-Type", s.mime)
                    .header("Accept-Ranges", "bytes")
                    .header("Access-Control-Allow-Origin", "*")
                    .header("Cache-Control", "max-age=3600");
                if let Some(cr) = s.content_range {
                    b = b.header("Content-Range", cr);
                }
                responder.respond(b.body(s.body).unwrap());
            });
        })
        .invoke_handler(tauri::generate_handler![
            gallery::gallery_list,
            gallery::gallery_set_meta,
            gallery::gallery_delete,
            gallery::gallery_restore,
            gallery::gallery_reveal,
            gallery::gallery_open_url,
            gallery::gallery_copy,
            gallery::gallery_import,
            gallery::gallery_tag,
            gallery::gallery_favorite,
            gallery::gallery_drag_icon,
            gallery::gallery_fav_tags,
            gallery::gallery_set_fav_tags,
            gallery::gallery_trash_list,
            gallery::gallery_purge,
            gallery::canvas_list,
            gallery::canvas_load,
            gallery::canvas_save,
            gallery::canvas_delete,
            gallery::gallery_backfill,
            gallery::gallery_export,
            gallery::gallery_open_path,
            settings::settings_get,
            settings::settings_save,
            settings::settings_close,
            settings::appearance_preview,
            note::note_reset_position,
            settings::hotkey_suspend,
            theme::theme_info,
            gallery::window_ready,
            gallery::perf_log,
            gallery::perf_enabled,
            gallery::gallery_hide_ready,
            gallery::gallery_warm_ready,
            gallery::gallery_warm_paint,
            gallery::gallery_paste,
            gallery::gallery_import_data,
            gallery::gallery_import_text,
            gallery::gallery_import_folder,
            gallery::gallery_hide,
            gallery::gallery_changed,
            gallery::gallery_hidden_tags,
            gallery::gallery_set_hidden_tags,
            stats::library_stats,
            tutorial::tutorial_info,
            tutorial::tutorial_finish,
            tutorial::tutorial_open,
            gallery::gallery_note_text,
            gallery::gallery_set_note_text
        ])
        .setup(move |app| {
            use tauri::Manager;
            app.manage(snip::SnipState::default());
            app.manage(gallery::GalleryState::default());
            app.manage(settings::PreviewBg::default());
            tags::init(&config.library_path);
            gfx::set_fps_cap(config.appearance.anim_fps);
            app.manage(AppState {
                config: RwLock::new(config),
                paused: AtomicBool::new(false),
                hotkey_down: Mutex::new(None),
            });
            let hotkey_ok = hotkeys::register(app.handle());
            settings::apply_autostart_on_start(app.handle());
            tray::build(app.handle(), hotkey_ok)?;
            // empty expired trash in the background (also done whenever the Trash tab opens)
            {
                let (root, days) = {
                    let c = app.state::<AppState>().cfg();
                    (c.library_path.clone(), c.trash_days)
                };
                std::thread::spawn(move || gallery::purge_expired(&root, days));
            }
            // load the gallery hidden once startup has settled, so opening it never waits for WebView2
            {
                let h = app.handle().clone();
                std::thread::spawn(move || {
                    // parse the UI fonts now, not on the first wheel / tag menu
                    let _ = (gfx::font(), gfx::font_regular());
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    if !h.state::<AppState>().cfg().tutorial_done {
                        tutorial::open(&h);
                    }
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    gallery::prewarm(&h);
                });
            }
            // coalesce bursts (a capture + its tags) into one event for the gallery
            let h = app.handle().clone();
            let pending = std::sync::Arc::new(AtomicBool::new(false));
            library::set_notifier(move || {
                if pending.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                let (h, p) = (h.clone(), pending.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    p.store(false, std::sync::atomic::Ordering::SeqCst);
                    use tauri::Emitter;
                    let _ = h.emit_to("gallery", "library-changed", ());
                    tutorial::step(&h, "saved");
                });
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Tray-only app: keep running with zero windows. An explicit `app.exit()` still quits.
    // (`demo` below is a debug-only testing aid.)
    app.run(|_app, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}

/// Debug builds: `stash.exe --demo radial|note|toast|textnote` (sent to the running instance) shows a native
/// feedback window with sample content, so it can be screenshotted without simulating input.
fn demo(app: &tauri::AppHandle, args: &[String]) {
    use tauri::Manager;
    let what = args.iter().skip_while(|a| *a != "--demo").nth(1).cloned().unwrap_or_default();
    // `--delay <ms>`: show it later, after the launching process has exited (its own startup feedback cursor
    // would otherwise mask what the window does to the cursor)
    if let Some(ms) = args.iter().skip_while(|a| *a != "--delay").nth(1).and_then(|v| v.parse::<u64>().ok()) {
        let (app, args) = (app.clone(), args.iter().filter(|a| *a != "--delay").cloned().collect::<Vec<_>>());
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            demo(&app, &args);
        });
        return;
    }
    let root = app.state::<AppState>().cfg().library_path;
    match what.as_str() {
        "wheel" => {
            // the hold wheel, released by itself after 1.2 s
            let app = app.clone();
            std::thread::spawn(move || {
                let mut cur = windows::Win32::Foundation::POINT::default();
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut cur);
                }
                let release = std::sync::Arc::new(AtomicBool::new(false));
                let r2 = release.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1200));
                    r2.store(true, std::sync::atomic::Ordering::SeqCst);
                });
                let scale = winutil::monitor_at(cur.x, cur.y).scale;
                let acts = wheel::enabled(&app.state::<AppState>().cfg().wheel_actions);
                let _ = wheel::run(cur, scale, release, theme::style(&app), None, acts);
            });
        }
        "note" => note::open(app, winutil::SourceInfo::default()),
        "toast" => toast::notice(app, "Capture paused"),
        "radial" | "textnote" => {
            // a real, fully saved sample item so tagging can be verified on disk
            let src = winutil::SourceInfo { process: "demo.exe".into(), ..Default::default() };
            let saved = if what == "radial" {
                let (w, h) = (480u32, 300u32);
                let rgba: Vec<u8> = (0..w * h).flat_map(|i| {
                    let (x, y) = (i % w, i / w);
                    [(40 + x * 180 / w) as u8, (60 + y * 120 / h) as u8, 200, 255]
                }).collect();
                let meta = library::ImageMeta { method: "snip", source: src, monitor: winutil::monitor_at(0, 0), rect: [0, 0, w as i32, h as i32] };
                library::save_image(&root, &rgba, w, h, meta)
            } else {
                library::save_text(&root, "Demo text item for the tag radial preview", "clipboard", &src)
            };
            match saved {
                Ok(s) => radial::show(app, radial::Request { sidecars: vec![s.sidecar], preview: s.preview, meta: s.meta, source_hwnd: 0, frame: None }),
                Err(e) => tracing::error!("demo: {e:#}"),
            }
        }
        _ => tracing::info!("demo: unknown {what:?}"),
    }
}
