//! Radial capture wheel: native Win32 layered window (per-pixel alpha), topmost + no-activate (never steals
//! focus, so the foreground app keeps its selection for "Stash selection"). Opens on hotkey hold, chooses on
//! release. Rendered with tiny-skia (anti-aliased); themed (dark / light / liquid glass with a real blurred
//! copy of the screen behind it).
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

use crate::gfx::draw_text;
use crate::theme::{Resolved, Style};
use crate::winutil::Frame;

/// Fixed wedge positions, clockwise from the top. Muscle memory depends on these never moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Snip,
    Window,
    Screen,
    Selection,
    Clipboard,
    Note,
    Gallery,
    Settings,
    Pause,
}

pub const ACTIONS: [Action; N] = [
    Action::Snip,
    Action::Window,
    Action::Screen,
    Action::Selection,
    Action::Clipboard,
    Action::Note,
    Action::Gallery,
    Action::Settings,
    Action::Pause,
];

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Snip => "Region",
            Action::Window => "Window",
            Action::Screen => "Screen",
            Action::Selection => "Selection",
            Action::Clipboard => "Clipboard",
            Action::Note => "Note",
            Action::Gallery => "Gallery",
            Action::Settings => "Settings",
            Action::Pause => "Pause",
        }
    }
}

const N: usize = 9; // wedges
const STEP: f32 = 360.0 / N as f32;
const GAP: f32 = 5.0; // constant (parallel) gap between slices, px at 100% scale
const TIMER: usize = 1;
const DEAD_R: f32 = 40.0;
const OUTER_R: f32 = 150.0;
const MARGIN: f32 = 22.0; // room for the shadow
const GROW: f32 = 9.0;
const OPEN_MS: f32 = 130.0;
const HOVER_MS: f32 = 90.0;

// ---------- palette ----------
struct Pal {
    base: [u8; 4],  // wedge body
    wedge: [u8; 4], // extra per-wedge tint
    rim: [u8; 4],
    text: [u8; 3],
    shadow: f32,
    glass: bool,
}

fn palette(t: Resolved) -> Pal {
    match t {
        Resolved::Dark => Pal { base: [26, 26, 33, 226], wedge: [255, 255, 255, 10], rim: [255, 255, 255, 30], text: [240, 240, 246], shadow: 0.07, glass: false },
        Resolved::Light => Pal { base: [250, 250, 253, 232], wedge: [0, 0, 0, 8], rim: [0, 0, 0, 34], text: [28, 28, 34], shadow: 0.05, glass: false },
        Resolved::Glass => Pal { base: [20, 24, 40, 70], wedge: [255, 255, 255, 30], rim: [255, 255, 255, 120], text: [255, 255, 255], shadow: 0.08, glass: true },
    }
}

struct State {
    center: POINT, // screen coords
    size: i32,
    scale: f32,
    pal: Pal,
    accent: [u8; 3],
    hover: Option<usize>,
    hprog: [f32; N],
    glass: [[u8; 3]; 3],
    t0: Instant,
    last: Instant,
    release: Arc<AtomicBool>,
    result: Option<Action>,
    done: bool,
    bg: Option<Pixmap>, // blurred screen behind the wheel (glass only)
    dib_dc: HDC,
    dib_bits: *mut u8,
    origin: POINT, // window top-left
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut State> {
    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State).as_mut() }
}

/// Wedge index under an offset from the wheel centre, or None inside the dead zone.
fn wedge_at(dx: f32, dy: f32, dead: f32) -> Option<usize> {
    if (dx * dx + dy * dy).sqrt() < dead {
        return None;
    }
    let mut deg = dx.atan2(-dy).to_degrees();
    if deg < 0.0 {
        deg += 360.0;
    }
    Some((((deg + STEP / 2.0) / STEP) as usize) % N)
}

