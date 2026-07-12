//! The active-state overlay: a small always-on-top, click-through pill that
//! appears near the bottom-center of the screen only while Sotto is working,
//! then fades away. Renders with egui/eframe onto a transparent, borderless
//! window so the real desktop shows through the pill's translucency (no blur).
//!
//! This module owns the process's event loop (eframe/winit), so it also hosts
//! the system-tray icon + menu — on Windows both must live on the same thread
//! that pumps window messages. `main` hands us an [`Overlay`] handle that the
//! dictation worker uses to drive the pill's state and feed it live mic level.
//!
//! Geometry, colors, and motion timings are the high-fidelity design spec
//! (see `docs/` — the `Sotto.dc.html` handoff). Everything is drawn from
//! primitives: rounded rects, translucency, bezier strokes, polylines, dots.

use eframe::egui::epaint::{CubicBezierShape, PathStroke};
use eframe::egui::{
    self, pos2, vec2, Color32, CornerRadius, Pos2, Rect, Shape, Stroke, StrokeKind,
};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ── Window / pill geometry ───────────────────────────────────────────────
const WIN_W: f32 = 200.0;
const WIN_H: f32 = 64.0; // 56px pill + 4px top/bottom margin for the enter/exit translate
const PILL_H: f32 = 56.0;
const PILL_MARGIN_TOP: f32 = 4.0;
const GAP_ABOVE_BOTTOM: f32 = 22.0;

// ── Palette (unmultiplied RGB; alpha applied per-draw) ───────────────────
const CYAN: (u8, u8, u8) = (79, 207, 219);
const AMBER: (u8, u8, u8) = (227, 168, 87);
const GOLD: (u8, u8, u8) = (240, 201, 130);
const GOLD_SPARK: (u8, u8, u8) = (252, 235, 198);
const ROSE: (u8, u8, u8) = (208, 149, 156);
const ROSE_TXT: (u8, u8, u8) = (203, 160, 165);
const MUTED: (u8, u8, u8) = (158, 153, 142);
const PILL: (u8, u8, u8) = (29, 27, 24);
const WHITE: (u8, u8, u8) = (255, 255, 255);

/// Maps mic RMS onto the waveform's level envelope. Speech RMS is small
/// (~0.02–0.15), so it needs gain to reach a lively amplitude.
// ponytail: single calibration knob — bump if the wave reads too flat/hot.
const MIC_GAIN: f32 = 8.0;

/// The five overlay states, 1:1 with Sotto's dictation state machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum OverlayState {
    Idle = 0,
    Listening = 1,
    Transcribing = 2,
    Polishing = 3,
    Done = 4,
    Error = 5,
}

impl OverlayState {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Listening,
            2 => Self::Transcribing,
            3 => Self::Polishing,
            4 => Self::Done,
            5 => Self::Error,
            _ => Self::Idle,
        }
    }
}

struct Shared {
    target: AtomicU8,
    generation: AtomicU64,
    /// Live mic RMS (f32 bits), written by the audio capture callback.
    level: Arc<AtomicU32>,
    ctx: Mutex<Option<egui::Context>>,
}

/// Cheap-to-clone handle the worker threads use to drive the overlay. Setting a
/// state wakes the UI thread so the pill appears without polling latency.
#[derive(Clone)]
pub struct Overlay {
    s: Arc<Shared>,
    controls: crate::Controls,
    open_settings: bool,
}

impl Overlay {
    pub fn new(controls: crate::Controls) -> Self {
        Self {
            s: Arc::new(Shared {
                target: AtomicU8::new(OverlayState::Idle as u8),
                generation: AtomicU64::new(0),
                level: Arc::new(AtomicU32::new(0)),
                ctx: Mutex::new(None),
            }),
            controls,
            open_settings: false,
        }
    }

    /// Dev affordance: open the settings window immediately on launch.
    pub fn open_settings_on_start(mut self) -> Self {
        self.open_settings = true;
        self
    }

    /// Request a new overlay state. Bumps a generation counter so the renderer
    /// re-triggers even on a repeat of the same state, and wakes the UI thread.
    pub fn set(&self, state: OverlayState) {
        self.s.target.store(state as u8, Ordering::SeqCst);
        self.s.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(ctx) = self.s.ctx.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }

    /// The atomic the audio callback writes live mic RMS into (f32 bits).
    pub fn level_slot(&self) -> Arc<AtomicU32> {
        self.s.level.clone()
    }

    fn attach_ctx(&self, ctx: egui::Context) {
        *self.s.ctx.lock().unwrap() = Some(ctx);
    }

