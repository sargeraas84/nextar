'use strict';
// ---------------------------------------------------------------------
// nextar icon generator — zero dependencies, pure Node.
// Produces resources/nextar.ico (16/24/32/48/64/128/256) plus a 256px PNG.
//
// Design: a CLEAN VECTOR mark. A rounded-square tile with a smooth glass
// gradient and a crisp hairline bezel carries the heavy chrome " >> "
// double chevron. Each chevron is an exact mitered vector polygon — the
// two bars' flat-cut ends meet at a true mitered point (no rounded caps,
// no seams), filled with one smooth linear gradient (steel base → bright
// tip). No texture, no noise, no glows: pure geometry, like a proper
// vector logo.
//
// `--dark` renders the deep-navy glass variant (nextar-dark.ico/.png);
// the app painter swaps palettes at runtime based on the Windows theme.
// NO text in the icon at any size — text belongs in the app.
//
// The .ico is embedded into the Windows executables at build time by
// build.rs (embed-resource). Re-run: `node scripts/generate-icon.js`.
// ---------------------------------------------------------------------

const zlib = require('zlib');
const fs = require('fs');
const path = require('path');

const ROOT = path.join(__dirname, '..');
const RES = path.join(ROOT, 'resources');
// --dark renders the deep-navy glass variant (writes nextar-dark.ico/.png,
// leaving the default light tile as nextar.ico/.png). The app painter swaps
// between the two automatically based on the Windows theme.
const DARK = process.argv.includes('--dark');

// ----------------------------- PNG encoder -----------------------------
const CRC_TABLE = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const t = Buffer.from(type, 'ascii');
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([t, data])));
  return Buffer.concat([len, t, data, crc]);
}

function encodePNG(width, height, rgba) {
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0; // filter: none
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

// ----------------------------- ICO encoder -----------------------------
function encodeICO(entries) {
  const count = entries.length;
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(count, 4);
  const dir = Buffer.alloc(16 * count);
  let offset = 6 + 16 * count;
  entries.forEach((e, i) => {
    const d = dir.slice(i * 16, i * 16 + 16);
    d[0] = e.size >= 256 ? 0 : e.size;
    d[1] = e.size >= 256 ? 0 : e.size;
    d[2] = 0;
    d[3] = 0;
    d.writeUInt16LE(1, 4); // planes
    d.writeUInt16LE(32, 6); // bpp
    d.writeUInt32LE(e.png.length, 8);
    d.writeUInt32LE(offset, 12);
    offset += e.png.length;
  });
  return Buffer.concat([header, dir, ...entries.map((e) => e.png)]);
}

// ------------------------------ Drawing --------------------------------
const clamp01 = (v) => (v < 0 ? 0 : v > 1 ? 1 : v);
const lerp = (a, b, t) => a + (b - a) * t;

function sdRoundRect(px, py, x0, y0, x1, y1, r) {
  const qx = Math.abs(px - (x0 + x1) / 2) - (x1 - x0) / 2 + r;
  const qy = Math.abs(py - (y0 + y1) / 2) - (y1 - y0) / 2 + r;
  const ox = Math.max(qx, 0);
  const oy = Math.max(qy, 0);
  return Math.hypot(ox, oy) + Math.min(Math.max(qx, qy), 0) - r;
}

function sdCircle(px, py, cx, cy, r) {
  return Math.hypot(px - cx, py - cy) - r;
}

const cover = (d) => Math.min(1, Math.max(0, 0.5 - d));

const mixc = (a, b, t) => [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)];

// ----- clean vector geometry -----

// Distance from (px,py) to the segment [a,b].
function segDist(px, py, ax, ay, bx, by) {
  const bax = bx - ax;
  const bay = by - ay;
  const len2 = bax * bax + bay * bay || 1e-12;
  const t = clamp01(((px - ax) * bax + (py - ay) * bay) / len2);
  const cx = ax + bax * t;
  const cy = ay + bay * t;
  return Math.hypot(px - cx, py - cy);
}

// Even-odd point-in-polygon (works for concave polygons).
function pointInPoly(px, py, P) {
  let inside = false;
  for (let i = 0, j = P.length - 1; i < P.length; j = i++) {
    const a = P[i];
    const b = P[j];
    if (a[1] > py !== b[1] > py && px < ((b[0] - a[0]) * (py - a[1])) / (b[1] - a[1]) + a[0]) {
      inside = !inside;
    }
  }
  return inside;
}

