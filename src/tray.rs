//! Wave-mark tray icon rasterization for the Tauri tray. The Marshmallow
//! palette tile + triple-wave logo are drawn into an RGBA buffer and handed to
//! Tauri as an `Image`.

use tauri::image::Image;

const TILE_LIGHT: (f32, f32, f32) = (240.0, 233.0, 223.0); // #F0E9DF cream
const TILE_ACTIVE: (f32, f32, f32) = (235.0, 227.0, 245.0); // #EBE3F5 tint
const TILE_DARK: (f32, f32, f32) = (44.0, 38.0, 52.0); // #2C2634 plum
const IDLE_MARK: (u8, u8, u8) = (181, 173, 160); // #B5ADA0 muted
const ACTIVE_MARK: (u8, u8, u8) = (110, 88, 168); // #6E58A8 accent ink
const IDLE_MARK_DARK: (u8, u8, u8) = (143, 133, 154); // #8F859A
const ACTIVE_MARK_DARK: (u8, u8, u8) = (201, 184, 238); // #C9B8EE

const SIZE: u32 = 32;

pub fn idle_icon() -> Image<'static> {
    Image::new_owned(render_tile(SIZE, IDLE_MARK, TILE_LIGHT), SIZE, SIZE)
}
pub fn active_icon() -> Image<'static> {
    Image::new_owned(render_tile(SIZE, ACTIVE_MARK, TILE_ACTIVE), SIZE, SIZE)
}
pub fn idle_icon_dark() -> Image<'static> {
    Image::new_owned(render_tile(SIZE, IDLE_MARK_DARK, TILE_DARK), SIZE, SIZE)
}
pub fn active_icon_dark() -> Image<'static> {
    Image::new_owned(render_tile(SIZE, ACTIVE_MARK_DARK, TILE_DARK), SIZE, SIZE)
}

/// Rasterize the tile + wave mark into a straight-alpha RGBA buffer.
fn render_tile(size: u32, mark: (u8, u8, u8), tile_color: (f32, f32, f32)) -> Vec<u8> {
    let s = size as f32;
    let inset = 0.75;
    let (x0, y0, x1, y1) = (inset, inset, s - inset, s - inset);
    let tile_r = 0.22 * s;
    let brush_r = (0.08 * s).max(0.9);
    let pts = wave_points(s / 2.0, s / 2.0, 60);

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
            let r = tile_color.0 * (1.0 - m) + mark.0 as f32 * m;
            let g = tile_color.1 * (1.0 - m) + mark.1 as f32 * m;
            let b = tile_color.2 * (1.0 - m) + mark.2 as f32 * m;
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

/// Sample points along the triple-wave logo path:
///   M 4 12 Q 10 3 16 12 T 28 12 T 40 12 T 52 12
/// scaled to fit (w, h) pixels and centered at (ox, oy).
fn wave_points(ox: f32, oy: f32, n: usize) -> Vec<(f32, f32)> {
    // Wave in 56×24 viewBox: segments
    let pts = [(4.0, 12.0), (16.0, 12.0), (28.0, 12.0), (40.0, 12.0), (52.0, 12.0)];
    let cps = [(10.0, 3.0), (22.0, 21.0), (34.0, 3.0), (46.0, 21.0)];
    // Desired rendered size: 24px wide, 10px tall
    let (w, h) = (24.0, 10.0);
    let sx = w / 56.0;
    let sy = h / 24.0;
    let mut v = Vec::with_capacity(4 * (n + 1));
    for seg in 0..4 {
        let p0 = pts[seg];
        let p1 = cps[seg];
        let p2 = pts[seg + 1];
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let x = u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0;
            let y = u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1;
            v.push((ox + (x - 28.0) * sx, oy + (y - 12.0) * sy));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_has_transparent_corner_opaque_body_and_visible_mark() {
        let px = render_tile(32, ACTIVE_MARK, TILE_LIGHT);
        let at = |x: u32, y: u32| {
            let i = ((y * 32 + x) * 4) as usize;
            (px[i], px[i + 1], px[i + 2], px[i + 3])
        };
        assert_eq!(at(0, 0).3, 0, "corner should be transparent");
        assert_eq!(at(16, 16).3, 255, "tile body should be opaque");
        assert!(px.chunks(4).any(|p| p[3] == 255 && (p[0] > 180 || p[2] > 180)), "wave mark should be visible");
    }
}
