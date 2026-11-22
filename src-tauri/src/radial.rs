//! Tag radial (SPEC §4): appears at the cursor right after a capture is saved. Centre = the capture's
//! thumbnail (the confirmation); ring = up to 6 recent/frequent tags; below = a text pill to type a new tag
//! or (Tab / the pen button) a caption. It stays while the pointer is on it and fades out once the pointer
//! moves off (the item stays saved, untagged).
//! Native layered window (tiny-skia), so it appears in a few ms. It takes focus only so typing works, and
//! hands focus back to the app the user was in when it closes (unless they clicked elsewhere themselves).
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tauri::{AppHandle, Manager};
use tiny_skia::*;
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::{self, Layered, Weight};
use crate::library::{Preview, Thumb};
use crate::textedit::{Outcome, TextField};
use crate::theme::Style;
use crate::winutil::Frame;
use crate::AppState;

const MAX_TAGS: usize = 6;
const R_OUT: f32 = 124.0;
const R_IN: f32 = 60.0;
const GROW: f32 = 6.0;
const MARGIN: f32 = 20.0;
const PILL_H: f32 = 40.0;
const PILL_W: f32 = 236.0;
const PILL_GAP: f32 = 14.0;
const META_H: f32 = 22.0;
const OPEN_MS: f32 = 150.0;
const CLOSE_MS: f32 = 130.0;
const HOVER_MS: f32 = 90.0;
const TIMER: usize = 1;
/// Pointer must stay off the radial this long before it closes (absorbs a jittery edge).
const OFF_GRACE_MS: u128 = 150;
/// Safety net: closes when the pointer has not moved at all for this long and nothing is being typed.
const IDLE_CLOSE_S: u64 = 12;
/// Extra room around the ring and pill that still counts as "on" the radial (logical px).
const SLACK: f32 = 14.0;

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// A fresh capture to offer tagging for.
pub struct Request {
    pub sidecars: Vec<PathBuf>,
    pub preview: Preview,
    pub meta: String,
    /// Window to give focus back to.
    pub source_hwnd: isize,
    /// Screen image for the glass backdrop (the frozen frame after a snip); grabbed fresh when None.
    pub frame: Option<Arc<Frame>>,
}

