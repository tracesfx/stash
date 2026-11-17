//! Native Win32 snip overlay (webview is too slow to cold-start). Shows a dimmed frozen copy of the desktop;
//! drag to select; returns the selected rect. Themed: accent border + corner brackets, a rounded size pill,
//! and in Liquid Glass the dim is tinted with the configured glass colour.
use std::ffi::c_void;
use std::mem::size_of;

use tiny_skia::*;
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::{draw_text, text_width};
use crate::theme::{Resolved, Style};
use crate::winutil::Frame;

const MIN_SIZE: i32 = 4;
const ESC_TIMER: usize = 1;

struct State {
    w: i32,
    h: i32,
    scale: f32,
    style: Style,
    orig_dc: HDC,
    dim_dc: HDC,
    back_dc: HDC,
    border: HBRUSH,
    bracket: HBRUSH,
    start: Option<POINT>,
    sel: Option<RECT>,
    result: Option<RECT>,
    done: bool,
}

fn norm(a: POINT, b: POINT) -> RECT {
    RECT { left: a.x.min(b.x), top: a.y.min(b.y), right: a.x.max(b.x), bottom: a.y.max(b.y) }
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut State> {
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
    unsafe { p.as_mut() }
}

unsafe fn dib_dc(screen: HDC, w: i32, h: i32) -> windows::core::Result<(HDC, *mut u8)> {
    unsafe {
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let dc = CreateCompatibleDC(Some(screen));
        SelectObject(dc, bmp.into());
        Ok((dc, bits as *mut u8))
    }
}

fn colorref(c: [u8; 3]) -> COLORREF {
    COLORREF(c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16)
}

fn lighten(c: [u8; 3], t: f32) -> [u8; 3] {
    [0, 1, 2].map(|i| (c[i] as f32 + (255.0 - c[i] as f32) * t) as u8)
}

/// Blocks until the user finishes or cancels. Returns the selection in virtual-desktop coordinates.
pub fn select_region(frame: &Frame, style: Style, scale: f32) -> Option<RECT> {
    unsafe {
        let screen = GetDC(None);
        let (orig_dc, orig_bits) = dib_dc(screen, frame.w, frame.h).ok()?;
        let (dim_dc, dim_bits) = dib_dc(screen, frame.w, frame.h).ok()?;
        let n = (frame.w * frame.h) as usize;
        let src = frame.bgra.as_ptr() as *const u32;
        let (o, d) = (orig_bits as *mut u32, dim_bits as *mut u32);
        // Liquid Glass: tint the dimmed backdrop a little with the first glass colour (BGRA packed)
        let tint: u32 = if style.theme == Resolved::Glass {
            let g = style.glass[0];
            (g[2] as u32 / 9) << 16 | (g[1] as u32 / 9) << 8 | (g[0] as u32 / 9)
        } else {
            0
        };
        for i in 0..n {
            let px = *src.add(i);
            *o.add(i) = px;
            *d.add(i) = ((px >> 2) & 0x3F3F3F3F) + tint; // 25% brightness (+tint; max channel 63+28, no overflow)
        }
        let back_bmp = CreateCompatibleBitmap(screen, frame.w, frame.h);
        let back_dc = CreateCompatibleDC(Some(screen));
        SelectObject(back_dc, back_bmp.into());
        ReleaseDC(None, screen);

        let mut st = Box::new(State {
            w: frame.w,
            h: frame.h,
            scale,
            style,
            orig_dc,
            dim_dc,
            back_dc,
            border: CreateSolidBrush(colorref(style.hover)),
            bracket: CreateSolidBrush(colorref(lighten(style.hover, 0.5))),
            start: None,
            sel: None,
            result: None,
            done: false,
        });

        let hinst = GetModuleHandleW(None).ok()?;
        let class = w!("StashSnipOverlay");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_CROSS).ok()?,
            ..Default::default()
        };
        RegisterClassW(&wc); // fails harmlessly if already registered

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class,
            w!("Stash snip"),
            WS_POPUP,
            frame.x,
            frame.y,
            frame.w,
            frame.h,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .ok()?;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut *st as *mut State as isize);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        SetTimer(Some(hwnd), ESC_TIMER, 15, None); // Esc fallback if focus was refused

        let mut msg = MSG::default();
        while !st.done && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = KillTimer(Some(hwnd), ESC_TIMER);
        let _ = DestroyWindow(hwnd);
        let _ = DeleteDC(orig_dc);
        let _ = DeleteDC(dim_dc);
        let _ = DeleteDC(back_dc);
        let _ = DeleteObject(back_bmp.into());
        let _ = DeleteObject(st.border.into());
        let _ = DeleteObject(st.bracket.into());
        st.result.map(|r| RECT {
            left: r.left + frame.x,
            top: r.top + frame.y,
            right: r.right + frame.x,
            bottom: r.bottom + frame.y,
        })
    }
}

