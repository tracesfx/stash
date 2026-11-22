//! Native toast: a small Liquid Glass pill in the configured screen corner. Never takes focus and is
//! click-through. Used for notices (pause, errors), saved notes, and as the capture confirmation when the tag
//! radial is turned off. One at a time: a new toast replaces the current one.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tauri::{AppHandle, Manager};
use tiny_skia::*;
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::{self, Layered, Weight};
use crate::library::Thumb;
use crate::theme::Style;
use crate::AppState;

const IN_MS: f32 = 160.0;
const OUT_MS: f32 = 220.0;
const TIMER: usize = 1;

static GENERATION: AtomicU64 = AtomicU64::new(0);

pub struct Toast {
    pub title: String,
    pub detail: String,
    pub thumb: Option<Arc<Thumb>>,
    pub ms: u32,
}

pub fn notice(app: &AppHandle, msg: &str) {
    show(app, Toast { title: msg.to_string(), detail: String::new(), thumb: None, ms: 1800 });
}

pub fn show(app: &AppHandle, t: Toast) {
    let cfg = app.state::<AppState>().cfg();
    let style = crate::theme::style_for(&cfg);
    let corner = cfg.appearance.popup_corner.clone();
    let seq = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || run(t, style, &corner, seq));
}

/// Top-left (physical px) for a w x h window in the configured corner of the primary work area.
pub fn corner_origin(corner: &str, w: i32, h: i32, margin: i32) -> POINT {
    let (wa, _) = crate::winutil::primary_work_area();
    let x = if corner.ends_with('l') { wa.left + margin } else { wa.right - w - margin };
    let y = if corner.starts_with('t') { wa.top + margin } else { wa.bottom - h - margin };
    POINT { x, y }
}

struct St {
    win: Layered,
    scene: Pixmap,
    t0: Instant,
    ms: f32,
    seq: u64,
    anim: bool,
    done: bool,
}

/// Window size (physical px) for a toast at DPI scale `s`.
fn size(t: &Toast, s: f32) -> (i32, i32) {
    let l = Layout::new(t, s);
    ((l.w_body + l.margin * 2.0).ceil() as i32, (l.h_body + l.margin * 2.0).ceil() as i32)
}

struct Layout {
    margin: f32,
    thumb_sz: f32,
    tpx: f32,
    dpx: f32,
    text_w: f32,
    pad: f32,
    h_body: f32,
    w_body: f32,
}

impl Layout {
    fn new(t: &Toast, s: f32) -> Layout {
        let margin = (14.0 * s).floor(); // room for the shadow inside the window
        let thumb_sz = if t.thumb.is_some() { 40.0 * s } else { 0.0 };
        let (tpx, dpx) = (13.5 * s, 11.5 * s);
        let text_w = gfx::text_width_w(&t.title, tpx, Weight::Semibold).max(gfx::text_width_w(&t.detail, dpx, Weight::Regular)).min(300.0 * s);
        let pad = 16.0 * s;
        let h_body = if t.thumb.is_some() || !t.detail.is_empty() { 56.0 * s } else { 42.0 * s };
        let w_body = pad * 2.0 + text_w + if thumb_sz > 0.0 { thumb_sz + 12.0 * s } else { 0.0 };
        Layout { margin, thumb_sz, tpx, dpx, text_w, pad, h_body, w_body }
    }
}

fn run(t: Toast, style: Style, corner: &str, seq: u64) {
    let (_, s) = crate::winutil::primary_work_area();
    let (w, h) = size(&t, s);
    let origin = corner_origin(corner, w, h, (10.0 * s) as i32);
    let bg = crate::winutil::grab_rect(RECT { left: origin.x, top: origin.y, right: origin.x + w, bottom: origin.y + h }, false)
        .ok()
        .and_then(|f| gfx::blur_backdrop(&f, origin, w, h, style.blur, style.saturation));
    let Some(pm) = render(&t, &style, s, bg.as_ref()) else { return };
    show_window(pm, &t, &style, origin, w, h, seq);
}

