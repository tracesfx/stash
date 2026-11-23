//! Capture orchestration. One hotkey:
//!   key-down  -> freeze the screen now (video frames / hover states), note source window (+ URL in background)
//!   tap (up before hold_ms) -> region-snip overlay on the frozen frame
//!   hold (>= hold_ms)       -> radial wheel; release over a wedge to choose
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use crate::wheel::{self, Action};
use crate::winutil::{self, Frame, SourceInfo};
use crate::radial::{self, Request};
use crate::{clip, gallery, library, note, overlay, toast, uia, AppState};

/// The frozen desktop frame, grabbed on a background thread at key-down and shared (read-only) by the wheel
/// (for the glass backdrop) and the capture that follows.
#[derive(Clone)]
struct FrameCell(Arc<(Mutex<Option<std::result::Result<Arc<Frame>, String>>>, Condvar)>);

impl FrameCell {
    fn spawn() -> Self {
        let cell = FrameCell(Arc::new((Mutex::new(None), Condvar::new())));
        let c2 = cell.clone();
        std::thread::spawn(move || {
            let t0 = Instant::now();
            let r = winutil::grab_virtual_screen().map(Arc::new).map_err(|e| format!("{e:#}"));
            tracing::info!("screen grab {} ms", t0.elapsed().as_millis());
            *c2.0 .0.lock().unwrap() = Some(r);
            c2.0 .1.notify_all();
        });
        cell
    }
    fn wait(&self) -> Result<Arc<Frame>> {
        let mut g = self.0 .0.lock().unwrap();
        while g.is_none() {
            g = self.0 .1.wait(g).unwrap();
        }
        g.as_ref().unwrap().clone().map_err(|e| anyhow!(e))
    }
}

struct Pending {
    started: Instant,
    source: SourceInfo,
    cursor: POINT,
    window_rect: Option<RECT>,
    frame: FrameCell,
    url: JoinHandle<Option<String>>,
}

/// Source metadata including the browser URL (joins the background lookup).
fn with_url(url: JoinHandle<Option<String>>, mut s: SourceInfo) -> SourceInfo {
    s.url = url.join().ok().flatten();
    s
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Phase {
    Idle,
    /// Key is down; deciding between tap and hold.
    Down,
    /// Wheel is showing (or a capture is running); ignore new key-downs.
    Busy,
}

struct Inner {
    phase: Phase,
    press_id: u64,
    pending: Option<Pending>,
    release: Arc<AtomicBool>,
    wheel_open: bool,
}

pub struct SnipState(Mutex<Inner>);

impl Default for SnipState {
    fn default() -> Self {
        Self(Mutex::new(Inner {
            phase: Phase::Idle,
            press_id: 0,
            pending: None,
            release: Arc::new(AtomicBool::new(false)),
            wheel_open: false,
        }))
    }
}

fn set_idle(app: &AppHandle) {
    let st = app.state::<SnipState>();
    st.0.lock().unwrap().phase = Phase::Idle;
}

/// Hotkey down.
pub fn begin(app: &AppHandle) {
    // While a clip is recording, the hotkey just stops it.
    if crate::record::stop() {
        tracing::info!("recording stop requested via hotkey");
        return;
    }
    let (hold_ms, capture_url) = {
        let c = app.state::<AppState>().cfg();
        (c.hold_ms, c.capture_url)
    };
    let press_id = {
        let st = app.state::<SnipState>();
        let mut s = st.0.lock().unwrap();
        if s.phase != Phase::Idle {
            return;
        }
        s.phase = Phase::Down;
        s.press_id += 1;
        s.wheel_open = false;
        s.release = Arc::new(AtomicBool::new(false));
        let fg = winutil::foreground_info();
        let mut cursor = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut cursor);
        }
        let (hwnd, process, window_rect) = (fg.hwnd, fg.source.process.clone(), fg.rect);
        s.pending = Some(Pending {
            started: Instant::now(),
            source: fg.source,
            cursor,
            window_rect,
            frame: FrameCell::spawn(),
            url: std::thread::spawn(move || if capture_url { uia::browser_url(hwnd, &process) } else { None }),
        });
        s.press_id
    };
    // If still held after hold_ms, open the wheel.
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(hold_ms));
        let (pending, release) = {
            let st = app.state::<SnipState>();
            let mut s = st.0.lock().unwrap();
            if s.press_id != press_id || s.phase != Phase::Down {
                return; // released already (tap)
            }
            s.phase = Phase::Busy;
            s.wheel_open = true;
            (s.pending.take(), s.release.clone())
        };
        let mut cursor = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut cursor);
        }
        let scale = winutil::monitor_at(cursor.x, cursor.y).scale;
        let frame = pending.as_ref().and_then(|p| p.frame.wait().ok());
        if let Some(p) = &pending {
            tracing::info!("wheel starting {} ms after key-down", p.started.elapsed().as_millis());
        }
        let choice = wheel::run(cursor, scale, release, crate::theme::style(&app), frame, wheel::enabled(&app.state::<AppState>().cfg().wheel_actions));
        tracing::info!("wheel choice: {choice:?}");
        if let (Some(action), Some(p)) = (choice, pending) {
            perform(&app, action, p);
        }
        set_idle(&app);
    });
}

/// Hotkey up.
pub fn finish(app: &AppHandle) {
    let pending = {
        let st = app.state::<SnipState>();
        let mut s = st.0.lock().unwrap();
        match s.phase {
            Phase::Down => {
                s.phase = Phase::Busy;
                s.pending.take()
            }
            Phase::Busy if s.wheel_open => {
                s.release.store(true, Ordering::SeqCst); // the wheel thread picks the wedge
                None
            }
            _ => None,
        }
    };
    if let Some(p) = pending {
        let app = app.clone();
        std::thread::spawn(move || {
            perform(&app, Action::Snip, p);
            set_idle(&app);
        });
    }
}

