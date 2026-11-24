//! Tag radial (SPEC §4): appears at the cursor right after a capture is saved. Ring = up to 6 recent/frequent
//! tags; centre = Favourite (top half) and Discard (bottom half, deletes the capture again); below = a text pill
//! to type a new tag or (Tab / the pen button) a caption. It stays while the pointer is on it and fades out once
//! the pointer moves off (the item stays saved, untagged). When it closes, a toast confirms what was kept.
//! The ring shows 6 tags at a time out of up to RING_POOL: the mouse wheel turns it to the others, number keys 1-6
//! pick a slice, clicking an applied tag removes it again. Each tag has its own colour (a band on its slice), and a
//! soft, blurred image of the capture glows through the glass (cover-cropped, never stretched).
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

/// Slices on the ring.
const MAX_TAGS: usize = 6;
/// Tags the ring can turn through with the mouse wheel.
const RING_POOL: usize = 24;
/// Opacity of the blurred capture behind the glass.
const GHOST_ALPHA: f32 = 0.30;
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
const OFF_GRACE_MS: u128 = 300;
/// After the grace period it fades out over this long; moving back onto it during the fade brings it back.
const LEAVE_MS: f32 = 260.0;
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
    let app = app.clone();
    if !cfg.tag_menu {
        let thumb = match &req.preview {
            Preview::Image(t) => Some(t.clone()),
            Preview::Text(_) => None,
        };
        crate::toast::show(&app, crate::toast::Toast { title: "Saved".into(), detail: req.meta, thumb, ms: 1600, fx: true });
        return;
    }
    let style = crate::theme::style_for(&cfg);
    let seq = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        let t0 = Instant::now();
        if let Some(t) = run(req, style, seq, t0) {
            crate::toast::show(&app, t);
        }
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
    /// Top half of the centre disc.
    Star,
    /// Bottom half of the centre disc.
    Discard,
}

struct St {
    win: Layered,
    s: f32,
    c: (f32, f32), // ring centre (window coords)
    style: Style,
    /// Every tag the ring can show (most used first); `applied` is indexed like it.
    tags: Vec<String>,
    applied: Vec<bool>,
    /// Hover progress per slice.
    hprog: Vec<f32>,
    /// First tag on the ring (the wheel turns it) and the turn still being animated (degrees, eases to 0).
    offset: usize,
    spin: f32,
    /// Blurred capture behind the glass (images only), centred on the ring.
    ghost: Option<Pixmap>,
    /// The WM_CHAR after a number key that picked a slice must not type the digit.
    eat_char: bool,
    hover: Hit,
    field: TextField,
    mode: Mode,
    sidecars: Vec<PathBuf>,
    /// The capture's thumbnail, for the toast that confirms the save when the radial closes.
    thumb: Option<Arc<Thumb>>,
    /// Text captures: the start of the text (the toast quotes it).
    snippet: Option<String>,
    meta: String,
    /// Starred from the centre (Favourites tab in the gallery); when it was last toggled (pop animation).
    fav: bool,
    fav_t: Option<Instant>,
    /// Tags applied so far (for the toast).
    tagged: Vec<String>,
    /// The capture was deleted again (Discard).
    discarded: bool,
    bg: Option<Pixmap>,
    source_hwnd: isize,
    seq: u64,
    t_open: Instant,
    last: Instant,
    last_cursor: (f32, f32),
    /// Cursor glow position (eases toward the cursor).
    glow_pos: (f32, f32),
    last_move: Instant,
    off_since: Option<Instant>,
    /// Where the pointer was when the radial opened (window coords). Near a screen edge the ring is moved away
    /// from the pointer, so pointer-off only counts once the pointer has been on it (`entered`) or moved away.
    open_at: (f32, f32),
    entered: bool,
    /// Pointer-off fade: 0 = fully shown .. 1 = gone. It can reverse while the pointer comes back.
    leave: f32,
    closing: Option<Instant>,
    restore_focus: bool,
    flash: Option<(usize, Instant)>,
    done: bool,
    blink: Instant,
    /// Caret blink phase of the last painted frame (ticks repaint only when something visible changed).
    blink_phase: u128,
}