// ---------- entry ----------
/// Shows the wheel centred on `cursor` until `release` is set, Esc, or a click. Returns the chosen action.
pub fn run(cursor: POINT, scale: f32, release: Arc<AtomicBool>, style: Style, frame: Option<Arc<Frame>>) -> Option<Action> {
    unsafe {
        let size = ((OUTER_R + MARGIN) * 2.0 * scale) as i32;
        let mon = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let m = mi.rcMonitor;
        let half = size / 2;
        let cx = cursor.x.clamp(m.left + half, (m.right - half).max(m.left + half));
        let cy = cursor.y.clamp(m.top + half, (m.bottom - half).max(m.top + half));
        let origin = POINT { x: cx - half, y: cy - half };

        let (pal, accent) = (palette(style.theme), style.accent);
        let bg = if pal.glass { frame.as_deref().map(|f| blurred_backdrop(f, origin, size)) } else { None };

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
            pal,
            accent,
            hover: None,
            hprog: [0.0; N],
            glass: style.glass,
            t0: Instant::now(),
            last: Instant::now(),
            release,
            result: None,
            done: false,
            bg,
            dib_dc,
            dib_bits: bits as *mut u8,
            origin,
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
        wedge_at((p.x - st.center.x) as f32, (p.y - st.center.y) as f32, DEAD_R * st.scale)
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
                    let mut animating = hover_changed || st.t0.elapsed().as_secs_f32() * 1000.0 < OPEN_MS + 16.0;
                    for i in 0..N {
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
                        st.result = st.hover.map(|i| ACTIONS[i]);
                        st.done = true;
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if let Some(st) = state(hwnd) {
                    st.result = hover_now(st).map(|i| ACTIONS[i]);
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
        let k = ease((age / OPEN_MS).clamp(0.0, 1.0));
        let scene = render(st);
        // open animation: scale up + fade in around the centre
        let mut out = Pixmap::new(s, s).unwrap();
        let sc = 0.9 + 0.1 * k;
        let c = s as f32 / 2.0;
        out.draw_pixmap(
            0,
            0,
            scene.as_ref(),
            &PixmapPaint { opacity: k, quality: FilterQuality::Bilinear, ..Default::default() },
            Transform::from_row(sc, 0.0, 0.0, sc, c * (1.0 - sc), c * (1.0 - sc)),
            None,
        );
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
fn wedge_path(c: f32, i: usize, r_in: f32, r_out: f32, gap: f32) -> Path {
    let mid = i as f32 * STEP;
    let half = STEP / 2.0;
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
    let (r_in, r_out) = (DEAD_R * sc, OUTER_R * sc);
    let pal = &st.pal;
    let mut pm = Pixmap::new(s, s).unwrap();

    let radii: Vec<f32> = (0..N).map(|i| r_out + GROW * sc * ease(st.hprog[i])).collect();
    let paths: Vec<Path> = (0..N).map(|i| wedge_path(c, i, r_in, radii[i], GAP * sc)).collect();

    // soft drop shadow: stacked translucent discs, offset down
    for k in (1..=7).rev() {
        let r = r_out + k as f32 * 2.6 * sc;
        if let Some(p) = PathBuilder::from_circle(c, c + 4.0 * sc, r) {
            let a = (pal.shadow * 255.0 * 1.4 / k as f32).min(255.0) as u8;
            pm.fill_path(&p, &paint_rgba(0, 0, 0, a), FillRule::Winding, Transform::identity(), None);
        }
    }

    // union of all wedges = clip for the body
    let mut mask = Mask::new(s, s).unwrap();
    for p in &paths {
        mask.fill_path(p, FillRule::Winding, true, Transform::identity());
    }
    if let Some(d) = PathBuilder::from_circle(c, c, r_in - GAP * sc) {
        mask.fill_path(&d, FillRule::Winding, true, Transform::identity()); // centre disc shares the body material
    }
    if pal.glass {
        if let Some(bg) = &st.bg {
            pm.draw_pixmap(0, 0, bg.as_ref(), &PixmapPaint::default(), Transform::identity(), Some(&mask));
        }
    }
    let full = PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, s as f32, s as f32).unwrap());
    pm.fill_path(&full, &paint_rgba(pal.base[0], pal.base[1], pal.base[2], pal.base[3]), FillRule::Winding, Transform::identity(), Some(&mask));
    if pal.glass {
        // diagonal sheen across the glass
        let mut sheen = Paint::default();
        sheen.shader = LinearGradient::new(
            Point::from_xy(c - r_out, c - r_out),
            Point::from_xy(c + r_out, c + r_out),
            vec![
                GradientStop::new(0.0, Color::from_rgba8(st.glass[0][0], st.glass[0][1], st.glass[0][2], 120)),
                GradientStop::new(0.5, Color::from_rgba8(st.glass[1][0], st.glass[1][1], st.glass[1][2], 78)),
                GradientStop::new(1.0, Color::from_rgba8(st.glass[2][0], st.glass[2][1], st.glass[2][2], 112)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap_or(Shader::SolidColor(Color::TRANSPARENT));
        pm.fill_path(&full, &sheen, FillRule::Winding, Transform::identity(), Some(&mask));
    }

    let acc = st.accent;
    for (i, p) in paths.iter().enumerate() {
        let h = st.hprog[i];
        pm.fill_path(p, &paint_rgba(pal.wedge[0], pal.wedge[1], pal.wedge[2], pal.wedge[3]), FillRule::Winding, Transform::identity(), None);
        if h > 0.0 {
            let a = if pal.glass { 150.0 } else { 235.0 } * h;
            pm.fill_path(p, &paint_rgba(acc[0], acc[1], acc[2], a as u8), FillRule::Winding, Transform::identity(), None);
        }
        let mut rim = paint_rgba(pal.rim[0], pal.rim[1], pal.rim[2], pal.rim[3]);
        if pal.glass {
            rim.shader = LinearGradient::new(
                Point::from_xy(c - r_out, c - r_out),
                Point::from_xy(c + r_out, c + r_out),
                vec![GradientStop::new(0.0, Color::from_rgba8(255, 255, 255, 190)), GradientStop::new(1.0, Color::from_rgba8(255, 255, 255, 30))],
                SpreadMode::Pad,
                Transform::identity(),
            )
            .unwrap_or(Shader::SolidColor(Color::WHITE));
        }
        pm.stroke_path(p, &rim, &Stroke { width: 1.0 * sc, ..Default::default() }, Transform::identity(), None);
    }

    // icons
    for i in 0..N {
        let h = ease(st.hprog[i]);
        let (x, y) = pt(c, (r_in + radii[i]) / 2.0 + 1.0 * sc, i as f32 * STEP);
        let col = if st.hprog[i] > 0.5 { [255, 255, 255] } else { pal.text };
        draw_icon(&mut pm, ACTIONS[i], x, y, (13.0 + 2.5 * h) * sc, col, 235, 1.7 * sc);
    }

    // centre disc with the hovered action's name
    if let Some(d) = PathBuilder::from_circle(c, c, r_in - GAP * sc) {
        let (fa, ra) = if pal.glass { (22, 110) } else { (14, 40) };
        pm.fill_path(&d, &paint_rgba(pal.rim[0], pal.rim[1], pal.rim[2], fa), FillRule::Winding, Transform::identity(), None);
        pm.stroke_path(&d, &paint_rgba(pal.rim[0], pal.rim[1], pal.rim[2], ra), &Stroke { width: 1.0 * sc, ..Default::default() }, Transform::identity(), None);
    }
    if let Some(i) = st.hover {
        draw_text(&mut pm, ACTIONS[i].label(), 11.5 * sc, c, c, pal.text, 255);
    } else if let Some(d) = PathBuilder::from_circle(c, c, 2.6 * sc) {
        pm.fill_path(&d, &paint_rgba(pal.text[0], pal.text[1], pal.text[2], 120), FillRule::Winding, Transform::identity(), None);
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
            seg(&[(-0.9, -0.28), (0.9, -0.28)], false);
        }
        Action::Screen => {
            seg(&[(-0.9, -0.75), (0.9, -0.75), (0.9, 0.35), (-0.9, 0.35)], true);
            seg(&[(0.0, 0.35), (0.0, 0.8)], false);
            seg(&[(-0.45, 0.8), (0.45, 0.8)], false);
        }
        Action::Selection => {
            seg(&[(0.0, -0.8), (0.0, 0.8)], false);
            seg(&[(-0.32, -0.8), (0.32, -0.8)], false);
            seg(&[(-0.32, 0.8), (0.32, 0.8)], false);
        }
        Action::Clipboard => {
            seg(&[(-0.65, -0.7), (0.65, -0.7), (0.65, 0.9), (-0.65, 0.9)], true);
            seg(&[(-0.3, -0.95), (0.3, -0.95), (0.3, -0.5), (-0.3, -0.5)], true);
            seg(&[(-0.3, -0.05), (0.3, -0.05)], false);
            seg(&[(-0.3, 0.4), (0.3, 0.4)], false);
        }
        Action::Note => {
            seg(&[(-0.75, 0.75), (-0.6, 0.25), (0.4, -0.75), (0.8, -0.35), (-0.2, 0.6)], true);
            seg(&[(0.15, -0.5), (0.55, -0.1)], false);
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
        Action::Pause => {
            seg(&[(-0.35, -0.7), (-0.35, 0.7)], false);
            seg(&[(0.35, -0.7), (0.35, 0.7)], false);
        }
    }
    if let Some(path) = pb.finish() {
        let stroke = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        pm.stroke_path(&path, &paint_rgba(col[0], col[1], col[2], alpha), &stroke, Transform::identity(), None);
    }
}

// ---------- glass backdrop ----------
/// Blurred, slightly saturated copy of the frozen screen region behind the wheel.
fn blurred_backdrop(frame: &Frame, origin: POINT, size: i32) -> Pixmap {
    let q = 4i32; // work at 1/4 resolution
    let sw = ((size + q - 1) / q) as usize;
    let mut small = vec![[0f32; 3]; sw * sw];
    for sy in 0..sw {
        for sx in 0..sw {
            let (mut r, mut g, mut b, mut n) = (0f32, 0f32, 0f32, 0f32);
            for dy in 0..q {
                for dx in 0..q {
                    let fx = origin.x + sx as i32 * q + dx - frame.x;
                    let fy = origin.y + sy as i32 * q + dy - frame.y;
                    if fx >= 0 && fy >= 0 && fx < frame.w && fy < frame.h {
                        let i = ((fy * frame.w + fx) * 4) as usize;
                        b += frame.bgra[i] as f32;
                        g += frame.bgra[i + 1] as f32;
                        r += frame.bgra[i + 2] as f32;
                        n += 1.0;
                    }
                }
            }
            small[sy * sw + sx] = if n > 0.0 { [r / n, g / n, b / n] } else { [20.0, 22.0, 34.0] };
        }
    }
    // 3 separable box-blur passes (radius 3 at 1/4 res ~ 12 px, gaussian-like)
    let mut tmp = small.clone();
    for _ in 0..3 {
        box_blur(&small, &mut tmp, sw, 3, true);
        box_blur(&tmp, &mut small, sw, 3, false);
    }
    let mut pm_small = Pixmap::new(sw as u32, sw as u32).unwrap();
    for (p, px) in small.iter().zip(pm_small.pixels_mut().iter_mut()) {
        let l = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
        let f = |v: f32| (l + (v - l) * 1.45).clamp(0.0, 255.0) as u8; // a little extra saturation
        *px = ColorU8::from_rgba(f(p[0]), f(p[1]), f(p[2]), 255).premultiply();
    }
    let mut out = Pixmap::new(size as u32, size as u32).unwrap();
    out.draw_pixmap(
        0,
        0,
        pm_small.as_ref(),
        &PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() },
        Transform::from_scale(q as f32, q as f32),
        None,
    );
    out
}

fn box_blur(src: &[[f32; 3]], dst: &mut [[f32; 3]], n: usize, r: usize, horizontal: bool) {
    for a in 0..n {
        for b in 0..n {
            let (mut acc, mut cnt) = ([0f32; 3], 0f32);
            for k in b.saturating_sub(r)..=(b + r).min(n - 1) {
                let i = if horizontal { a * n + k } else { k * n + a };
                for ch in 0..3 {
                    acc[ch] += src[i][ch];
                }
                cnt += 1.0;
            }
            let o = if horizontal { a * n + b } else { b * n + a };
            dst[o] = [acc[0] / cnt, acc[1] / cnt, acc[2] / cnt];
        }
    }
}