fn perform(app: &AppHandle, action: Action, p: Pending) {
    let r = match action {
        Action::Snip => run_snip(app, p),
        Action::Note => {
            note::open(app, with_url(p.url, p.source));
            Ok(())
        }
        Action::Record => run_record(app, p),
        Action::Window => match p.window_rect {
            Some(rect) => p.frame.wait().and_then(|f| save_crop(app, f, rect, with_url(p.url, p.source), "window")),
            None => Err(anyhow!("no foreground window")),
        },
        Action::Screen => {
            let rect = winutil::monitor_rect_at(p.cursor.x, p.cursor.y);
            p.frame.wait().and_then(|f| save_crop(app, f, rect, with_url(p.url, p.source), "screen"))
        }
        Action::Selection => run_clip(app, true, with_url(p.url, p.source)),
        Action::Clipboard => run_clip(app, false, with_url(p.url, p.source)),
        Action::Pause => {
            let st = app.state::<AppState>();
            let now = !st.paused.load(Ordering::Relaxed);
            st.paused.store(now, Ordering::Relaxed);
            crate::tray::sync_pause(app, now);
            tracing::info!("paused = {now} (wheel)");
            toast::notice(app, if now { "Capture paused" } else { "Capture resumed" });
            Ok(())
        }
        Action::Settings => {
            crate::settings::open(app);
            Ok(())
        }
        Action::Gallery => {
            gallery::open(app);
            Ok(())
        }
    };
    if let Err(e) = r {
        tracing::error!("{action:?} failed: {e:#}");
        toast::notice(app, "Capture failed — see log");
    }
}

fn library_root(app: &AppHandle) -> std::path::PathBuf {
    app.state::<AppState>().cfg().library_path
}

/// Crop a rect (virtual-desktop coords) out of the frozen frame and save it. Runs on its own thread so the
/// next capture is never blocked by PNG encoding. The frame stays alive until the tag radial has used it as
/// its glass backdrop.
fn save_crop(app: &AppHandle, frame: Arc<Frame>, rect: RECT, source: SourceInfo, method: &'static str) -> Result<()> {
    let app = app.clone();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let (rgba, w, h) = frame.crop_rgba(rect);
        if w == 0 || h == 0 {
            tracing::error!("empty capture rect");
            return;
        }
        let meta = library::ImageMeta {
            method,
            source,
            monitor: winutil::monitor_at((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2),
            rect: [rect.left, rect.top, w as i32, h as i32],
        };
        let hwnd = meta.source.hwnd;
        match library::save_image(&library_root(&app), &rgba, w, h, meta) {
            Ok(saved) => {
                tracing::info!("saved {method} {w}x{h} in {} ms", t0.elapsed().as_millis());
                crate::ocr::spawn(&app, saved.sidecar.clone());
                radial::show(&app, Request { sidecars: vec![saved.sidecar], preview: saved.preview, meta: saved.meta, source_hwnd: hwnd, frame: Some(frame) });
            }
            Err(e) => {
                tracing::error!("save {method} failed: {e:#}");
                toast::notice(&app, "Capture failed — see log");
            }
        }
    });
    Ok(())
}

fn run_snip(app: &AppHandle, p: Pending) -> Result<()> {
    let frame = p.frame.wait()?;
    tracing::info!("frame ready {}x{} {} ms after key-down", frame.w, frame.h, p.started.elapsed().as_millis());
    let Some(rect) = overlay::select_region(&frame, crate::theme::style(app), winutil::monitor_at(p.cursor.x, p.cursor.y).scale) else {
        tracing::info!("snip cancelled");
        return Ok(());
    };
    save_crop(app, frame, rect, with_url(p.url, p.source), "snip")
}

/// Choose a region, then record it until the hotkey is pressed again.
fn run_record(app: &AppHandle, p: Pending) -> Result<()> {
    let frame = p.frame.wait()?;
    let monitor = winutil::monitor_at(p.cursor.x, p.cursor.y);
    let Some(rect) = overlay::select_region(&frame, crate::theme::style(app), monitor.scale) else {
        tracing::info!("record cancelled");
        return Ok(());
    };
    drop(frame);
    crate::record::start(app, rect, with_url(p.url, p.source), monitor);
    Ok(())
}


/// Stash the foreground app's selection (`selection`) or the current clipboard.
fn run_clip(app: &AppHandle, selection: bool, source: SourceInfo) -> Result<()> {
    let (got, method) = if selection { (clip::copy_selection()?, "selection") } else { (clip::read(), "clipboard") };
    let Some(got) = got else {
        toast::notice(app, if selection { "Nothing selected" } else { "Clipboard is empty" });
        return Ok(());
    };
    let (saved, n) = clip::save(&library_root(app), got, method, &source)?;
    for s in &saved {
        if !matches!(s.preview, library::Preview::Text(_)) {
            crate::ocr::spawn(app, s.sidecar.clone());
        }
    }
    let sidecars: Vec<_> = saved.iter().map(|s| s.sidecar.clone()).collect();
    if let Some(last) = saved.into_iter().last() {
        let meta = if n > 1 { format!("{} files · {}", n, last.meta) } else { last.meta };
        radial::show(app, Request { sidecars, preview: last.preview, meta, source_hwnd: source.hwnd, frame: None });
    }
    Ok(())
}