    fn target(&self) -> OverlayState {
        OverlayState::from_u8(self.s.target.load(Ordering::SeqCst))
    }

    fn generation(&self) -> u64 {
        self.s.generation.load(Ordering::SeqCst)
    }

    fn level(&self) -> f32 {
        f32::from_bits(self.s.level.load(Ordering::Relaxed))
    }

    /// Runs the eframe event loop (which also hosts the tray). Blocks until the
    /// user quits. Must be called on the process's main thread.
    pub fn run(self) -> eframe::Result {
        let viewport = egui::ViewportBuilder::default()
            .with_inner_size([WIN_W, WIN_H])
            .with_min_inner_size([WIN_W, WIN_H])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_taskbar(false)
            .with_active(false);

        let options = eframe::NativeOptions {
            viewport,
            ..Default::default()
        };

        eframe::run_native(
            "Sotto",
            options,
            Box::new(move |cc| {
                self.attach_ctx(cc.egui_ctx.clone());
                Ok(Box::new(OverlayApp::new(self)))
            }),
        )
    }
}

// ── The eframe app ───────────────────────────────────────────────────────

struct OverlayApp {
    ov: Overlay,
    tray: Option<crate::tray::Tray>,
    settings: crate::settings::Settings,
    clock: Instant, // monotonic ms source for phase/dot animation
    last_gen: u64,
    disp: OverlayState, // currently displayed state
    phase_start: Instant,
    visible: bool,
    positioned: bool,
    appear_start: Instant,
    env: f32,          // smoothed level envelope
    sparkles: Vec<Sparkle>,
    last_spark_ms: f32,
    rng: u32,
    /// Per-frame render plan computed in `logic`, consumed in `ui`: (pill, alpha, now_ms).
    cur_plan: Option<(Rect, f32, f32)>,
}

struct Sparkle {
    x: f32,
    y: f32,
    born_ms: f32,
    r: f32,
}

impl OverlayApp {
    fn new(ov: Overlay) -> Self {
        let tray = match crate::tray::Tray::build(ov.controls.clone()) {
            Ok(t) => Some(t),
            Err(err) => {
                tracing::error!(?err, "failed to build tray icon");
                None
            }
        };
        let mut settings = crate::settings::Settings::new(ov.controls.clone());
        settings.open = ov.open_settings;
        let now = Instant::now();
        Self {
            ov,
            tray,
            settings,
            clock: now,
            last_gen: 0,
            disp: OverlayState::Idle,
            phase_start: now,
            visible: false,
            positioned: false,
            appear_start: now,
            env: 0.4,
            sparkles: Vec::new(),
            last_spark_ms: 0.0,
            rng: 0x1234_5678,
            cur_plan: None,
        }
    }

    fn rand(&mut self) -> f32 {
        // xorshift32 → [0,1)
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Position the window so the pill sits bottom-center, `GAP_ABOVE_BOTTOM`px
    /// above the screen edge. Returns true once the monitor size is known.
    fn reposition(&self, ctx: &egui::Context) -> bool {
        if let Some(mon) = ctx.input(|i| i.viewport().monitor_size) {
            if mon.x > 1.0 && mon.y > 1.0 {
                let x = ((mon.x - WIN_W) / 2.0).round();
                let y = (mon.y - GAP_ABOVE_BOTTOM - (PILL_H + PILL_MARGIN_TOP)).round();
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos2(x, y)));
                return true;
            }
        }
        false
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        let Some(tray) = &self.tray else { return };
        while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            match tray.handle_menu_event(&event.id) {
                crate::tray::TrayAction::Quit => {
                    tracing::info!("quit requested from tray menu");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                crate::tray::TrayAction::OpenSettings => {
                    self.settings.open = true;
                    ctx.request_repaint(); // wake so the settings window renders
                }
                crate::tray::TrayAction::None => {}
            }
        }
    }
}

