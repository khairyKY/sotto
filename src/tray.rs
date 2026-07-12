//! S-mark tray icon rasterization for the Tauri tray. The charcoal tile + cyan
//! (active) / stone (idle) S mark are drawn into an RGBA buffer and handed to
//! Tauri as an `Image`. The tray menu itself is built in `main.rs`.
//!
//! DRAFT (Tauri migration): compiles against `tauri` once the MSVC toolchain is
//! installed; see docs/msvc-setup.md.

use tauri::image::Image;

const CHARCOAL: (f32, f32, f32) = (29.0, 27.0, 24.0); // #1D1B18 tile
const IDLE_MARK: (u8, u8, u8) = (154, 150, 140); // #9A968C muted stone
const ACTIVE_MARK: (u8, u8, u8) = (79, 207, 219); // #4FCFDB cyan

const SIZE: u32 = 32;

pub fn idle_icon() -> Image<'static> {
    Image::new_owned(render_tile(SIZE, IDLE_MARK), SIZE, SIZE)
}
pub fn active_icon() -> Image<'static> {
    Image::new_owned(render_tile(SIZE, ACTIVE_MARK), SIZE, SIZE)
}

/// Rasterize the charcoal tile + S mark into a straight-alpha RGBA buffer. The
/// mark is drawn by stamping a soft round brush along a densely-sampled bezier —
/// cheap, dependency-free anti-aliasing.
fn render_tile(size: u32, mark: (u8, u8, u8)) -> Vec<u8> {
    let s = size as f32;
    let inset = 0.75;
    let (x0, y0, x1, y1) = (inset, inset, s - inset, s - inset);
    let tile_r = 0.22 * s;
    let (cx, cy) = (s / 2.0, s / 2.0);
    let scale = 0.052 * s;
    let brush_r = (0.06 * s).max(0.9);
    let pts = s_mark_points(cx, cy, scale, 60);

    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let tile = rrect_cov(fx, fy, x0, y0, x1, y1, tile_r);
            if tile <= 0.001 {
                continue;
            }
            let mut m = 0.0f32;
            for &(px, py) in &pts {
                let d = ((fx - px) * (fx - px) + (fy - py) * (fy - py)).sqrt();
                let c = (brush_r - d + 0.5).clamp(0.0, 1.0);
                if c > m {
                    m = c;
                    if m >= 1.0 {
                        break;
                    }
                }
            }
            let m = m.min(tile);
            let r = CHARCOAL.0 * (1.0 - m) + mark.0 as f32 * m;
            let g = CHARCOAL.1 * (1.0 - m) + mark.1 as f32 * m;
            let b = CHARCOAL.2 * (1.0 - m) + mark.2 as f32 * m;
            let i = ((y * size + x) * 4) as usize;
            out[i] = r as u8;
            out[i + 1] = g as u8;
            out[i + 2] = b as u8;
            out[i + 3] = (tile * 255.0) as u8;
        }
    }
    out
}

fn rrect_cov(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let hw = (x1 - x0) / 2.0;
    let hh = (y1 - y0) / 2.0;
    let cx = (x0 + x1) / 2.0;
    let cy = (y0 + y1) / 2.0;
    let dx = (px - cx).abs() - (hw - r);
    let dy = (py - cy).abs() - (hh - r);
    let outside = dx.max(0.0).hypot(dy.max(0.0));
    let inside = dx.max(dy).min(0.0);
    let sd = outside + inside - r;
    (0.5 - sd).clamp(0.0, 1.0)
}

fn s_mark_points(ox: f32, oy: f32, s: f32, n: usize) -> Vec<(f32, f32)> {
    let segs = [
        [(ox + 3.8 * s, oy - 4.83 * s), (ox + 3.8 * s, oy - 4.83 * s), (ox - 3.2 * s, oy - 5.75 * s), (ox - 3.2 * s, oy - 2.42 * s)],
        [(ox - 3.2 * s, oy - 2.42 * s), (ox - 3.2 * s, oy + 0.69 * s), (ox + 3.91 * s, oy - 0.69 * s), (ox + 3.91 * s, oy + 2.53 * s)],
        [(ox + 3.91 * s, oy + 2.53 * s), (ox + 3.91 * s, oy + 5.75 * s), (ox - 2.99 * s, oy + 4.83 * s), (ox - 2.99 * s, oy + 4.83 * s)],
    ];
    let mut v = Vec::with_capacity(3 * (n + 1));
    for seg in segs {
        for i in 0..=n {
            v.push(cubic(seg, i as f32 / n as f32));
        }
    }
    v
}

fn cubic(p: [(f32, f32); 4], t: f32) -> (f32, f32) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * p[0].0 + b * p[1].0 + c * p[2].0 + d * p[3].0,
        a * p[0].1 + b * p[1].1 + c * p[2].1 + d * p[3].1,
    )
}

#[cfg(test)]
mod tests {
    use super::render_tile;

    #[test]
    fn tile_has_transparent_corner_opaque_body_and_visible_mark() {
        let px = render_tile(32, (79, 207, 219));
        let at = |x: u32, y: u32| {
            let i = ((y * 32 + x) * 4) as usize;
            (px[i], px[i + 1], px[i + 2], px[i + 3])
        };
        assert_eq!(at(0, 0).3, 0, "corner should be transparent");
        assert_eq!(at(16, 16).3, 255, "tile body should be opaque");
        assert!(px.chunks(4).any(|p| p[3] == 255 && p[2] > 150), "S mark should be visible");
    }
}
