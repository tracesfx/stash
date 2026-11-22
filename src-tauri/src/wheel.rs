//! Radial capture wheel: native Win32 layered window (per-pixel alpha), topmost + no-activate (never steals
//! focus, so the foreground app keeps its selection for "Stash selection"). Opens on hotkey hold, chooses on
//! release. Rendered with tiny-skia (anti-aliased) in the shared Liquid Glass material (gfx::glass_fill) over a
//! blurred copy of the frozen screen behind it.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tiny_skia::*;
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::{self, draw_text};
use crate::theme::Style;
use crate::winutil::Frame;

/// Everything the wheel can offer. Settings chooses which are shown; the wheel lays the enabled ones out
/// clockwise from the top in this fixed order, so a given set of toggles always yields the same layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Snip,
    Window,
    Screen,
    Selection,
    Clipboard,
    Note,
    Record,
    Pause,
    Gallery,
    Settings,
}

pub const ALL_ACTIONS: [Action; 10] = [
    Action::Snip,
    Action::Window,
    Action::Screen,
    Action::Selection,
    Action::Clipboard,
    Action::Note,
    Action::Record,
    Action::Pause,
    Action::Gallery,
    Action::Settings,
];

/// Default wheel: the five core actions.
pub fn default_ids() -> Vec<String> {
    ["snip", "note", "record", "gallery", "settings"].iter().map(|s| s.to_string()).collect()
}

/// Enabled actions (by config id) in canonical wheel order. Never empty: falls back to the defaults.
pub fn enabled(ids: &[String]) -> Vec<Action> {
    let v: Vec<Action> = ALL_ACTIONS.iter().copied().filter(|a| ids.iter().any(|i| i == a.id())).collect();
    if v.len() < 2 {
        ALL_ACTIONS.iter().copied().filter(|a| default_ids().iter().any(|i| i == a.id())).collect()
    } else {
        v
    }
}

impl Action {
    pub fn id(self) -> &'static str {
        match self {
            Action::Snip => "snip",
            Action::Window => "window",
            Action::Screen => "screen",
            Action::Selection => "selection",
            Action::Clipboard => "clipboard",
            Action::Note => "note",
            Action::Record => "record",
            Action::Pause => "pause",
            Action::Gallery => "gallery",
            Action::Settings => "settings",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Action::Snip => "Region",
            Action::Window => "Window",
            Action::Screen => "Screen",
            Action::Selection => "Selection",
            Action::Clipboard => "Clipboard",
            Action::Note => "Note",
            Action::Record => "Record",
            Action::Pause => "Pause",
            Action::Gallery => "Gallery",
            Action::Settings => "Settings",
        }
    }
}

const TIMER: usize = 1;
const OUTER_R: f32 = 150.0;
const MARGIN: f32 = 22.0; // room for the shadow
const GROW: f32 = 9.0;
const HOVER_MS: f32 = 90.0;

struct State {
    center: POINT, // screen coords
    size: i32,
    scale: f32,
    style: Style,
    hover: Option<usize>,
    hprog: Vec<f32>,
    acts: Vec<Action>,
    gap: f32,
    dead: f32,
    opacity: f32,
    t0: Instant,
    last: Instant,
    release: Arc<AtomicBool>,
    result: Option<Action>,
    done: bool,
    bg: Option<Pixmap>, // blurred screen behind the wheel
    dib_dc: HDC,
    dib_bits: *mut u8,
    origin: POINT, // window top-left
    /// Cursor in window pixels, for the cursor glow.
    glow_pt: (f32, f32),
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut State> {
    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State).as_mut() }
}

/// Wedge index under an offset from the wheel centre, or None inside the dead zone.
fn wedge_at(dx: f32, dy: f32, dead: f32, n: usize) -> Option<usize> {
    let step = 360.0 / n as f32;
    if (dx * dx + dy * dy).sqrt() < dead {
        return None;
    }
    let mut deg = dx.atan2(-dy).to_degrees();
    if deg < 0.0 {
        deg += 360.0;
    }
    Some((((deg + step / 2.0) / step) as usize) % n)
}