// Signed distance to a polygon (any simple polygon): negative inside.
// For outside points the min over edges of the point-to-segment distance is
// the exact polygon distance (the closest boundary feature is an edge or a
// vertex, and point-to-segment covers both).
function distPoly(px, py, P) {
  let dmin = Infinity;
  for (let i = 0; i < P.length; i++) {
    const a = P[i];
    const b = P[(i + 1) % P.length];
    dmin = Math.min(dmin, segDist(px, py, a[0], a[1], b[0], b[1]));
  }
  return pointInPoly(px, py, P) ? -dmin : dmin;
}

// Intersection of line (p, dir u) and line (q, dir v).
function lineInt(p, ux, uy, q, vx, vy) {
  const denom = ux * vy - uy * vx;
  if (Math.abs(denom) < 1e-9) return [p[0] + ux, p[1] + uy];
  const wx = q[0] - p[0];
  const wy = q[1] - p[1];
  const t = (wx * vy - wy * vx) / denom;
  return [p[0] + ux * t, p[1] + uy * t];
}

// One chevron as a mitered hexagon. A = top bar start, B = bottom bar
// start, T = where the centerlines meet, hw = half bar width (device px).
// The hexagon [am, o, bp, bm, i, ap]:
//   am  top bar upper (outer) edge start
//   o   tip — where the two outer edges meet (past T, mitered point)
//   bp  bottom bar lower (outer) edge start
//   bm  bottom bar upper (inner) edge start
//   i   notch — where the two inner edges meet (the ">" opening)
//   ap  top bar lower (inner) edge start
// This is exactly an SVG stroke with stroke-linejoin="miter": crisp flat
// bar ends, a true point at the tip, and no seams between the bars.
function chevronHex(a, b, t, hw) {
  const d1x = t[0] - a[0];
  const d1y = t[1] - a[1];
  const d2x = t[0] - b[0];
  const d2y = t[1] - b[1];
  const l1 = Math.hypot(d1x, d1y) || 1;
  const l2 = Math.hypot(d2x, d2y) || 1;
  const u1x = d1x / l1;
  const u1y = d1y / l1;
  const u2x = d2x / l2;
  const u2y = d2y / l2;
  const n1x = -u1y;
  const n1y = u1x; // perp of d1 (points down/right for a down-right bar)
  const n2x = -u2y;
  const n2y = u2x; // perp of d2
  const ap = [a[0] + n1x * hw, a[1] + n1y * hw]; // top bar lower (inner) edge start
  const am = [a[0] - n1x * hw, a[1] - n1y * hw]; // top bar upper (outer) edge start
  const bp = [b[0] + n2x * hw, b[1] + n2y * hw]; // bottom bar lower (outer) edge start
  const bm = [b[0] - n2x * hw, b[1] - n2y * hw]; // bottom bar upper (inner) edge start
  const o = lineInt(am, u1x, u1y, bp, u2x, u2y); // tip — the two OUTER edges meet
  const i = lineInt(ap, u1x, u1y, bm, u2x, u2y); // notch — the two INNER edges meet
  return {
    hex: [am, o, bp, bm, i, ap],
    a,
    b,
    u1x,
    u1y,
    l1,
    u2x,
    u2y,
    l2,
    n1x,
    n1y,
    n2x,
    n2y,
  };
}

// Per-pixel color of a chevron: coverage + one smooth along-bar gradient
// (steel base → bright tip). Negative distance = inside = full coverage.
// The pixel's color follows whichever bar it's closest to, so the gradient
// flows along each bar and meets seamlessly at the shared tip.
function chevronAt(px, py, ch, base, tip) {
  const d = distPoly(px, py, ch.hex);
  const dt = Math.abs((px - ch.a[0]) * ch.n1x + (py - ch.a[1]) * ch.n1y);
  const db = Math.abs((px - ch.b[0]) * ch.n2x + (py - ch.b[1]) * ch.n2y);
  let t;
  if (dt <= db) {
    t = clamp01(((px - ch.a[0]) * ch.u1x + (py - ch.a[1]) * ch.u1y) / ch.l1);
  } else {
    t = clamp01(((px - ch.b[0]) * ch.u2x + (py - ch.b[1]) * ch.u2y) / ch.l2);
  }
  return { cov: cover(d), c: mixc(base, tip, t) };
}

