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
// null when the current state has no button (idle-invisible / done).
let activeBtn = null;

const tauriWin = window.__TAURI__?.window ? window.__TAURI__.window.getCurrentWindow() : null;
const invoke = (cmd) => { if (window.__TAURI__) window.__TAURI__.core.invoke(cmd); else console.log('[mock invoke]', cmd); };

function setState(name) {
  if (name === state.name) return;
  state.name = name;
  state.since = performance.now();
  inst.sparkles = [];
  inst.dots = [];
  if (name === "idle" && tauriWin) tauriWin.hide();
}

function rr(x, y, w, h, r) { ctx.beginPath(); ctx.roundRect(x, y, w, h, r); }
const easeOut = (t) => 1 - Math.pow(1 - t, 3);

function pillBase(x, y, w, h, alpha, dark) {
  const bg = dark ? CL.plum : CL.cream;
  const sh = dark ? CL.plumShadow : CL.creamShadow;
  const lt = dark ? CL.plumLight : CL.creamLight;
  ctx.save();
  rr(x, y, w, h, CR);
  ctx.shadowColor = sh;
  ctx.shadowBlur = 10;
  ctx.shadowOffsetX = 4;
  ctx.shadowOffsetY = 4;
  ctx.fillStyle = 'rgba(0,0,0,0)';
  ctx.fill();
  ctx.shadowColor = lt;
  ctx.shadowBlur = 10;
  ctx.shadowOffsetX = -4;
  ctx.shadowOffsetY = -4;
  ctx.fillStyle = 'rgba(0,0,0,0)';
  ctx.fill();
  ctx.restore();
  rr(x, y, w, h, CR);
  ctx.fillStyle = bg;
  ctx.globalAlpha = alpha;
  ctx.fill();
  ctx.globalAlpha = 1;
  ctx.save();
  rr(x, y, w, h, CR);
  ctx.clip();
  const hg = ctx.createLinearGradient(0, y, 0, y + h * 0.5);
  hg.addColorStop(0, `rgba(255,255,255,${0.10 * alpha})`);
  hg.addColorStop(1, 'rgba(255,255,255,0)');
  ctx.fillStyle = hg;
  ctx.fillRect(x, y, w, h * 0.5);
  ctx.restore();
  ctx.lineWidth = 1;
  ctx.strokeStyle = dark ? `rgba(255,255,255,${0.06 * alpha})` : `rgba(150,128,104,${0.12 * alpha})`;
  rr(x + 0.5, y + 0.5, w - 1, h - 1, CR - 0.5);
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

function countdownBar(x, y, w, h, pct, dark) {
  const bg = dark ? CL.baseDark : CL.base;
  const fg = dark ? '#8F859A' : '#B5ADA0';
  ctx.save();
  rr(x, y, w, h, h / 2);
  ctx.fillStyle = bg;
  ctx.fill();
  rr(x, y, w * pct, h, h / 2);
  ctx.fillStyle = fg;
  ctx.fill();
  ctx.restore();
}

function drawState(x, y, w, h, now) {
  const name = state.name;
  const sincePhase = now - state.since;
  const dark = document.documentElement.getAttribute('data-theme') === 'dark' ||
    (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches &&
     document.documentElement.getAttribute('data-theme') !== 'light');

  let alpha = 1, dy = 0;
  if (sincePhase < 180) { const e = easeOut(sincePhase / 180); alpha = e; dy = (1 - e) * 4; }
  if (name === 'done') {
    if (sincePhase > 600) { const f = Math.min(1, (sincePhase - 600) / 400); alpha *= 1 - f; dy += f * 4; }
    if (sincePhase >= 1000) { setState('idle'); return; }
  }
  if (name === 'error') {
    if (sincePhase > 5400) { const f = Math.min(1, (sincePhase - 5400) / 300); alpha *= 1 - f; dy += f * 4; }
    if (sincePhase >= 5700) { setState('idle'); return; }
  }
  if (name === 'cancelled') {
    if (sincePhase > 5400) { const f = Math.min(1, (sincePhase - 5400) / 300); alpha *= 1 - f; dy += f * 4; }
    if (sincePhase >= 5700) { setState('idle'); return; }
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

  pillBase(x, y, w, h, alpha, dark);
  ctx.globalAlpha = alpha;
  // Recomputed below for whichever branch runs; 'done' has no button.
  activeBtn = null;

  if (name === 'idle') {
    const cx = (contentL + xr) / 2;
    const breathe = 0.5 + 0.5 * Math.sin(now * 0.0015);
    const rd = 11 + breathe * 2;
    const grad = ctx.createRadialGradient(cx - 2, yc - 2, 1, cx, yc, rd);
    if (dark) {
      grad.addColorStop(0, 'rgba(230,218,247,0.9)');
      grad.addColorStop(1, 'rgba(201,184,238,0.3)');
    } else {
      grad.addColorStop(0, 'rgba(183,161,228,0.9)');
      grad.addColorStop(1, 'rgba(142,116,208,0.4)');
    }
    ctx.fillStyle = grad;
    ctx.beginPath();
    ctx.arc(cx, yc, rd, 0, Math.PI * 2);
    ctx.fill();
    cancelBtn(xr, yc, btnR, alpha, dark);
    activeBtn = { x: xr, y: yc, r: btnR, action: 'cancel' };
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
    const pct = Math.max(0, 1 - sincePhase / 6000);
    const barY = y + h - 3;
    countdownBar(x + 2, barY, w - 4, 3, pct, dark);
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
    const pct = Math.max(0, 1 - sincePhase / 6000);
    const barY = y + h - 3;
    countdownBar(x + 2, barY, w - 4, 3, pct, dark);
  }
  ctx.globalAlpha = 1;
}

function frame(now) {
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const cw = window.innerWidth, ch = window.innerHeight;
  if (canvas.width !== Math.round(cw * dpr)) canvas.width = Math.round(cw * dpr);
  if (canvas.height !== Math.round(ch * dpr)) canvas.height = Math.round(ch * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, cw, ch);
  if (state.name !== 'idle') {
    const px = Math.round((cw - PW) / 2);
    const py = Math.round((ch - PH) / 2);
    drawState(px, py, PW, PH, now);
  }
  requestAnimationFrame(frame);
}

// The overlay window is click-through except while a button is showing (see
// emit_state in main.rs, which toggles set_ignore_cursor_events per state) —
// so a click reaching here always means the window is meant to be clickable
// right now. Hit-test against the button's own circle, not just "anywhere
// on the pill", so a click on the pill body (no button under it) is a no-op.
canvas.addEventListener('click', (e) => {
  if (!activeBtn) return;
  const d = Math.hypot(e.offsetX - activeBtn.x, e.offsetY - activeBtn.y);
  if (d <= activeBtn.r + 3) { // +3px forgiving hit-area for a small target
    invoke(activeBtn.action === 'retry' ? 'retry_last' : 'cancel_dictation');
  }
});

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