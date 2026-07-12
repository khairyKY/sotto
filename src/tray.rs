//! System-tray icon + menu. The window-message loop that pumps tray events
//! lives in `overlay.rs` (eframe/winit) — on Windows the tray must be created
//! on that same thread, so this module only builds the icon + menu and exposes
//! the handles the overlay app needs to keep it alive, swap the active/idle
//! icon, and dispatch menu clicks.
//!
//! The icon is the S mark (a bezier squiggle) stroked in the state accent over
//! a charcoal rounded-square tile, rasterized here into an RGBA bitmap — the
//! same mark the overlay paints, so branding stays consistent.

use crate::config::PolishMode;
use crate::Controls;
use std::sync::atomic::Ordering;
use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

const CHARCOAL: (f32, f32, f32) = (29.0, 27.0, 24.0); // #1D1B18 tile
const IDLE_MARK: (u8, u8, u8) = (154, 150, 140); // #9A968C muted stone
const ACTIVE_MARK: (u8, u8, u8) = (79, 207, 219); // #4FCFDB cyan

/// What a menu click means to the app. Runtime toggles (pause, polish tier) are
/// applied internally; only actions the overlay app must perform are returned.
pub enum TrayAction {
    Quit,
    OpenSettings,
    None,
}

/// A live tray icon + menu. Dropping it removes the icon, so the overlay app
/// holds onto it for the process lifetime.
pub struct Tray {
    icon: TrayIcon,
    idle_icon: Icon,
    active_icon: Icon,
    controls: Controls,
    header: MenuItem,
    pause_item: CheckMenuItem,
    polish_off: CheckMenuItem,
    polish_rules: CheckMenuItem,
    polish_ai: CheckMenuItem,
    settings_id: MenuId,
    pub quit_id: MenuId,
}

impl Tray {
    /// Build the tray icon + menu. Must be called on the thread that runs the
    /// window-message loop (the eframe main thread).
    pub fn build(controls: Controls) -> anyhow::Result<Self> {
        let menu = Menu::new();

        let header = MenuItem::new("Sotto — Ready", false, None);
        let pause_item = CheckMenuItem::new("Pause dictation", true, false, None);

        let cur = PolishMode::from_u8(controls.polish_mode.load(Ordering::Relaxed));
        let polish_off = CheckMenuItem::new("Off", true, cur == PolishMode::Off, None);
        let polish_rules = CheckMenuItem::new("Rules only", true, cur == PolishMode::Rules, None);
        let polish_ai = CheckMenuItem::new("AI", true, cur == PolishMode::Ai, None);
        let polish = Submenu::new("Polish", true);
        polish.append(&polish_off)?;
        polish.append(&polish_rules)?;
        polish.append(&polish_ai)?;

        let settings = MenuItem::new("Settings…", true, None);
        let quit = MenuItem::new("Quit Sotto", true, None);

        menu.append(&header)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&pause_item)?;
        menu.append(&polish)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&settings)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit)?;

        let idle_icon = build_icon(IDLE_MARK)?;
        let active_icon = build_icon(ACTIVE_MARK)?;

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Sotto")
            .with_icon(idle_icon.clone())
            .build()?;

        Ok(Self {
            icon,
            idle_icon,
            active_icon,
            controls,
            settings_id: settings.id().clone(),
            quit_id: quit.id().clone(),
            header,
            pause_item,
            polish_off,
            polish_rules,
            polish_ai,
        })
    }

    /// Swap the tray icon between the idle (stone) and active (cyan) marks.
    pub fn set_active(&self, active: bool) {
        let icon = if active { &self.active_icon } else { &self.idle_icon };
        let _ = self.icon.set_icon(Some(icon.clone()));
    }

    /// Apply a menu click. Pause / polish-tier are handled here (runtime state);
    /// quit / settings are returned for the overlay app to act on.
    pub fn handle_menu_event(&self, id: &MenuId) -> TrayAction {
        if *id == self.quit_id {
            return TrayAction::Quit;
        }
        if *id == self.settings_id {
            return TrayAction::OpenSettings;
        }
        if *id == *self.pause_item.id() {
            let paused = self.pause_item.is_checked();
            self.controls.paused.store(paused, Ordering::Relaxed);
            self.header
                .set_text(if paused { "Sotto — Paused" } else { "Sotto — Ready" });
            tracing::info!(paused, "dictation pause toggled from tray");
            return TrayAction::None;
        }

        // Polish tier behaves as a radio group.
        let mode = if *id == *self.polish_off.id() {
            Some(PolishMode::Off)
        } else if *id == *self.polish_rules.id() {
            Some(PolishMode::Rules)
        } else if *id == *self.polish_ai.id() {
            Some(PolishMode::Ai)
        } else {
            None
        };
        if let Some(mode) = mode {
            self.polish_off.set_checked(mode == PolishMode::Off);
            self.polish_rules.set_checked(mode == PolishMode::Rules);
            self.polish_ai.set_checked(mode == PolishMode::Ai);
            self.controls.polish_mode.store(mode.as_u8(), Ordering::Relaxed);
            tracing::info!(?mode, "polish tier changed from tray");
        }
        TrayAction::None
    }
}

