'use strict';
// ---------------------------------------------------------------------
// nextar icon generator — zero dependencies, pure Node.
// Produces resources/nextar.ico (16/24/32/48/64/128/256) plus a 256px PNG.
//
// The mark is a RASTER logo: resources/logo-source.png is the single
// source of truth (a wide isometric ribbon with an upward arrow on a
// black tile). It is decoded in pure Node, padded to a square tile, and
// downscaled with an area-average box filter into every icon size. The
// previous procedural "convergence core" painter remains below as a
// fallback when the source PNG is absent, so the pipeline never breaks.
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

// ------------------------- raster logo source ---------------------------
// resources/logo-source.png is the canonical logo. When present it drives
// every asset below; the procedural painter below is only a fallback.
const SRC = path.join(RES, 'logo-source.png');

let _srcCache = null; // { size, rgba } square master once decoded

// Minimal PNG decoder: non-interlaced, 8-bit, color types 0/2/3/4/6.
function decodePNG(buf) {
  if (buf.readUInt32BE(0) !== 0x89504e47) throw new Error('not a PNG');
  let w = 0;
  let h = 0;
  let depth = 0;
  let color = 0;
  const idat = [];
  let off = 8;
  while (off + 8 <= buf.length) {
    const len = buf.readUInt32BE(off);
    const type = buf.toString('ascii', off + 4, off + 8);
    const data = buf.subarray(off + 8, off + 8 + len);
    if (type === 'IHDR') {
      w = data.readUInt32BE(0);
      h = data.readUInt32BE(4);
      depth = data[8];
      color = data[9];
      if (depth !== 8) throw new Error(`unsupported bit depth ${depth}`);
      if (color === 3 && data[12] !== 0) throw new Error('interlaced palette unsupported');
      if (data[12] !== 0) throw new Error('interlaced PNG unsupported');
    } else if (type === 'IDAT') {
      idat.push(data);
    } else if (type === 'IEND') {
      break;
    }
    off += 12 + len;
  }
  if (!w || !h) throw new Error('missing IHDR');
  const ch = color === 0 ? 1 : color === 2 ? 3 : color === 4 ? 2 : color === 6 ? 4 : 0;
  if (!ch) throw new Error(`unsupported color type ${color}`);
  let raw = zlib.inflateSync(Buffer.concat(idat));
  // palette for color type 3
  let pal = null;
  off = 8;
  while (off + 8 <= buf.length) {
    const len = buf.readUInt32BE(off);
    const type = buf.toString('ascii', off + 4, off + 8);
    const data = buf.subarray(off + 8, off + 8 + len);
    if (type === 'PLTE') pal = data;
    off += 12 + len;
  }
  const bpp = ch;
  const stride = w * bpp;
  const out = Buffer.alloc(w * h * 4);
  let rp = 0;
  let prev = Buffer.alloc(stride);
  for (let y = 0; y < h; y++) {
    const filter = raw[rp++];
    const row = Buffer.alloc(stride);
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? row[x - bpp] : 0;
      const b = prev[x];
      const c = x >= bpp ? prev[x - bpp] : 0;
      let v = raw[rp + x];
      if (filter === 1) v = (v + a) & 0xff;
      else if (filter === 2) v = (v + b) & 0xff;
      else if (filter === 3) v = (v + ((a + b) >> 1)) & 0xff;
      else if (filter === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a);
        const pb = Math.abs(p - b);
        const pc = Math.abs(p - c);
        v = (v + (pa <= pb && pa <= pc ? a : pb <= pc ? b : c)) & 0xff;
      }
      row[x] = v;
    }
    rp += stride;
    for (let x = 0; x < w; x++) {
      const s = x * bpp;
      const d = (y * w + x) * 4;
      if (color === 0) {
        out[d] = out[d + 1] = out[d + 2] = row[s];
        out[d + 3] = 255;
      } else if (color === 2) {
        out[d] = row[s]; out[d + 1] = row[s + 1]; out[d + 2] = row[s + 2]; out[d + 3] = 255;
      } else if (color === 4) {
        out[d] = out[d + 1] = out[d + 2] = row[s];
        out[d + 3] = row[s + 1];
      } else if (color === 6) {
        out[d] = row[s]; out[d + 1] = row[s + 1]; out[d + 2] = row[s + 2]; out[d + 3] = row[s + 3];
      } else {
        const pi = row[s] * 3;
        out[d] = pal[pi]; out[d + 1] = pal[pi + 1]; out[d + 2] = pal[pi + 2]; out[d + 3] = 255;
      }
    }
    prev = row;
  }
  return { width: w, height: h, rgba: out };
}

