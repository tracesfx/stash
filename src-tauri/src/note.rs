//! Quick note (C6): a native Liquid Glass input. Enter writes the text item and the window is gone in the same
//! frame; `#words` become tags; Esc discards. Drag it anywhere: it reopens where it was left (ui_state.json,
//! outside config.json so the settings window can never overwrite it); until then it sits in the configured
//! corner. The one intentional window that takes focus; focus goes back to the previous app when it closes.
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Instant;

use tauri::{AppHandle, Manager};
use tiny_skia::*;
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::{self, Layered, Weight};
use crate::textedit::{Outcome, TextField};
use crate::theme::Style;
use crate::winutil::SourceInfo;
use crate::{library, AppState};

const W: f32 = 460.0;
const H: f32 = 52.0;
const MARGIN: f32 = 16.0;
const OPEN_MS: f32 = 150.0;
const TIMER: usize = 1;

static OPEN: AtomicIsize = AtomicIsize::new(0);

// ---------- remembered position ----------
fn state_path() -> std::path::PathBuf {
    crate::config::app_dir().join("ui_state.json")
}

/// Saved top-left (physical px) of the note window, if it is still on a connected monitor.
fn saved_pos(w: i32, h: i32) -> Option<POINT> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(state_path()).ok()?).ok()?;
    let a = v["note_pos"].as_array()?;
    let (x, y) = (a.first()?.as_i64()? as i32, a.get(1)?.as_i64()? as i32);
    let r = RECT { left: x, top: y, right: x + w, bottom: y + h };
    let on_screen = unsafe { !windows::Win32::Graphics::Gdi::MonitorFromRect(&r, windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONULL).is_invalid() };
    on_screen.then_some(POINT { x, y })
}

fn write_pos(pos: Option<POINT>) {
    let path = state_path();
    let mut v: serde_json::Value = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| serde_json::json!({}));
    match pos {
        Some(p) => v["note_pos"] = serde_json::json!([p.x, p.y]),
        None => {
            if let Some(o) = v.as_object_mut() {
                o.remove("note_pos");
            }
        }
    }
    if let Err(e) = crate::library::write_atomic(&path, v.to_string().as_bytes()) {
        tracing::error!("save note position: {e:#}");
    }
}

/// Settings > "Reset note position": back to the configured corner.
#[tauri::command]
pub fn note_reset_position() {
    write_pos(None);
}

pub fn open(app: &AppHandle, source: SourceInfo) {
    let existing = OPEN.load(Ordering::SeqCst);
    if existing != 0 {
        crate::winutil::bring_to_front(existing);
        return;
    }
    let cfg = app.state::<AppState>().cfg();
    let style = crate::theme::style_for(&cfg);
    let corner = cfg.appearance.popup_corner.clone();
    let app = app.clone();
    std::thread::spawn(move || run(app, source, style, &corner));
}

struct St {
    app: AppHandle,
    win: Layered,
    s: f32,
    style: Style,
    field: TextField,
    bg: Option<Pixmap>,
    source: SourceInfo,
    t_open: Instant,
    blink: Instant,
    done: bool,
    restore_focus: bool,
    /// Last painted caret phase; the timer repaints only when it (or the open animation) changes.
    phase: u128,
    /// Being dragged. The backdrop is re-grabbed at the new position as it moves (throttled to ~1 per frame);
    /// without capture exclusion it would capture itself, so it is then drawn without one until dropped.
    moving: bool,
    bg_dirty: bool,
    last_grab: Instant,
    /// The window is excluded from screen capture, so the backdrop can be re-grabbed after a move.
    excluded: bool,
}

fn run(app: AppHandle, source: SourceInfo, style: Style, corner: &str) {
    let t0 = Instant::now();
    let (_, s) = crate::winutil::primary_work_area();
    let (w, h) = size(s);
    let origin = saved_pos(w, h).unwrap_or_else(|| crate::toast::corner_origin(corner, w, h, (12.0 * s) as i32));
    let bg = backdrop(origin, w, h, &style);
    unsafe {
        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
        let Some(win) = Layered::new(w!("StashQuickNote"), w!("Stash note"), Some(wnd_proc), ex, origin, w, h) else { return };
        let hwnd = win.hwnd;
        // keep the note out of screen captures: its backdrop can then be re-grabbed after a drag (and it never
        // shows up in the user's own snips)
        let excluded = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).is_ok();
        let mut st = Box::new(St { app, win, s, style, field: TextField::new(4000), bg, source, t_open: Instant::now(), blink: Instant::now(), done: false, restore_focus: true, phase: u128::MAX, moving: false, bg_dirty: false, last_grab: Instant::now(), excluded });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut *st as *mut St as isize);
        OPEN.store(hwnd.0 as isize, Ordering::SeqCst);
        paint(&mut st);
        let _ = ShowWindow(hwnd, SW_SHOW);
        crate::winutil::bring_to_front(hwnd.0 as isize);
        tracing::info!("note visible {} ms after open", t0.elapsed().as_millis());
        SetTimer(Some(hwnd), TIMER, 15, None);
        let mut msg = MSG::default();
        while !st.done && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = KillTimer(Some(hwnd), TIMER);
        OPEN.store(0, Ordering::SeqCst);
        let restore = st.restore_focus && GetForegroundWindow() == hwnd;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        let src = st.source.hwnd;
        drop(st);
        if restore {
            crate::winutil::restore_focus(src);
        }
    }
}

