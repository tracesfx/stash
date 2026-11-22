//! Shared 2D helpers for native windows: system fonts, anti-aliased text, Liquid Glass panels drawn with
//! tiny-skia, a blurred backdrop from a screen frame, and per-pixel-alpha layered windows.
use std::sync::OnceLock;

use tiny_skia::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::theme::Style;
use crate::winutil::Frame;

// ---------- fonts ----------
fn load(paths: &[&str]) -> Option<fontdue::Font> {
    for p in paths {
        if let Ok(bytes) = std::fs::read(p) {
            if let Ok(f) = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()) {
                return Some(f);
            }
        }
    }
    None
}

/// Semibold UI font (labels).
pub fn font() -> Option<&'static fontdue::Font> {
    static F: OnceLock<Option<fontdue::Font>> = OnceLock::new();
    F.get_or_init(|| load(&["C:\\Windows\\Fonts\\seguisb.ttf", "C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\arial.ttf"])).as_ref()
}

/// Regular UI font (typed text).
pub fn font_regular() -> Option<&'static fontdue::Font> {
    static F: OnceLock<Option<fontdue::Font>> = OnceLock::new();
    F.get_or_init(|| load(&["C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\arial.ttf"])).as_ref()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Semibold,
}

fn pick(w: Weight) -> Option<&'static fontdue::Font> {
    match w {
        Weight::Regular => font_regular().or_else(font),
        Weight::Semibold => font(),
    }
}

/// Width of `text` at `px` size (semibold).
pub fn text_width(text: &str, px: f32) -> f32 {
    text_width_w(text, px, Weight::Semibold)
}

pub fn text_width_w(text: &str, px: f32, w: Weight) -> f32 {
    pick(w).map(|f| text.chars().map(|c| f.metrics(c, px).advance_width).sum()).unwrap_or(0.0)
}

/// Caret positions: x offset before each char, plus the end (len + 1 entries).
pub fn advances(text: &[char], px: f32, w: Weight) -> Vec<f32> {
    let mut out = Vec::with_capacity(text.len() + 1);
    let mut x = 0.0;
    out.push(0.0);
    if let Some(f) = pick(w) {
        for c in text {
            x += f.metrics(*c, px).advance_width;
            out.push(x);
        }
    } else {
        out.resize(text.len() + 1, 0.0);
    }
    out
}

/// Shortens `text` with an ellipsis so it fits in `max` px.
pub fn ellipsize(text: &str, px: f32, max: f32, w: Weight) -> String {
    if text_width_w(text, px, w) <= max {
        return text.to_string();
    }
    let mut s: String = text.to_string();
    while !s.is_empty() && text_width_w(&format!("{s}…"), px, w) > max {
        s.pop();
    }
    format!("{}…", s.trim_end())
}

/// Draws `text` centred on (cx, cy).
pub fn draw_text(pm: &mut Pixmap, text: &str, px: f32, cx: f32, cy: f32, col: [u8; 3], alpha: u8) {
    let w = text_width(text, px);
    draw_text_at(pm, text, px, cx - w / 2.0, cy, col, alpha, Weight::Semibold, None);
}

