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
    /// A save worth celebrating: a short sparkle burst (Settings > Save sparkle).
    pub fx: bool,
}

pub fn notice(app: &AppHandle, msg: &str) {
    show(app, Toast { title: msg.to_string(), detail: String::new(), thumb: None, ms: 1800, fx: false });
}

pub fn show(app: &AppHandle, t: Toast) {
    let cfg = app.state::<AppState>().cfg();
    let style = crate::theme::style_for(&cfg);
    let corner = cfg.appearance.popup_corner.clone();
    let seq = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let mut t = t;
    t.fx &= cfg.save_fx && style.animations;
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
    /// The fully shown frame is on screen already.
    still: bool,
    /// Sparkle burst (empty when off or finished) and the room around the toast it flies in (px per side).
    sparks: Vec<Spark>,
    pad: i32,
}

/// One particle of the save sparkle: starts at the thumbnail / left end of the toast and drifts outward.
struct Spark {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    size: f32,
    delay: f32,
    life: f32,
    color: [u8; 3],
    star: bool,
}

/// Room around the toast for the sparkle (logical px per side).
const FX_PAD: f32 = 44.0;
/// The burst starts as the toast lands (ms after it opens), and is over this long after that.
const FX_START: f32 = 70.0;
const FX_END: f32 = 900.0;

fn sparks(t: &Toast, style: &Style, s: f32, pad: f32, corner: &str, seq: u64) -> Vec<Spark> {
    let l = Layout::new(t, s);
    let (ox, oy) = (pad + l.margin + l.pad + l.thumb_sz / 2.0, pad + l.margin + l.h_body / 2.0);
    // from the rim of the thumbnail (not over the picture), or a small point at the left end of a text toast
    let r0 = if l.thumb_sz > 0.0 { l.thumb_sz * 0.5 } else { 6.0 * s };
    // drifts a little into the screen, away from the corner the toast sits in
    let up = if corner.starts_with('b') { -1.0 } else { 1.0 };
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    let mut r = (seq.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ nanos) | 1;
    let mut rnd = move || {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        (r >> 11) as f32 / (1u64 << 53) as f32
    };
    let cols = [style.glass[0], style.glass[1], style.glass[2], style.hover, [255, 255, 255], [255, 236, 190]];
    (0..16)
        .map(|i| {
            let a = rnd() * std::f32::consts::TAU;
            let sp = (90.0 + rnd() * 170.0) * s;
            let (dx, dy) = (a.cos(), a.sin());
            Spark {
                x: ox + dx * r0,
                y: oy + dy * r0,
                vx: dx * sp,
                vy: (dy * 0.8 + 0.45 * up) * sp,
                size: (1.3 + rnd() * 2.0) * s,
                delay: rnd() * 90.0,
                life: 420.0 + rnd() * 300.0,
                color: cols[(i + (rnd() * 6.0) as usize) % cols.len()],
                star: i % 3 == 0,
            }
        })
        .collect()
}