/// Feedback after a capture: the tag radial, or (tag menu off) a toast with the thumbnail.
pub fn show(app: &AppHandle, req: Request) {
    let cfg = app.state::<AppState>().cfg();
    if !cfg.tag_menu {
        let thumb = match &req.preview {
            Preview::Image(t) => Some(t.clone()),
            Preview::Text(_) => None,
        };
        crate::toast::show(app, crate::toast::Toast { title: "Saved".into(), detail: req.meta, thumb, ms: 1600 });
        return;
    }
    let style = crate::theme::style_for(&cfg);
    let seq = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        let t0 = Instant::now();
        run(req, style, seq, t0);
    });
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Mode {
    Tag,
    Caption,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Hit {
    None,
    Wedge(usize),
    Pill,
    PenButton,
    Centre,
}

struct St {
    win: Layered,
    s: f32,
    c: (f32, f32), // ring centre (window coords)
    style: Style,
    tags: Vec<String>,
    applied: Vec<bool>,
    hprog: Vec<f32>,
    hover: Hit,
    field: TextField,
    mode: Mode,
    sidecars: Vec<PathBuf>,
    thumb: Option<Pixmap>,
    text_preview: Option<String>,
    meta: String,
    bg: Option<Pixmap>,
    source_hwnd: isize,
    seq: u64,
    t_open: Instant,
    last: Instant,
    last_cursor: (f32, f32),
    last_move: Instant,
    off_since: Option<Instant>,
    closing: Option<Instant>,
    restore_focus: bool,
    flash: Option<(usize, Instant)>,
    done: bool,
    blink: Instant,
}

fn run(mut req: Request, style: Style, seq: u64, t_req: Instant) {
    unsafe {
        let mut cur = POINT::default();
        let _ = GetCursorPos(&mut cur);
        let s = crate::winutil::monitor_at(cur.x, cur.y).scale;
        let tags = crate::tags::top(MAX_TAGS);
        let ring = !tags.is_empty();
        let outer = if ring { R_OUT + GROW } else { R_IN + 4.0 };
        let w = ((outer + MARGIN) * 2.0 * s).max((PILL_W + MARGIN * 2.0) * s).ceil() as i32;
        let ring_h = (outer + MARGIN) * 2.0 * s;
        let h = (ring_h + (PILL_GAP + PILL_H + META_H) * s).ceil() as i32;
        let half_w = w / 2;
        let mon = crate::winutil::monitor_rect_at(cur.x, cur.y);
        // ring centre on the cursor, the whole window kept on this monitor
        let cy_off = (ring_h / 2.0) as i32;
        let ox = (cur.x - half_w).clamp(mon.left, (mon.right - w).max(mon.left));
        let oy = (cur.y - cy_off).clamp(mon.top, (mon.bottom - h).max(mon.top));
        let origin = POINT { x: ox, y: oy };
        let c = ((cur.x - ox) as f32, (cur.y - oy) as f32);

        let frame = req.frame.take().or_else(|| crate::winutil::grab_rect(RECT { left: ox, top: oy, right: ox + w, bottom: oy + h }, false).ok().map(Arc::new));
        let bg = frame.as_deref().and_then(|f| gfx::blur_backdrop(f, origin, w, h, style.blur, style.saturation));
        drop(frame); // the full-desktop frame is large; only the blurred patch is kept
        let (thumb, text_preview) = match &req.preview {
            Preview::Image(t) => (thumb_pixmap(t), None),
            Preview::Text(t) => (None, Some(t.clone())),
        };

        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
        let Some(win) = Layered::new(w!("StashTagRadial"), w!("Stash tags"), Some(wnd_proc), ex, origin, w, h) else { return };
        let n = tags.len();
        let mut st = Box::new(St {
            win,
            s,
            c,
            style,
            applied: vec![false; n],
            hprog: vec![0.0; n],
            tags,
            hover: Hit::None,
            field: TextField::new(120),
            mode: Mode::Tag,
            sidecars: req.sidecars,
            thumb,
            text_preview,
            meta: req.meta,
            bg,
            source_hwnd: req.source_hwnd,
            seq,
            t_open: Instant::now(),
            last: Instant::now(),
            last_cursor: (c.0, c.1),
            last_move: Instant::now(),
            off_since: None,
            closing: None,
            restore_focus: true,
            flash: None,
            done: false,
            blink: Instant::now(),
        });
        SetWindowLongPtrW(st.win.hwnd, GWLP_USERDATA, &mut *st as *mut St as isize);
        paint(&mut st);
        let _ = ShowWindow(st.win.hwnd, SW_SHOW);
        crate::winutil::bring_to_front(st.win.hwnd.0 as isize);
        tracing::info!("tag radial visible {} ms after request ({} tags)", t_req.elapsed().as_millis(), st.tags.len());
        SetTimer(Some(st.win.hwnd), TIMER, 15, None);
        let mut msg = MSG::default();
        while !st.done && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = KillTimer(Some(st.win.hwnd), TIMER);
        let restore = st.restore_focus && GetForegroundWindow() == st.win.hwnd;
        SetWindowLongPtrW(st.win.hwnd, GWLP_USERDATA, 0);
        let src = st.source_hwnd;
        drop(st); // destroys the window
        if restore {
            crate::winutil::restore_focus(src);
        }
    }
}

fn thumb_pixmap(t: &Thumb) -> Option<Pixmap> {
    let mut pm = Pixmap::new(t.w, t.h)?;
    for (px, p) in pm.pixels_mut().iter_mut().zip(t.rgba.chunks_exact(4)) {
        *px = ColorU8::from_rgba(p[0], p[1], p[2], p[3]).premultiply();
    }
    Some(pm)
}

// ---------- geometry ----------
impl St {
    fn ring(&self) -> bool {
        !self.tags.is_empty()
    }
    fn n(&self) -> usize {
        self.tags.len()
    }
    fn pill_rect(&self) -> Rect {
        let s = self.s;
        let top = self.c.1 + (if self.ring() { R_OUT + GROW } else { R_IN + 4.0 } + PILL_GAP) * s;
        Rect::from_xywh(self.c.0 - PILL_W * s / 2.0, top, PILL_W * s, PILL_H * s).unwrap()
    }
    fn pen_button(&self) -> (f32, f32, f32) {
        let p = self.pill_rect();
        let r = (PILL_H / 2.0 - 5.0) * self.s;
        (p.right() - r - 5.0 * self.s, p.top() + p.height() / 2.0, r)
    }
    fn hit(&self, x: f32, y: f32) -> Hit {
        let (bx, by, br) = self.pen_button();
        if (x - bx).hypot(y - by) <= br + 2.0 {
            return Hit::PenButton;
        }
        let p = self.pill_rect();
        if x >= p.left() && x <= p.right() && y >= p.top() && y <= p.bottom() {
            return Hit::Pill;
        }
        let (dx, dy) = (x - self.c.0, y - self.c.1);
        let d = dx.hypot(dy);
        if d < R_IN * self.s {
            return Hit::Centre;
        }
        if self.ring() && d <= (R_OUT + GROW) * self.s {
            let n = self.n();
            let step = 360.0 / n as f32;
            let mut deg = dx.atan2(-dy).to_degrees();
            if deg < 0.0 {
                deg += 360.0;
            }
            return Hit::Wedge((((deg + step / 2.0) / step) as usize) % n);
        }
        Hit::None
    }
    /// Index of the tag Enter would apply for the current text (prefix match among the ring tags).
    fn enter_target(&self) -> Option<usize> {
        let q = self.field.text().trim().trim_start_matches('#').to_lowercase();
        if q.is_empty() || self.mode != Mode::Tag {
            return None;
        }
        self.tags.iter().position(|t| t.to_lowercase() == q).or_else(|| self.tags.iter().position(|t| t.to_lowercase().starts_with(&q)))
    }
    /// A known tag (anywhere in the library) completing the typed text, when no ring tag matches.
    fn completion(&self) -> Option<String> {
        if self.mode != Mode::Tag || self.enter_target().is_some() {
            return None;
        }
        let q = self.field.text().trim().trim_start_matches('#').to_lowercase();
        if q.is_empty() {
            return None;
        }
        crate::tags::all().into_iter().find(|t| t.to_lowercase().starts_with(&q) && t.to_lowercase() != q)
    }
    /// Pointer over the ring (or centre disc), the pill, or the gap between them, with a little slack.
    fn inside(&self, x: f32, y: f32) -> bool {
        let s = self.s;
        let slack = SLACK * s;
        let r = if self.ring() { (R_OUT + GROW) * s } else { R_IN * s };
        if (x - self.c.0).hypot(y - self.c.1) <= r + slack {
            return true;
        }
        let p = self.pill_rect();
        x >= p.left() - slack && x <= p.right() + slack && y >= self.c.1 && y <= p.bottom() + slack
    }
    fn busy(&self) -> bool {
        !self.field.is_empty() || self.mode == Mode::Caption
    }
}

// ---------- actions ----------
fn apply_tag(st: &mut St, tag: String, keep_open: bool, wedge: Option<usize>) {
    let sidecars = st.sidecars.clone();
    let t = vec![tag.clone()];
    crate::tags::bump(&t);
    std::thread::spawn(move || {
        for sc in &sidecars {
            if let Err(e) = crate::library::add_tags(sc, &t) {
                tracing::error!("tag {sc:?}: {e:#}");
            }
        }
    });
    tracing::info!("tagged #{tag} ({} items) after {} ms", st.sidecars.len(), st.t_open.elapsed().as_millis());
    if let Some(i) = wedge {
        st.applied[i] = true;
        st.flash = Some((i, Instant::now()));
    }
    st.field.clear();
    if !keep_open {
        begin_close(st, 110.0);
    }
}

fn apply_caption(st: &mut St) {
    let cap = st.field.text().trim().to_string();
    if !cap.is_empty() {
        let sidecars = st.sidecars.clone();
        let cap2 = cap.clone();
        std::thread::spawn(move || {
            for sc in &sidecars {
                if let Err(e) = crate::library::set_caption(sc, &cap2) {
                    tracing::error!("caption {sc:?}: {e:#}");
                }
            }
        });
        tracing::info!("caption set after {} ms", st.t_open.elapsed().as_millis());
    }
    begin_close(st, 0.0);
}

fn enter(st: &mut St, ctrl: bool) {
    if st.mode == Mode::Caption {
        apply_caption(st);
        return;
    }
    if let Some(i) = st.enter_target() {
        let tag = st.tags[i].clone();
        apply_tag(st, tag, ctrl, Some(i));
    } else if let Some(full) = st.completion() {
        apply_tag(st, full, ctrl, None);
    } else if let Some(tag) = crate::tags::clean(&st.field.text()) {
        apply_tag(st, tag, ctrl, None);
    } else {
        begin_close(st, 0.0);
    }
}

fn begin_close(st: &mut St, delay_ms: f32) {
    if st.closing.is_none() {
        st.closing = Some(Instant::now() + std::time::Duration::from_millis(delay_ms as u64));
    }
}

fn set_mode(st: &mut St, m: Mode) {
    if st.mode != m {
        st.mode = m;
        st.field.clear();
    }
}

// ---------- message loop ----------
fn client_cursor(st: &St) -> (f32, f32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    ((p.x - st.win.origin.x) as f32, (p.y - st.win.origin.y) as f32)
}

fn tick(st: &mut St) {
    let dt = st.last.elapsed().as_secs_f32() * 1000.0;
    st.last = Instant::now();
    if GENERATION.load(Ordering::SeqCst) != st.seq {
        st.restore_focus = false; // a newer capture took over
        begin_close(st, 0.0);
    }
    let (x, y) = client_cursor(st);
    st.hover = if st.closing.is_some() { Hit::None } else { st.hit(x, y) };
    for i in 0..st.n() {
        let target: f32 = if st.hover == Hit::Wedge(i) { 1.0 } else { 0.0 };
        let d = target - st.hprog[i];
        if d.abs() > 0.001 {
            st.hprog[i] += d.signum() * (dt / HOVER_MS).min(d.abs());
        }
    }
    // No timer: it stays while the pointer is on it and disappears once the pointer moves off (unless the user
    // is typing a tag / caption). A long idle fallback covers a pointer that never moves at all.
    if (x, y) != st.last_cursor {
        st.last_cursor = (x, y);
        st.last_move = Instant::now();
    }
    if st.closing.is_none() && st.t_open.elapsed().as_secs_f32() * 1000.0 > OPEN_MS {
        if st.inside(x, y) || st.busy() {
            st.off_since = None;
        } else if st.off_since.get_or_insert_with(Instant::now).elapsed().as_millis() > OFF_GRACE_MS {
            begin_close(st, 0.0);
        }
        if !st.busy() && st.last_move.elapsed().as_secs() >= IDLE_CLOSE_S {
            begin_close(st, 0.0);
        }
    }
    if let Some(t) = st.closing {
        if Instant::now() >= t && t.elapsed().as_secs_f32() * 1000.0 >= if st.style.animations { CLOSE_MS } else { 0.0 } {
            st.done = true;
            unsafe {
                let _ = PostMessageW(Some(st.win.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            }
            return;
        }
    }
    paint(st);
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let Some(st) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut St).as_mut() else {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        };
        let ctrl = (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
        let shift = (GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
        match msg {
            WM_TIMER => {
                tick(st);
                LRESULT(0)
            }
            WM_ACTIVATE => {
                // clicked somewhere else: close and leave focus where the user put it
                if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE && st.t_open.elapsed().as_millis() > 120 {
                    st.restore_focus = false;
                    begin_close(st, 0.0);
                }
                LRESULT(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if st.closing.is_some() {
                    return LRESULT(0);
                }
                let vk = wparam.0 as u32;
                st.blink = Instant::now();
                match vk {
                    0x0D => enter(st, ctrl),     // Enter
                    0x1B => begin_close(st, 0.0), // Esc
                    0x09 => set_mode(st, if st.mode == Mode::Tag { Mode::Caption } else { Mode::Tag }), // Tab
                    _ => {
                        if st.field.key_down(vk, ctrl, shift) == Outcome::Ignored && msg == WM_SYSKEYDOWN {
                            return DefWindowProcW(hwnd, msg, wparam, lparam);
                        }
                    }
                }
                paint(st);
                LRESULT(0)
            }
            WM_CHAR => {
                if st.closing.is_none() {
                    if let Some(c) = char::from_u32(wparam.0 as u32) {
                        if st.mode == Mode::Tag && c == ' ' && st.field.is_empty() {
                            return LRESULT(0); // a leading space is never part of a tag
                        }
                        st.field.char_input(c);
                        st.blink = Instant::now();
                        paint(st);
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if st.closing.is_some() {
                    return LRESULT(0);
                }
                let (x, y) = ((lparam.0 & 0xFFFF) as i16 as f32, ((lparam.0 >> 16) & 0xFFFF) as i16 as f32);
                match st.hit(x, y) {
                    Hit::Wedge(i) => {
                        let tag = st.tags[i].clone();
                        apply_tag(st, tag, ctrl, Some(i));
                    }
                    Hit::PenButton => set_mode(st, if st.mode == Mode::Tag { Mode::Caption } else { Mode::Tag }),
                    Hit::Pill | Hit::Centre => {}
                    Hit::None => {
                        st.restore_focus = false;
                        begin_close(st, 0.0);
                    }
                }
                paint(st);
                LRESULT(0)
            }
            WM_RBUTTONDOWN | WM_CLOSE => {
                // (Alt+F4 included: close through our own fade so the loop and the window end together)
                begin_close(st, 0.0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

// ---------- drawing ----------
fn wedge_path(c: (f32, f32), i: usize, n: usize, r_in: f32, r_out: f32, gap: f32) -> Option<Path> {
    if n == 1 {
        let mut pb = PathBuilder::new();
        pb.push_circle(c.0, c.1, r_out);
        pb.push_circle(c.0, c.1, r_in);
        return pb.finish();
    }
    let step = 360.0 / n as f32;
    let mid = i as f32 * step;
    let half = step / 2.0;
    let d = gap / 2.0;
    let off = |r: f32| (d / r).clamp(0.0, 1.0).asin().to_degrees();
    let pt = |r: f32, deg: f32| {
        let a = deg.to_radians();
        (c.0 + r * a.sin(), c.1 - r * a.cos())
    };
    let mut pb = PathBuilder::new();
    let steps = 16;
    let (a0, a1) = (mid - half + off(r_out), mid + half - off(r_out));
    let p = pt(r_out, a0);
    pb.move_to(p.0, p.1);
    for k in 1..=steps {
        let p = pt(r_out, a0 + (a1 - a0) * k as f32 / steps as f32);
        pb.line_to(p.0, p.1);
    }
    let (b0, b1) = (mid - half + off(r_in), mid + half - off(r_in));
    for k in (0..=steps).rev() {
        let p = pt(r_in, b0 + (b1 - b0) * k as f32 / steps as f32);
        pb.line_to(p.0, p.1);
    }
    pb.close();
    pb.finish()
}

fn paint(st: &mut St) {
    let scene = render(st);
    let age = st.t_open.elapsed().as_secs_f32() * 1000.0;
    let (mut op, mut sc) = if st.style.animations {
        let k = gfx::ease_out(age / OPEN_MS);
        (k, 0.86 + 0.14 * k)
    } else {
        (1.0, 1.0)
    };
    if let Some(t) = st.closing {
        if Instant::now() >= t {
            let k = if st.style.animations { (t.elapsed().as_secs_f32() * 1000.0 / CLOSE_MS).clamp(0.0, 1.0) } else { 1.0 };
            op *= 1.0 - k;
            sc *= 1.0 - 0.06 * k;
        }
    }
    let out = gfx::compose(&scene, op, sc, st.c.0, st.c.1);
    st.win.present(&out);
}

fn render(st: &mut St) -> Pixmap {
    let s = st.s;
    let (w, h) = (st.win.w as u32, st.win.h as u32);
    let mut pm = Pixmap::new(w, h).unwrap();
    let c = st.c;
    let acc = st.style.hover;
    let white = [255u8, 255, 255];
    let n = st.n();
    let gap = 4.0 * s;
    let r_in = R_IN * s;
    let radii: Vec<f32> = (0..n).map(|i| (R_OUT + GROW * gfx::ease_out(st.hprog[i])) * s).collect();
    let wedges: Vec<Path> = (0..n).filter_map(|i| wedge_path(c, i, n, r_in, radii[i], gap)).collect();
    let pill = st.pill_rect();
    let pill_path = gfx::rrect(pill.left(), pill.top(), pill.width(), pill.height(), pill.height() / 2.0);
    let disc = gfx::circle(c.0, c.1, r_in - gap);

    // shadows
    let outer = if st.ring() { R_OUT * s } else { r_in };
    for k in (1..=7).rev() {
        if let Some(p) = gfx::circle(c.0, c.1 + 4.0 * s, outer + k as f32 * 2.4 * s) {
            pm.fill_path(&p, &gfx::paint([0, 0, 0, (16.0 / k as f32) as u8]), FillRule::Winding, Transform::identity(), None);
        }
    }
    gfx::shadow_rrect(&mut pm, pill.left(), pill.top(), pill.width(), pill.height(), pill.height() / 2.0, 8.0 * s, 0.06);

    // glass body: ring wedges + centre disc + pill share one material
    let mut pb = PathBuilder::new();
    for p in &wedges {
        pb.push_path(p);
    }
    if let Some(d) = &disc {
        pb.push_path(d);
    }
    if let Some(p) = &pill_path {
        pb.push_path(p);
    }
    let bounds = Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap();
    if let Some(body) = pb.finish() {
        pm.fill_path(&body, &Paint { blend_mode: BlendMode::Clear, ..Default::default() }, FillRule::Winding, Transform::identity(), None);
        gfx::glass_fill(&mut pm, &body, bounds, st.bg.as_ref(), &st.style, 1.0);
        gfx::cursor_glow(&mut pm, &body, st.last_cursor.0, st.last_cursor.1, R_OUT * s * 0.85, &st.style);
    }

    // wedges: hover / match / applied states, labels
    let target = st.enter_target();
    let q = st.field.text().trim().trim_start_matches('#').to_lowercase();
    let typing = st.mode == Mode::Tag && !q.is_empty();
    for (i, p) in wedges.iter().enumerate() {
        let hv = st.hprog[i];
        let is_match = typing && st.tags[i].to_lowercase().starts_with(&q);
        let mut fill = [255, 255, 255, 8];
        if st.applied[i] {
            fill = [acc[0], acc[1], acc[2], 170];
        } else if Some(i) == target {
            fill = [acc[0], acc[1], acc[2], 120];
        } else if hv > 0.0 {
            fill = [acc[0], acc[1], acc[2], (125.0 * hv) as u8];
        } else if is_match {
            fill = [acc[0], acc[1], acc[2], 50];
        }
        if let Some((fi, t)) = st.flash {
            let k = t.elapsed().as_secs_f32() * 1000.0 / 220.0;
            if fi == i && k < 1.0 {
                fill = [acc[0], acc[1], acc[2], (230.0 - 60.0 * k) as u8];
            }
        }
        pm.fill_path(p, &gfx::paint(fill), FillRule::Winding, Transform::identity(), None);
        gfx::glass_rim(&mut pm, p, Rect::from_xywh(c.0 - radii[i], c.1 - radii[i], radii[i] * 2.0, radii[i] * 2.0).unwrap(), 1.0 * s, 0.9);
        // label at the wedge middle
        let step = 360.0 / n as f32;
        let a = (i as f32 * step).to_radians();
        let rm = (r_in + radii[i]) / 2.0 + 2.0 * s;
        let (lx, ly) = (c.0 + rm * a.sin(), c.1 - rm * a.cos());
        let px = 12.5 * s;
        let room = if n == 1 { 120.0 * s } else { (2.0 * std::f32::consts::PI * rm / n as f32 - 14.0 * s).min(96.0 * s) };
        let dim = typing && !is_match && !st.applied[i];
        let alpha = if dim { 95 } else { 245 };
        let label = gfx::ellipsize(&format!("#{}", st.tags[i]), px, room, Weight::Semibold);
        let lw = gfx::text_width(&label, px);
        if crate::theme::frost(&st.style) < 0.5 {
            // clear glass: a dark halo keeps labels readable over a busy background
            let ly2 = if st.applied[i] { ly + 6.0 * s } else { ly };
            for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (1.0, 1.0)] {
                gfx::draw_text_at(&mut pm, &label, px, lx - lw / 2.0 + dx * s, ly2 + dy * s, [0, 0, 0], 90, Weight::Semibold, None);
            }
        }
        if st.applied[i] {
            check(&mut pm, lx, ly - 11.0 * s, 5.0 * s, white);
            gfx::draw_text_at(&mut pm, &label, px, lx - lw / 2.0, ly + 6.0 * s, white, 255, Weight::Semibold, None);
        } else {
            gfx::draw_text_at(&mut pm, &label, px, lx - lw / 2.0, ly, white, alpha, Weight::Semibold, None);
        }
    }

    // centre: the capture itself
    if let Some(d) = &disc {
        let r = r_in - gap;
        let dr = Rect::from_xywh(c.0 - r, c.1 - r, r * 2.0, r * 2.0).unwrap();
        if let Some(t) = &st.thumb {
            let k = (dr.width() / t.width() as f32).max(dr.height() / t.height() as f32);
            let (dw, dh) = (t.width() as f32 * k, t.height() as f32 * k);
            if let Some(mut mask) = Mask::new(w, h) {
                mask.fill_path(d, FillRule::Winding, true, Transform::identity());
                pm.draw_pixmap(
                    0,
                    0,
                    t.as_ref(),
                    &PixmapPaint { quality: FilterQuality::Bicubic, ..Default::default() },
                    Transform::from_row(k, 0.0, 0.0, k, dr.left() + (dr.width() - dw) / 2.0, dr.top() + (dr.height() - dh) / 2.0),
                    Some(&mask),
                );
            }
        } else if let Some(t) = &st.text_preview {
            pm.fill_path(d, &gfx::paint([255, 255, 255, 14]), FillRule::Winding, Transform::identity(), None);
            let px = 11.5 * s;
            let words: Vec<&str> = t.split_whitespace().collect();
            let mut lines: Vec<String> = Vec::new();
            let mut cur = String::new();
            let mut cut = false;
            for wd in words {
                let next = if cur.is_empty() { wd.to_string() } else { format!("{cur} {wd}") };
                if gfx::text_width_w(&next, px, Weight::Regular) > r * 1.45 && !cur.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                    cur = wd.to_string();
                    if lines.len() == 3 {
                        cut = true;
                        break;
                    }
                } else {
                    cur = next;
                }
            }
            if lines.len() < 3 && !cur.is_empty() {
                lines.push(cur);
            }
            if cut {
                if let Some(last) = lines.last_mut() {
                    last.push('…');
                }
            }
            let lh = px * 1.35;
            let y0 = c.1 - lh * (lines.len() as f32 - 1.0) / 2.0;
            for (k, l) in lines.iter().enumerate() {
                let l = gfx::ellipsize(l, px, r * 1.45, Weight::Regular);
                let lw = gfx::text_width_w(&l, px, Weight::Regular);
                gfx::draw_text_at(&mut pm, &l, px, c.0 - lw / 2.0, y0 + lh * k as f32, white, 225, Weight::Regular, None);
            }
        }
        gfx::glass_rim(&mut pm, d, dr, 1.2 * s, 1.0);
    }

    // text pill
    if let Some(p) = &pill_path {
        if st.mode == Mode::Caption {
            pm.fill_path(p, &gfx::paint([acc[0], acc[1], acc[2], 40]), FillRule::Winding, Transform::identity(), None);
        }
        gfx::glass_rim(&mut pm, p, pill, 1.0 * s, 1.0);
        let (bx, by, br) = st.pen_button();
        // leading glyph: '#' for tags, pen for captions
        let gx = pill.left() + 16.0 * s;
        if st.mode == Mode::Tag {
            gfx::draw_text_at(&mut pm, "#", 15.0 * s, gx - 3.0 * s, by, white, 150, Weight::Semibold, None);
        } else {
            pen(&mut pm, gx + 2.0 * s, by, 6.0 * s, white, 170, 1.4 * s);
        }
        let field_r = Rect::from_ltrb(pill.left() + 30.0 * s, pill.top(), bx - br - 8.0 * s, pill.bottom()).unwrap();
        let blink_on = (st.blink.elapsed().as_millis() / 530) % 2 == 0;
        let focused = unsafe { GetForegroundWindow() == st.win.hwnd };
        let placeholder = match st.mode {
            Mode::Tag if st.ring() => "Type a tag…",
            Mode::Tag => "Type a tag, Enter to add",
            Mode::Caption => "Caption, Enter to save",
        };
        st.field.draw(&mut pm, field_r, 13.5 * s, white, placeholder, focused && blink_on && st.closing.is_none(), acc);
        // ghost completion
        if let Some(full) = st.completion() {
            let typed = st.field.text();
            let tw = gfx::text_width_w(&typed, 13.5 * s, Weight::Regular);
            let rest: String = full.chars().skip(typed.trim_start_matches('#').chars().count()).collect();
            gfx::draw_text_at(&mut pm, &rest, 13.5 * s, field_r.left() + tw + 1.0, by, white, 95, Weight::Regular, Some((field_r.left(), field_r.right())));
        }
        // pen / tag toggle button
        if let Some(b) = gfx::circle(bx, by, br) {
            let on = st.hover == Hit::PenButton;
            let a = if st.mode == Mode::Caption { 150 } else if on { 70 } else { 28 };
            let col = if st.mode == Mode::Caption { [acc[0], acc[1], acc[2], a] } else { [255, 255, 255, a] };
            pm.fill_path(&b, &gfx::paint(col), FillRule::Winding, Transform::identity(), None);
            gfx::glass_rim(&mut pm, &b, Rect::from_xywh(bx - br, by - br, br * 2.0, br * 2.0).unwrap(), 1.0 * s, 0.8);
            if st.mode == Mode::Caption {
                gfx::draw_text_at(&mut pm, "#", 14.0 * s, bx - 4.0 * s, by, white, 235, Weight::Semibold, None);
            } else {
                pen(&mut pm, bx, by, 6.5 * s, white, 230, 1.5 * s);
            }
        }
    }
    // meta line ("Saved · Region 640×480 · chrome") under the pill
    let meta = gfx::ellipsize(&format!("Saved · {}", st.meta), 11.0 * s, w as f32 - 24.0 * s, Weight::Regular);
    let mw = gfx::text_width_w(&meta, 11.0 * s, Weight::Regular);
    // soft dark halo first so the line reads on bright backgrounds too
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        gfx::draw_text_at(&mut pm, &meta, 11.0 * s, c.0 - mw / 2.0 + dx, pill.bottom() + 13.0 * s + dy, [0, 0, 0], 70, Weight::Regular, None);
    }
    gfx::draw_text_at(&mut pm, &meta, 11.0 * s, c.0 - mw / 2.0, pill.bottom() + 13.0 * s, white, 200, Weight::Regular, None);
    pm
}

fn check(pm: &mut Pixmap, x: f32, y: f32, size: f32, col: [u8; 3]) {
    let mut pb = PathBuilder::new();
    pb.move_to(x - size, y);
    pb.line_to(x - size * 0.3, y + size * 0.7);
    pb.line_to(x + size, y - size * 0.7);
    if let Some(p) = pb.finish() {
        let stroke = Stroke { width: size * 0.38, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        pm.stroke_path(&p, &gfx::paint([col[0], col[1], col[2], 255]), &stroke, Transform::identity(), None);
    }
}

fn pen(pm: &mut Pixmap, x: f32, y: f32, size: f32, col: [u8; 3], alpha: u8, width: f32) {
    let mut pb = PathBuilder::new();
    let pts = [(-0.75, 0.75), (-0.6, 0.25), (0.4, -0.75), (0.8, -0.35), (-0.2, 0.6)];
    pb.move_to(x + pts[0].0 * size, y + pts[0].1 * size);
    for p in &pts[1..] {
        pb.line_to(x + p.0 * size, y + p.1 * size);
    }
    pb.close();
    if let Some(p) = pb.finish() {
        let stroke = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        pm.stroke_path(&p, &gfx::paint([col[0], col[1], col[2], alpha]), &stroke, Transform::identity(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::preview;

    #[allow(clippy::too_many_arguments)]
    fn scene(tags: &[&str], typed: &str, mode: Mode, hover: Option<usize>, applied: Option<usize>, text: Option<&str>, name: &str) {
        let s = 1.5;
        let style = preview::style();
        let frame = preview::backdrop(1000, 800);
        let ring = !tags.is_empty();
        let outer = if ring { R_OUT + GROW } else { R_IN + 4.0 };
        let w = ((outer + MARGIN) * 2.0 * s).max((PILL_W + MARGIN * 2.0) * s).ceil() as i32;
        let ring_h = (outer + MARGIN) * 2.0 * s;
        let h = (ring_h + (PILL_GAP + PILL_H + META_H) * s).ceil() as i32;
        let origin = POINT { x: 150, y: 120 };
        let bg = gfx::blur_backdrop(&frame, origin, w, h, style.blur, style.saturation);
        // a hidden window: never shown
        let win = unsafe { Layered::new(w!("StashTagRadialPreview"), w!("preview"), Some(wnd_proc), WINDOW_EX_STYLE(0), origin, w, h) }.unwrap();
        // the capture: a crop of the backdrop
        let (tw, th) = (300u32, 200u32);
        let mut rgba = Vec::new();
        for y in 0..th as i32 {
            for x in 0..tw as i32 {
                let i = (((y + 380) * frame.w + x + 600) * 4) as usize;
                rgba.extend_from_slice(&[frame.bgra[i + 2], frame.bgra[i + 1], frame.bgra[i], 255]);
            }
        }
        let n = tags.len();
        let mut st = St {
            win,
            s,
            c: (w as f32 / 2.0, ring_h / 2.0),
            style,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            applied: (0..n).map(|i| Some(i) == applied).collect(),
            hprog: (0..n).map(|i| if Some(i) == hover { 1.0 } else { 0.0 }).collect(),
            hover: hover.map(Hit::Wedge).unwrap_or(Hit::None),
            field: TextField::new(120),
            mode,
            sidecars: vec![],
            thumb: if text.is_none() { thumb_pixmap(&Thumb { rgba, w: tw, h: th }) } else { None },
            text_preview: text.map(String::from),
            meta: "Region · 1115×628 · chrome".into(),
            bg,
            source_hwnd: 0,
            seq: 0,
            t_open: Instant::now(),
            last: Instant::now(),
            last_cursor: (w as f32 * 0.66, ring_h * 0.3), // where the cursor glow shows in the previews
            last_move: Instant::now(),
            off_since: None,
            closing: None,
            restore_focus: false,
            flash: None,
            done: false,
            blink: Instant::now(),
        };
        st.field.set_text(typed);
        let pm = render(&mut st);
        preview::save(&pm, &frame, origin, name);
    }

    #[test]
    #[ignore]
    fn previews() {
        let tags = ["moodboard", "typography", "ui-ref", "motion", "colour", "yt-ideas"];
        scene(&tags, "", Mode::Tag, None, None, None, "radial_idle.png");
        scene(&tags, "", Mode::Tag, Some(1), None, None, "radial_hover.png");
        scene(&tags, "mo", Mode::Tag, None, Some(2), None, "radial_typing.png");
        scene(&tags, "Love the kerning on this headline", Mode::Caption, None, None, None, "radial_caption.png");
        scene(&tags[..3], "", Mode::Tag, None, None, Some("A quote worth keeping about creative constraints and limits"), "radial_text_3tags.png");
        scene(&[], "", Mode::Tag, None, None, None, "radial_first_run.png");
    }
}