fn point_of(lparam: LPARAM) -> POINT {
    POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_LBUTTONDOWN => {
                if let Some(st) = state(hwnd) {
                    st.start = Some(point_of(lparam));
                    SetCapture(hwnd);
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if let Some(st) = state(hwnd) {
                    if let Some(s) = st.start {
                        let new = norm(s, point_of(lparam));
                        let mut dirty = new;
                        if let Some(old) = st.sel {
                            dirty = RECT {
                                left: dirty.left.min(old.left),
                                top: dirty.top.min(old.top),
                                right: dirty.right.max(old.right),
                                bottom: dirty.bottom.max(old.bottom),
                            };
                        }
                        st.sel = Some(new);
                        let k = st.scale;
                        // room for the border, corner brackets and the size pill (above or below, up to ~160px wide)
                        let d = RECT {
                            left: dirty.left - (10.0 * k) as i32,
                            top: dirty.top - (60.0 * k) as i32,
                            right: dirty.right + (170.0 * k) as i32,
                            bottom: dirty.bottom + (60.0 * k) as i32,
                        };
                        let _ = InvalidateRect(Some(hwnd), Some(&d), false);
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if let Some(st) = state(hwnd) {
                    let _ = ReleaseCapture();
                    if let Some(s) = st.start.take() {
                        let r = norm(s, point_of(lparam));
                        if r.right - r.left >= MIN_SIZE && r.bottom - r.top >= MIN_SIZE {
                            st.result = Some(r);
                        }
                    }
                    st.done = true; // a plain click without a drag cancels
                }
                LRESULT(0)
            }
            WM_RBUTTONDOWN => {
                if let Some(st) = state(hwnd) {
                    st.done = true;
                }
                LRESULT(0)
            }
            WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
                if let Some(st) = state(hwnd) {
                    st.done = true;
                }
                LRESULT(0)
            }
            WM_TIMER => {
                if (GetAsyncKeyState(VK_ESCAPE.0 as i32) as u16 & 0x8000) != 0 {
                    if let Some(st) = state(hwnd) {
                        st.done = true;
                    }
                }
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if let Some(st) = state(hwnd) {
                    paint(st, hdc, ps.rcPaint);
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn bar(dc: HDC, brush: HBRUSH, l: i32, t: i32, r: i32, b: i32) {
    unsafe {
        FillRect(dc, &RECT { left: l, top: t, right: r, bottom: b }, brush);
    }
}

unsafe fn paint(st: &State, hdc: HDC, clip: RECT) {
    unsafe {
        let (cx, cy, cw, ch) = (clip.left, clip.top, clip.right - clip.left, clip.bottom - clip.top);
        let b = st.back_dc;
        let k = st.scale;
        let _ = BitBlt(b, cx, cy, cw, ch, Some(st.dim_dc), cx, cy, SRCCOPY);
        if let Some(r) = st.sel {
            let (rw, rh) = (r.right - r.left, r.bottom - r.top);
            let _ = BitBlt(b, r.left, r.top, rw, rh, Some(st.orig_dc), r.left, r.top, SRCCOPY);

            // 2px accent border just outside the selection
            let t = (2.0 * k).max(2.0) as i32;
            bar(b, st.border, r.left - t, r.top - t, r.right + t, r.top);
            bar(b, st.border, r.left - t, r.bottom, r.right + t, r.bottom + t);
            bar(b, st.border, r.left - t, r.top, r.left, r.bottom);
            bar(b, st.border, r.right, r.top, r.right + t, r.bottom);
            // corner brackets (the app's logo), lighter accent, on top of the border
            let (len, th) = ((16.0 * k) as i32, (4.0 * k).max(4.0) as i32);
            let (l, tp, rt, bt) = (r.left - t, r.top - t, r.right + t, r.bottom + t);
            for (x, y, sx, sy) in [(l, tp, 1, 1), (rt, tp, -1, 1), (l, bt, 1, -1), (rt, bt, -1, -1)] {
                let (x0, x1) = if sx > 0 { (x, x + len) } else { (x - len, x) };
                let (y0, y1) = if sy > 0 { (y, y + th) } else { (y - th, y) };
                bar(b, st.bracket, x0, y0, x1, y1);
                let (vx0, vx1) = if sx > 0 { (x, x + th) } else { (x - th, x) };
                let (vy0, vy1) = if sy > 0 { (y, y + len) } else { (y - len, y) };
                bar(b, st.bracket, vx0, vy0, vx1, vy1);
            }
            draw_pill(st, b, r, rw, rh);
        }
        let _ = BitBlt(hdc, cx, cy, cw, ch, Some(b), cx, cy, SRCCOPY);
        let _ = st.w;
    }
}

/// Rounded "W × H" pill, anti-aliased with tiny-skia and alpha-blended onto the back buffer.
unsafe fn draw_pill(st: &State, dc: HDC, r: RECT, rw: i32, rh: i32) {
    unsafe {
        let k = st.scale;
        let text = format!("{rw} × {rh}");
        let px = 12.5 * k;
        let (pw, ph) = ((text_width(&text, px) + 24.0 * k).ceil() as u32, (26.0 * k).ceil() as u32);
        let Some(mut pm) = Pixmap::new(pw, ph) else { return };

        let (bg, fg, rim): ([u8; 4], [u8; 3], [u8; 4]) = match st.style.theme {
            Resolved::Dark => ([22, 22, 28, 232], [245, 245, 250], [255, 255, 255, 40]),
            Resolved::Light => ([255, 255, 255, 240], [28, 28, 34], [0, 0, 0, 45]),
            Resolved::Glass => ([0, 0, 0, 0], [255, 255, 255], [255, 255, 255, 140]),
        };
        let rr = ph as f32 / 2.0;
        let mut pb = PathBuilder::new();
        let (x0, y0, x1, y1) = (0.5, 0.5, pw as f32 - 0.5, ph as f32 - 0.5);
        pb.move_to(x0 + rr, y0);
        pb.line_to(x1 - rr, y0);
        pb.quad_to(x1, y0, x1, y0 + rr);
        pb.quad_to(x1, y1, x1 - rr, y1);
        pb.line_to(x0 + rr, y1);
        pb.quad_to(x0, y1, x0, y1 - rr);
        pb.quad_to(x0, y0, x0 + rr, y0);
        pb.close();
        let Some(path) = pb.finish() else { return };

        let mut fill = Paint::default();
        fill.anti_alias = true;
        if st.style.theme == Resolved::Glass {
            let (a, b) = (st.style.glass[0], st.style.glass[1]);
            fill.shader = LinearGradient::new(
                Point::from_xy(0.0, 0.0),
                Point::from_xy(pw as f32, ph as f32),
                vec![GradientStop::new(0.0, Color::from_rgba8(a[0], a[1], a[2], 235)), GradientStop::new(1.0, Color::from_rgba8(b[0], b[1], b[2], 235))],
                SpreadMode::Pad,
                Transform::identity(),
            )
            .unwrap_or(Shader::SolidColor(Color::BLACK));
        } else {
            fill.set_color_rgba8(bg[0], bg[1], bg[2], bg[3]);
        }
        pm.fill_path(&path, &fill, FillRule::Winding, Transform::identity(), None);
        let mut rp = Paint::default();
        rp.anti_alias = true;
        rp.set_color_rgba8(rim[0], rim[1], rim[2], rim[3]);
        pm.stroke_path(&path, &rp, &Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
        draw_text(&mut pm, &text, px, pw as f32 / 2.0, ph as f32 / 2.0, fg, 255);

        // position: above the selection's top-left; else below; else inside
        let gap = (8.0 * k) as i32;
        let mut x = r.left.clamp(2, (st.w - pw as i32 - 2).max(2));
        let mut y = r.top - ph as i32 - gap;
        if y < 2 {
            y = r.bottom + gap;
            if y + ph as i32 > st.h - 2 {
                y = r.top + gap;
                x = (r.left + gap).clamp(2, (st.w - pw as i32 - 2).max(2));
            }
        }

        // premultiplied RGBA -> premultiplied BGRA DIB, then AlphaBlend
        let screen = GetDC(None);
        let Ok((pdc, bits)) = dib_dc(screen, pw as i32, ph as i32) else {
            ReleaseDC(None, screen);
            return;
        };
        ReleaseDC(None, screen);
        let dst = std::slice::from_raw_parts_mut(bits, (pw * ph * 4) as usize);
        for (d, p) in dst.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
            d[0] = p[2];
            d[1] = p[1];
            d[2] = p[0];
            d[3] = p[3];
        }
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        let _ = AlphaBlend(dc, x, y, pw as i32, ph as i32, pdc, 0, 0, pw as i32, ph as i32, blend);
        let bmp = GetCurrentObject(pdc, OBJ_BITMAP);
        let _ = DeleteDC(pdc);
        let _ = DeleteObject(bmp);
    }
}
