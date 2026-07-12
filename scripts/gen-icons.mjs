// Generates Sotto's app icons (the cyan S mark on a charcoal tile) into ./icons.
// Pure Node (built-in zlib) — no image dependency. Run: node scripts/gen-icons.mjs
import zlib from "node:zlib";
import fs from "node:fs";
import path from "node:path";

const CHARCOAL = [29, 27, 24];
const MARK = [79, 207, 219]; // cyan #4FCFDB

// ── S-mark rasterizer (ported from src/tray.rs render_tile) ──
function rrectCov(px, py, x0, y0, x1, y1, r) {
  const hw = (x1 - x0) / 2, hh = (y1 - y0) / 2, cx = (x0 + x1) / 2, cy = (y0 + y1) / 2;
  const dx = Math.abs(px - cx) - (hw - r), dy = Math.abs(py - cy) - (hh - r);
  const outside = Math.hypot(Math.max(dx, 0), Math.max(dy, 0));
  const inside = Math.min(Math.max(dx, dy), 0);
  return Math.min(Math.max(0.5 - (outside + inside - r), 0), 1);
}
function cubic(p, t) {
  const u = 1 - t, a = u * u * u, b = 3 * u * u * t, c = 3 * u * t * t, d = t * t * t;
  return [a * p[0][0] + b * p[1][0] + c * p[2][0] + d * p[3][0], a * p[0][1] + b * p[1][1] + c * p[2][1] + d * p[3][1]];
}
function sMarkPoints(ox, oy, s, n) {
  const segs = [
    [[ox + 3.8 * s, oy - 4.83 * s], [ox + 3.8 * s, oy - 4.83 * s], [ox - 3.2 * s, oy - 5.75 * s], [ox - 3.2 * s, oy - 2.42 * s]],
    [[ox - 3.2 * s, oy - 2.42 * s], [ox - 3.2 * s, oy + 0.69 * s], [ox + 3.91 * s, oy - 0.69 * s], [ox + 3.91 * s, oy + 2.53 * s]],
    [[ox + 3.91 * s, oy + 2.53 * s], [ox + 3.91 * s, oy + 5.75 * s], [ox - 2.99 * s, oy + 4.83 * s], [ox - 2.99 * s, oy + 4.83 * s]],
  ];
  const v = [];
  for (const seg of segs) for (let i = 0; i <= n; i++) v.push(cubic(seg, i / n));
  return v;
}
function renderTile(size) {
  const s = size, inset = 0.75, x0 = inset, y0 = inset, x1 = s - inset, y1 = s - inset;
  const tileR = 0.22 * s, cx = s / 2, cy = s / 2, scale = 0.052 * s, brushR = Math.max(0.06 * s, 0.9);
  const pts = sMarkPoints(cx, cy, scale, 80);
  const out = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
    const fx = x + 0.5, fy = y + 0.5, i = (y * size + x) * 4;
    const tile = rrectCov(fx, fy, x0, y0, x1, y1, tileR);
    if (tile <= 0.001) continue;
    let m = 0;
    for (const [px, py] of pts) {
      const c = Math.min(Math.max(brushR - Math.hypot(fx - px, fy - py) + 0.5, 0), 1);
      if (c > m) m = c;
      if (m >= 1) break;
    }
    m = Math.min(m, tile);
    out[i] = Math.round(CHARCOAL[0] * (1 - m) + MARK[0] * m);
    out[i + 1] = Math.round(CHARCOAL[1] * (1 - m) + MARK[1] * m);
    out[i + 2] = Math.round(CHARCOAL[2] * (1 - m) + MARK[2] * m);
    out[i + 3] = Math.round(tile * 255);
  }
  return out;
}

// ── PNG encoder ──
const crcTable = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = crcTable[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const t = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([t, data])), 0);
  return Buffer.concat([len, t, data, crc]);
}
function encodePNG(size, rgba) {
  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0); ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; ihdr[9] = 6; // 8-bit, RGBA
  const stride = size * 4;
  const raw = Buffer.alloc((stride + 1) * size);
  for (let y = 0; y < size; y++) rgba.copy(raw, y * (stride + 1) + 1, y * stride, y * stride + stride);
  const idat = zlib.deflateSync(raw, { level: 9 });
  return Buffer.concat([sig, chunk("IHDR", ihdr), chunk("IDAT", idat), chunk("IEND", Buffer.alloc(0))]);
}
function encodeICO(png, size) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(1, 2); header.writeUInt16LE(1, 4);
  const entry = Buffer.alloc(16);
  entry[0] = size >= 256 ? 0 : size; entry[1] = size >= 256 ? 0 : size;
  entry.writeUInt16LE(1, 4); entry.writeUInt16LE(32, 6);
  entry.writeUInt32LE(png.length, 8); entry.writeUInt32LE(22, 12);
  return Buffer.concat([header, entry, png]);
}

// ── write ──
const dir = path.join(import.meta.dirname, "..", "icons");
fs.mkdirSync(dir, { recursive: true });
const png = (n) => encodePNG(n, renderTile(n));
fs.writeFileSync(path.join(dir, "32x32.png"), png(32));
fs.writeFileSync(path.join(dir, "128x128.png"), png(128));
fs.writeFileSync(path.join(dir, "128x128@2x.png"), png(256));
fs.writeFileSync(path.join(dir, "icon.png"), png(256));
fs.writeFileSync(path.join(dir, "icon.ico"), encodeICO(png(256), 256));
console.log("icons written to", dir);