// ---------- entry ----------
/// Shows the wheel centred on `cursor` until `release` is set, Esc, or a click. Returns the chosen action.
pub fn run(cursor: POINT, scale: f32, release: Arc<AtomicBool>, style: Style, frame: Option<Arc<Frame>>, acts: Vec<Action>) -> Option<Action> {
    unsafe {
        let scale = scale * style.wheel_size;
        let size = ((OUTER_R + MARGIN) * 2.0 * scale) as i32;
        let mon = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let m = mi.rcMonitor;
        let half = size / 2;
        let cx = cursor.x.clamp(m.left + half, (m.right - half).max(m.left + half));
        let cy = cursor.y.clamp(m.top + half, (m.bottom - half).max(m.top + half));
        let origin = POINT { x: cx - half, y: cy - half };

        let bg = frame.as_deref().and_then(|f| gfx::blur_backdrop(f, origin, size, size, style.blur, style.saturation));

        // 32-bit DIB the layered window reads from
        let screen = GetDC(None);
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let dib_dc = CreateCompatibleDC(Some(screen));
        SelectObject(dib_dc, dib.into());
        ReleaseDC(None, screen);

        let mut st = Box::new(State {
            center: POINT { x: cx, y: cy },
            size,
            scale,
            style,
            hover: None,
            hprog: vec![0.0; acts.len()],
            acts,
            gap: style.wheel_gap,
            dead: style.wheel_hole,
            opacity: style.wheel_opacity,
            t0: Instant::now(),
            last: Instant::now(),
            release,
            result: None,
            done: false,
            bg,
            dib_dc,
            dib_bits: bits as *mut u8,
            origin,
            glow_pt: (-1000.0, -1000.0),
        });

        let hinst = GetModuleHandleW(None).ok()?;
        let class = w!("StashWheel2");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
            class,
            w!("Stash wheel"),
            WS_POPUP,
            origin.x,
            origin.y,
            size,
            size,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .ok()?;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut *st as *mut State as isize);
        present(hwnd, &mut st); // first (transparent) frame before showing: no flash
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(Some(hwnd), TIMER, 8, None);

        let mut msg = MSG::default();
        while !st.done && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = KillTimer(Some(hwnd), TIMER);
        let _ = DestroyWindow(hwnd);
        let _ = DeleteDC(st.dib_dc);
        let _ = DeleteObject(dib.into());
        st.result
    }
}