fn backdrop(origin: POINT, w: i32, h: i32, style: &Style) -> Option<Pixmap> {
    crate::winutil::grab_rect(RECT { left: origin.x, top: origin.y, right: origin.x + w, bottom: origin.y + h }, false)
        .ok()
        .and_then(|f| gfx::blur_backdrop(&f, origin, w, h, style.blur, style.saturation))
}

fn regrab(st: &mut St) {
    st.bg = backdrop(st.win.origin, st.win.w, st.win.h, &st.style);
    st.bg_dirty = false;
    st.last_grab = Instant::now();
}

/// Enter: hide at once, then write the item (a few ms) and confirm with a toast.
fn save(st: &mut St) {
    let text = st.field.text().trim().to_string();
    unsafe {
        let _ = ShowWindow(st.win.hwnd, SW_HIDE);
    }
    st.done = true;
    if text.is_empty() {
        return;
    }
    let t0 = Instant::now();
    let root = st.app.state::<AppState>().cfg().library_path;
    match library::save_text(&root, &text, "note", &st.source) {
        Ok(saved) => {
            let tags = crate::tags::hashtags(&text);
            if !tags.is_empty() {
                if let Err(e) = library::add_tags(&saved.sidecar, &tags) {
                    tracing::error!("note tags: {e:#}");
                }
                crate::tags::bump(&tags);
            }
            tracing::info!("note saved in {} ms ({} tags)", t0.elapsed().as_millis(), tags.len());
            let detail = if tags.is_empty() { saved.meta } else { tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ") };
            crate::toast::show(&st.app, crate::toast::Toast { title: "Note saved".into(), detail, thumb: None, ms: 1400 });
        }
        Err(e) => {
            tracing::error!("note save: {e:#}");
            crate::toast::notice(&st.app, "Saving the note failed — see log");
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let Some(st) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut St).as_mut() else {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        };
        match msg {
            WM_TIMER => {
                if st.moving && st.bg_dirty {
                    regrab(st);
                    paint(st);
                    return LRESULT(0);
                }
                let phase = (st.blink.elapsed().as_millis() / 530) % 2;
                if phase != st.phase || st.t_open.elapsed().as_secs_f32() * 1000.0 < OPEN_MS + 20.0 {
                    paint(st);
                }
                LRESULT(0)
            }
            WM_ACTIVATE => {
                // clicked away: an empty note just closes; typed text stays until Enter or Esc
                if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE && st.field.is_empty() && st.t_open.elapsed().as_millis() > 200 {
                    st.restore_focus = false;
                    st.done = true;
                }
                LRESULT(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let ctrl = (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
                let shift = (GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
                st.blink = Instant::now();
                match wparam.0 as u32 {
                    0x0D => save(st),
                    0x1B => st.done = true,
                    vk => {
                        if st.field.key_down(vk, ctrl, shift) == Outcome::Ignored && msg == WM_SYSKEYDOWN {
                            return DefWindowProcW(hwnd, msg, wparam, lparam);
                        }
                        paint(st);
                    }
                }
                if st.done {
                    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                }
                LRESULT(0)
            }
            // the whole bar is a drag handle (there is no mouse text selection); Windows runs the move loop
            WM_NCHITTEST => LRESULT(HTCAPTION as isize),
            WM_NCLBUTTONDBLCLK => LRESULT(0), // no maximize on double-click
            WM_SETCURSOR => {
                if let Ok(c) = LoadCursorW(None, IDC_SIZEALL) {
                    SetCursor(Some(c));
                }
                LRESULT(1)
            }
            WM_ENTERSIZEMOVE => {
                st.moving = true;
                paint(st);
                LRESULT(0)
            }
            WM_MOVE => {
                st.win.origin = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
                if st.moving && st.excluded {
                    st.bg_dirty = true;
                    if st.last_grab.elapsed().as_millis() >= 14 {
                        regrab(st);
                        paint(st);
                    }
                }
                LRESULT(0)
            }
            WM_EXITSIZEMOVE => {
                st.moving = false;
                if st.excluded {
                    regrab(st);
                } else {
                    st.bg = None; // would capture ourselves; plain tint instead
                }
                write_pos(Some(st.win.origin));
                paint(st);
                LRESULT(0)
            }
            WM_CLOSE => {
                // Alt+F4 = Esc: end the loop ourselves (the Layered drop destroys the window)
                st.done = true;
                let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                LRESULT(0)
            }
            WM_CHAR => {
                if let Some(c) = char::from_u32(wparam.0 as u32) {
                    if st.field.char_input(c) == Outcome::Changed {
                        st.blink = Instant::now();
                        paint(st);
                    }
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn paint(st: &mut St) {
    let (w, h) = (st.win.w as u32, st.win.h as u32);
    st.phase = (st.blink.elapsed().as_millis() / 530) % 2;
    let caret = st.phase == 0 && unsafe { GetForegroundWindow() == st.win.hwnd };
    let bg = if st.moving && !st.excluded { None } else { st.bg.as_ref() };
    let Some(pm) = render(&mut st.field, &st.style, st.s, w, h, bg, caret) else { return };
    // open animation: fade + rise
    let s = st.s;
    let age = st.t_open.elapsed().as_secs_f32() * 1000.0;
    let k = if st.style.animations { gfx::ease_out(age / OPEN_MS) } else { 1.0 };
    let mut out = gfx::compose(&pm, k, 0.97 + 0.03 * k, w as f32 / 2.0, h as f32 / 2.0);
    if k < 1.0 {
        let mut moved = Pixmap::new(w, h).unwrap();
        moved.draw_pixmap(0, ((1.0 - k) * 10.0 * s) as i32, out.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        out = moved;
    }
    st.win.present(&out);
}

/// The note card (no animation), w x h physical px at DPI scale `s`.
fn render(field: &mut TextField, style: &Style, s: f32, w: u32, h: u32, bg: Option<&Pixmap>, caret: bool) -> Option<Pixmap> {
    let mut pm = Pixmap::new(w, h)?;
    let (bx, by, bw, bh) = (MARGIN * s, MARGIN * s, W * s, H * s);
    let r = bh / 2.0;
    gfx::shadow_rrect(&mut pm, bx, by, bw, bh, r, 10.0 * s, 0.07);
    let path = gfx::rrect(bx, by, bw, bh, r)?;
    let bounds = Rect::from_xywh(bx, by, bw, bh)?;
    pm.fill_path(&path, &Paint { blend_mode: BlendMode::Clear, ..Default::default() }, FillRule::Winding, Transform::identity(), None);
    gfx::glass_fill(&mut pm, &path, bounds, bg, style, 1.15);
    gfx::glass_rim(&mut pm, &path, bounds, 1.0 * s, 1.0);
    let cy = by + bh / 2.0;
    let white = [255u8, 255, 255];
    // pen glyph in a soft round well
    let gx = bx + 26.0 * s;
    if let Some(c) = gfx::circle(gx, cy, 15.0 * s) {
        pm.fill_path(&c, &gfx::paint([255, 255, 255, 22]), FillRule::Winding, Transform::identity(), None);
    }
    let mut pb = PathBuilder::new();
    let k = 6.5 * s;
    let pts = [(-0.75f32, 0.75f32), (-0.6, 0.25), (0.4, -0.75), (0.8, -0.35), (-0.2, 0.6)];
    pb.move_to(gx + pts[0].0 * k, cy + pts[0].1 * k);
    for p in &pts[1..] {
        pb.line_to(gx + p.0 * k, cy + p.1 * k);
    }
    pb.close();
    if let Some(p) = pb.finish() {
        let stroke = Stroke { width: 1.5 * s, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        pm.stroke_path(&p, &gfx::paint([255, 255, 255, 220]), &stroke, Transform::identity(), None);
    }
    // hint on the right
    let hint = if field.is_empty() { "Esc" } else { "Enter" };
    let hpx = 11.0 * s;
    let hw = gfx::text_width_w(hint, hpx, Weight::Semibold);
    let (hx, hpad) = (bx + bw - 16.0 * s - hw, 8.0 * s);
    if let Some(chip) = gfx::rrect(hx - hpad, cy - 11.0 * s, hw + hpad * 2.0, 22.0 * s, 11.0 * s) {
        pm.fill_path(&chip, &gfx::paint([255, 255, 255, 20]), FillRule::Winding, Transform::identity(), None);
    }
    gfx::draw_text_at(&mut pm, hint, hpx, hx, cy, white, 150, Weight::Semibold, None);
    // field
    let field_r = Rect::from_ltrb(bx + 50.0 * s, by, hx - hpad - 10.0 * s, by + bh)?;
    field.draw(&mut pm, field_r, 14.5 * s, white, "Quick note…  #tags work too", caret, style.hover);
    Some(pm)
}

/// Window size (physical px) at DPI scale `s`.
fn size(s: f32) -> (i32, i32) {
    (((W + MARGIN * 2.0) * s).ceil() as i32, ((H + MARGIN * 2.0) * s).ceil() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::preview;

    #[test]
    #[ignore]
    fn previews() {
        let s = 1.5;
        let style = preview::style();
        let frame = preview::backdrop(900, 600);
        let (w, h) = size(s);
        let origin = POINT { x: 40, y: 200 };
        let bg = gfx::blur_backdrop(&frame, origin, w, h, style.blur, style.saturation);
        let mut f = TextField::new(4000);
        let pm = render(&mut f, &style, s, w as u32, h as u32, bg.as_ref(), true).unwrap();
        preview::save(&pm, &frame, origin, "note_empty.png");
        f.set_text("Great hook at 3:20, steal the zoom transition #idea #yt-ref");
        let pm = render(&mut f, &style, s, w as u32, h as u32, bg.as_ref(), true).unwrap();
        preview::save(&pm, &frame, origin, "note_typed.png");
    }
}
