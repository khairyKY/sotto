// Sotto overlay pill — the always-on-top, click-through status pill.
//
// Ported from the design reference's own canvas logic (Sotto.dc.html) so the
// geometry, colors, waveform, motion, and flat translucency match the design
// exactly.
//
// State + live mic level arrive from the Rust side over Tauri events; with no
// Tauri present (plain browser preview) it runs a self-cycling demo.

const CL = {
  cyan: '#4FCFDB',
  amber: '#E3A857', amberDim: 'rgba(227,168,87,.30)',
  gold: '#F0C982', goldSpark: '#FCEBC6', goldDim: 'rgba(240,201,130,.30)',
  rose: '#D0959C', roseTxt: '#CBA0A5',
  txt: '#ECE8E1', muted: '#9E998E',
};

const PW = 200, PH = 56; // pill size

const canvas = document.getElementById('pill');
const ctx = canvas.getContext('2d');

// Runtime state, driven by Rust (or the demo).
const state = {
  name: 'idle',       // idle | listening | transcribing | polishing | done | error
  level: 0.0,         // live mic RMS 0..1
  since: performance.now(), // when the current state began
};
const inst = { env: 0.4, envTarget: 0.6, lastEnv: 0, sparkles: [], lastSpark: 0 };

const tauriWin = window.__TAURI__?.window ? window.__TAURI__.window.getCurrentWindow() : null;

function setState(name) {
  if (name === state.name) return;
  state.name = name;
  state.since = performance.now();
  inst.sparkles = [];
  // When the pill finishes and returns to idle, hide the OS window entirely
  // (Rust shows it again on the next dictation).
  if (name === "idle" && tauriWin) tauriWin.hide();
}

// ── geometry helpers ─────────────────────────────────────────────────────
function rr(x, y, w, h, r) {
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, r);
}
const easeOut = (t) => 1 - Math.pow(1 - t, 3);
const easeInOut = (t) => (t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2);

function pillBase(x, y, w, h, alpha) {
  // Charcoal fill @ 90% — flat translucency, exactly as the design (no shadow).
  rr(x, y, w, h, h / 2);
  ctx.fillStyle = `rgba(29,27,24,${0.9 * alpha})`;
  ctx.fill();

  // Top glass highlight (flat translucent gradient — no blur needed).
  ctx.save();
  rr(x, y, w, h, h / 2);
  ctx.clip();
  const hg = ctx.createLinearGradient(0, y, 0, y + h * 0.55);
  hg.addColorStop(0, `rgba(255,255,255,${0.08 * alpha})`);
  hg.addColorStop(1, 'rgba(255,255,255,0)');
  ctx.fillStyle = hg;
  ctx.fillRect(x, y, w, h * 0.55);
  ctx.restore();

  // 1px inner border.
  ctx.lineWidth = 1;
  ctx.strokeStyle = `rgba(255,255,255,${0.11 * alpha})`;
  rr(x + 0.5, y + 0.5, w - 1, h - 1, (h - 1) / 2);
  ctx.stroke();
}

// The S mark (drawn squiggle), centered at (ox, oy).
function sMark(ox, oy, scale, color, alpha, lw) {
  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.strokeStyle = color;
  ctx.lineWidth = lw;
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  const s = scale;
  ctx.beginPath();
  ctx.moveTo(ox + 3.8 * s, oy - 4.83 * s);
  ctx.bezierCurveTo(ox + 3.8 * s, oy - 4.83 * s, ox - 3.2 * s, oy - 5.75 * s, ox - 3.2 * s, oy - 2.42 * s);
  ctx.bezierCurveTo(ox - 3.2 * s, oy + 0.69 * s, ox + 3.91 * s, oy - 0.69 * s, ox + 3.91 * s, oy + 2.53 * s);
  ctx.bezierCurveTo(ox + 3.91 * s, oy + 5.75 * s, ox - 2.99 * s, oy + 4.83 * s, ox - 2.99 * s, oy + 4.83 * s);
  ctx.stroke();
  ctx.restore();
}
function monogram(x, y, h, color, alpha) {
  const ox = x + 17, oy = y + h / 2;
  sMark(ox, oy, 1, color, alpha, 2.1);
  return ox + 8;
}

function wave(x0, x1, yc, amp, env, phase, color, glow) {
  ctx.lineJoin = 'round';
  ctx.lineCap = 'round';
  const passes = glow ? [[5, glow], [2.4, color]] : [[2.4, color]];
  const steps = 56;
  for (const [lw, col] of passes) {
    ctx.beginPath();
    for (let s = 0; s <= steps; s++) {
      const u = s / steps, fx = x0 + (x1 - x0) * u;
      const taper = Math.sin(u * Math.PI);
      const val = Math.sin(u * 7 * Math.PI - phase * 2) * 0.58 + Math.sin(u * 2.6 * Math.PI + phase) * 0.42;
      const fy = yc + val * amp * env * taper;
      s ? ctx.lineTo(fx, fy) : ctx.moveTo(fx, fy);
    }
    ctx.strokeStyle = col;
    ctx.lineWidth = lw;
    ctx.stroke();
  }
}