/// Draws `text` starting at x, vertically centred on cy, optionally clipped to [x0, x1).
#[allow(clippy::too_many_arguments)]
pub fn draw_text_at(pm: &mut Pixmap, text: &str, px: f32, x: f32, cy: f32, col: [u8; 3], alpha: u8, weight: Weight, clip: Option<(f32, f32)>) {
    let Some(f) = pick(weight) else { return };
    let (ascent, descent) = f.horizontal_line_metrics(px).map(|l| (l.ascent, l.descent)).unwrap_or((px * 0.9, -px * 0.25));
    let baseline = cy + (ascent + descent) / 2.0; // centre the line box on cy
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let (cx0, cx1) = clip.map(|(a, b)| (a.floor() as i32, b.ceil() as i32)).unwrap_or((0, w));
    let data = pm.data_mut();
    let mut pen = x;
    for ch in text.chars() {
        let (m, bmp) = f.rasterize(ch, px);
        let gx = (pen + m.xmin as f32).round() as i32;
        let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
        for row in 0..m.height {
            for colx in 0..m.width {
                let cov = bmp[row * m.width + colx] as f32 / 255.0 * alpha as f32 / 255.0;
                if cov <= 0.0 {
                    continue;
                }
                let (px_x, px_y) = (gx + colx as i32, gy + row as i32);
                if px_x < cx0.max(0) || px_y < 0 || px_x >= cx1.min(w) || px_y >= h {
                    continue;
                }
                let i = ((px_y * w + px_x) * 4) as usize;
                // premultiplied source-over
                for k in 0..3 {
                    data[i + k] = (col[k] as f32 * cov + data[i + k] as f32 * (1.0 - cov)) as u8;
                }
                data[i + 3] = (255.0 * cov + data[i + 3] as f32 * (1.0 - cov)) as u8;
            }
        }
        pen += m.advance_width;
    }
}

// ---------- shapes ----------
pub fn paint(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

/// Rounded rectangle (radius clamped to half the shorter side).
pub fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let k = 0.5523 * r; // cubic circle approximation
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

pub fn circle(cx: f32, cy: f32, r: f32) -> Option<Path> {
    PathBuilder::from_circle(cx, cy, r.max(0.1))
}

/// Soft drop shadow under a rounded rect: stacked translucent rounded rects.
pub fn shadow_rrect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, spread: f32, strength: f32) {
    let steps = 8;
    for k in (1..=steps).rev() {
        let g = spread * k as f32 / steps as f32;
        if let Some(p) = rrect(x - g, y - g + spread * 0.35, w + 2.0 * g, h + 2.0 * g, r + g) {
            let a = (strength * 255.0 / (k as f32 * 1.6)).min(255.0) as u8;
            pm.fill_path(&p, &paint([0, 0, 0, a]), FillRule::Winding, Transform::identity(), None);
        }
    }
}

/// The Liquid Glass material (SPEC §11, reference picks): blurred backdrop, neutral smoky body, a faint
/// diagonal sheen of the configured glass colours, and a light rim that is brightest along the top-left.
pub fn glass_fill(pm: &mut Pixmap, path: &Path, bounds: Rect, bg: Option<&Pixmap>, style: &Style, body_alpha: f32) {
    let (w, h) = (pm.width(), pm.height());
    let Some(mut mask) = Mask::new(w, h) else { return };
    mask.fill_path(path, FillRule::Winding, true, Transform::identity());
    // low blur fades the frosted copy out, so at blur 0 the real, sharp screen shows through
    let frost = crate::theme::frost(style);
    if let Some(bg) = bg {
        let op = 0.1 + 0.9 * frost;
        pm.draw_pixmap(0, 0, bg.as_ref(), &PixmapPaint { opacity: op, ..Default::default() }, Transform::identity(), Some(&mask));
    }
    let full = PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap());
    // smoky neutral body: low translucency = denser tint; without frost only a light tint is kept
    let (frosted, clear) = (0.74 - style.translucency * 0.5, 0.08 + (1.0 - style.translucency) * 0.3);
    let base = clear + (frosted - clear) * frost;
    let a = (base * 255.0 * body_alpha).clamp(14.0, 235.0) as u8;
    pm.fill_path(&full, &paint([16, 17, 22, a]), FillRule::Winding, Transform::identity(), Some(&mask));
    // diagonal sheen: white highlight top-left fading out, tinted by the glass colours (Colour sheen 0-100 %)
    let g = style.glass;
    let t = style.tint.clamp(0.0, 1.0);
    let stops = vec![
        GradientStop::new(0.0, Color::from_rgba8(255, 255, 255, 34)),
        GradientStop::new(0.35, Color::from_rgba8(g[0][0], g[0][1], g[0][2], (125.0 * t) as u8)),
        GradientStop::new(0.7, Color::from_rgba8(g[1][0], g[1][1], g[1][2], (95.0 * t) as u8)),
        GradientStop::new(1.0, Color::from_rgba8(g[2][0], g[2][1], g[2][2], (115.0 * t) as u8)),
    ];
    if let Some(sh) = LinearGradient::new(
        Point::from_xy(bounds.left(), bounds.top()),
        Point::from_xy(bounds.right(), bounds.bottom()),
        stops,
        SpreadMode::Pad,
        Transform::identity(),
    ) {
        let mut p = Paint::default();
        p.shader = sh;
        p.anti_alias = true;
        pm.fill_path(&full, &p, FillRule::Winding, Transform::identity(), Some(&mask));
    }
}