/// Draws the sparks at `age` ms (since the toast opened). False once they have all faded.
fn draw_sparks(pm: &mut Pixmap, sparks: &[Spark], age: f32) -> bool {
    let mut alive = false;
    for p in sparks {
        let t = age - FX_START - p.delay;
        if t < 0.0 {
            alive = true;
            continue;
        }
        if t > p.life {
            continue;
        }
        alive = true;
        let k = t / p.life;
        let ts = t / 1000.0;
        // drag: the burst slows down as it spreads
        let drift = (1.0 - (-3.2 * ts).exp()) / 3.2;
        let (x, y) = (p.x + p.vx * drift, p.y + p.vy * drift);
        let fade = if k < 0.15 { k / 0.15 } else { 1.0 - ((k - 0.15) / 0.85).powf(1.6) };
        let sz = p.size * (1.0 - 0.5 * k);
        let [r, g, b] = p.color;
        let a = (fade * 255.0) as u8;
        if let Some(c) = PathBuilder::from_circle(x, y, sz * 2.6) {
            pm.fill_path(&c, &gfx::paint([r, g, b, (fade * 46.0) as u8]), FillRule::Winding, Transform::identity(), None);
        }
        if p.star {
            // a four-point glint
            let (l, w) = (sz * 3.2, sz * 0.55);
            let mut pb = PathBuilder::new();
            pb.move_to(x, y - l);
            pb.quad_to(x + w, y - w, x + l, y);
            pb.quad_to(x + w, y + w, x, y + l);
            pb.quad_to(x - w, y + w, x - l, y);
            pb.quad_to(x - w, y - w, x, y - l);
            pb.close();
            if let Some(path) = pb.finish() {
                pm.fill_path(&path, &gfx::paint([255, 255, 255, a]), FillRule::Winding, Transform::identity(), None);
            }
        } else if let Some(c) = PathBuilder::from_circle(x, y, sz) {
            pm.fill_path(&c, &gfx::paint([r.saturating_add(40), g.saturating_add(40), b.saturating_add(40), a]), FillRule::Winding, Transform::identity(), None);
        }
    }
    alive
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
    // the sparkle needs room around the toast: a bigger (click-through, transparent) window, same toast position
    let pad = if t.fx { (FX_PAD * s) as i32 } else { 0 };
    let sp = if t.fx { sparks(&t, &style, s, pad as f32, corner, seq) } else { Vec::new() };
    show_window(pm, &t, &style, POINT { x: origin.x - pad, y: origin.y - pad }, (w + 2 * pad, h + 2 * pad), seq, sp, pad);
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

#[allow(clippy::too_many_arguments)]
fn show_window(pm: Pixmap, t: &Toast, style: &Style, origin: POINT, (w, h): (i32, i32), seq: u64, sparks: Vec<Spark>, pad: i32) {
    unsafe {
        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
        let Some(win) = Layered::new(w!("StashToast"), w!("Stash"), Some(wnd_proc), ex, origin, w, h) else { return };
        let mut st = Box::new(St { win, scene: pm, t0: Instant::now(), ms: t.ms as f32, seq, anim: style.animations, done: false, still: false, sparks, pad });
        SetWindowLongPtrW(st.win.hwnd, GWLP_USERDATA, &mut *st as *mut St as isize);
        frame(&mut st);
        let _ = ShowWindow(st.win.hwnd, SW_SHOWNOACTIVATE);
        let pacer = crate::gfx::FramePacer::start(st.win.hwnd, TIMER);
        let mut msg = MSG::default();
        while !st.done && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        drop(pacer);
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
    let sparking = !st.sparks.is_empty() && age < FX_START + FX_END;
    // while it just sits there the frame never changes: present it once
    let still = op == 1.0 && rise == 0.0 && !sparking;
    if still && st.still {
        return;
    }
    st.still = still;
    let (w, h) = (st.scene.width() as f32, st.scene.height() as f32);
    let mut out = gfx::compose(&st.scene, op, 0.97 + 0.03 * op, w / 2.0, h / 2.0);
    if rise != 0.0 || st.pad > 0 {
        // slide a few px (up when appearing), inside the room left for the sparkle
        let pad = st.pad.max(0) as u32;
        let Some(mut moved) = Pixmap::new(out.width() + 2 * pad, out.height() + 2 * pad) else { return };
        moved.draw_pixmap(st.pad, st.pad + (rise * 10.0) as i32, out.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        out = moved;
    }
    if sparking && !draw_sparks(&mut out, &st.sparks, age) {
        st.sparks.clear();
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
        let s: f32 = std::env::var("STASH_PREVIEW_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.5);
        let style = preview::style();
        let frame = preview::backdrop(900, 600);
        let (tw, th) = (90u32, 60u32);
        let rgba: Vec<u8> = (0..tw * th).flat_map(|i| [(60 + (i % tw) * 2) as u8, (90 + (i / tw) * 2) as u8, 210, 255]).collect();
        let toasts = [
            ("toast_notice.png", Toast { title: "Capture paused".into(), detail: String::new(), thumb: None, ms: 0, fx: false }),
            ("toast_saved.png", Toast { title: "Saved".into(), detail: "Region · 1115×628 · chrome".into(), thumb: Some(Arc::new(Thumb { rgba, w: tw, h: th })), ms: 0, fx: false }),
            ("toast_note.png", Toast { title: "Note saved".into(), detail: "#idea #yt-ref".into(), thumb: None, ms: 0, fx: false }),
        ];
        for (name, t) in toasts {
            let (w, h) = size(&t, s);
            let origin = POINT { x: 120, y: 120 };
            let bg = gfx::blur_backdrop(&frame, origin, w, h, style.blur, style.saturation);
            let pm = render(&t, &style, s, bg.as_ref()).unwrap();
            if name == "toast_saved.png" {
                // the save sparkle at a few moments (pad = the room the real window leaves around the toast)
                let pad = (FX_PAD * s) as i32;
                let sp = sparks(&t, &style, s, pad as f32, "br", 7);
                for age in [120u32, 260, 420, 640] {
                    let mut out = Pixmap::new(pm.width() + 2 * pad as u32, pm.height() + 2 * pad as u32).unwrap();
                    out.draw_pixmap(pad, pad, pm.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
                    draw_sparks(&mut out, &sp, age as f32);
                    preview::save(&out, &frame, POINT { x: origin.x - pad, y: origin.y - pad }, &format!("toast_fx_{age}.png"));
                }
            }
            preview::save(&pm, &frame, origin, name);
        }
    }
}
