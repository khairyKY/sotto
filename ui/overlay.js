const CL = {
  lilac: '#8E74D0', lilacGlow: 'rgba(142,116,208,0.40)',
  lilacDark: '#C9B8EE', lilacDarkGlow: 'rgba(201,184,238,0.40)',
  amber: '#D4A06A', amberGlow: 'rgba(212,160,106,0.35)',
  amberDark: '#E8C48E', amberDarkGlow: 'rgba(232,196,142,0.35)',
  gold: '#E8C78A', goldGlow: 'rgba(232,199,138,0.30)',
  goldDark: '#F0D9A4', goldDarkGlow: 'rgba(240,217,164,0.30)',
  blush: '#F0BFCF', blushTxt: '#A85874',
  blushDark: '#5B3F4E', blushDarkTxt: '#F0C3D2',
  muted: '#948C86', mutedDark: '#8F859A',
  txt: '#544F5A', txtDark: '#CDC5D2',
  cream: '#F0E9DF', creamShadow: 'rgba(196,183,165,0.40)',
  creamLight: 'rgba(255,254,250,0.60)',
  plum: '#2C2634', plumShadow: 'rgba(0,0,0,0.5)',
  plumLight: 'rgba(255,255,255,0.035)',
  base: '#E6DFD4', baseDark: '#241F2A',
};

const PW = 148, PH = 40, CR = PH / 2;

// ── Tucked pill (per "Sotto Pill Tucked" design doc) ────────────────────
// The always-visible pill rests small and translucent, grows on hover, dips
// on press, and morphs to the full 148x40 while a dictation runs. The doc
// expresses this as CSS transitions on a DOM node; this overlay is a canvas,
// so each animated property becomes an explicit tween below.
const GEO = {
  tucked: { w: 46, h: 16, r: 8, op: 0.62, shadow: 0.4 },
  hover: { w: 62, h: 20, r: 10, op: 1, shadow: 1 },
};
const MORPH_MS = 300; // width/height/radius
const OPACITY_MS = 200;
const PRESS_MS = 110; // scale(.95) dip before the dictation actually starts
const DOT_MS = 220;

// cubic-bezier(.32,.72,0,1) — the doc's morph curve. Newton-Raphson to invert
// x(t), then read y(t); 5 iterations is well past visual convergence.
function cubicBezier(x1, y1, x2, y2) {
  const cx = 3 * x1, bx = 3 * (x2 - x1) - cx, ax = 1 - cx - bx;
  const cy = 3 * y1, by = 3 * (y2 - y1) - cy, ay = 1 - cy - by;
  const xAt = (t) => ((ax * t + bx) * t + cx) * t;
  const yAt = (t) => ((ay * t + by) * t + cy) * t;
  const dxAt = (t) => (3 * ax * t + 2 * bx) * t + cx;
  return (x) => {
    let t = x;
    for (let i = 0; i < 5; i++) {
      const d = dxAt(t);
      if (Math.abs(d) < 1e-6) break;
      t -= (xAt(t) - x) / d;
    }
    return yAt(Math.max(0, Math.min(1, t)));
  };
}
const EASE_MORPH = cubicBezier(0.32, 0.72, 0, 1);
const EASE_LINEAR = (t) => t;

// A CSS-transition-shaped tween: retargeting mid-flight restarts from wherever
// the value currently is, so interrupting a morph never snaps.
const mkTween = (v) => ({ from: v, to: v, t0: 0, dur: 0, ease: EASE_MORPH });
function tweenValue(tw, now) {
  if (tw.dur <= 0) return tw.to;
  const p = Math.min(1, (now - tw.t0) / tw.dur);
  return tw.from + (tw.to - tw.from) * tw.ease(p);
}
function tweenTo(tw, to, now, dur, ease) {
  if (tw.to === to) return;
  tw.from = tweenValue(tw, now);
  tw.to = to;
  tw.t0 = now;
  tw.dur = dur;
  tw.ease = ease || EASE_MORPH;
}

const tw = {
  w: mkTween(GEO.tucked.w), h: mkTween(GEO.tucked.h), r: mkTween(GEO.tucked.r),
  op: mkTween(GEO.tucked.op), shadow: mkTween(GEO.tucked.shadow),
  dot: mkTween(12), dotOp: mkTween(1), press: mkTween(1),
};
let pillHover = false;
let pressUntil = 0;
// Which screen edge the window is anchored to. The canvas (280x120) is sized
// for the widest toast, so centring the much smaller pill inside it left the
// tucked pill floating ~60px off the edge it's meant to hug — and 125px in on
// left/right anchors. Anchoring the pill within the canvas, to the same edge
// as the window, is what makes "tucked" actually tucked.
let overlayPosition = 'bottom-center';
const EDGE_INSET = 8; // room for the pill's own drop shadow