/// Runs the radial until it closes. Returns the toast that confirms the outcome (saved, starred, discarded).
fn run(mut req: Request, style: Style, seq: u64, t_req: Instant) -> Option<crate::toast::Toast> {
    unsafe {
        let mut cur = POINT::default();
        let _ = GetCursorPos(&mut cur);
        let s = crate::winutil::monitor_at(cur.x, cur.y).scale;
        let tags = crate::tags::top(RING_POOL);
        let ring = !tags.is_empty();
        let outer = if ring { R_OUT + GROW } else { R_IN + 4.0 };
        let w = ((outer + MARGIN) * 2.0 * s).max((PILL_W + MARGIN * 2.0) * s).ceil() as i32;
        let ring_h = (outer + MARGIN) * 2.0 * s;
        let h = (ring_h + (PILL_GAP + PILL_H + META_H) * s).ceil() as i32;
        let half_w = w / 2;
        // ring centred on the cursor, but the whole radial (ring + text pill below it) stays inside the work area
        // (not under the taskbar): near an edge the ring moves inward instead of being cut off. The window
        // margin around the ring only holds its shadow, so it may hang over the edge.
        let wa = crate::winutil::work_rect_at(cur.x, cur.y);
        let cy_off = (ring_h / 2.0) as i32;
        let m = (MARGIN * s) as i32;
        let ox = (cur.x - half_w).clamp(wa.left - m, (wa.right + m - w).max(wa.left - m));
        let oy = (cur.y - cy_off).clamp(wa.top - m, (wa.bottom - h).max(wa.top - m));
        let origin = POINT { x: ox, y: oy };
        let c = (half_w as f32, cy_off as f32);

        let frame = req.frame.take().or_else(|| crate::winutil::grab_rect(RECT { left: ox, top: oy, right: ox + w, bottom: oy + h }, false).ok().map(Arc::new));
        let bg = frame.as_deref().and_then(|f| gfx::blur_backdrop(f, origin, w, h, style.blur, style.saturation));
        drop(frame); // the full-desktop frame is large; only the blurred patch is kept
        let (thumb, snippet) = match &req.preview {
            Preview::Image(t) => (Some(t.clone()), None),
            Preview::Text(t) => (None, Some(t.split_whitespace().collect::<Vec<_>>().join(" "))),
        };

        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
        let win = Layered::new(w!("StashTagRadial"), w!("Stash tags"), Some(wnd_proc), ex, origin, w, h)?;
        let n = tags.len();
        let ghost = thumb.as_deref().and_then(|t| ghost_pixmap(t, ((R_OUT + GROW) * 2.0 * s).ceil() as u32));
        let mut st = Box::new(St {
            win,
            s,
            c,
            style,
            applied: vec![false; n],
            hprog: vec![0.0; n.min(MAX_TAGS)],
            offset: 0,
            spin: 0.0,
            ghost,
            eat_char: false,
            tags,
            hover: Hit::None,
            field: TextField::new(120),
            mode: Mode::Tag,
            sidecars: req.sidecars,
            thumb,
            snippet,
            meta: req.meta,
            fav: false,
            fav_t: None,
            tagged: Vec::new(),
            discarded: false,
            bg,
            source_hwnd: req.source_hwnd,
            seq,
            t_open: Instant::now(),
            last: Instant::now(),
            last_cursor: ((cur.x - ox) as f32, (cur.y - oy) as f32),
            glow_pos: (-1000.0, -1000.0),
            last_move: Instant::now(),
            off_since: None,
            open_at: ((cur.x - ox) as f32, (cur.y - oy) as f32),
            entered: false,
            leave: 0.0,
            closing: None,
            restore_focus: true,
            flash: None,
            done: false,
            blink: Instant::now(),
            blink_phase: 0,
        });
        SetWindowLongPtrW(st.win.hwnd, GWLP_USERDATA, &mut *st as *mut St as isize);
        paint(&mut st);
        crate::winutil::prime_cursor(IDC_ARROW);
        let _ = ShowWindow(st.win.hwnd, SW_SHOW);
        crate::winutil::bring_to_front(st.win.hwnd.0 as isize);
        tracing::info!("tag radial visible {} ms after request ({} tags)", t_req.elapsed().as_millis(), st.tags.len());
        let pacer = gfx::FramePacer::start(st.win.hwnd, TIMER);
        let mut msg = MSG::default();
        while !st.done && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        drop(pacer);
        let restore = st.restore_focus && GetForegroundWindow() == st.win.hwnd;
        SetWindowLongPtrW(st.win.hwnd, GWLP_USERDATA, 0);
        let src = st.source_hwnd;
        let toast = outcome_toast(&st);
        drop(st); // destroys the window
        if restore {
            crate::winutil::restore_focus(src);
        }
        Some(toast)
    }
}

/// "Saved" (with the thumbnail, the tags and where it came from), "Saved to Favourites" or "Discarded".
fn outcome_toast(st: &St) -> crate::toast::Toast {
    if st.discarded {
        let n = st.sidecars.len();
        let detail = if n > 1 { format!("{n} items were not kept") } else { "The capture was not kept".into() };
        return crate::toast::Toast { title: "Discarded".into(), detail, thumb: None, ms: 1500, fx: false };
    }
    let tags: Vec<String> = st.tagged.iter().map(|t| format!("#{t}")).collect();
    let what = st.snippet.as_ref().filter(|t| !t.is_empty()).map(|t| format!("“{t}”")).unwrap_or_else(|| st.meta.clone());
    let detail = if tags.is_empty() { what } else { format!("{} · {what}", tags.join(" ")) };
    let title = if st.fav { "Saved to Favourites" } else { "Saved" };
    crate::toast::Toast { title: title.into(), detail, thumb: st.thumb.clone(), ms: 1700, fx: true }
}