/// 1 px rim: bright at the top-left, faint at the bottom-right (the "edge light" of the references).
pub fn glass_rim(pm: &mut Pixmap, path: &Path, bounds: Rect, width: f32, strength: f32) {
    let s = strength.clamp(0.0, 1.0);
    let mut p = Paint::default();
    p.anti_alias = true;
    p.shader = LinearGradient::new(
        Point::from_xy(bounds.left(), bounds.top()),
        Point::from_xy(bounds.left() + bounds.width() * 0.4, bounds.bottom()),
        vec![
            GradientStop::new(0.0, Color::from_rgba8(255, 255, 255, (150.0 * s) as u8)),
            GradientStop::new(0.55, Color::from_rgba8(255, 255, 255, (46.0 * s) as u8)),
            GradientStop::new(1.0, Color::from_rgba8(255, 255, 255, (78.0 * s) as u8)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap_or(Shader::SolidColor(Color::from_rgba8(255, 255, 255, 60)));
    pm.stroke_path(path, &p, &Stroke { width, ..Default::default() }, Transform::identity(), None);
}

// ---------- backdrop ----------
/// Blurred, slightly saturated copy of the screen region (origin, w x h) taken from `frame`.
pub fn blur_backdrop(frame: &Frame, origin: POINT, w: i32, h: i32, blur: u8, sat: f32) -> Option<Pixmap> {
    let q = 4i32; // work at 1/4 resolution
    let (sw, sh) = (((w + q - 1) / q) as usize, ((h + q - 1) / q) as usize);
    if sw == 0 || sh == 0 {
        return None;
    }
    let mut small = vec![[0f32; 3]; sw * sh];
    for sy in 0..sh {
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
            small[sy * sw + sx] = if n > 0.0 { [r / n, g / n, b / n] } else { [20.0, 22.0, 30.0] };
        }
    }
    // 3 separable box-blur passes (gaussian-like)
    let rad = if blur == 0 { 0 } else { (blur as usize * 5 / 26).max(1) }; // ~0.8x the CSS blur radius at 1/4 res
    if rad > 0 {
        let mut tmp = small.clone();
        for _ in 0..3 {
            box_blur(&small, &mut tmp, sw, sh, rad, true);
            box_blur(&tmp, &mut small, sw, sh, rad, false);
        }
    }
    let mut pm_small = Pixmap::new(sw as u32, sh as u32)?;
    for (p, px) in small.iter().zip(pm_small.pixels_mut().iter_mut()) {
        let l = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
        let f = |v: f32| (l + (v - l) * sat).clamp(0.0, 255.0) as u8;
        *px = ColorU8::from_rgba(f(p[0]), f(p[1]), f(p[2]), 255).premultiply();
    }
    let mut out = Pixmap::new(w as u32, h as u32)?;
    out.draw_pixmap(
        0,
        0,
        pm_small.as_ref(),
        &PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() },
        Transform::from_scale(q as f32, q as f32),
        None,
    );
    Some(out)
}

fn box_blur(src: &[[f32; 3]], dst: &mut [[f32; 3]], w: usize, h: usize, r: usize, horizontal: bool) {
    let (outer, inner) = if horizontal { (h, w) } else { (w, h) };
    for a in 0..outer {
        for b in 0..inner {
            let (mut acc, mut cnt) = ([0f32; 3], 0f32);
            for k in b.saturating_sub(r)..=(b + r).min(inner - 1) {
                let i = if horizontal { a * w + k } else { k * w + a };
                for ch in 0..3 {
                    acc[ch] += src[i][ch];
                }
                cnt += 1.0;
            }
            let o = if horizontal { a * w + b } else { b * w + a };
            dst[o] = [acc[0] / cnt, acc[1] / cnt, acc[2] / cnt];
        }
    }
}

/// RGBA (straight alpha) image drawn into a rect, cover-fit and clipped to `clip`.
pub fn draw_image_cover(pm: &mut Pixmap, rgba: &[u8], iw: u32, ih: u32, dst: Rect, clip: &Path, opacity: f32) {
    let Some(mut img) = Pixmap::new(iw, ih) else { return };
    for (px, s) in img.pixels_mut().iter_mut().zip(rgba.chunks_exact(4)) {
        *px = ColorU8::from_rgba(s[0], s[1], s[2], s[3]).premultiply();
    }
    let k = (dst.width() / iw as f32).max(dst.height() / ih as f32);
    let (dw, dh) = (iw as f32 * k, ih as f32 * k);
    let (ox, oy) = (dst.left() + (dst.width() - dw) / 2.0, dst.top() + (dst.height() - dh) / 2.0);
    let Some(mut mask) = Mask::new(pm.width(), pm.height()) else { return };
    mask.fill_path(clip, FillRule::Winding, true, Transform::identity());
    pm.draw_pixmap(
        0,
        0,
        img.as_ref(),
        &PixmapPaint { opacity, quality: FilterQuality::Bicubic, ..Default::default() },
        Transform::from_row(k, 0.0, 0.0, k, ox, oy),
        Some(&mask),
    );
}

// ---------- layered window ----------
/// A per-pixel-alpha popup window backed by a DIB; `present` pushes a tiny-skia pixmap to it.
pub struct Layered {
    pub hwnd: HWND,
    pub origin: POINT,
    pub w: i32,
    pub h: i32,
    dc: HDC,
    bmp: HBITMAP,
    bits: *mut u8,
}

impl Layered {
    /// Creates (hidden) a layered popup of class `class` (registered with `proc_`).
    pub unsafe fn new(class: windows::core::PCWSTR, title: windows::core::PCWSTR, proc_: WNDPROC, ex: WINDOW_EX_STYLE, origin: POINT, w: i32, h: i32) -> Option<Layered> {
        unsafe {
            use windows::Win32::System::LibraryLoader::GetModuleHandleW;
            let screen = GetDC(None);
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok();
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let bmp = bmp?;
            SelectObject(dc, bmp.into());
            let hinst = GetModuleHandleW(None).ok()?;
            let wc = WNDCLASSW { lpfnWndProc: proc_, hInstance: hinst.into(), lpszClassName: class, hCursor: LoadCursorW(None, IDC_ARROW).ok()?, ..Default::default() };
            RegisterClassW(&wc); // fails harmlessly when already registered
            let hwnd = CreateWindowExW(ex | WS_EX_LAYERED, class, title, WS_POPUP, origin.x, origin.y, w, h, None, None, Some(hinst.into()), None).ok()?;
            Some(Layered { hwnd, origin, w, h, dc, bmp, bits: bits as *mut u8 })
        }
    }

    /// Pushes a premultiplied RGBA pixmap of exactly w x h to the window.
    pub fn present(&self, pm: &Pixmap) {
        unsafe {
            let n = (self.w * self.h * 4) as usize;
            let dst = std::slice::from_raw_parts_mut(self.bits, n);
            for (d, p) in dst.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
                d[0] = p[2];
                d[1] = p[1];
                d[2] = p[0];
                d[3] = p[3];
            }
            let size = SIZE { cx: self.w, cy: self.h };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let screen = GetDC(None);
            let _ = UpdateLayeredWindow(self.hwnd, Some(screen), Some(&self.origin), Some(&size), Some(self.dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            ReleaseDC(None, screen);
        }
    }
}

impl Drop for Layered {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let _ = DeleteDC(self.dc);
            let _ = DeleteObject(self.bmp.into());
        }
    }
}

