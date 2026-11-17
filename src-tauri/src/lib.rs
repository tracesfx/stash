mod board;
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
mod popup;
mod snip;
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
            if args.iter().any(|a| a == "--board") {
                board::open(_app, None);
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
            popup::popup_data,
            popup::popup_focus,
            popup::set_caption,
            popup::close_popup,
            gallery::gallery_list,
            gallery::gallery_set_meta,
            gallery::gallery_delete,
            gallery::gallery_restore,
            gallery::gallery_reveal,
            gallery::gallery_open_url,
            gallery::gallery_copy,
            gallery::gallery_import,
            gallery::gallery_set_board,
            gallery::gallery_backfill,
            gallery::gallery_export,
            gallery::gallery_open_path,
            gallery::gallery_edit_board,
            board::board_init,
            board::board_save,
            board::board_close,
            board::board_read_image,
            board::board_add_video,
            gallery::gallery_new_board,
            settings::settings_get,
            settings::settings_save,
            settings::hotkey_suspend,
            theme::theme_info,
            settings::settings_set_glass,
            settings::settings_set_appearance,
            settings::settings_set_wheel,
            gallery::gallery_paste,
            gallery::gallery_import_data,
            gallery::gallery_import_text,
            settings::settings_set_theme,
            note::note_save,
            note::note_close
        ])
        .setup(move |app| {
            use tauri::Manager;
            app.manage(popup::PopupState::default());
            app.manage(board::BoardState::default());
            app.manage(snip::SnipState::default());
            app.manage(note::NoteState::default());
            app.manage(gallery::GalleryState::default());
            app.manage(AppState {
                config: RwLock::new(config),
                paused: AtomicBool::new(false),
                hotkey_down: Mutex::new(None),
            });
            let hotkey_ok = hotkeys::register(app.handle());
            settings::apply_autostart_on_start(app.handle());
            tray::build(app.handle(), hotkey_ok)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Tray-only app: keep running with zero windows. An explicit `app.exit()` still quits.
    app.run(|_app, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