fn build_icon(mark: (u8, u8, u8)) -> anyhow::Result<Icon> {
    const SIZE: u32 = 32;
    let rgba = render_tile(SIZE, mark);
    Ok(Icon::from_rgba(rgba, SIZE, SIZE)?)
}

/// Rasterize the charcoal tile + S mark into a straight-alpha RGBA buffer. The
/// mark is drawn by stamping a soft round brush along a densely-sampled bezier —
/// cheap, dependency-free anti-aliasing that matches the overlay's mark.
fn render_tile(size: u32, mark: (u8, u8, u8)) -> Vec<u8> {
    let s = size as f32;
    let inset = 0.75;
    let (x0, y0, x1, y1) = (inset, inset, s - inset, s - inset);
    let tile_r = 0.22 * s;
    let (cx, cy) = (s / 2.0, s / 2.0);
    let scale = 0.052 * s;
    let brush_r = (0.06 * s).max(0.9); // half of the ~12%-of-box stroke width
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

/// Coverage in [0,1] of a pixel against a rounded rectangle (SDF, 1px AA edge).
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

/// The S mark (3 cubic beziers), sampled into points centered at (ox, oy).
fn s_mark_points(ox: f32, oy: f32, s: f32, n: usize) -> Vec<(f32, f32)> {
    let segs = [
        [
            (ox + 3.8 * s, oy - 4.83 * s),
            (ox + 3.8 * s, oy - 4.83 * s),
            (ox - 3.2 * s, oy - 5.75 * s),
            (ox - 3.2 * s, oy - 2.42 * s),
        ],
        [
            (ox - 3.2 * s, oy - 2.42 * s),
            (ox - 3.2 * s, oy + 0.69 * s),
            (ox + 3.91 * s, oy - 0.69 * s),
            (ox + 3.91 * s, oy + 2.53 * s),
        ],
        [
            (ox + 3.91 * s, oy + 2.53 * s),
            (ox + 3.91 * s, oy + 5.75 * s),
            (ox - 2.99 * s, oy + 4.83 * s),
            (ox - 2.99 * s, oy + 4.83 * s),
        ],
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
    use super::*;

    #[test]
    fn tile_has_transparent_corner_opaque_body_and_visible_mark() {
        const SIZE: u32 = 32;
        let mark = (79u8, 207u8, 219u8); // cyan — blue channel far from charcoal's 24
        let px = render_tile(SIZE, mark);
        let at = |x: u32, y: u32| {
            let i = ((y * SIZE + x) * 4) as usize;
            (px[i], px[i + 1], px[i + 2], px[i + 3])
        };

        // Corner is outside the rounded tile → fully transparent.
        assert_eq!(at(0, 0).3, 0, "corner should be transparent");
        // Center is well inside the tile → fully opaque.
        assert_eq!(at(SIZE / 2, SIZE / 2).3, 255, "tile body should be opaque");
        // The mark actually drew somewhere: at least one pixel is clearly the
        // accent, not the charcoal tile (blue channel well above charcoal's 24).
        let drew_mark = px.chunks(4).any(|p| p[3] == 255 && p[2] > 150);
        assert!(drew_mark, "S mark should be visible in the tile");
    }
}