impl eframe::App for OverlayApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0] // transparent — let the desktop show through
    }

    /// Non-painting side of the frame: tray, state adoption, visibility, and
    /// the animation math. eframe forbids painting here, so `ui` does that.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_tray(ctx);

        // Adopt any new state the worker requested.
        let g = self.ov.generation();
        if g != self.last_gen {
            self.last_gen = g;
            let target = self.ov.target();
            self.phase_start = Instant::now();
            if target == OverlayState::Idle {
                self.visible = false;
            } else {
                if !self.visible {
                    self.appear_start = Instant::now();
                    self.positioned = false;
                }
                self.visible = true;
            }
            self.disp = target;
            self.sparkles.clear();
            if let Some(tray) = &self.tray {
                tray.set_active(target == OverlayState::Listening);
            }
        }

        // The overlay window is always present (transparent + click-through), so
        // it paints nothing when idle rather than being hidden — that keeps the
        // event loop running the settings child viewport even when the pill is
        // down. Position it once, at the bottom-center, when the monitor is known.
        if !self.positioned {
            self.positioned = self.reposition(ctx);
        }

        if !self.visible {
            self.cur_plan = None;
            if self.settings.open {
                ctx.request_repaint(); // keep the settings window live
            } else {
                // Idle: tick slowly so tray menu events are still served.
                ctx.request_repaint_after(Duration::from_millis(250));
            }
            return;
        }

        let now_ms = self.clock.elapsed().as_secs_f32() * 1000.0;
        let since_phase = self.phase_start.elapsed().as_secs_f32() * 1000.0;
        let since_appear = self.appear_start.elapsed().as_secs_f32() * 1000.0;

        // Global pill opacity + vertical travel (enter / exit).
        let mut alpha = 1.0;
        let mut dy = 0.0;
        // Enter: opacity 0→1 + translateY 4→0, 180ms ease-out.
        if since_appear < 180.0 {
            let e = ease_out(since_appear / 180.0);
            alpha = e;
            dy += (1.0 - e) * 4.0;
        }
        // Per-state exit / auto-hide.
        let mut hide_now = false;
        match self.disp {
            OverlayState::Done => {
                // tick 260ms, hold 140ms, fade 400ms → 800ms total.
                if since_phase >= 800.0 {
                    hide_now = true;
                } else if since_phase > 400.0 {
                    let f = (since_phase - 400.0) / 400.0;
                    alpha *= 1.0 - f;
                    dy += f * 4.0;
                }
            }
            OverlayState::Error => {
                // fade-in (appear), dwell ~2400ms, fade-out 300ms → 2700ms.
                if since_phase >= 2700.0 {
                    hide_now = true;
                } else if since_phase > 2400.0 {
                    let f = (since_phase - 2400.0) / 300.0;
                    alpha *= 1.0 - f;
                    dy += f * 4.0;
                }
            }
            _ => {}
        }

        if hide_now {
            self.visible = false;
            self.cur_plan = None;
            ctx.request_repaint(); // one more frame → idle branch takes over
            return;
        }

        // Advance the level envelope toward the live mic level (spec: k=0.12/frame).
        let target_env = (0.22 + (self.ov.level() * MIC_GAIN).clamp(0.0, 1.0) * 0.75).clamp(0.22, 0.97);
        self.env += (target_env - self.env) * 0.12;

        let pill = Rect::from_min_size(pos2(0.0, PILL_MARGIN_TOP + dy), vec2(WIN_W, PILL_H));
        self.cur_plan = Some((pill, alpha, now_ms));
        ctx.request_repaint(); // animate continuously while visible
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some((pill, alpha, now_ms)) = self.cur_plan {
            let p = ui.painter().clone();
            paint_pill_base(&p, pill, alpha);
            self.paint_state(&p, pill, alpha, now_ms);
        }

        // The settings window is a separate, normal (interactive, decorated)
        // child window — rendered here as an immediate viewport so its state can
        // live in `self.settings` and borrow freely.
        if self.settings.open {
            let ctx = ui.ctx().clone();
            let settings = &mut self.settings;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("sotto_settings"),
                egui::ViewportBuilder::default()
                    .with_title("Sotto — Settings")
                    .with_inner_size([580.0, 720.0])
                    .with_min_inner_size([548.0, 480.0]),
                |ui, _class| {
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        settings.open = false;
                    }
                    settings.render(ui);
                },
            );
        }
    }
}

