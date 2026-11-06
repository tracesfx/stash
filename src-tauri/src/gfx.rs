//! Shared 2D helpers for native windows: system font + anti-aliased text onto a tiny-skia pixmap.
use std::sync::OnceLock;

use tiny_skia::Pixmap;

pub fn font() -> Option<&'static fontdue::Font> {
    static F: OnceLock<Option<fontdue::Font>> = OnceLock::new();
    F.get_or_init(|| {
        for p in ["C:\\Windows\\Fonts\\seguisb.ttf", "C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\arial.ttf"] {
            if let Ok(bytes) = std::fs::read(p) {
                if let Ok(f) = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()) {
                    return Some(f);
                }
            }
        }
        None
    })
    .as_ref()
}

/// Width of `text` at `px` size.
pub fn text_width(text: &str, px: f32) -> f32 {
    font().map(|f| text.chars().map(|c| f.metrics(c, px).advance_width).sum()).unwrap_or(0.0)
}

/// Draws `text` centred on (cx, cy).
pub fn draw_text(pm: &mut Pixmap, text: &str, px: f32, cx: f32, cy: f32, col: [u8; 3], alpha: u8) {
    let Some(f) = font() else { return };
    let glyphs: Vec<_> = text.chars().map(|ch| f.rasterize(ch, px)).collect();
    let width: f32 = glyphs.iter().map(|(m, _)| m.advance_width).sum();
    let (ascent, descent) = f.horizontal_line_metrics(px).map(|l| (l.ascent, l.descent)).unwrap_or((px * 0.9, -px * 0.25));
    let baseline = cy + (ascent + descent) / 2.0; // centre the line box on cy
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    let mut x = cx - width / 2.0;
    for (m, bmp) in &glyphs {
        let gx = (x + m.xmin as f32).round() as i32;
        let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
        for row in 0..m.height {
            for colx in 0..m.width {
                let cov = bmp[row * m.width + colx] as f32 / 255.0 * alpha as f32 / 255.0;
                if cov <= 0.0 {
                    continue;
                }
                let (px_x, px_y) = (gx + colx as i32, gy + row as i32);
                if px_x < 0 || px_y < 0 || px_x >= w || px_y >= h {
                    continue;
                }
                let i = ((px_y * w + px_x) * 4) as usize;
                for k in 0..3 {
                    data[i + k] = (col[k] as f32 * cov + data[i + k] as f32 * (1.0 - cov)) as u8;
                }
                data[i + 3] = (255.0 * cov + data[i + 3] as f32 * (1.0 - cov)) as u8;
            }
        }
        x += m.advance_width;
    }
}