/// Fades/scales a rendered scene into a window-sized pixmap (open/close animations).
pub fn compose(scene: &Pixmap, opacity: f32, scale: f32, cx: f32, cy: f32) -> Pixmap {
    let mut out = Pixmap::new(scene.width(), scene.height()).unwrap();
    out.draw_pixmap(
        0,
        0,
        scene.as_ref(),
        &PixmapPaint { opacity: opacity.clamp(0.0, 1.0), quality: FilterQuality::Bilinear, ..Default::default() },
        Transform::from_row(scale, 0.0, 0.0, scale, cx * (1.0 - scale), cy * (1.0 - scale)),
        None,
    );
    out
}

pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Offscreen previews of the native windows (no window is shown, no focus taken, no input simulated).
/// Run: STASH_PREVIEW_BG=<photo> STASH_PREVIEW_OUT=<dir> cargo test previews -- --ignored
#[cfg(test)]
pub mod preview {
    use super::*;

    /// The backdrop photo as a screen frame of size w x h.
    pub fn backdrop(w: i32, h: i32) -> Frame {
        let path = std::env::var("STASH_PREVIEW_BG").expect("STASH_PREVIEW_BG");
        let img = image::open(path).expect("backdrop").to_rgb8();
        let img = image::imageops::resize(&img, w as u32, h as u32, image::imageops::FilterType::Triangle);
        let bgra = img.pixels().flat_map(|p| [p[2], p[1], p[0], 255]).collect();
        Frame { x: 0, y: 0, w, h, bgra }
    }