function label(text, size, x, yc) {
  ctx.font = `500 ${size}px Inter, system-ui, sans-serif`;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  const w = ctx.measureText(text).width;
  ctx.fillStyle = CL.muted;
  ctx.fillText(text, x, yc);
  return w;
}

// ── per-state rendering ──────────────────────────────────────────────────
function drawState(x, y, w, h, now) {
  const name = state.name;
  const sincePhase = now - state.since;

  // enter (fade + rise) and per-state exit.
  let alpha = 1, dy = 0;
  if (sincePhase < 180) { const e = easeOut(sincePhase / 180); alpha = e; dy = (1 - e) * 4; }
  if (name === 'done') {
    if (sincePhase > 400) { const f = Math.min(1, (sincePhase - 400) / 400); alpha *= 1 - f; dy += f * 4; }
    if (sincePhase >= 800) { setState('idle'); return; }
  }
  if (name === 'error') {
    if (sincePhase > 2400) { const f = Math.min(1, (sincePhase - 2400) / 300); alpha *= 1 - f; dy += f * 4; }
    if (sincePhase >= 2700) { setState('idle'); return; }
  }
  if (name === 'cancelled') {
    if (sincePhase > 1600) { const f = Math.min(1, (sincePhase - 1600) / 300); alpha *= 1 - f; dy += f * 4; }
    if (sincePhase >= 1900) { setState('idle'); return; }
  }
  y += dy;

  // level envelope, driven by live mic level (falls back to a gentle idle wobble).
  const target = Math.max(0.22, Math.min(0.97, 0.22 + Math.min(1, state.level * 8) * 0.75));
  inst.env += (target - inst.env) * 0.12;
  const phase = now * 0.005;
  const yc = y + h / 2;
  const contentR = x + w - 18;

  pillBase(x, y, w, h, alpha);
  ctx.globalAlpha = alpha;

  if (name === 'listening') {
    const mr = monogram(x, y, h, CL.cyan, alpha);
    wave(mr + 14, contentR, yc, 12, inst.env, phase, CL.cyan, 'rgba(79,207,219,.22)');
  } else if (name === 'transcribing' || name === 'polishing') {
    const poly = name === 'polishing';
    const accent = poly ? CL.gold : CL.amber;
    const dim = poly ? CL.goldDim : CL.amberDim;
    const mr = monogram(x, y, h, accent, alpha);
    const lbl = poly ? 'Polishing' : 'Transcribing';
    ctx.font = `500 12.5px Inter, system-ui, sans-serif`;
    const lw = ctx.measureText(lbl).width, lx = contentR - lw;
    ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
    ctx.fillStyle = CL.muted; ctx.fillText(lbl, lx, yc);

    const t0 = mr + 14, t1 = lx - 14;
    ctx.lineCap = 'round'; ctx.lineWidth = 2; ctx.strokeStyle = dim;
    ctx.beginPath(); ctx.moveTo(t0, yc); ctx.lineTo(t1, yc); ctx.stroke();

    const dur = poly ? 1250 : 1150;
    const p = easeInOut((now % dur) / dur), dx = t0 + (t1 - t0) * p;
    const grad = ctx.createLinearGradient(dx - 22, 0, dx, 0);
    grad.addColorStop(0, 'rgba(0,0,0,0)'); grad.addColorStop(1, accent);
    ctx.strokeStyle = grad; ctx.lineWidth = 2.4;
    ctx.beginPath(); ctx.moveTo(Math.max(t0, dx - 22), yc); ctx.lineTo(dx, yc); ctx.stroke();
    ctx.fillStyle = accent; ctx.beginPath(); ctx.arc(dx, yc, 3, 0, Math.PI * 2); ctx.fill();

    if (poly) {
      if (now - inst.lastSpark > 210) {
        inst.sparkles.push({ x: t0 + 8 + Math.random() * (t1 - t0 - 16), y: yc + (Math.random() * 22 - 11), born: now, r: 4 + Math.random() * 2.5 });
        inst.lastSpark = now;
      }
      inst.sparkles = inst.sparkles.filter((s) => now - s.born < 760);
      for (const sp of inst.sparkles) {
        const age = (now - sp.born) / 760, e = Math.sin(age * Math.PI), sc = sp.r * e, d = sc * 0.62;
        ctx.globalAlpha = alpha * e; ctx.strokeStyle = CL.goldSpark; ctx.lineWidth = 1.4; ctx.lineCap = 'round';
        ctx.beginPath();
        ctx.moveTo(sp.x - sc, sp.y); ctx.lineTo(sp.x + sc, sp.y);
        ctx.moveTo(sp.x, sp.y - sc); ctx.lineTo(sp.x, sp.y + sc);
        ctx.moveTo(sp.x - d, sp.y - d); ctx.lineTo(sp.x + d, sp.y + d);
        ctx.moveTo(sp.x - d, sp.y + d); ctx.lineTo(sp.x + d, sp.y - d);
        ctx.stroke();
      }
      ctx.globalAlpha = alpha;
    }
  } else if (name === 'done') {
    const mr = monogram(x, y, h, CL.cyan, alpha);
    const prog = easeOut(Math.min(sincePhase, 260) / 260);
    const cx = (mr + 14 + contentR) / 2;
    const p1 = [cx - 11, yc + 1], p2 = [cx - 3, yc + 8], p3 = [cx + 13, yc - 9];
    const L1 = Math.hypot(p2[0] - p1[0], p2[1] - p1[1]), L2 = Math.hypot(p3[0] - p2[0], p3[1] - p2[1]);
    const d = prog * (L1 + L2);
    ctx.strokeStyle = `rgba(79,207,219,${alpha})`; ctx.lineWidth = 3; ctx.lineCap = 'round'; ctx.lineJoin = 'round';
    ctx.beginPath(); ctx.moveTo(p1[0], p1[1]);
    if (d <= L1) { const t = d / L1; ctx.lineTo(p1[0] + (p2[0] - p1[0]) * t, p1[1] + (p2[1] - p1[1]) * t); }
    else { ctx.lineTo(p2[0], p2[1]); const t = (d - L1) / L2; ctx.lineTo(p2[0] + (p3[0] - p2[0]) * t, p2[1] + (p3[1] - p2[1]) * t); }
    ctx.stroke();
  } else if (name === 'error') {
    const mr = monogram(x, y, h, CL.rose, alpha);
    const gx = mr + 16;
    ctx.strokeStyle = CL.rose; ctx.lineWidth = 2.4; ctx.lineCap = 'round';
    ctx.beginPath(); ctx.moveTo(gx, yc - 8); ctx.lineTo(gx, yc + 2); ctx.stroke();
    ctx.fillStyle = CL.rose; ctx.beginPath(); ctx.arc(gx, yc + 8, 1.5, 0, Math.PI * 2); ctx.fill();
    ctx.font = `500 13px Inter, system-ui, sans-serif`; ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
    ctx.fillStyle = CL.roseTxt; ctx.fillText('Didn’t catch that', gx + 14, yc);
  } else if (name === 'cancelled') {
    // Muted "cancelled" state — a small X mark next to the S monogram and a
    // "Cancelled" label. Same rose palette family as error (both are non-
    // successful outcomes) but distinct glyph so it reads at a glance.
    const mr = monogram(x, y, h, CL.rose, alpha);
    const gx = mr + 14;
    ctx.strokeStyle = CL.rose; ctx.lineWidth = 2.2; ctx.lineCap = 'round';
    ctx.beginPath();
    ctx.moveTo(gx - 5, yc - 5); ctx.lineTo(gx + 5, yc + 5);
    ctx.moveTo(gx + 5, yc - 5); ctx.lineTo(gx - 5, yc + 5);
    ctx.stroke();
    ctx.font = `500 13px Inter, system-ui, sans-serif`; ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
    ctx.fillStyle = CL.roseTxt; ctx.fillText('Cancelled', gx + 14, yc);
  }
  ctx.globalAlpha = 1;
}