/// Top-left of the pill inside the canvas, anchored to the configured edge.
/// Expanded states therefore grow *inward*, away from the screen edge, which
/// is the natural direction.
function pillOrigin(cw, ch, w, h) {
  const [v, hz] = overlayPosition.split('-'); // e.g. "bottom-center"
  const x = hz === 'left' ? EDGE_INSET
    : hz === 'right' ? cw - w - EDGE_INSET
    : (cw - w) / 2;
  const y = v === 'top' ? EDGE_INSET
    : v === 'bottom' ? ch - h - EDGE_INSET
    : (ch - h) / 2;
  return [Math.round(x), Math.round(y)];
}

const isExpandedState = (n) => n !== 'idle';

/// Target geometry for the current phase. Expanded states keep their existing
/// per-state widths (error/cancelled toasts are wider) — this only decides the
/// tucked/hover/expanded morph.
function targetGeo(name) {
  if (isExpandedState(name)) {
    return { w: pillWidthFor(name), h: PH, r: CR, op: 1, shadow: 1 };
  }
  return pillHover ? GEO.hover : GEO.tucked;
}

const canvas = document.getElementById('pill');
const ctx = canvas.getContext('2d');

const state = {
  name: 'idle',
  level: 0.0,
  since: performance.now(),
};
const inst = { env: 0.4, sparkles: [], lastSpark: 0, dots: [] };
// Hit-region for whatever button the current state draws (✕ cancel or ↻
// retry) — recomputed every frame in drawState, read by the click handler.
// null when the current state has no button (idle / done).
let activeBtn = null;

// Payload of the last "overlay-flyout" event ({heard, corrected, more}),
// rendered by the 'smart' state. See main.rs's FlyoutDto.
let flyout = null;

// Opt-in "keep the idle pill on screen, click it to start a dictation" mode
// (N1). Off by default — matches config.rs's OverlayConfig default, so a
// window that never hears otherwise keeps today's hide-when-idle behavior.
let alwaysVisible = false;

const tauriWin = window.__TAURI__?.window ? window.__TAURI__.window.getCurrentWindow() : null;
const invoke = (cmd) => { if (window.__TAURI__) window.__TAURI__.core.invoke(cmd); else console.log('[mock invoke]', cmd); };

if (window.__TAURI__) {
  window.__TAURI__.core.invoke('get_overlay_settings').then((s) => {
    alwaysVisible = !!s.alwaysVisible;
    if (s.position) overlayPosition = s.position;
  });
}

function setState(name) {
  if (name === state.name) return;
  state.name = name;
  state.since = performance.now();
  inst.sparkles = [];
  inst.dots = [];
  if (name === "idle") {
    // Rust's overlay_state only tracks DictationEvent-driven transitions;
    // this one is timed entirely in here (see the "done"/"error"/"cancelled"/
    // "nomodel" auto-dismiss branches in drawState), so tell it explicitly —
    // the hit-test poll needs to know idle just started so it can start
    // treating the pill as clickable.
    if (alwaysVisible) invoke('mark_overlay_idle');
    else if (tauriWin) tauriWin.hide();
  }
}

function rr(x, y, w, h, r) { ctx.beginPath(); ctx.roundRect(x, y, w, h, r); }
const easeOut = (t) => 1 - Math.pow(1 - t, 3);

/// `r` is the corner radius and `sc` scales the neumorphic shadow — both
/// animate during the tucked↔expanded morph, so neither can stay the constant
/// it used to be (the doc shrinks the shadow along with the pill).
function pillBase(x, y, w, h, alpha, dark, r, sc) {
  const bg = dark ? CL.plum : CL.cream;
  const sh = dark ? CL.plumShadow : CL.creamShadow;
  const lt = dark ? CL.plumLight : CL.creamLight;
  const blur = 10 * sc, off = 4 * sc;
  ctx.save();
  rr(x, y, w, h, r);
  ctx.shadowColor = sh;
  ctx.shadowBlur = blur;
  ctx.shadowOffsetX = off;
  ctx.shadowOffsetY = off;
  ctx.fillStyle = 'rgba(0,0,0,0)';
  ctx.fill();
  ctx.shadowColor = lt;
  ctx.shadowBlur = blur;
  ctx.shadowOffsetX = -off;
  ctx.shadowOffsetY = -off;
  ctx.fillStyle = 'rgba(0,0,0,0)';
  ctx.fill();
  ctx.restore();
  rr(x, y, w, h, r);
  ctx.fillStyle = bg;
  ctx.globalAlpha = alpha;
  ctx.fill();
  ctx.globalAlpha = 1;
  ctx.save();
  rr(x, y, w, h, r);
  ctx.clip();
  const hg = ctx.createLinearGradient(0, y, 0, y + h * 0.5);
  hg.addColorStop(0, `rgba(255,255,255,${0.10 * alpha})`);
  hg.addColorStop(1, 'rgba(255,255,255,0)');
  ctx.fillStyle = hg;
  ctx.fillRect(x, y, w, h * 0.5);
  ctx.restore();
  ctx.lineWidth = 1;
  ctx.strokeStyle = dark ? `rgba(255,255,255,${0.06 * alpha})` : `rgba(150,128,104,${0.12 * alpha})`;
  rr(x + 0.5, y + 0.5, w - 1, h - 1, Math.max(0, r - 0.5));
  ctx.stroke();
}