impl OverlayApp {
    fn paint_state(&mut self, p: &egui::Painter, pill: Rect, alpha: f32, now_ms: f32) {
        let (x, y, w, h) = (pill.left(), pill.top(), pill.width(), pill.height());
        let yc = y + h / 2.0;
        let content_r = x + w - 18.0;
        let phase = now_ms * 0.005;

        match self.disp {
            OverlayState::Listening => {
                let mr = paint_monogram(p, x, yc, CYAN, alpha);
                paint_wave(p, mr + 14.0, content_r, yc, 12.0, self.env, phase, CYAN, alpha);
            }
            OverlayState::Transcribing | OverlayState::Polishing => {
                let poly = self.disp == OverlayState::Polishing;
                let accent = if poly { GOLD } else { AMBER };
                let mr = paint_monogram(p, x, yc, accent, alpha);
                let label = if poly { "Polishing" } else { "Transcribing" };
                // Right-aligned label; galley width lets us reserve its space.
                let font = egui::FontId::proportional(12.5);
                let galley = p.layout_no_wrap(label.to_string(), font, rgba(MUTED, alpha));
                let lx = content_r - galley.size().x;
                p.galley(pos2(lx, yc - galley.size().y / 2.0), galley, rgba(MUTED, alpha));

                let t0 = mr + 14.0;
                let t1 = lx - 14.0;
                // Dim flat track.
                p.line_segment([pos2(t0, yc), pos2(t1, yc)], Stroke::new(2.0, rgba(accent, 0.30 * alpha)));
                // Traveling dot + leading trail.
                let dur = if poly { 1250.0 } else { 1150.0 };
                let prog = ease_in_out((now_ms % dur) / dur);
                let dx = t0 + (t1 - t0) * prog;
                paint_trail(p, t0.max(dx - 22.0), dx, yc, accent, alpha);
                p.circle_filled(pos2(dx, yc), 3.0, rgba(accent, alpha));

                if poly {
                    self.tick_sparkles(now_ms, t0, t1, yc);
                    for s in &self.sparkles {
                        let age = (now_ms - s.born_ms) / 760.0;
                        let env = (age * std::f32::consts::PI).sin();
                        paint_sparkle(p, s.x, s.y, s.r * env, alpha * env);
                    }
                }
            }
            OverlayState::Done => {
                let mr = paint_monogram(p, x, yc, CYAN, alpha);
                let prog = ease_out((self.phase_start.elapsed().as_secs_f32() * 1000.0 / 260.0).min(1.0));
                paint_check(p, (mr + 14.0 + content_r) / 2.0, yc, prog, alpha);
            }
            OverlayState::Error => {
                let mr = paint_monogram(p, x, yc, ROSE, alpha);
                let gx = mr + 16.0;
                // "!" glyph: stem + dot.
                p.line_segment([pos2(gx, yc - 8.0), pos2(gx, yc + 2.0)], Stroke::new(2.4, rgba(ROSE, alpha)));
                p.circle_filled(pos2(gx, yc + 8.0), 1.5, rgba(ROSE, alpha));
                let font = egui::FontId::proportional(13.0);
                let galley = p.layout_no_wrap("Didn\u{2019}t catch that".to_string(), font, rgba(ROSE_TXT, alpha));
                p.galley(pos2(gx + 14.0, yc - galley.size().y / 2.0), galley, rgba(ROSE_TXT, alpha));
            }
            OverlayState::Idle => {}
        }
    }

    fn tick_sparkles(&mut self, now_ms: f32, t0: f32, t1: f32, yc: f32) {
        if now_ms - self.last_spark_ms > 210.0 {
            let sx = t0 + 8.0 + self.rand() * (t1 - t0 - 16.0);
            let sy = yc + (self.rand() * 22.0 - 11.0);
            let r = 4.0 + self.rand() * 2.5;
            self.sparkles.push(Sparkle { x: sx, y: sy, born_ms: now_ms, r });
            self.last_spark_ms = now_ms;
        }
        self.sparkles.retain(|s| now_ms - s.born_ms < 760.0);
    }
}

// ── Painting primitives (ported from the design's canvas reference) ──────