// ---------- geometry ----------
impl St {
    fn ring(&self) -> bool {
        !self.tags.is_empty()
    }
    /// Slices on the ring.
    fn n(&self) -> usize {
        self.tags.len().min(MAX_TAGS)
    }
    /// Index (into `tags`) of the tag on slice `i`.
    fn tag_at(&self, i: usize) -> usize {
        (self.offset + i) % self.tags.len().max(1)
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
            return if dy < 0.0 { Hit::Star } else { Hit::Discard };
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
    if !st.tagged.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
        st.tagged.push(tag);
    }
    if let Some(i) = wedge {
        st.applied[i] = true;
        st.flash = Some((i, Instant::now()));
    }
    // a tag typed by name that is on the ring lights up there too
    if let Some(i) = st.tagged.last().and_then(|t| st.tags.iter().position(|x| x.eq_ignore_ascii_case(t))) {
        st.applied[i] = true;
    }
    st.field.clear();
    if !keep_open {
        begin_close(st, 110.0);
    }
}

/// Clicking an applied tag again takes it off the capture (an instant undo for a misclick).
fn unapply_tag(st: &mut St, i: usize) {
    let tag = st.tags[i].clone();
    st.applied[i] = false;
    st.tagged.retain(|t| !t.eq_ignore_ascii_case(&tag));
    st.flash = None;
    let sidecars = st.sidecars.clone();
    std::thread::spawn(move || {
        for sc in &sidecars {
            if let Err(e) = crate::library::remove_tags(sc, std::slice::from_ref(&tag)) {
                tracing::error!("untag {sc:?}: {e:#}");
            }
        }
    });
}

/// Applies the tag on slice `slot`, or removes it when it is already applied.
fn pick_slot(st: &mut St, slot: usize, keep_open: bool) {
    if slot >= st.n() {
        return;
    }
    let i = st.tag_at(slot);
    if st.applied[i] {
        unapply_tag(st, i);
    } else {
        let tag = st.tags[i].clone();
        apply_tag(st, tag, keep_open, Some(i));
    }
}