function cancelBtn(xr, yc, r, alpha, dark) {
  const bg = dark ? 'rgba(255,255,255,0.06)' : 'rgba(0,0,0,0.05)';
  const col = dark ? '#8F859A' : '#948C86';
  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.fillStyle = bg;
  ctx.beginPath();
  ctx.arc(xr, yc, r, 0, Math.PI * 2);
  ctx.fill();
  ctx.strokeStyle = col;
  ctx.lineWidth = 1.4;
  ctx.lineCap = 'round';
  const s = r * 0.45;
  ctx.beginPath();
  ctx.moveTo(xr - s, yc - s); ctx.lineTo(xr + s, yc + s);
  ctx.moveTo(xr + s, yc - s); ctx.lineTo(xr - s, yc + s);
  ctx.stroke();
  ctx.restore();
}

function retryBtn(xr, yc, r, alpha, dark) {
  const bg = dark ? 'rgba(201,184,238,0.12)' : 'rgba(110,88,168,0.10)';
  const col = dark ? '#C9B8EE' : '#6E58A8';
  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.fillStyle = bg;
  ctx.beginPath();
  ctx.arc(xr, yc, r, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = col;
  ctx.font = `600 ${r}px "Hanken Grotesk", system-ui, sans-serif`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText('\u21BB', xr, yc + 0.5);
  ctx.restore();
}

// Auto-dismiss countdown, hugging the pill's bottom edge. Takes the PILL's
// rect (not the bar's) and clips to it: the pill is a CR-radius round-rect, so
// at the bottom 3px its body is ~25px narrower than its bounding box on each
// side. Drawing the bar to the bounding box left it hanging outside the pill.
function countdownBar(x, y, w, h, pct, dark) {
  const bh = 3, by = y + h - bh;
  ctx.save();
  rr(x, y, w, h, CR);
  ctx.clip();
  ctx.fillStyle = dark ? CL.baseDark : CL.base;
  ctx.fillRect(x, by, w, bh);
  ctx.fillStyle = dark ? '#8F859A' : '#B5ADA0';
  ctx.fillRect(x, by, w * pct, bh);
  ctx.restore();
}

/// The resting indicator: a lilac orb with three slow clouds drifting inside
/// it, clipped to the orb so they read as depth rather than as separate dots.
/// `d` is the diameter, tweened 12→14 on hover (and scaled up as it dissolves
/// into the expanded pill), so every offset below is expressed relative to the
/// doc's 12px baseline.
function drawTuckedDot(cx, cy, d, alpha, dark, now) {
  if (d <= 0.5 || alpha <= 0.01) return;
  const rad = d / 2, k = d / 12;
  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.beginPath();
  ctx.arc(cx, cy, rad, 0, Math.PI * 2);
  ctx.clip();

  // Base orb: radial-gradient(circle at 38% 32%, #D6C8F0, #B7A1E4 40%, #8E74D0 80%)
  const gx = cx - rad + d * 0.38, gy = cy - rad + d * 0.32;
  const g = ctx.createRadialGradient(gx, gy, 0, gx, gy, d * 0.9);
  if (dark) {
    g.addColorStop(0, '#EDE4FB'); g.addColorStop(0.4, '#D6C8F0'); g.addColorStop(0.8, '#C9B8EE');
  } else {
    g.addColorStop(0, '#D6C8F0'); g.addColorStop(0.4, '#B7A1E4'); g.addColorStop(0.8, '#8E74D0');
  }
  ctx.fillStyle = g;
  ctx.fillRect(cx - rad, cy - rad, d, d);

  // mm-blob / mm-bounce, sampled analytically. Each cloud is a soft radial
  // fading to transparent, so they blend instead of stacking as hard circles.
  const blob = (t, per, delay) => {
    const p = (((now / 1000 - delay) / per) % 1 + 1) % 1;
    const seg = (a, b, u) => a + (b - a) * (0.5 - 0.5 * Math.cos(Math.PI * u));
    if (p < 1 / 3) { const u = p * 3; return [seg(0, 3, u), seg(0, -2, u), seg(1, 1.16, u)]; }
    if (p < 2 / 3) { const u = (p - 1 / 3) * 3; return [seg(3, -3, u), seg(-2, 1, u), seg(1.16, 0.88, u)]; }
    const u = (p - 2 / 3) * 3; return [seg(-3, 0, u), seg(1, 0, u), seg(0.88, 1, u)];
  };
  const bounce = (per, delay) => {
    const p = (((now / 1000 - delay) / per) % 1 + 1) % 1;
    return 4 * Math.cos(2 * Math.PI * p);
  };
  const cloud = (ox, oy, size, color, a) => {
    const cg = ctx.createRadialGradient(ox, oy, 0, ox, oy, size);
    cg.addColorStop(0, color.replace('ALPHA', a));
    cg.addColorStop(0.7, color.replace('ALPHA', '0'));
    ctx.fillStyle = cg;
    ctx.beginPath();
    ctx.arc(ox, oy, size, 0, Math.PI * 2);
    ctx.fill();
  };
  const left = cx - rad, top = cy - rad;
  let [bx, by, bs] = blob(now, 3.8, -1.2);
  cloud(left + (-2 + 4 + bx) * k, top + (-1 + 4 + by) * k, 4 * k * bs, 'rgba(214,200,240,ALPHA)', '0.90');
  [bx, by, bs] = blob(now, 4.6, -2.8);
  cloud(left + (d / k - 1 - 3 + bx) * k, top + (1 + 3 + by) * k, 3 * k * bs, 'rgba(255,254,250,ALPHA)', '0.85');
  const yb = bounce(3.2, -0.8);
  cloud(left + (2 + 3.5) * k, top + (d / k + 1 - 3.5 + yb) * k, 3.5 * k, 'rgba(183,161,228,ALPHA)', '0.80');

  ctx.restore();
  ctx.globalAlpha = 1;
}

function drawState(x, y, w, h, now, radius, shadowScale, baseAlpha) {
  const name = state.name;
  const sincePhase = now - state.since;
  const dark = document.documentElement.getAttribute('data-theme') === 'dark' ||
    (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches &&
     document.documentElement.getAttribute('data-theme') !== 'light');

  // baseAlpha carries the tucked pill's resting translucency (0.62 per the
  // doc); the per-state fades below multiply into it rather than replace it.
  let alpha = baseAlpha === undefined ? 1 : baseAlpha, dy = 0;
  if (sincePhase < 180) { const e = easeOut(sincePhase / 180); alpha *= e; dy = (1 - e) * 4; }
  if (name === 'done') {
    if (sincePhase > 600) { const f = Math.min(1, (sincePhase - 600) / 400); alpha *= 1 - f; dy += f * 4; }
    if (sincePhase >= 1000) { setState('idle'); return; }
  }
  if (name === 'error') {
    if (sincePhase > 5400) { const f = Math.min(1, (sincePhase - 5400) / 300); alpha *= 1 - f; dy += f * 4; }
    if (sincePhase >= 5700) { setState('idle'); return; }
  }
  if (name === 'cancelled' || name === 'nomodel') {
    if (sincePhase > 5400) { const f = Math.min(1, (sincePhase - 5400) / 300); alpha *= 1 - f; dy += f * 4; }
    if (sincePhase >= 5700) { setState('idle'); return; }
  }
  if (name === 'smart') {
    // A beat longer than 'done' (1s) so the correction is readable, but still
    // a glance, not a nag.
    if (sincePhase > 2600) { const f = Math.min(1, (sincePhase - 2600) / 400); alpha *= 1 - f; dy += f * 4; }
    if (sincePhase >= 3000) { setState('idle'); return; }
  }
  y += dy;

  const target = Math.max(0.22, Math.min(0.97, 0.22 + Math.min(1, state.level * 8) * 0.75));
  inst.env += (target - inst.env) * 0.12;
  const yc = y + h / 2;
  const padL = 14, padR = 8, btnR = 11;
  const xr = x + w - padR - btnR;
  const contentL = x + padL;
  const contentR = xr - 4;

  const accent = dark ? CL.lilacDark : CL.lilac;
  const amber = dark ? CL.amberDark : CL.amber;
  const gold = dark ? CL.goldDark : CL.gold;
  const blush = dark ? CL.blushDark : CL.blush;
  const blushTxt = dark ? CL.blushDarkTxt : CL.blushTxt;
  const muted = dark ? CL.mutedDark : CL.muted;
  const txt = dark ? CL.txtDark : CL.txt;

  pillBase(x, y, w, h, alpha, dark, radius, shadowScale);
  ctx.globalAlpha = alpha;
  // Recomputed below for whichever branch runs; 'done' has no button.
  activeBtn = null;

  if (name === 'idle') {
    // The tucked dot. No button: the only interaction here is a body click
    // (see the canvas click handler) that starts a dictation — nothing to
    // cancel yet — so it sits centred rather than offset for a button that
    // isn't drawn.
    drawTuckedDot(x + w / 2, yc, tweenValue(tw.dot, now), alpha * tweenValue(tw.dotOp, now), dark, now);
  } else if (name === 'listening') {
    const cx = (contentL + xr) / 2;
    const nBars = 5;
    const barW = 3.5;
    const gap = 4;
    const totalW = nBars * barW + (nBars - 1) * gap;
    const startX = cx - totalW / 2;
    const amplitude = Math.min(1, (state.level || 0) * 6);
    for (let i = 0; i < nBars; i++) {
      const delay = i * 0.18;
      const wave = Math.sin((now * 0.003 + delay) * Math.PI * 2);
      const mix = amplitude * 0.7 + 0.3 * (0.5 + 0.5 * wave);
      const val = 0.15 + 0.85 * mix;
      const bh = Math.max(4, val * 18);
      const bx = startX + i * (barW + gap);
      const by = yc - bh / 2;
      ctx.fillStyle = accent;
      ctx.globalAlpha = alpha * (0.5 + val * 0.5);
      rr(bx, by, barW, bh, barW / 2);
      ctx.fill();
    }
    ctx.globalAlpha = alpha;
    cancelBtn(xr, yc, btnR, alpha, dark);
    activeBtn = { x: xr, y: yc, r: btnR, action: 'cancel' };
  } else if (name === 'transcribing') {
    const cx = (contentL + xr) / 2;
    const dotSpan = 28;
    const sizes = [9, 7, 11, 8];
    const delays = [0, 0.5, 1.0, 1.5];
    const cycle = 2.3;
    for (let i = 0; i < 4; i++) {
      const p = ((now * 0.001 - delays[i]) % cycle) / cycle;
      const px = cx - dotSpan + p * dotSpan * 2;
      const size = sizes[i] * 0.5;
      const opacity = p < 0.25 ? p / 0.25 : p > 0.72 ? 1 - (p - 0.72) / 0.28 : 0.95;
      ctx.fillStyle = amber;
      ctx.globalAlpha = alpha * Math.max(0, opacity);
      ctx.shadowColor = dark ? CL.amberDarkGlow : CL.amberGlow;
      ctx.shadowBlur = 6;
      ctx.beginPath();
      ctx.arc(px, yc, size, 0, Math.PI * 2);
      ctx.fill();
      ctx.shadowBlur = 0;
    }
    ctx.globalAlpha = alpha;
    cancelBtn(xr, yc, btnR, alpha, dark);
    activeBtn = { x: xr, y: yc, r: btnR, action: 'cancel' };
  } else if (name === 'polishing') {
    const cx = (contentL + xr) / 2;
    const gld = gold;
    const bPhase = now * 0.0024;
    const blobOffs = [
      [Math.sin(bPhase) * 3, Math.cos(bPhase * 0.7) * 2],
      [Math.sin(bPhase + 2.1) * 4, Math.cos(bPhase * 0.7 + 1.4) * 3],
      [Math.sin(bPhase + 4.2) * 2, Math.cos(bPhase * 0.7 + 2.8) * 1.5],
    ];
    const blobSizes = [8, 10, 7.5];
    for (let i = 0; i < 3; i++) {
      ctx.fillStyle = gld;
      ctx.globalAlpha = alpha * 0.25;
      ctx.beginPath();
      ctx.arc(cx + blobOffs[i][0], yc + blobOffs[i][1], blobSizes[i], 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.globalAlpha = alpha * 0.5;
    const shimmer = ctx.createLinearGradient(cx - 14, yc, cx + 14, yc);
    shimmer.addColorStop(0, 'transparent');
    shimmer.addColorStop(0.5, dark ? '#FAF0D6' : '#F5E2B0');
    shimmer.addColorStop(1, 'transparent');
    const shX = ((now * 0.001 * 28) % 28) - 14;
    ctx.fillStyle = shimmer;
    ctx.beginPath();
    ctx.ellipse(cx + shX, yc, 3, 1.5, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = alpha;
    if (now - inst.lastSpark > 200) {
      inst.sparkles.push({
        x: cx + (Math.random() - 0.5) * 30,
        y: yc + (Math.random() - 0.5) * 14,
        born: now, r: 2 + Math.random() * 2,
      });
      inst.lastSpark = now;
    }
    inst.sparkles = inst.sparkles.filter(s => now - s.born < 800);
    for (const sp of inst.sparkles) {
      const age = (now - sp.born) / 800, e = Math.sin(age * Math.PI);
      ctx.globalAlpha = alpha * e;
      ctx.fillStyle = dark ? '#FAF0D6' : '#F5E2B0';
      ctx.beginPath();
      for (let j = 0; j < 4; j++) {
        const a = j * Math.PI / 2 + now * 0.001;
        const sx = sp.x + Math.cos(a) * sp.r * e;
        const sy = sp.y + Math.sin(a) * sp.r * e;
        j === 0 ? ctx.moveTo(sx, sy) : ctx.lineTo(sx, sy);
      }
      ctx.closePath();
      ctx.fill();
    }
    ctx.globalAlpha = alpha;
    cancelBtn(xr, yc, btnR, alpha, dark);
    activeBtn = { x: xr, y: yc, r: btnR, action: 'cancel' };
  } else if (name === 'done') {
    const prog = easeOut(Math.min(sincePhase, 260) / 260);
    const cx = (contentL + xr) / 2;
    const p1 = [cx - 9, yc + 1], p2 = [cx - 3, yc + 7], p3 = [cx + 11, yc - 8];
    const L1 = Math.hypot(p2[0] - p1[0], p2[1] - p1[1]), L2 = Math.hypot(p3[0] - p2[0], p3[1] - p2[1]);
    const d = prog * (L1 + L2);
    ctx.strokeStyle = accent;
    ctx.lineWidth = 2.8;
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    ctx.beginPath();
    ctx.moveTo(p1[0], p1[1]);
    if (d <= L1) {
      const t = d / L1;
      ctx.lineTo(p1[0] + (p2[0] - p1[0]) * t, p1[1] + (p2[1] - p1[1]) * t);
    } else {
      ctx.lineTo(p2[0], p2[1]);
      const t = (d - L1) / L2;
      ctx.lineTo(p2[0] + (p3[0] - p2[0]) * t, p2[1] + (p3[1] - p2[1]) * t);
    }
    ctx.stroke();
  } else if (name === 'error') {
    const gx = contentL + 8;
    const toastW = contentR - gx;
    ctx.fillStyle = blush;
    ctx.globalAlpha = alpha * 0.2;
    ctx.beginPath();
    ctx.arc(gx + 8, yc, 10, 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = alpha;
    ctx.fillStyle = blushTxt;
    ctx.font = '700 11px "Hanken Grotesk", system-ui, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText('!', gx + 8, yc);
    ctx.font = '500 12px "Hanken Grotesk", system-ui, sans-serif';
    ctx.textAlign = 'left';
    ctx.fillStyle = txt;
    ctx.fillText("Didn't catch that", gx + 18, yc);
    retryBtn(xr, yc, btnR, alpha, dark);
    activeBtn = { x: xr, y: yc, r: btnR, action: 'retry' };
    countdownBar(x, y, w, h, Math.max(0, 1 - sincePhase / 6000), dark);
  } else if (name === 'cancelled') {
    const gx = contentL + 8;
    ctx.save();
    ctx.globalAlpha = alpha;
    ctx.fillStyle = dark ? '#3A3340' : '#E6DFD4';
    ctx.beginPath();
    ctx.arc(gx + 8, yc, 10, 0, Math.PI * 2);
    ctx.fill();
    ctx.strokeStyle = muted;
    ctx.lineWidth = 1.8;
    ctx.lineCap = 'round';
    const s = 4;
    ctx.beginPath();
    ctx.moveTo(gx + 8 - s, yc - s); ctx.lineTo(gx + 8 + s, yc + s);
    ctx.moveTo(gx + 8 + s, yc - s); ctx.lineTo(gx + 8 - s, yc + s);
    ctx.stroke();
    ctx.restore();
    ctx.font = '500 12px "Hanken Grotesk", system-ui, sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = txt;
    ctx.fillText('Cancelled', gx + 20, yc);
    retryBtn(xr, yc, btnR, alpha, dark);
    activeBtn = { x: xr, y: yc, r: btnR, action: 'retry' };
    countdownBar(x, y, w, h, Math.max(0, 1 - sincePhase / 6000), dark);
  } else if (name === 'nomodel') {
    // First-run: the speech model is still downloading. Same neutral toast
    // shape as Cancelled — the take is stashed, so ↻ works once it lands.
    const gx = contentL + 8;
    ctx.save();
    ctx.globalAlpha = alpha;
    ctx.fillStyle = dark ? '#3A3340' : '#E6DFD4';
    ctx.beginPath();
    ctx.arc(gx + 8, yc, 10, 0, Math.PI * 2);
    ctx.fill();
    // Down-arrow glyph: downloading.
    ctx.strokeStyle = muted;
    ctx.lineWidth = 1.8;
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    ctx.beginPath();
    ctx.moveTo(gx + 8, yc - 5); ctx.lineTo(gx + 8, yc + 4);
    ctx.moveTo(gx + 4.5, yc + 0.5); ctx.lineTo(gx + 8, yc + 4); ctx.lineTo(gx + 11.5, yc + 0.5);
    ctx.stroke();
    ctx.restore();
    ctx.font = '500 12px "Hanken Grotesk", system-ui, sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = txt;
    ctx.fillText('Model downloading…', gx + 20, yc);
    retryBtn(xr, yc, btnR, alpha, dark);
    activeBtn = { x: xr, y: yc, r: btnR, action: 'retry' };
    countdownBar(x, y, w, h, Math.max(0, 1 - sincePhase / 6000), dark);
  } else if (name === 'smart') {
    // "Something smart just happened": a correction toward a trained word.
    // Sparkle + "heard -> corrected", no button — a glance, then it fades.
    const gx = contentL + 6, sx = gx + 8, sy = yc;
    ctx.save();
    ctx.globalAlpha = alpha;
    ctx.strokeStyle = accent;
    ctx.lineWidth = 1.6;
    ctx.lineCap = 'round';
    ctx.beginPath();
    ctx.moveTo(sx, sy - 6); ctx.lineTo(sx, sy + 6);
    ctx.moveTo(sx - 6, sy); ctx.lineTo(sx + 6, sy);
    ctx.moveTo(sx - 3.5, sy - 3.5); ctx.lineTo(sx + 3.5, sy + 3.5);
    ctx.moveTo(sx + 3.5, sy - 3.5); ctx.lineTo(sx - 3.5, sy + 3.5);
    ctx.stroke();
    ctx.restore();

    const f = flyout || { heard: '', corrected: '', more: 0 };
    const maxX = x + w - padR;
    let tx = gx + 22;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.globalAlpha = alpha;
    const seg = (s, color, weight) => {
      ctx.font = `${weight} 12px "Hanken Grotesk", system-ui, sans-serif`;
      ctx.fillStyle = color;
      let str = s;
      while (str.length > 1 && tx + ctx.measureText(str).width > maxX) str = str.slice(0, -1);
      if (str !== s) str = str.replace(/.$/, '…');
      ctx.fillText(str, tx, yc);
      tx += ctx.measureText(str).width;
    };
    seg(f.heard, muted, '500');
    seg('  →  ', muted, '500');
    seg(f.corrected, txt, '700');
    if (f.more > 0) seg('  +' + f.more, muted, '500');
  }
  ctx.globalAlpha = 1;
}

// Toasts widen to fit their label + retry button (design spec: cancelled
// 220px, error 236px); live states keep the compact 148px pill.
function pillWidthFor(name) {
  if (name === 'error') return 236;
  if (name === 'cancelled') return 220;
  if (name === 'nomodel') return 248;
  if (name === 'smart') return 264; // "heard -> corrected" flyout
  return PW;
}

function frame(now) {
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const cw = window.innerWidth, ch = window.innerHeight;
  if (canvas.width !== Math.round(cw * dpr)) canvas.width = Math.round(cw * dpr);
  if (canvas.height !== Math.round(ch * dpr)) canvas.height = Math.round(ch * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, cw, ch);
  if (state.name !== 'idle' || alwaysVisible) {
    // Retarget every frame: cheap (tweenTo early-outs when the target is
    // unchanged) and it means a state change mid-morph is picked up
    // immediately rather than waiting for the current tween to land.
    const g = targetGeo(state.name);
    tweenTo(tw.w, g.w, now, MORPH_MS);
    tweenTo(tw.h, g.h, now, MORPH_MS);
    tweenTo(tw.r, g.r, now, MORPH_MS);
    tweenTo(tw.op, g.op, now, OPACITY_MS, EASE_LINEAR);
    tweenTo(tw.shadow, g.shadow, now, MORPH_MS);
    // The dot swells and dissolves as the pill expands (doc: scale 3.4,
    // opacity 0) so it reads as becoming the pill rather than being replaced.
    const expanded = isExpandedState(state.name);
    tweenTo(tw.dot, expanded ? 12 * 3.4 : (pillHover ? 14 : 12), now, expanded ? MORPH_MS : DOT_MS);
    tweenTo(tw.dotOp, expanded ? 0 : 1, now, 180, EASE_LINEAR);
    // Press dip: a short scale(.95) that self-releases, so a click reads as
    // physical even though the state change comes back asynchronously.
    tweenTo(tw.press, now < pressUntil ? 0.95 : 1, now, PRESS_MS, EASE_LINEAR);

    const scale = tweenValue(tw.press, now);
    const w = tweenValue(tw.w, now) * scale;
    const h = tweenValue(tw.h, now) * scale;
    const r = Math.min(tweenValue(tw.r, now), h / 2);
    const [px, py] = pillOrigin(cw, ch, w, h);
    drawState(px, py, w, h, now, r, tweenValue(tw.shadow, now), tweenValue(tw.op, now));
  }
  requestAnimationFrame(frame);
}

// The overlay window is click-through except while a button is showing (see
// emit_state in main.rs, which toggles set_ignore_cursor_events per state) —
// so a click reaching here always means the window is meant to be clickable
// right now. Hit-test against the button's own circle, not just "anywhere
// on the pill", so a click on the pill body (no button under it) is a no-op.
canvas.addEventListener('click', (e) => {
  if (activeBtn) {
    const d = Math.hypot(e.offsetX - activeBtn.x, e.offsetY - activeBtn.y);
    if (d <= activeBtn.r + 3) { // +3px forgiving hit-area for a small target
      invoke(activeBtn.action === 'retry' ? 'retry_last' : 'cancel_dictation');
    }
    return;
  }
  // Idle has no button — the whole pill body starts a dictation. Only
  // reachable at all when always-visible mode makes idle clickable at the
  // Windows hit-test level (spawn_overlay_hittest in main.rs); the bounds
  // here mirror pillWidthFor('idle') so a click just past the pill's edge
  // (still inside the transparent window) is a no-op.
  if (state.name === 'idle' && alwaysVisible && overTuckedPill(e)) {
    // Dip first, then start. The visual press is local and immediate; the
    // actual 'listening' state arrives from Rust a beat later, and the morph
    // tween picks it up from wherever the dip left off.
    pressUntil = performance.now() + PRESS_MS;
    invoke('start_dictation');
  }
});

/// Hit-test against the HOVER rect (62x20), not the resting 46x16 one. The
/// pill grows the moment the cursor arrives, so the hover box is what's
/// actually under the pointer at click time — and using one stable rect for
/// both states stops the pill oscillating at the boundary as it resizes.
function overTuckedPill(e) {
  const g = GEO.hover;
  const [px, py] = pillOrigin(window.innerWidth, window.innerHeight, g.w, g.h);
  return e.offsetX >= px && e.offsetX <= px + g.w && e.offsetY >= py && e.offsetY <= py + g.h;
}

// Hover only reaches us at all while Rust has turned click-through off (the
// cursor poll in spawn_overlay_hittest), so this refines that coarse gate to
// the exact pill rect rather than the whole transparent window.
canvas.addEventListener('mousemove', (e) => {
  pillHover = alwaysVisible && !isExpandedState(state.name) && overTuckedPill(e);
});
canvas.addEventListener('mouseleave', () => { pillHover = false; });

if (document.fonts && document.fonts.load) {
  Promise.all([
    document.fonts.load('500 11px "Hanken Grotesk"'),
    document.fonts.load('500 11px "JetBrains Mono"'),
  ]).finally(() => requestAnimationFrame(frame));
} else {
  requestAnimationFrame(frame);
}

const tauri = window.__TAURI__;
if (tauri && tauri.event) {
  tauri.event.listen('overlay-state', (e) => setState(e.payload));
  tauri.event.listen('overlay-level', (e) => { state.level = e.payload; });
  tauri.event.listen('overlay-flyout', (e) => { flyout = e.payload; setState('smart'); });
  tauri.event.listen('overlay-always-visible-changed', (e) => { alwaysVisible = !!e.payload; });
  tauri.event.listen('theme-changed', (e) => {
    const theme = e.payload;
    if (theme === "system") {
      document.documentElement.removeAttribute('data-theme');
    } else {
      document.documentElement.dataset.theme = theme;
    }
  });
} else {
  const seq = [['listening', 3200], ['transcribing', 2600], ['polishing', 3200], ['done', 1400], ['error', 3400], ['idle', 700]];
  let i = 0;
  const tick = () => {
    const [name, ms] = seq[i % seq.length];
    setState(name);
    i++;
    setTimeout(tick, ms);
  };
  setInterval(() => { state.level = state.name === 'listening' ? 0.05 + 0.06 * Math.abs(Math.sin(performance.now() / 160)) : 0; }, 60);
  tick();
}