// ── render loop ──────────────────────────────────────────────────────────
function frame(now) {
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const cw = window.innerWidth, ch = window.innerHeight;
  if (canvas.width !== Math.round(cw * dpr)) canvas.width = Math.round(cw * dpr);
  if (canvas.height !== Math.round(ch * dpr)) canvas.height = Math.round(ch * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, cw, ch);

  if (state.name !== 'idle') {
    const px = Math.round((cw - PW) / 2);
    const py = Math.round((ch - PH) / 2); // window is sized/positioned by the OS; center the pill in it
    drawState(px, py, PW, PH, now);
  }
  requestAnimationFrame(frame);
}

// Preload the bundled fonts so the canvas labels render in Inter from frame one,
// then start the loop.
if (document.fonts && document.fonts.load) {
  Promise.all([
    document.fonts.load('500 12.5px Inter'),
    document.fonts.load('500 13px Inter'),
  ]).finally(() => requestAnimationFrame(frame));
} else {
  requestAnimationFrame(frame);
}

// ── wiring: Tauri events, or a browser demo ──────────────────────────────
const tauri = window.__TAURI__;
if (tauri && tauri.event) {
  tauri.event.listen('overlay-state', (e) => setState(e.payload));
  tauri.event.listen('overlay-level', (e) => { state.level = e.payload; });
} else {
  // Browser preview: cycle every state so the visuals can be eyeballed.
  const seq = [['listening', 3200], ['transcribing', 2600], ['polishing', 3200], ['done', 1400], ['error', 3400], ['idle', 700]];
  let i = 0;
  const tick = () => {
    const [name, ms] = seq[i % seq.length];
    setState(name);
    i++;
    setTimeout(tick, ms);
  };
  // fake a wobbling mic level for the listening wave
  setInterval(() => { state.level = state.name === 'listening' ? 0.05 + 0.06 * Math.abs(Math.sin(performance.now() / 160)) : 0; }, 60);
  tick();
}