/// Turns the ring by one tag (mouse wheel): `dir` +1 = next tags, -1 = previous.
fn turn(st: &mut St, dir: i32) {
    let total = st.tags.len();
    if total <= MAX_TAGS {
        return;
    }
    st.offset = ((st.offset as i32 + dir).rem_euclid(total as i32)) as usize;
    // the labels start where they were and glide one slice over
    let step = 360.0 / st.n() as f32;
    st.spin = if st.style.animations { (st.spin + dir as f32 * step).clamp(-2.0 * step, 2.0 * step) } else { 0.0 };
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

/// Stars / un-stars the capture (written in the background; the gallery shows it in Favourites).
fn toggle_fav(st: &mut St) {
    st.fav = !st.fav;
    st.fav_t = Some(Instant::now());
    let (sidecars, on) = (st.sidecars.clone(), st.fav);
    std::thread::spawn(move || {
        for sc in &sidecars {
            if let Err(e) = crate::library::set_favorite(sc, on) {
                tracing::error!("favourite {sc:?}: {e:#}");
            }
        }
    });
}

/// Cancels the save: the capture is deleted for good and the radial closes (the toast says so).
fn discard(st: &mut St) {
    st.discarded = true;
    let sidecars = st.sidecars.clone();
    std::thread::spawn(move || {
        for sc in &sidecars {
            if let Err(e) = crate::library::discard(sc) {
                tracing::error!("discard {sc:?}: {e:#}");
            }
        }
    });
    tracing::info!("capture discarded after {} ms", st.t_open.elapsed().as_millis());
    begin_close(st, 0.0);
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
    // repaint only when something on screen moves: the pacer ticks at the display rate (up to 240 Hz)
    let age = st.t_open.elapsed().as_secs_f32() * 1000.0;
    let mut dirty = age < gfx::open_total_ms(&st.style).max(OPEN_MS) + 40.0 || st.spin != 0.0 || st.closing.is_some();
    if st.style.glow > 0.0 {
        let (nx, ny) = gfx::GLOW_NUDGE;
        let style = st.style;
        dirty |= gfx::glow_follow(&mut st.glow_pos, (x + nx * st.s, y + ny * st.s), dt, &style);
    }
    let hover = if st.closing.is_some() { Hit::None } else { st.hit(x, y) };
    dirty |= hover != st.hover;
    st.hover = hover;
    dirty |= st.flash.is_some_and(|(_, t)| t.elapsed().as_millis() < 260) || st.fav_t.is_some_and(|t| t.elapsed().as_millis() < 380);
    let phase = st.blink.elapsed().as_millis() / 530;
    dirty |= phase != st.blink_phase;
    let leave0 = st.leave;
    if st.spin != 0.0 {
        st.spin -= st.spin * (1.0 - (-dt / 55.0).exp());
        if st.spin.abs() < 0.15 {
            st.spin = 0.0;
        }
    }
    for i in 0..st.n() {
        let target: f32 = if st.hover == Hit::Wedge(i) { 1.0 } else { 0.0 };
        let d = target - st.hprog[i];
        if d.abs() > 0.001 {
            st.hprog[i] += d.signum() * (dt / HOVER_MS).min(d.abs());
            dirty = true;
        }
    }
    // No timer: it stays while the pointer is on it and disappears once the pointer moves off (unless the user
    // is typing a tag / caption). A long idle fallback covers a pointer that never moves at all.
    if (x, y) != st.last_cursor {
        st.last_cursor = (x, y);
        st.last_move = Instant::now();
    }
    if st.closing.is_none() && st.t_open.elapsed().as_secs_f32() * 1000.0 > OPEN_MS {
        // pointer off for OFF_GRACE_MS: fade out; back on it (even mid-fade): fade back in
        if !st.entered && (st.inside(x, y) || (x - st.open_at.0).hypot(y - st.open_at.1) > 90.0 * st.s) {
            st.entered = true;
        }
        let leaving = if st.inside(x, y) || st.busy() || !st.entered {
            st.off_since = None;
            false
        } else {
            st.off_since.get_or_insert_with(Instant::now).elapsed().as_millis() > OFF_GRACE_MS
        };
        let target = if leaving { 1.0 } else { 0.0 };
        if !st.style.animations {
            st.leave = target;
        } else if leaving {
            st.leave = (st.leave + dt / LEAVE_MS).min(1.0);
        } else {
            st.leave = (st.leave - dt / 140.0).max(0.0);
        }
        if leaving && st.leave >= 1.0 {
            st.done = true;
            unsafe {
                let _ = PostMessageW(Some(st.win.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            }
            return;
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
    if dirty || st.leave != leave0 {
        paint(st);
    }
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
                // 1-6 (top row or numpad) pick a slice while nothing is typed
                let digit = match vk {
                    0x31..=0x39 => Some((vk - 0x31) as usize),
                    0x61..=0x69 => Some((vk - 0x61) as usize),
                    _ => None,
                };
                if let Some(d) = digit.filter(|&d| st.mode == Mode::Tag && st.field.is_empty() && d < st.n()) {
                    st.eat_char = true;
                    pick_slot(st, d, ctrl);
                    paint(st);
                    return LRESULT(0);
                }
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
                if std::mem::take(&mut st.eat_char) {
                    return LRESULT(0);
                }
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
                    Hit::Wedge(i) => pick_slot(st, i, ctrl),
                    Hit::PenButton => set_mode(st, if st.mode == Mode::Tag { Mode::Caption } else { Mode::Tag }),
                    Hit::Star => toggle_fav(st),
                    Hit::Discard if st.fav => {} // starred captures are protected (un-star first)
                    Hit::Discard => discard(st),
                    Hit::Pill => {}
                    Hit::None => {
                        st.restore_focus = false;
                        begin_close(st, 0.0);
                    }
                }
                paint(st);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
                if st.closing.is_none() && delta != 0 {
                    turn(st, if delta < 0 { 1 } else { -1 });
                    paint(st);
                }
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
    st.blink_phase = st.blink.elapsed().as_millis() / 530;
    let scene = render(st);
    let age = st.t_open.elapsed().as_secs_f32() * 1000.0;
    let (mut op, mut sc) = gfx::open_window(&st.style, age);
    if st.leave > 0.0 {
        // eased so the fade starts gently (and reverses smoothly)
        let k = st.leave * st.leave * (3.0 - 2.0 * st.leave);
        op *= 1.0 - k;
        sc *= 1.0 - 0.05 * k;
    }
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
    // per-slice part of the opening animation (Appearance > Menu animation)
    let age = st.t_open.elapsed().as_secs_f32() * 1000.0;
    // per-slice opening animation, plus the wheel turn (the whole ring rotates back into place)
    let pops: Vec<gfx::SliceAnim> = (0..n)
        .map(|i| {
            let mut a = gfx::open_slice(&st.style, age, i, n);
            a.rot += st.spin;
            a
        })
        .collect();
    let wedges: Vec<Path> = (0..n)
        .filter_map(|i| wedge_path(c, i, n, r_in, radii[i], gap).map(|p| gfx::pose_slice(&p, pops[i], c.0, c.1).unwrap_or(p)))
        .collect();
    let pill = st.pill_rect();
    let pill_path = gfx::rrect(pill.left(), pill.top(), pill.width(), pill.height(), pill.height() / 2.0);
    let halves = [half_disc(c, r_in - gap, true, gap), half_disc(c, r_in - gap, false, gap)];

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
    for (i, p) in wedges.iter().enumerate() {
        if pops.get(i).is_some_and(|q| q.a > 0.0) {
            pb.push_path(p);
        }
    }
    for h in halves.iter().flatten() {
        pb.push_path(h);
    }
    if let Some(p) = &pill_path {
        pb.push_path(p);
    }
    let bounds = Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap();
    if let Some(body) = pb.finish() {
        pm.fill_path(&body, &Paint { blend_mode: BlendMode::Clear, ..Default::default() }, FillRule::Winding, Transform::identity(), None);
        gfx::glass_fill(&mut pm, &body, bounds, st.bg.as_ref(), &st.style, 1.0);
        // the capture, blurred, glowing through the ring (cover-cropped square, so never stretched)
        if let (Some(g), Some(mut mask)) = (&st.ghost, Mask::new(w, h)) {
            mask.fill_path(&body, FillRule::Winding, true, Transform::identity());
            let (gw, gh) = (g.width() as f32, g.height() as f32);
            let (op, _) = gfx::open_window(&st.style, age);
            pm.draw_pixmap(
                0,
                0,
                g.as_ref(),
                &PixmapPaint { opacity: GHOST_ALPHA * op, quality: FilterQuality::Bilinear, ..Default::default() },
                Transform::from_translate(c.0 - gw / 2.0, c.1 - gh / 2.0),
                Some(&mask),
            );
        }
        gfx::cursor_glow(&mut pm, &body, st.glow_pos.0, st.glow_pos.1, R_OUT * s * 1.3, &st.style);
    }

    // wedges: hover / match / applied states, labels
    let target = st.enter_target();
    let q = st.field.text().trim().trim_start_matches('#').to_lowercase();
    let typing = st.mode == Mode::Tag && !q.is_empty();
    for (slot, p) in wedges.iter().enumerate() {
        let gfx::SliceAnim { k: pk, a: pa, rot } = pops[slot];
        if pa <= 0.0 {
            continue;
        }
        let hv = st.hprog[slot];
        let i = st.tag_at(slot); // the tag on this slice
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
        // the tag's own colour: a band along the outer edge (brighter when applied or hovered)
        let col = tag_color(&st.tags[i]);
        let band_a = if st.applied[i] { 235.0 } else { 150.0 + 80.0 * hv };
        if let Some(band) = wedge_path(c, slot, n, radii[slot] - 5.0 * s, radii[slot], gap).and_then(|b| gfx::pose_slice(&b, pops[slot], c.0, c.1)) {
            pm.fill_path(&band, &gfx::paint([col[0], col[1], col[2], (band_a * pa) as u8]), FillRule::Winding, Transform::identity(), None);
        }
        gfx::glass_rim(&mut pm, p, Rect::from_xywh(c.0 - radii[slot], c.1 - radii[slot], radii[slot] * 2.0, radii[slot] * 2.0).unwrap(), 1.0 * s, 0.9);
        // label at the wedge middle
        let step = 360.0 / n as f32;
        let a = (slot as f32 * step + rot).to_radians();
        let rm = (r_in + radii[slot]) / 2.0 + 2.0 * s;
        // its number key, small, near the inner edge
        let (nx, ny) = (c.0 + (r_in + 11.0 * s) * pk * a.sin(), c.1 - (r_in + 11.0 * s) * pk * a.cos());
        let num = format!("{}", slot + 1);
        let nw = gfx::text_width_w(&num, 9.5 * s, Weight::Semibold);
        gfx::draw_text_at(&mut pm, &num, 9.5 * s, nx - nw / 2.0, ny, white, (110.0 * pa) as u8, Weight::Semibold, None);
        let (lx, ly) = (c.0 + rm * pk * a.sin(), c.1 - rm * pk * a.cos());
        let px = 12.5 * s;
        let room = if n == 1 { 120.0 * s } else { (2.0 * std::f32::consts::PI * rm / n as f32 - 14.0 * s).min(96.0 * s) };
        let dim = typing && !is_match && !st.applied[i];
        let alpha = ((if dim { 95.0 } else { 245.0 }) * pa) as u8;
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
            gfx::draw_text_at(&mut pm, &label, px, lx - lw / 2.0, ly + 6.0 * s, white, (255.0 * pa) as u8, Weight::Semibold, None);
        } else {
            gfx::draw_text_at(&mut pm, &label, px, lx - lw / 2.0, ly, white, alpha, Weight::Semibold, None);
        }
    }

    // centre: Favourite (top half) and Discard (bottom half)
    let r = r_in - gap;
    let gold = [255u8, 206, 84];
    let red = [229u8, 72, 77];
    for (top, half) in [(true, &halves[0]), (false, &halves[1])] {
        let Some(hp) = half else { continue };
        let on = st.hover == if top { Hit::Star } else { Hit::Discard };
        let fill = match (top, on) {
            (true, _) if st.fav => [gold[0], gold[1], gold[2], if on { 74 } else { 48 }],
            (true, true) => [255, 255, 255, 34],
            (false, true) if st.fav => [255, 255, 255, 18],
            (false, true) => [red[0], red[1], red[2], 170],
            _ => [255, 255, 255, 12],
        };
        pm.fill_path(hp, &gfx::paint(fill), FillRule::Winding, Transform::identity(), None);
        let hb = if top { Rect::from_xywh(c.0 - r, c.1 - r, r * 2.0, r) } else { Rect::from_xywh(c.0 - r, c.1, r * 2.0, r) };
        if let Some(hb) = hb {
            gfx::glass_rim(&mut pm, hp, hb, 1.1 * s, 0.95);
        }
    }
    // the star pops when it is toggled, with a ring of sparks while it fills
    let pop = st.fav_t.map(|t| t.elapsed().as_secs_f32() * 1000.0 / 340.0).unwrap_or(1.0).min(1.0);
    let anim = st.style.animations && pop < 1.0;
    let bump = if anim { 1.0 + 0.4 * (pop * std::f32::consts::PI).sin() * (1.0 - 0.5 * pop) } else { 1.0 };
    let (sx, sy) = (c.0, c.1 - 31.0 * s);
    if st.fav && anim {
        sparks(&mut pm, sx, sy, (11.0 + 12.0 * gfx::ease_out(pop)) * s, 4.0 * s, gold, (230.0 * (1.0 - pop)) as u8, 1.4 * s);
    }
    star(&mut pm, sx, sy, 9.5 * s * bump, st.fav.then_some(gold), white, if st.hover == Hit::Star || st.fav { 245 } else { 200 }, 1.5 * s);
    let lab = |pm: &mut Pixmap, t: &str, y: f32, hit: Hit| {
        let px = 10.0 * s;
        let lw = gfx::text_width_w(t, px, Weight::Semibold);
        gfx::draw_text_at(pm, t, px, c.0 - lw / 2.0, y, white, if st.hover == hit { 245 } else { 150 }, Weight::Semibold, None);
    };
    lab(&mut pm, if st.fav { "Favourite" } else { "Star" }, c.1 - 13.0 * s, Hit::Star);
    lab(&mut pm, "Discard", c.1 + 14.0 * s, Hit::Discard);
    trash(&mut pm, c.0, c.1 + 33.0 * s, 17.0 * s, white, if st.hover == Hit::Discard { 250 } else { 200 }, 1.5 * s);

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
    // meta line ("Saved · Region 640×480 · chrome") under the pill; it explains the centre buttons on hover
    let more = st.tags.len() > st.n();
    let line = match st.hover {
        Hit::Wedge(slot) if st.applied[st.tag_at(slot)] => format!("Click again to remove #{}", st.tags[st.tag_at(slot)]),
        Hit::Wedge(_) if more => format!("Click or press 1-{} · scroll for {} more tags", st.n(), st.tags.len() - st.n()),
        Hit::Wedge(_) => format!("Click or press 1-{}", st.n()),
        Hit::Star if st.fav => "Click to remove from Favourites".to_string(),
        Hit::Star => "Add to Favourites".to_string(),
        Hit::Discard if st.fav => "Starred: un-star it first to discard".to_string(),
        Hit::Discard => "Discard: delete this capture, nothing is saved".to_string(),
        _ => format!("Saved · {}", st.meta),
    };
    let meta = gfx::ellipsize(&line, 11.0 * s, w as f32 - 24.0 * s, Weight::Regular);
    let mw = gfx::text_width_w(&meta, 11.0 * s, Weight::Regular);
    // soft dark halo first so the line reads on bright backgrounds too
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        gfx::draw_text_at(&mut pm, &meta, 11.0 * s, c.0 - mw / 2.0 + dx, pill.bottom() + 13.0 * s + dy, [0, 0, 0], 70, Weight::Regular, None);
    }
    gfx::draw_text_at(&mut pm, &meta, 11.0 * s, c.0 - mw / 2.0, pill.bottom() + 13.0 * s, white, 200, Weight::Regular, None);
    pm
}

/// A tag's own colour (stable: hashed from its lower-case name), bright enough to read on dark glass.
fn tag_color(tag: &str) -> [u8; 3] {
    let mut h: u32 = 2166136261;
    for b in tag.to_lowercase().bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    let hue = (h % 360) as f32;
    let (s, l) = (0.72, 0.64);
    let c = (1.0 - (2.0 * l - 1.0f32).abs()) * s;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (hue / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8]
}

/// The capture as a soft glow for behind the ring: the centre square of the thumbnail (cover crop, so the picture
/// keeps its proportions), shrunk to 28 px, box-blurred, then scaled up to `d` x `d`.
fn ghost_pixmap(t: &Thumb, d: u32) -> Option<Pixmap> {
    const N: usize = 28;
    let side = t.w.min(t.h) as usize;
    if side < 4 || d < 8 {
        return None;
    }
    let (x0, y0) = ((t.w as usize - side) / 2, (t.h as usize - side) / 2);
    // area-average into N x N
    let mut small = vec![[0f32; 3]; N * N];
    for (k, px) in small.iter_mut().enumerate() {
        let (sx0, sy0) = ((k % N) * side / N, (k / N) * side / N);
        let (sx1, sy1) = ((((k % N) + 1) * side / N).max(sx0 + 1), (((k / N) + 1) * side / N).max(sy0 + 1));
        let mut acc = [0f32; 3];
        let mut cnt = 0f32;
        for y in sy0..sy1 {
            for x in sx0..sx1 {
                let i = ((y0 + y) * t.w as usize + x0 + x) * 4;
                for (a, v) in acc.iter_mut().zip(&t.rgba[i..i + 3]) {
                    *a += *v as f32;
                }
                cnt += 1.0;
            }
        }
        *px = acc.map(|v| v / cnt);
    }
    // two box-blur passes (3 x 3)
    for _ in 0..2 {
        let src = small.clone();
        for y in 0..N {
            for x in 0..N {
                let mut acc = [0f32; 3];
                let mut cnt = 0f32;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (xx, yy) = (x as i32 + dx, y as i32 + dy);
                        if xx >= 0 && yy >= 0 && (xx as usize) < N && (yy as usize) < N {
                            let v = src[yy as usize * N + xx as usize];
                            for (a, x) in acc.iter_mut().zip(v) {
                                *a += x;
                            }
                            cnt += 1.0;
                        }
                    }
                }
                small[y * N + x] = acc.map(|v| v / cnt);
            }
        }
    }
    let mut pm = Pixmap::new(N as u32, N as u32)?;
    for (px, v) in pm.pixels_mut().iter_mut().zip(&small) {
        *px = ColorU8::from_rgba(v[0] as u8, v[1] as u8, v[2] as u8, 255).premultiply();
    }
    let mut out = Pixmap::new(d, d)?;
    let k = d as f32 / N as f32;
    out.draw_pixmap(0, 0, pm.as_ref(), &PixmapPaint { quality: FilterQuality::Bicubic, ..Default::default() }, Transform::from_scale(k, k), None);
    Some(out)
}

