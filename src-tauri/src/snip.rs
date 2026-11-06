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
use crate::{gallery, clip, library, note, overlay, popup, tray, uia, AppState};

/// The frozen desktop frame, grabbed on a background thread at key-down and shared (read-only) by the wheel
/// (for the glass backdrop) and the capture that follows.
#[derive(Clone)]
struct FrameCell(Arc<(Mutex<Option<std::result::Result<Arc<Frame>, String>>>, Condvar)>);

impl FrameCell {
    fn spawn() -> Self {
        let cell = FrameCell(Arc::new((Mutex::new(None), Condvar::new())));
        let c2 = cell.clone();
        std::thread::spawn(move || {
            let r = winutil::grab_virtual_screen().map(Arc::new).map_err(|e| format!("{e:#}"));
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
    window_rect: Option<RECT>,
    cursor: POINT,
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
        let (hwnd, process) = (fg.hwnd, fg.source.process.clone());
        s.pending = Some(Pending {
            started: Instant::now(),
            source: fg.source,
            window_rect: fg.rect,
            cursor,
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
        let choice = wheel::run(cursor, scale, release, crate::theme::style(&app), frame);
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
        Action::Window => run_rect(app, p, "window"),
        Action::Screen => run_rect(app, p, "screen"),
        Action::Selection => run_clip(app, p, true),
        Action::Clipboard => run_clip(app, p, false),
        Action::Note => {
            note::open(app, with_url(p.url, p.source));
            Ok(())
        }
        Action::Settings => {
            crate::settings::open(app);
            Ok(())
        }
        Action::Pause => {
            toggle_pause(app);
            Ok(())
        }
        Action::Gallery => {
            gallery::open(app);
            Ok(())
        }
    };
    if let Err(e) = r {
        tracing::error!("{action:?} failed: {e:#}");
        popup::notice(app, "Capture failed — see log");
    }
}

fn toggle_pause(app: &AppHandle) {
    let st = app.state::<AppState>();
    let now = !st.paused.load(Ordering::Relaxed);
    st.paused.store(now, Ordering::Relaxed);
    tray::sync_pause(app, now);
    tracing::info!("paused = {now} (via wheel)");
}

fn library_root(app: &AppHandle) -> std::path::PathBuf {
    app.state::<AppState>().cfg().library_path
}

/// Crop a rect (virtual-desktop coords) out of the frozen frame and save it. Runs on its own thread so the
/// next capture is never blocked by PNG encoding.
fn save_crop(app: &AppHandle, frame: Arc<Frame>, rect: RECT, source: SourceInfo, method: &'static str) -> Result<()> {
    let app = app.clone();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let (rgba, w, h) = frame.crop_rgba(rect);
        drop(frame);
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
        match library::save_image(&library_root(&app), &rgba, w, h, meta) {
            Ok(saved) => {
                tracing::info!("saved {method} {w}x{h} in {} ms", t0.elapsed().as_millis());
                crate::ocr::spawn(&app, saved.sidecar.clone());
                popup::show(&app, Some(saved.sidecar), saved.preview, saved.meta);
            }
            Err(e) => {
                tracing::error!("save {method} failed: {e:#}");
                popup::notice(&app, "Capture failed — see log");
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

/// Active window / full screen (monitor under the cursor): instant, no overlay.
fn run_rect(app: &AppHandle, p: Pending, method: &'static str) -> Result<()> {
    let frame = p.frame.wait()?;
    let rect = if method == "window" {
        p.window_rect.ok_or_else(|| anyhow!("no foreground window"))?
    } else {
        winutil::monitor_rect_at(p.cursor.x, p.cursor.y)
    };
    save_crop(app, frame, rect, with_url(p.url, p.source), method)
}

/// Stash selection (sends Ctrl+C first) or the current clipboard.
fn run_clip(app: &AppHandle, p: Pending, copy_first: bool) -> Result<()> {
    let method = if copy_first { "selection" } else { "clipboard" };
    let source = with_url(p.url, p.source);
    let clip = if copy_first { clip::copy_selection()? } else { clip::read() };
    let Some(clip) = clip else {
        popup::notice(app, if copy_first { "Nothing selected" } else { "Clipboard is empty" });
        return Ok(());
    };
    let monitor = winutil::monitor_at(p.cursor.x, p.cursor.y);
    let (saved, n) = library::save_clip(&library_root(app), clip, method, &source, monitor)?;
    tracing::info!("saved {method} ({n} item(s))");
    crate::ocr::spawn(app, saved.sidecar.clone());
    popup::show(app, Some(saved.sidecar), saved.preview, saved.meta);
    Ok(())
}