    /// Composites `scene` (premultiplied) over the frame region at `origin` and writes a PNG.
    pub fn save(scene: &Pixmap, frame: &Frame, origin: POINT, name: &str) {
        let out = std::path::PathBuf::from(std::env::var("STASH_PREVIEW_OUT").expect("STASH_PREVIEW_OUT"));
        let (w, h) = (scene.width() as i32, scene.height() as i32);
        let mut img = image::RgbImage::new(w as u32, h as u32);
        for y in 0..h {
            for x in 0..w {
                let fi = (((y + origin.y) * frame.w + x + origin.x) * 4) as usize;
                let (b, g, r) = (frame.bgra[fi] as f32, frame.bgra[fi + 1] as f32, frame.bgra[fi + 2] as f32);
                let p = scene.pixel(x as u32, y as u32).unwrap();
                let a = p.alpha() as f32 / 255.0;
                let mix = |s: u8, d: f32| (s as f32 + d * (1.0 - a)).min(255.0) as u8;
                img.put_pixel(x as u32, y as u32, image::Rgb([mix(p.red(), r), mix(p.green(), g), mix(p.blue(), b)]));
            }
        }
        img.save(out.join(name)).expect("save preview");
    }

    /// Default look, with optional STASH_PREVIEW_TR / STASH_PREVIEW_TINT overrides (0-100).
    pub fn style() -> Style {
        let mut c = crate::config::Config::default();
        if let Some(v) = std::env::var("STASH_PREVIEW_TR").ok().and_then(|v| v.parse().ok()) {
            c.appearance.glass_translucency = v;
        }
        if let Some(v) = std::env::var("STASH_PREVIEW_TINT").ok().and_then(|v| v.parse().ok()) {
            c.appearance.glass_tint = v;
        }
        if let Some(v) = std::env::var("STASH_PREVIEW_BLUR").ok().and_then(|v| v.parse().ok()) { c.appearance.glass_blur = v; }
        if let Some(v) = std::env::var("STASH_PREVIEW_SAT").ok().and_then(|v| v.parse().ok()) { c.appearance.glass_saturation = v; }
        crate::theme::style_for(&c)
    }
}