/// Half of the centre disc (top or bottom), split by a horizontal channel `gap` wide.
fn half_disc(c: (f32, f32), r: f32, top: bool, gap: f32) -> Option<Path> {
    let d = (gap / 2.0).min(r * 0.5);
    let a0 = (d / r).asin();
    let sign = if top { -1.0 } else { 1.0 };
    let steps = 32;
    let mut pb = PathBuilder::new();
    for k in 0..=steps {
        let a = a0 + (std::f32::consts::PI - 2.0 * a0) * k as f32 / steps as f32;
        let (px, py) = (c.0 + r * a.cos(), c.1 + sign * r * a.sin());
        if k == 0 {
            pb.move_to(px, py);
        } else {
            pb.line_to(px, py);
        }
    }
    pb.close();
    pb.finish()
}

/// Five-point star centred on (x, y): outlined, or filled with `fill`.
#[allow(clippy::too_many_arguments)]
fn star(pm: &mut Pixmap, x: f32, y: f32, r: f32, fill: Option<[u8; 3]>, col: [u8; 3], alpha: u8, width: f32) {
    let mut pb = PathBuilder::new();
    for k in 0..10 {
        let rr = if k % 2 == 0 { r } else { r * 0.47 };
        let a = (-90.0 + 36.0 * k as f32).to_radians();
        let (px, py) = (x + rr * a.cos(), y + rr * a.sin());
        if k == 0 {
            pb.move_to(px, py);
        } else {
            pb.line_to(px, py);
        }
    }
    pb.close();
    let Some(p) = pb.finish() else { return };
    let line = fill.unwrap_or(col);
    if let Some(f) = fill {
        pm.fill_path(&p, &gfx::paint([f[0], f[1], f[2], alpha]), FillRule::Winding, Transform::identity(), None);
    }
    let stroke = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
    pm.stroke_path(&p, &gfx::paint([line[0], line[1], line[2], alpha]), &stroke, Transform::identity(), None);
}