// Two palettes: clean frosted-glass light tile + dark gunmetal chrome, and
// the deep-navy glass variant + white-hot chrome for Windows dark mode.
// Colors match the Rust painters (nextar-gui / setup).
function palette(dark) {
  if (dark) {
    return {
      tileA: [0x0e, 0x1b, 0x38], // deep navy, lit top-left
      tileB: [0x14, 0x2b, 0x52], // mid navy
      tileC: [0x1c, 0x3a, 0x6a], // deeper blue bottom
      tileMag: [0xff, 0x2b, 0xd6], // soft pink bottom reflection
      backA: [0x1d, 0x33, 0x4c], // back chevron — dim, receding cool steel
      backB: [0x5c, 0x8f, 0xad], // back chevron tip — muted cyan steel
      frontA: [0x2c, 0x3a, 0x50], // front chevron — cool gunmetal (base)
      frontB: [0xf4, 0xf8, 0xfc], // front chevron tip — white-hot chrome
      lit: [0x8a, 0xe8, 0xff], // cyan lit-chrome edge highlight
      bezel: [0x5e, 0xf2, 0xff, 235], // neon ice-cyan ring (matches lit-chrome)
    };
  }
  return {
    tileA: [0xfb, 0xfd, 0xff], // frosted white (lit, top-left)
    tileB: [0xe0, 0xee, 0xf9], // pale cyan mid
    tileC: [0xbf, 0xd9, 0xf0], // cool blue bottom
    tileMag: [0xff, 0x9e, 0xe6], // soft pink glass reflection
    backA: [0x21, 0x36, 0x4e], // back chevron — dim, receding cool steel
    backB: [0x66, 0xa0, 0xbe], // back chevron tip — muted cyan steel
    frontA: [0x19, 0x23, 0x37], // front chevron — deep cool gunmetal (base)
    frontB: [0xd8, 0xe2, 0xec], // front chevron tip — bright cool silver
    lit: [0x9d, 0xee, 0xff], // cyan lit-chrome edge highlight
    bezel: [0x00, 0xd9, 0xff, 230], // neon cyan ring (matches lit-chrome)
  };
}