// Pad a (possibly non-square) RGBA image onto a square black canvas.
// The logo's own background is black, so the padding blends seamlessly.
function padSquare(img, size, scale) {
  const s = scale || 0.86;
  const dw = Math.round(size * s);
  const dh = Math.round(dw * (img.height / img.width));
  const dx = Math.round((size - dw) / 2);
  const dy = Math.round((size - dh) / 2);
  const out = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      out[(y * size + x) * 4 + 3] = 255;
    }
  }
  const sy = (img.height / dh) || 1;
  const sx = (img.width / dw) || 1;
  for (let y = 0; y < dh; y++) {
    const iy = Math.min(img.height - 1, Math.floor(y * sy));
    for (let x = 0; x < dw; x++) {
      const ix = Math.min(img.width - 1, Math.floor(x * sx));
      const s = (iy * img.width + ix) * 4;
      const d = ((dy + y) * size + dx + x) * 4;
      const a = img.rgba[s + 3] / 255;
      out[d] = Math.round(img.rgba[s] * a);
      out[d + 1] = Math.round(img.rgba[s + 1] * a);
      out[d + 2] = Math.round(img.rgba[s + 2] * a);
      out[d + 3] = 255;
    }
  }
  return { size, rgba: out };
}

// Area-average (box) downscale: the standard quality choice for shrinking
// a logo to icon sizes. `img` is { size, rgba } of a square master.
function downscaleSquare(img, size) {
  const src = img.size;
  if (size === src) return { size, rgba: Buffer.from(img.rgba) };
  const out = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) {
    const y0 = Math.floor((y * src) / size);
    const y1 = Math.max(y0, Math.ceil(((y + 1) * src) / size) - 1);
    for (let x = 0; x < size; x++) {
      const x0 = Math.floor((x * src) / size);
      const x1 = Math.max(x0, Math.ceil(((x + 1) * src) / size) - 1);
      let r = 0, g = 0, b = 0, a = 0, n = 0;
      for (let yy = y0; yy <= y1; yy++) {
        for (let xx = x0; xx <= x1; xx++) {
          const i = (yy * src + xx) * 4;
          const al = img.rgba[i + 3];
          r += img.rgba[i] * al;
          g += img.rgba[i + 1] * al;
          b += img.rgba[i + 2] * al;
          a += al;
          n++;
        }
      }
      const d = (y * size + x) * 4;
      if (a > 0) {
        out[d] = Math.round(r / a);
        out[d + 1] = Math.round(g / a);
        out[d + 2] = Math.round(b / a);
        out[d + 3] = Math.round(a / n);
      } else {
        out[d] = out[d + 1] = out[d + 2] = 0;
        out[d + 3] = 0;
      }
    }
  }
  return { size, rgba: out };
}

// Decode the source logo once and cache the square master.
function rasterMaster() {
  if (_srcCache) return _srcCache;
  if (!fs.existsSync(SRC)) return null;
  const img = decodePNG(fs.readFileSync(SRC));
  _srcCache = padSquare(img, 1024);
  return _srcCache;
}

// Render the raster logo at `size` (square). Returns null when the source
// is missing so callers can fall back to the procedural painter.
function renderRaster(size) {
  const master = rasterMaster();
  if (!master) return null;
  return downscaleSquare(master, size);
}

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



// Two palettes: clean frosted-glass light tile + deep-navy glass for
// Windows dark mode. The converging planes run violet → indigo → cyan in
// both, tuned for contrast against each tile. Colors match the Rust
// painters (nextar-gui / setup).
function palette(dark) {
  if (dark) {
    return {
      tileA: [0x0e, 0x1b, 0x38], // deep navy, lit top-left
      tileB: [0x14, 0x2b, 0x52], // mid navy
      tileC: [0x1c, 0x3a, 0x6a], // deeper blue bottom
      tileMag: [0xff, 0x2b, 0xd6], // soft pink bottom reflection
      layerA: [0x8b, 0x5c, 0xf6], // outer plane — electric violet
      layerB: [0x5a, 0x7c, 0xf8], // mid plane — neon indigo
      layerC: [0x37, 0xe6, 0xff], // inner plane — electric cyan
      core: [0x5e, 0xf2, 0xff], // core node — ice cyan
      glow: [0x5e, 0xf2, 0xff, 150], // cyan core glow (with alpha)
      bezel: [0x5e, 0xf2, 0xff, 235], // neon ice-cyan ring
    };
  }
  return {
    tileA: [0xfb, 0xfd, 0xff], // frosted white (lit, top-left)
    tileB: [0xe0, 0xee, 0xf9], // pale cyan mid
    tileC: [0xbf, 0xd9, 0xf0], // cool blue bottom
    tileMag: [0xff, 0x9e, 0xe6], // soft pink glass reflection
    layerA: [0x6b, 0x33, 0xb8], // outer plane — deep violet (contrast on white)
    layerB: [0x2f, 0x5f, 0xc8], // mid plane — indigo
    layerC: [0x00, 0x8f, 0xc7], // inner plane — cyan
    core: [0x00, 0xa8, 0xdd], // core node — cyan
    glow: [0x00, 0xb3, 0xe6, 120], // cyan core glow (with alpha)
    bezel: [0x00, 0xd9, 0xff, 230], // neon cyan ring
  };
}