/// Short rays around (x, y) at radius `r` (the star's sparkle).
#[allow(clippy::too_many_arguments)]
fn sparks(pm: &mut Pixmap, x: f32, y: f32, r: f32, len: f32, col: [u8; 3], alpha: u8, width: f32) {
    let mut pb = PathBuilder::new();
    for k in 0..8 {
        let a = (22.5 + 45.0 * k as f32).to_radians();
        let l = if k % 2 == 0 { len } else { len * 0.6 };
        pb.move_to(x + r * a.cos(), y + r * a.sin());
        pb.line_to(x + (r + l) * a.cos(), y + (r + l) * a.sin());
    }
    if let Some(p) = pb.finish() {
        let stroke = Stroke { width, line_cap: LineCap::Round, ..Default::default() };
        pm.stroke_path(&p, &gfx::paint([col[0], col[1], col[2], alpha]), &stroke, Transform::identity(), None);
    }
}

/// Bin icon (the gallery's trash glyph) in a `size` box centred on (x, y).
fn trash(pm: &mut Pixmap, x: f32, y: f32, size: f32, col: [u8; 3], alpha: u8, width: f32) {
    let k = size / 16.0;
    let p = |u: f32, v: f32| (x + (u - 8.0) * k, y + (v - 8.0) * k);
    let mut pb = PathBuilder::new();
    let line = |pb: &mut PathBuilder, pts: &[(f32, f32)]| {
        let (a, b) = p(pts[0].0, pts[0].1);
        pb.move_to(a, b);
        for &(u, v) in &pts[1..] {
            let (a, b) = p(u, v);
            pb.line_to(a, b);
        }
    };
    line(&mut pb, &[(2.5, 4.5), (13.5, 4.5)]);
    line(&mut pb, &[(6.0, 4.5), (6.0, 3.0), (10.0, 3.0), (10.0, 4.5)]);
    line(&mut pb, &[(4.0, 4.5), (4.6, 13.0), (11.4, 13.0), (12.0, 4.5)]);
    line(&mut pb, &[(6.8, 7.0), (7.0, 11.0)]);
    line(&mut pb, &[(9.2, 7.0), (9.0, 11.0)]);
    if let Some(path) = pb.finish() {
        let stroke = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        pm.stroke_path(&path, &gfx::paint([col[0], col[1], col[2], alpha]), &stroke, Transform::identity(), None);
    }
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

    fn scene(tags: &[&str], typed: &str, mode: Mode, hover: Hit, applied: Option<usize>, fav: bool, name: &str) {
        scene_at(tags, typed, mode, hover, applied, fav, name, 2000, None, true);
    }

    /// `age_ms`: how long after opening the frame is drawn; `anim`: Menu animation override (petals, fan, ...).
    #[allow(clippy::too_many_arguments)]
    fn scene_at(tags: &[&str], typed: &str, mode: Mode, hover: Hit, applied: Option<usize>, fav: bool, name: &str, age_ms: u64, anim: Option<&str>, ghost: bool) {
        let s: f32 = std::env::var("STASH_PREVIEW_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.5);
        let mut style = preview::style();
        if let Some(a) = anim {
            style.anim = crate::theme::OpenAnim::parse(a);
        }
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
        let n = tags.len();
        let mut st = St {
            win,
            s,
            c: (w as f32 / 2.0, ring_h / 2.0),
            style,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            applied: (0..n).map(|i| Some(i) == applied).collect(),
            hprog: (0..n.min(MAX_TAGS)).map(|i| if hover == Hit::Wedge(i) { 1.0 } else { 0.0 }).collect(),
            offset: 0,
            spin: 0.0,
            ghost: ghost.then(|| {
                // the capture: STASH_PREVIEW_GHOST (any picture), else a crop of the backdrop
                if let Some(img) = std::env::var("STASH_PREVIEW_GHOST").ok().and_then(|p| image::open(p).ok()) {
                    let t = img.thumbnail(320, 320).to_rgba8();
                    return ghost_pixmap(&Thumb { w: t.width(), h: t.height(), rgba: t.into_raw() }, ((R_OUT + GROW) * 2.0 * s).ceil() as u32);
                }
                let (tw, th) = (360u32, 200u32);
                let mut rgba = Vec::new();
                for y in 0..th as i32 {
                    for x in 0..tw as i32 {
                        let i = (((y + 380) * frame.w + x + 560) * 4) as usize;
                        rgba.extend_from_slice(&[frame.bgra[i + 2], frame.bgra[i + 1], frame.bgra[i], 255]);
                    }
                }
                ghost_pixmap(&Thumb { rgba, w: tw, h: th }, ((R_OUT + GROW) * 2.0 * s).ceil() as u32)
            }).flatten(),
            eat_char: false,
            hover,
            field: TextField::new(120),
            mode,
            sidecars: vec![],
            thumb: None,
            snippet: None,
            meta: "Region · 1115×628 · chrome".into(),
            fav,
            fav_t: None,
            tagged: Vec::new(),
            discarded: false,
            bg,
            source_hwnd: 0,
            seq: 0,
            t_open: Instant::now() - std::time::Duration::from_millis(age_ms),
            last: Instant::now(),
            last_cursor: (w as f32 * 0.66, ring_h * 0.3),
            glow_pos: (w as f32 * 0.66, ring_h * 0.3), // where the cursor glow shows in the previews
            last_move: Instant::now(),
            off_since: None,
            open_at: (w as f32 * 0.66, ring_h * 0.3),
            entered: true,
            leave: 0.0,
            closing: None,
            restore_focus: false,
            flash: None,
            done: false,
            blink: Instant::now(),
            blink_phase: 0,
        };
        st.field.set_text(typed);
        let scene = render(&mut st);
        // the whole-window part of the opening animation, as paint() applies it
        let (op, sc) = gfx::open_window(&st.style, age_ms as f32);
        let pm = gfx::compose(&scene, op, sc, st.c.0, st.c.1);
        preview::save(&pm, &frame, origin, name);
    }

    #[test]
    #[ignore]
    fn previews() {
        let tags = ["moodboard", "typography", "ui-ref", "motion", "colour", "yt-ideas"];
        scene(&tags, "", Mode::Tag, Hit::None, None, false, "radial_idle.png");
        scene(&tags, "", Mode::Tag, Hit::Wedge(1), None, false, "radial_hover.png");
        scene(&tags, "mo", Mode::Tag, Hit::None, Some(2), false, "radial_typing.png");
        scene(&tags, "Love the kerning on this headline", Mode::Caption, Hit::None, None, false, "radial_caption.png");
        scene(&tags, "", Mode::Tag, Hit::Star, None, false, "radial_star_hover.png");
        scene(&tags[..3], "", Mode::Tag, Hit::None, None, true, "radial_fav_3tags.png");
        scene(&tags, "", Mode::Tag, Hit::Discard, None, false, "radial_discard_hover.png");
        scene(&[], "", Mode::Tag, Hit::None, None, false, "radial_first_run.png");
        scene_at(&tags, "", Mode::Tag, Hit::None, None, false, "radial_no_ghost.png", 2000, None, false);
        // more tags than slices: the meta line offers scrolling; an applied one offers removal
        let many = ["moodboard", "typography", "ui-ref", "motion", "colour", "yt-ideas", "logo", "web", "3d"];
        scene(&many, "", Mode::Tag, Hit::Wedge(2), None, false, "radial_many_hover.png");
        scene(&many, "", Mode::Tag, Hit::Wedge(0), Some(0), false, "radial_applied_hover.png");
        for anim in crate::config::MENU_ANIMS {
            for ms in [30, 80, 130, 190, 260] {
                scene_at(&tags, "", Mode::Tag, Hit::None, None, false, &format!("radial_anim_{anim}_{ms:03}.png"), ms, Some(anim), true);
            }
        }
    }
}
