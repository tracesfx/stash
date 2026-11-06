//! Dominant colours: a 16x16x16 histogram, weighted a little towards saturated colours, then greedily
//! picking the heaviest bins that are visibly different from the ones already chosen.

/// Returns up to `n` colours as `#rrggbb`, most dominant first. `rgb` is tightly packed RGB.
pub fn dominant(rgb: &[u8], n: usize) -> Vec<String> {
    let mut bins = vec![(0u32, [0u64; 3]); 4096];
    for p in rgb.chunks_exact(3) {
        let i = ((p[0] as usize >> 4) << 8) | ((p[1] as usize >> 4) << 4) | (p[2] as usize >> 4);
        let b = &mut bins[i];
        b.0 += 1;
        for k in 0..3 {
            b.1[k] += p[k] as u64;
        }
    }
    let mut cands: Vec<(f32, [f32; 3])> = bins
        .iter()
        .filter(|b| b.0 > 0)
        .map(|b| {
            let c = [b.1[0] as f32 / b.0 as f32, b.1[1] as f32 / b.0 as f32, b.1[2] as f32 / b.0 as f32];
            let (mx, mn) = (c.iter().cloned().fold(0.0, f32::max), c.iter().cloned().fold(255.0, f32::min));
            let sat = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
            (b.0 as f32 * (1.0 + 1.5 * sat), c)
        })
        .collect();
    cands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

    let mut out: Vec<[f32; 3]> = Vec::new();
    for (_, c) in cands {
        if out.iter().all(|o| dist(o, &c) > 52.0) {
            out.push(c);
            if out.len() == n {
                break;
            }
        }
    }
    out.iter().map(|c| format!("#{:02x}{:02x}{:02x}", c[0] as u8, c[1] as u8, c[2] as u8)).collect()
}

fn dist(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