/// The static toast scene (rendered once; the window only fades/slides it).
fn render(t: &Toast, style: &Style, s: f32, bg: Option<&Pixmap>) -> Option<Pixmap> {
    let (w, h) = size(t, s);
    let Layout { margin, thumb_sz, tpx, dpx, text_w, pad, h_body, w_body } = Layout::new(t, s);
    let mut pm = Pixmap::new(w as u32, h as u32)?;
    let (bx, by) = (margin, margin);
    let r = h_body / 2.0;
    gfx::shadow_rrect(&mut pm, bx, by, w_body, h_body, r, 10.0 * s, 0.07);
    let path = gfx::rrect(bx, by, w_body, h_body, r)?;
    let bounds = Rect::from_xywh(bx, by, w_body, h_body)?;
    // the shadow must not show through the glass: clear under the body, then draw the material
    pm.fill_path(&path, &Paint { blend_mode: BlendMode::Clear, ..Default::default() }, FillRule::Winding, Transform::identity(), None);
    gfx::glass_fill(&mut pm, &path, bounds, bg, style, 1.15);
    gfx::glass_rim(&mut pm, &path, bounds, 1.0 * s, 1.0);
    let mut tx = bx + pad;
    if let Some(th) = &t.thumb {
        let ty = by + (h_body - thumb_sz) / 2.0;
        if let Some(clip) = gfx::rrect(tx, ty, thumb_sz, thumb_sz, 10.0 * s) {
            gfx::draw_image_cover(&mut pm, &th.rgba, th.w, th.h, Rect::from_xywh(tx, ty, thumb_sz, thumb_sz).unwrap(), &clip, 1.0);
            gfx::glass_rim(&mut pm, &clip, Rect::from_xywh(tx, ty, thumb_sz, thumb_sz).unwrap(), 1.0 * s, 0.7);
        }
        tx += thumb_sz + 12.0 * s;
    }
    let clip = Some((tx, bx + w_body - pad + 2.0));
    let title = gfx::ellipsize(&t.title, tpx, text_w, Weight::Semibold);
    if t.detail.is_empty() {
        gfx::draw_text_at(&mut pm, &title, tpx, tx, by + h_body / 2.0, [255, 255, 255], 250, Weight::Semibold, clip);
    } else {
        let detail = gfx::ellipsize(&t.detail, dpx, text_w, Weight::Regular);
        gfx::draw_text_at(&mut pm, &title, tpx, tx, by + h_body / 2.0 - 8.5 * s, [255, 255, 255], 250, Weight::Semibold, clip);
        gfx::draw_text_at(&mut pm, &detail, dpx, tx, by + h_body / 2.0 + 10.0 * s, [255, 255, 255], 165, Weight::Regular, clip);
    }
    Some(pm)
}

fn show_window(pm: Pixmap, t: &Toast, style: &Style, origin: POINT, w: i32, h: i32, seq: u64) {
    unsafe {
        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
        let Some(win) = Layered::new(w!("StashToast"), w!("Stash"), Some(wnd_proc), ex, origin, w, h) else { return };
        let mut st = Box::new(St { win, scene: pm, t0: Instant::now(), ms: t.ms as f32, seq, anim: style.animations, done: false });
        SetWindowLongPtrW(st.win.hwnd, GWLP_USERDATA, &mut *st as *mut St as isize);
        frame(&mut st);
        let _ = ShowWindow(st.win.hwnd, SW_SHOWNOACTIVATE);
        SetTimer(Some(st.win.hwnd), TIMER, 15, None);
        let mut msg = MSG::default();
        while !st.done && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = KillTimer(Some(st.win.hwnd), TIMER);
        SetWindowLongPtrW(st.win.hwnd, GWLP_USERDATA, 0);
    }
}

fn frame(st: &mut St) {
    let age = st.t0.elapsed().as_secs_f32() * 1000.0;
    let replaced = GENERATION.load(Ordering::SeqCst) != st.seq;
    if replaced && age < st.ms {
        st.ms = age; // fade out now
    }
    let (op, rise) = if !st.anim {
        (if age < st.ms { 1.0 } else { 0.0 }, 0.0)
    } else if age < IN_MS {
        let k = gfx::ease_out(age / IN_MS);
        (k, 1.0 - k)
    } else if age < st.ms {
        (1.0, 0.0)
    } else {
        let k = ((age - st.ms) / OUT_MS).clamp(0.0, 1.0);
        (1.0 - k, -0.5 * k)
    };
    if age >= st.ms + if st.anim { OUT_MS } else { 0.0 } {
        st.done = true;
        return;
    }
    let (w, h) = (st.scene.width() as f32, st.scene.height() as f32);
    let mut out = gfx::compose(&st.scene, op, 0.97 + 0.03 * op, w / 2.0, h / 2.0);
    if rise != 0.0 {
        // slide a few px (up when appearing)
        let mut moved = Pixmap::new(out.width(), out.height()).unwrap();
        moved.draw_pixmap(0, (rise * 10.0) as i32, out.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        out = moved;
    }
    st.win.present(&out);
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let st = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut St).as_mut();
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            WM_TIMER => {
                if let Some(st) = st {
                    frame(st);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
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
        let (tw, th) = (90u32, 60u32);
        let rgba: Vec<u8> = (0..tw * th).flat_map(|i| [(60 + (i % tw) * 2) as u8, (90 + (i / tw) * 2) as u8, 210, 255]).collect();
        let toasts = [
            ("toast_notice.png", Toast { title: "Capture paused".into(), detail: String::new(), thumb: None, ms: 0 }),
            ("toast_saved.png", Toast { title: "Saved".into(), detail: "Region · 1115×628 · chrome".into(), thumb: Some(Arc::new(Thumb { rgba, w: tw, h: th })), ms: 0 }),
            ("toast_note.png", Toast { title: "Note saved".into(), detail: "#idea #yt-ref".into(), thumb: None, ms: 0 }),
        ];
        for (name, t) in toasts {
            let (w, h) = size(&t, s);
            let origin = POINT { x: 60, y: 60 };
            let bg = gfx::blur_backdrop(&frame, origin, w, h, style.blur, style.saturation);
            let pm = render(&t, &style, s, bg.as_ref()).unwrap();
            preview::save(&pm, &frame, origin, name);
        }
    }
}