fn rgba(rgb: (u8, u8, u8), a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(rgb.0, rgb.1, rgb.2, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn paint_pill_base(p: &egui::Painter, pill: Rect, alpha: f32) {
    let r = (pill.height() / 2.0) as u8;
    // 1. Charcoal fill @ 90%.
    p.rect_filled(pill, CornerRadius::same(r), rgba(PILL, 0.90 * alpha));
    // 2. Top highlight — flat translucency faking a glass edge (no blur), top 55%.
    let hl = Rect::from_min_max(pill.min, pos2(pill.max.x, pill.min.y + pill.height() * 0.55));
    p.rect_filled(
        hl,
        CornerRadius { nw: r, ne: r, sw: 0, se: 0 },
        rgba(WHITE, 0.06 * alpha),
    );
    // 3. 1px inner border @ 11%.
    p.rect_stroke(
        pill,
        CornerRadius::same(r),
        Stroke::new(1.0, rgba(WHITE, 0.11 * alpha)),
        StrokeKind::Inside,
    );
}

/// Draws the S mark centered at (x+17, yc). Returns the content anchor x (=+8).
fn paint_monogram(p: &egui::Painter, x: f32, yc: f32, color: (u8, u8, u8), alpha: f32) -> f32 {
    let ox = x + 17.0;
    s_mark(p, ox, yc, 1.0, color, alpha, 2.1);
    ox + 8.0
}

fn s_mark(p: &egui::Painter, ox: f32, oy: f32, s: f32, color: (u8, u8, u8), alpha: f32, lw: f32) {
    let c = rgba(color, alpha);
    let segs: [[Pos2; 4]; 3] = [
        [
            pos2(ox + 3.8 * s, oy - 4.83 * s),
            pos2(ox + 3.8 * s, oy - 4.83 * s),
            pos2(ox - 3.2 * s, oy - 5.75 * s),
            pos2(ox - 3.2 * s, oy - 2.42 * s),
        ],
        [
            pos2(ox - 3.2 * s, oy - 2.42 * s),
            pos2(ox - 3.2 * s, oy + 0.69 * s),
            pos2(ox + 3.91 * s, oy - 0.69 * s),
            pos2(ox + 3.91 * s, oy + 2.53 * s),
        ],
        [
            pos2(ox + 3.91 * s, oy + 2.53 * s),
            pos2(ox + 3.91 * s, oy + 5.75 * s),
            pos2(ox - 2.99 * s, oy + 4.83 * s),
            pos2(ox - 2.99 * s, oy + 4.83 * s),
        ],
    ];
    for seg in segs {
        p.add(CubicBezierShape::from_points_stroke(
            seg,
            false,
            Color32::TRANSPARENT,
            PathStroke::new(lw, c),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_wave(
    p: &egui::Painter,
    x0: f32,
    x1: f32,
    yc: f32,
    amp: f32,
    env: f32,
    phase: f32,
    color: (u8, u8, u8),
    alpha: f32,
) {
    const STEPS: usize = 56;
    let mut pts = Vec::with_capacity(STEPS + 1);
    for s in 0..=STEPS {
        let u = s as f32 / STEPS as f32;
        let fx = x0 + (x1 - x0) * u;
        let taper = (u * std::f32::consts::PI).sin();
        let val = (u * 7.0 * std::f32::consts::PI - phase * 2.0).sin() * 0.58
            + (u * 2.6 * std::f32::consts::PI + phase).sin() * 0.42;
        let fy = yc + val * amp * env * taper;
        pts.push(pos2(fx, fy));
    }
    // Soft bloom underlay, then the crisp line.
    p.add(Shape::line(pts.clone(), PathStroke::new(5.0, rgba(color, 0.22 * alpha))));
    p.add(Shape::line(pts, PathStroke::new(2.4, rgba(color, alpha))));
}

/// A ~22px leading gradient trail behind the traveling dot, faked with a few
/// segments of rising alpha.
fn paint_trail(p: &egui::Painter, x_from: f32, x_to: f32, yc: f32, color: (u8, u8, u8), alpha: f32) {
    const N: usize = 8;
    for i in 0..N {
        let a0 = i as f32 / N as f32;
        let a1 = (i + 1) as f32 / N as f32;
        let xa = x_from + (x_to - x_from) * a0;
        let xb = x_from + (x_to - x_from) * a1;
        p.line_segment([pos2(xa, yc), pos2(xb, yc)], Stroke::new(2.4, rgba(color, a1 * alpha)));
    }
}

fn paint_sparkle(p: &egui::Painter, x: f32, y: f32, sc: f32, alpha: f32) {
    if sc <= 0.1 {
        return;
    }
    let d = sc * 0.62;
    let stroke = Stroke::new(1.4, rgba(GOLD_SPARK, alpha));
    p.line_segment([pos2(x - sc, y), pos2(x + sc, y)], stroke);
    p.line_segment([pos2(x, y - sc), pos2(x, y + sc)], stroke);
    p.line_segment([pos2(x - d, y - d), pos2(x + d, y + d)], stroke);
    p.line_segment([pos2(x - d, y + d), pos2(x + d, y - d)], stroke);
}

fn paint_check(p: &egui::Painter, cx: f32, yc: f32, prog: f32, alpha: f32) {
    let p1 = pos2(cx - 11.0, yc + 1.0);
    let p2 = pos2(cx - 3.0, yc + 8.0);
    let p3 = pos2(cx + 13.0, yc - 9.0);
    let l1 = p1.distance(p2);
    let l2 = p2.distance(p3);
    let d = prog * (l1 + l2);
    let stroke = Stroke::new(3.0, rgba(CYAN, alpha));
    if d <= l1 {
        let t = d / l1;
        p.line_segment([p1, p1 + (p2 - p1) * t], stroke);
    } else {
        p.line_segment([p1, p2], stroke);
        let t = ((d - l1) / l2).min(1.0);
        p.line_segment([p2, p2 + (p3 - p2) * t], stroke);
    }
}

// ── Easing ───────────────────────────────────────────────────────────────
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}
fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
    }
}