function renderIcon(size, ss, dark) {
  const SS = ss || 4;
  const S = size * SS;
  const acc = new Float32Array(S * S * 4);
  const P = size * SS; // pixel scale for 0..1 units

  const pal = palette(!!dark);
  const B = (u, v) => [u * P, v * P];
  // Mark geometry (unit coords, y down): back chevron receding left, front
  // chevron hero right. Mitered hexagons, tip and notch computed exactly.
  // The chevrons are scaled up to fill the circular tile (the circle
  // inscribed in the 0.06..0.94 content box, radius 0.44): front tip at
  // ~68% of the circle radius, bar ends tucked inside the rim.
  const backCh = chevronHex(B(0.125, 0.365), B(0.125, 0.635), B(0.345, 0.5), 0.038 * P);
  const frontCh = chevronHex(B(0.485, 0.315), B(0.485, 0.685), B(0.795, 0.5), 0.075 * P);
  const bezelW = 0.018 * P;

  const paint = (acc, i, cov, rgb, alpha) => {
    const al = (alpha === undefined ? 1 : alpha) * cov;
    if (al <= 0) return;
    const keep = 1 - al;
    acc[i] = rgb[0] * al + acc[i] * keep;
    acc[i + 1] = rgb[1] * al + acc[i + 1] * keep;
    acc[i + 2] = rgb[2] * al + acc[i + 2] * keep;
    acc[i + 3] = al + acc[i + 3] * keep;
  };

  for (let y = 0; y < S; y++) {
    for (let x = 0; x < S; x++) {
      const px = x + 0.5;
      const py = y + 0.5;
      const i = (y * S + x) * 4;
      const un = (x + 0.5) / P; // unit coords
      const vn = (y + 0.5) / P;

      // --- tile: smooth diagonal glass gradient on a perfect circle ---
      const dT = sdCircle(px, py, P * 0.5, P * 0.5, 0.44 * P);
      const tCov = cover(dT);
      if (tCov <= 0) continue;
      const gx = clamp01((un - 0.06) / 0.88);
      const gy = clamp01((vn - 0.06) / 0.88);
      let tile = mixc(pal.tileA, pal.tileB, clamp01((gx + gy) * 0.5));
      tile = mixc(tile, pal.tileC, clamp01(Math.max(gx, gy) * 0.85));
      const magGlow = Math.exp(-Math.pow((gy - 1.12) / 0.30, 2));
      tile = mixc(tile, pal.tileMag, magGlow * 0.22);
      paint(acc, i, tCov, tile, 1);

      // neon cyan ring: a thin vector ring just inside the edge
      const bezelCov = cover(dT) * (1 - cover(dT + bezelW));
      paint(acc, i, bezelCov, pal.bezel.slice(0, 3), pal.bezel[3] / 255);

      // --- mark: back chevron first, front chevron over it ---
      const bk = chevronAt(px, py, backCh, pal.backA, pal.backB);
      if (bk.cov > 0) paint(acc, i, bk.cov * tCov, bk.c, 1);
      const fr = chevronAt(px, py, frontCh, pal.frontA, pal.frontB);
      if (fr.cov > 0) paint(acc, i, fr.cov * tCov, fr.c, 1);

      // subtle cyan lit-chrome edge along the front chevron's upper bars
      // (top bar's outer edge am→o, bottom bar's inner edge bm→o): a thin
      // band just inside each edge, positive signed distance into the bar.
      const edgeW = 0.022 * P;
      const am = frontCh.hex[0];
      const bm = frontCh.hex[3];
      const sTop = frontCh.u1x * (py - am[1]) - frontCh.u1y * (px - am[0]);
      const sBot = -(frontCh.u2x * (py - bm[1]) - frontCh.u2y * (px - bm[0]));
      const band = Math.max(
        cover(-sTop) * (1 - cover(-sTop + edgeW)),
        cover(-sBot) * (1 - cover(-sBot + edgeW))
      );
      if (band > 0) paint(acc, i, fr.cov * tCov * band, pal.lit, 0.5);
    }
  }

  const out = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let sr = 0;
      let sg = 0;
      let sb = 0;
      let sa = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const idx = ((y * SS + sy) * S + (x * SS + sx)) * 4;
          sr += acc[idx];
          sg += acc[idx + 1];
          sb += acc[idx + 2];
          sa += acc[idx + 3];
        }
      }
      const n = SS * SS;
      const o = (y * size + x) * 4;
      out[o] = sa > 0 ? Math.round(sr / sa) : 0;
      out[o + 1] = sa > 0 ? Math.round(sg / sa) : 0;
      out[o + 2] = sa > 0 ? Math.round(sb / sa) : 0;
      out[o + 3] = Math.round((sa / n) * 255);
    }
  }
  return { size, rgba: out, png: encodePNG(size, size, out) };
}

// ------------------------------ Entry point ----------------------------
function main() {
  fs.mkdirSync(RES, { recursive: true });

  const sizes = [16, 24, 32, 48, 64, 128, 256];
  const rendered = sizes.map((s) => renderIcon(s, 4, DARK));

  const tag = DARK ? '-dark' : '';
  const ico = encodeICO(rendered);
  fs.writeFileSync(path.join(RES, `nextar${tag}.ico`), ico);

  const png256 = rendered.find((r) => r.size === 256);
  fs.writeFileSync(path.join(RES, `nextar${tag}.png`), png256.png);
  fs.writeFileSync(path.join(RES, `nextar${tag}-chevron.png`), png256.png);

  console.log(`[generate-icon] ${DARK ? 'dark' : 'light'} wrote resources/nextar${tag}.ico (16/24/32/48/64/128/256) + nextar${tag}.png (+ nextar${tag}-chevron.png)`);
}

if (require.main === module) {
  main();
} else {
  // Reusable by other tooling (e.g. scripts/build-icns.js):
  // renderIcon(size, supersample, dark) -> { size, rgba, png }.
  module.exports = { renderIcon, palette };
}