fn hover_now(st: &State) -> Option<usize> {
    unsafe {
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        wedge_at((p.x - st.center.x) as f32, (p.y - st.center.y) as f32, st.dead * st.scale, st.acts.len())
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_TIMER => {
                if let Some(st) = state(hwnd) {
                    let new_hover = hover_now(st);
                    let hover_changed = new_hover != st.hover;
                    st.hover = new_hover;
                    // animate hover progress and the open transition
                    let dt = st.last.elapsed().as_secs_f32() * 1000.0;
                    st.last = Instant::now();
                    let open_ms = gfx::open_total_ms(&st.style);
                    let mut animating = hover_changed || st.t0.elapsed().as_secs_f32() * 1000.0 < open_ms + 16.0;
                    if st.style.glow > 0.0 {
                        let mut p = POINT::default();
                        let _ = GetCursorPos(&mut p);
                        let (nx, ny) = gfx::GLOW_NUDGE;
                        let target = ((p.x - st.origin.x) as f32 + nx * st.scale, (p.y - st.origin.y) as f32 + ny * st.scale);
                        if gfx::glow_follow(&mut st.glow_pt, target, dt, &st.style) {
                            animating = true;
                        }
                    }
                    for i in 0..st.acts.len() {
                        let target = if st.hover == Some(i) { 1.0 } else { 0.0 };
                        let d = target - st.hprog[i];
                        if d.abs() > 0.001 {
                            st.hprog[i] += d.signum() * (dt / HOVER_MS).min(d.abs());
                            animating = true;
                        }
                    }
                    if animating {
                        present(hwnd, st);
                    }
                    if (GetAsyncKeyState(VK_ESCAPE.0 as i32) as u16 & 0x8000) != 0 {
                        st.done = true; // cancel
                    } else if st.release.load(Ordering::SeqCst) {
                        st.result = st.hover.map(|i| st.acts[i]);
                        st.done = true;
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if let Some(st) = state(hwnd) {
                    st.result = hover_now(st).map(|i| st.acts[i]);
                    st.done = true;
                }
                LRESULT(0)
            }
            WM_RBUTTONDOWN => {
                if let Some(st) = state(hwnd) {
                    st.done = true;
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

// ---------- presenting ----------
unsafe fn present(hwnd: HWND, st: &mut State) {
    unsafe {
        let s = st.size as u32;
        let age = st.t0.elapsed().as_secs_f32() * 1000.0;
        let scene = render(st);
        // open animation (Appearance > Menu animation): the whole wheel fades / scales around its centre
        let (op, sc) = gfx::open_window(&st.style, age);
        let c = s as f32 / 2.0;
        let out = gfx::compose(&scene, op * st.opacity, sc, c, c);
        // RGBA (premultiplied) -> BGRA DIB
        let dst = std::slice::from_raw_parts_mut(st.dib_bits, (s * s * 4) as usize);
        for (d, p) in dst.chunks_exact_mut(4).zip(out.data().chunks_exact(4)) {
            d[0] = p[2];
            d[1] = p[1];
            d[2] = p[0];
            d[3] = p[3];
        }
        let size = SIZE { cx: st.size, cy: st.size };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        let screen = GetDC(None);
        let _ = UpdateLayeredWindow(hwnd, Some(screen), Some(&st.origin), Some(&size), Some(st.dib_dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
        ReleaseDC(None, screen);
    }
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

// ---------- drawing ----------
fn pt(c: f32, r: f32, deg: f32) -> (f32, f32) {
    let a = deg.to_radians();
    (c + r * a.sin(), c - r * a.cos())
}

/// Annular sector for wedge `i`. Both side edges are straight lines parallel to the wedge's own centre line and
/// offset by half the gap, so the channel between neighbouring slices has constant width.
fn wedge_path(c: f32, i: usize, n: usize, r_in: f32, r_out: f32, gap: f32) -> Path {
    let step = 360.0 / n as f32;
    let mid = i as f32 * step;
    let half = step / 2.0;
    let d = gap / 2.0;
    let off = |r: f32| (d / r).clamp(0.0, 1.0).asin().to_degrees(); // angular inset of the offset edge at radius r
    let (o_out, o_in) = (off(r_out), off(r_in));
    let mut pb = PathBuilder::new();
    let steps = 14;
    let (a0, a1) = (mid - half + o_out, mid + half - o_out);
    let p = pt(c, r_out, a0);
    pb.move_to(p.0, p.1);
    for k in 1..=steps {
        let p = pt(c, r_out, a0 + (a1 - a0) * k as f32 / steps as f32);
        pb.line_to(p.0, p.1);
    }
    let (b0, b1) = (mid - half + o_in, mid + half - o_in);
    for k in (0..=steps).rev() {
        let p = pt(c, r_in, b0 + (b1 - b0) * k as f32 / steps as f32);
        pb.line_to(p.0, p.1);
    }
    pb.close();
    pb.finish().unwrap()
}

fn paint_rgba(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(r, g, b, a);
    p.anti_alias = true;
    p
}

fn render(st: &State) -> Pixmap {
    let s = st.size as u32;
    let sc = st.scale;
    let c = s as f32 / 2.0;
    let (r_in, r_out) = (st.dead * sc, OUTER_R * sc);
    let white = [255u8, 255, 255];
    let mut pm = Pixmap::new(s, s).unwrap();

    let n = st.acts.len();
    let radii: Vec<f32> = (0..n).map(|i| r_out + GROW * sc * ease(st.hprog[i])).collect();
    // per-slice part of the opening animation (petals pop in one by one, a fan spreads, a swirl turns)
    let age = st.t0.elapsed().as_secs_f32() * 1000.0;
    let pops: Vec<gfx::SliceAnim> = (0..n).map(|i| gfx::open_slice(&st.style, age, i, n)).collect();
    let paths: Vec<Path> = (0..n)
        .map(|i| {
            let p = wedge_path(c, i, n, r_in, radii[i], st.gap * sc);
            gfx::pose_slice(&p, pops[i], c, c).unwrap_or(p)
        })
        .collect();

    // soft drop shadow: stacked translucent discs, offset down
    for k in (1..=7).rev() {
        let r = r_out + k as f32 * 2.6 * sc;
        if let Some(p) = PathBuilder::from_circle(c, c + 4.0 * sc, r) {
            pm.fill_path(&p, &paint_rgba(0, 0, 0, (16.0 / k as f32) as u8), FillRule::Winding, Transform::identity(), None);
        }
    }

    // one glass body: all wedges + the centre disc
    let disc = PathBuilder::from_circle(c, c, r_in - st.gap * sc);
    let mut pb = PathBuilder::new();
    for (i, p) in paths.iter().enumerate() {
        if pops[i].a > 0.0 {
            pb.push_path(p);
        }
    }
    if let Some(d) = &disc {
        pb.push_path(d);
    }
    let bounds = Rect::from_xywh(0.0, 0.0, s as f32, s as f32).unwrap();
    if let Some(body) = pb.finish() {
        pm.fill_path(&body, &Paint { blend_mode: BlendMode::Clear, ..Default::default() }, FillRule::Winding, Transform::identity(), None);
        gfx::glass_fill(&mut pm, &body, bounds, st.bg.as_ref(), &st.style, 1.0);
        gfx::cursor_glow(&mut pm, &body, st.glow_pt.0, st.glow_pt.1, r_out * 1.3, &st.style);
    }

    let acc = st.style.hover;
    for (i, p) in paths.iter().enumerate() {
        if pops[i].a <= 0.0 {
            continue;
        }
        let h = st.hprog[i];
        pm.fill_path(p, &paint_rgba(255, 255, 255, 8), FillRule::Winding, Transform::identity(), None);
        if h > 0.0 {
            pm.fill_path(p, &paint_rgba(acc[0], acc[1], acc[2], (135.0 * h) as u8), FillRule::Winding, Transform::identity(), None);
        }
        gfx::glass_rim(&mut pm, p, Rect::from_xywh(c - radii[i], c - radii[i], radii[i] * 2.0, radii[i] * 2.0).unwrap(), 1.0 * sc, 0.9);
    }

    // icons
    for i in 0..st.acts.len() {
        let h = ease(st.hprog[i]);
        let gfx::SliceAnim { k, a, rot } = pops[i];
        let (x, y) = pt(c, (r_in + radii[i]) / 2.0 + 1.0 * sc, i as f32 * (360.0 / n as f32) + rot);
        if a <= 0.0 {
            continue;
        }
        let (x, y) = (c + (x - c) * k, c + (y - c) * k);
        draw_icon(&mut pm, st.acts[i], x, y, (13.0 + 2.5 * h) * sc * k, white, (235.0 * a) as u8, 1.7 * sc);
    }

    // centre disc with the hovered action's name
    if let Some(d) = &disc {
        let r = r_in - st.gap * sc;
        pm.fill_path(d, &paint_rgba(255, 255, 255, 14), FillRule::Winding, Transform::identity(), None);
        gfx::glass_rim(&mut pm, d, Rect::from_xywh(c - r, c - r, r * 2.0, r * 2.0).unwrap(), 1.0 * sc, 1.0);
    }
    if let Some(i) = st.hover {
        draw_text(&mut pm, st.acts[i].label(), 11.5 * sc, c, c, white, 255);
    } else if let Some(d) = PathBuilder::from_circle(c, c, 2.6 * sc) {
        pm.fill_path(&d, &paint_rgba(255, 255, 255, 120), FillRule::Winding, Transform::identity(), None);
    }
    pm
}

fn draw_icon(pm: &mut Pixmap, a: Action, x: f32, y: f32, size: f32, col: [u8; 3], alpha: u8, width: f32) {
    let mut pb = PathBuilder::new();
    let mut seg = |pts: &[(f32, f32)], close: bool| {
        pb.move_to(x + pts[0].0 * size, y + pts[0].1 * size);
        for p in &pts[1..] {
            pb.line_to(x + p.0 * size, y + p.1 * size);
        }
        if close {
            pb.close();
        }
    };
    match a {
        Action::Snip => {
            for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                seg(&[(sx * 0.5, sy * 0.9), (sx * 0.9, sy * 0.9), (sx * 0.9, sy * 0.5)], false);
            }
        }
        Action::Window => {
            seg(&[(-0.9, -0.7), (0.9, -0.7), (0.9, 0.7), (-0.9, 0.7)], true);
            seg(&[(-0.9, -0.3), (0.9, -0.3)], false);
        }
        Action::Screen => {
            seg(&[(-0.95, -0.7), (0.95, -0.7), (0.95, 0.3), (-0.95, 0.3)], true);
            seg(&[(0.0, 0.3), (0.0, 0.75)], false);
            seg(&[(-0.5, 0.75), (0.5, 0.75)], false);
        }
        Action::Selection => {
            // text lines with an I-beam caret
            seg(&[(-0.9, -0.6), (0.9, -0.6)], false);
            seg(&[(-0.9, -0.1), (0.2, -0.1)], false);
            seg(&[(-0.9, 0.4), (0.5, 0.4)], false);
            seg(&[(0.6, -0.1), (0.6, 0.8)], false);
        }
        Action::Clipboard => {
            seg(&[(-0.7, -0.6), (0.7, -0.6), (0.7, 0.9), (-0.7, 0.9)], true);
            seg(&[(-0.3, -0.9), (0.3, -0.9), (0.3, -0.45), (-0.3, -0.45)], true);
        }
        Action::Pause => {
            seg(&[(-0.45, -0.7), (-0.45, 0.7)], false);
            seg(&[(0.45, -0.7), (0.45, 0.7)], false);
        }
        Action::Note => {
            seg(&[(-0.75, 0.75), (-0.6, 0.25), (0.4, -0.75), (0.8, -0.35), (-0.2, 0.6)], true);
            seg(&[(0.15, -0.5), (0.55, -0.1)], false);
        }
        Action::Record => {
            // camera: body + lens-side triangle
            seg(&[(-0.9, -0.5), (0.3, -0.5), (0.3, 0.5), (-0.9, 0.5)], true);
            seg(&[(0.3, -0.12), (0.9, -0.5), (0.9, 0.5), (0.3, 0.12)], true);
        }
        Action::Gallery => {
            for (ox, oy) in [(-0.8f32, -0.8f32), (0.1, -0.8), (-0.8, 0.1), (0.1, 0.1)] {
                seg(&[(ox, oy), (ox + 0.7, oy), (ox + 0.7, oy + 0.7), (ox, oy + 0.7)], true);
            }
        }
        Action::Settings => {
            // gear: 8 teeth
            let mut pts: Vec<(f32, f32)> = Vec::new();
            for k in 0..8 {
                let a = k as f32 * 45.0;
                for (da, r) in [(-15.0f32, 0.62f32), (-9.0, 0.92), (9.0, 0.92), (15.0, 0.62)] {
                    let t = (a + da).to_radians();
                    pts.push((r * t.sin(), -r * t.cos()));
                }
            }
            seg(&pts, true);
            // hub hole (octagon)
            let hub: Vec<(f32, f32)> = (0..8).map(|k| {
                let t = (k as f32 * 45.0 + 22.5).to_radians();
                (0.3 * t.sin(), -0.3 * t.cos())
            }).collect();
            seg(&hub, true);
        }
    }
    if let Some(path) = pb.finish() {
        let stroke = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        pm.stroke_path(&path, &paint_rgba(col[0], col[1], col[2], alpha), &stroke, Transform::identity(), None);
    }
}
