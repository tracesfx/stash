mod backdrop;
mod clip;
mod record;
mod config;
mod gfx;
mod settings;
mod theme;
mod gallery;
mod hotkeys;
mod library;
mod note;
mod ocr;
mod palette;
mod overlay;
mod radial;
mod snip;
mod tags;
mod textedit;
mod toast;
mod tray;
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
            gallery::gallery_paste,
            gallery::gallery_import_data,
            gallery::gallery_import_text
        ])
        .setup(move |app| {
            use tauri::Manager;
            app.manage(snip::SnipState::default());
            app.manage(gallery::GalleryState::default());
            app.manage(settings::PreviewBg::default());
            tags::init(&config.library_path);
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
    let root = app.state::<AppState>().cfg().library_path;
    match what.as_str() {
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