function renderIcon(size, ss, dark) {
  // When the raster logo source exists it is the single source of truth;
  // the procedural painter below is the fallback. The `dark` palette is
  // ignored for the raster (the logo is its own black tile on both themes).
  const raster = renderRaster(size);
  if (raster) {
    return { size, rgba: raster.rgba, png: encodePNG(size, size, raster.rgba) };
  }
  const SS = ss || 4;
  const S = size * SS;
  const acc = new Float32Array(S * S * 4);
  const P = size * SS; // pixel scale for 0..1 units

  const pal = palette(!!dark);
  // Convergence-core mark geometry (unit coords, y down). Three nested
  // chevron planes fold inward — outer violet → inner cyan — and feed a
  // bright core node: files/streams compressed into one intelligent point.
  const ux = (v) => (0.06 + 0.88 * v) * P;
  const uy = (t) => (0.06 + 0.88 * t) * P;
  const planes = [
    { reach: 0.300, top: 0.240, apex: 0.520, hw: 0.030, col: pal.layerA },
    { reach: 0.215, top: 0.360, apex: 0.610, hw: 0.028, col: pal.layerB },
    { reach: 0.130, top: 0.470, apex: 0.680, hw: 0.026, col: pal.layerC },
  ];
  const core = { cx: ux(0.5), cy: uy(0.735), r: 0.050 * 0.88 * P, gr: 0.085 * 0.88 * P };
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

      // --- mark: three converging chevron planes (violet → cyan), each a
      //      rounded V-stroke; the fold narrows and brightens toward the core ---
      for (const pl of planes) {
        const lx = ux(0.5 - pl.reach);
        const ly = uy(pl.top);
        const ax = ux(0.5);
        const ay = uy(pl.apex);
        const rx = ux(0.5 + pl.reach);
        const ry = uy(pl.top);
        const d = Math.min(segDist(px, py, lx, ly, ax, ay), segDist(px, py, ax, ay, rx, ry)) - pl.hw * 0.88 * P;
        const cov = cover(d);
        if (cov > 0) paint(acc, i, cov * tCov, pl.col, 1);
      }

      // --- mark: core node — soft cyan glow, bright dot, white inner spark ---
      const dCore = sdCircle(px, py, core.cx, core.cy, core.r);
      const glow = Math.exp(-Math.max(0, dCore) / (core.gr * 0.35));
      if (glow > 0.01) {
        paint(acc, i, glow * tCov, pal.glow.slice(0, 3), (pal.glow[3] / 255) * 0.6);
      }
      const cCov = cover(dCore);
      if (cCov > 0) {
        paint(acc, i, cCov * tCov, pal.core, 1);
        const dSpark = Math.hypot(px - core.cx, py - (core.cy - core.r * 0.25)) - core.r * 0.38;
        const sCov = cover(dSpark);
        if (sCov > 0) paint(acc, i, sCov * cCov * tCov, [0xff, 0xff, 0xff], 0.9);
      }
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

  // The square master the Rust painters embed and draw at runtime
  // (512 px is crisp at every in-app size and keeps the binary lean).
  // renderIcon prefers the raster source and always returns a .png.
  const master = renderIcon(512, 1, DARK);
  fs.writeFileSync(path.join(RES, 'logo-master.png'), master.png);

  console.log(`[generate-icon] ${DARK ? 'dark' : 'light'} wrote resources/nextar${tag}.ico (16/24/32/48/64/128/256) + nextar${tag}.png (+ nextar${tag}-chevron.png, logo-master.png)`);
}

if (require.main === module) {
  main();
} else {
  // Reusable by other tooling (e.g. scripts/build-icns.js and
  // scripts/build-site-assets.js): renderIcon(size, supersample, dark) ->
  // { size, rgba, png }, plus the raw PNG encoder for compositing.
  module.exports = { renderIcon, palette, encodePNG };
}
