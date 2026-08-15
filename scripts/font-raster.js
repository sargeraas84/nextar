'use strict';
// ---------------------------------------------------------------------
// Minimal TrueType rasterizer — zero dependencies, enough to render the
// "NEXTAR" wordmark from the bundled Space Grotesk font onto marketing
// assets (og:image). Parses sfnt + cmap(format 4) + head + hhea/hmtx +
// loca + glyf (simple glyphs with quadratic-Bezier outlines), flattens
// the contours and scanline-fills them (even-odd) at 3x supersampling.
// ---------------------------------------------------------------------

function parseFont(buf) {
  const numTables = buf.readUInt16BE(4);
  const tables = {};
  for (let i = 0; i < numTables; i++) {
    const o = 12 + i * 16;
    tables[buf.toString('ascii', o, o + 4)] = {
      off: buf.readUInt32BE(o + 8),
      len: buf.readUInt32BE(o + 12),
    };
  }
  const u16 = (o) => buf.readUInt16BE(o);
  const s16 = (o) => buf.readInt16BE(o);

  const head = tables.head;
  const unitsPerEm = u16(head.off + 18);
  const indexToLocFormat = s16(head.off + 50);

  const hhea = tables.hhea;
  const ascender = s16(hhea.off + 4);
  const descender = s16(hhea.off + 6);
  const numberOfHMetrics = u16(hhea.off + 34);

  // cmap: use the first format-4 (BMP) subtable — "NEXTAR" is all < 0x80.
  const cmap = tables.cmap.off;
  const numSub = u16(cmap + 2);
  let sub = null;
  for (let s = 0; s < numSub && !sub; s++) {
    const so = cmap + 4 + s * 8;
    const off = buf.readUInt32BE(so + 4);
    if (u16(cmap + off) === 4) sub = cmap + off;
  }
  const segCount = u16(sub + 6) / 2;
  const endCodes = sub + 14;
  const startCodes = endCodes + segCount * 2 + 2;
  const idDelta = startCodes + segCount * 2;
  const idRangeOffset = idDelta + segCount * 2;

  function gidFor(cp) {
    for (let i = 0; i < segCount; i++) {
      const start = u16(startCodes + i * 2);
      const end = u16(endCodes + i * 2);
      if (cp >= start && cp <= end) {
        const ro = u16(idRangeOffset + i * 2);
        if (ro === 0) return (cp + u16(idDelta + i * 2)) & 0xffff;
        const g = u16(idRangeOffset + i * 2 + ro + (cp - start) * 2);
        return g === 0 ? 0 : (g + u16(idDelta + i * 2)) & 0xffff;
      }
    }
    return 0;
  }

  const loca = tables.loca.off;
  const glyf = tables.glyf.off;
  function glyphOffset(g) {
    return indexToLocFormat === 0
      ? glyf + u16(loca + g * 2) * 2
      : glyf + buf.readUInt32BE(loca + g * 4);
  }

  const hmtx = tables.hmtx.off;
  const lastAdvance = u16(hmtx + (numberOfHMetrics - 1) * 4);
  function metrics(g) {
    if (g < numberOfHMetrics) {
      return { aw: u16(hmtx + g * 4), lsb: s16(hmtx + g * 4 + 2) };
    }
    return { aw: lastAdvance, lsb: s16(hmtx + numberOfHMetrics * 4 + (g - numberOfHMetrics) * 2) };
  }

  // Decode a simple glyph into contours of {x,y,on} points (font units, y-up).
  function glyphPoints(g) {
    const go = glyphOffset(g);
    const nc = s16(go);
    if (nc < 0) return null; // composite — never hit by the wordmark
    const endPts = [];
    for (let i = 0; i < nc; i++) endPts.push(u16(go + 10 + i * 2));
    const nPts = endPts[nc - 1] + 1;
    let p = go + 10 + nc * 2;
    const instrLen = u16(p);
    p += 2 + instrLen;
    const flags = new Uint8Array(nPts);
    for (let i = 0; i < nPts; i++) {
      const f = buf[p++];
      flags[i] = f;
      if (f & 0x08) {
        const rep = buf[p++];
        for (let r = 1; r <= rep && i + r < nPts; r++) flags[i + r] = f;
        i += rep;
      }
    }
    const xs = new Int16Array(nPts);
    let x = 0;
    for (let i = 0; i < nPts; i++) {
      const f = flags[i];
      if (f & 0x02) {
        const v = buf[p++];
        x += f & 0x10 ? v : -v;
      } else if (!(f & 0x10)) {
        x += buf.readInt16BE(p);
        p += 2;
      }
      xs[i] = x;
    }
    const ys = new Int16Array(nPts);
    let y = 0;
    for (let i = 0; i < nPts; i++) {
      const f = flags[i];
      if (f & 0x04) {
        const v = buf[p++];
        y += f & 0x20 ? v : -v;
      } else if (!(f & 0x20)) {
        y += buf.readInt16BE(p);
        p += 2;
      }
      ys[i] = y;
    }
    const contours = [];
    let start = 0;
    for (const e of endPts) {
      const pts = [];
      for (let i = start; i <= e; i++) pts.push({ x: xs[i], y: ys[i], on: !!(flags[i] & 0x01) });
      contours.push(pts);
      start = e + 1;
    }
    return contours;
  }

  return { unitsPerEm, ascender, descender, gidFor, metrics, glyphPoints };
}

const mid = (a, b) => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });

function pushQuad(segs, p0, c, p1, n) {
  let lx = p0.x;
  let ly = p0.y;
  for (let s = 1; s <= n; s++) {
    const t = s / n;
    const mt = 1 - t;
    const x = mt * mt * p0.x + 2 * mt * t * c.x + t * t * p1.x;
    const y = mt * mt * p0.y + 2 * mt * t * c.y + t * t * p1.y;
    segs.push(lx, ly, x, y);
    lx = x;
    ly = y;
  }
}

function flattenContour(pts, n) {
  const segs = [];
  const len = pts.length;
  if (len < 2) return segs;
  let i0 = -1;
  for (let i = 0; i < len; i++) if (pts[i].on) {
    i0 = i;
    break;
  }
  let cur = i0 === -1 ? mid(pts[len - 1], pts[0]) : pts[i0];
  const startIdx = i0 === -1 ? 0 : i0;
  let i = startIdx;
  for (let guard = 0; guard <= len * 2 + 4; guard++) {
    const p1 = pts[(i + 1) % len];
    if (p1.on) {
      segs.push(cur.x, cur.y, p1.x, p1.y);
      cur = p1;
      i = (i + 1) % len;
    } else {
      const p2 = pts[(i + 2) % len];
      if (p2.on) {
        pushQuad(segs, cur, p1, p2, n);
        cur = p2;
        i = (i + 2) % len;
      } else {
        const implied = mid(p1, p2);
        pushQuad(segs, cur, p1, implied, n);
        cur = implied;
        i = (i + 1) % len;
      }
    }
    if (i === startIdx) break;
  }
  return segs;
}

// Render `text` from the font at `px` cap height-ish size. `colorFor(i, n)`
// returns [r,g,b] for the i-th glyph of n. Returns straight-alpha RGBA plus
// the natural baseline offset (so callers can align the text).
function rasterizeText(buf, text, px, colorFor) {
  const font = parseFont(buf);
  const SS = 3;
  const scale = (px * SS) / font.unitsPerEm;
  const subdivisions = Math.max(4, Math.round(px / 14));
  const trackingSS = px * SS * 0.06;

  const glyphs = [];
  for (const ch of text) {
    const g = font.gidFor(ch.codePointAt(0));
    glyphs.push({ ch, g, m: font.metrics(g) });
  }

  const baselineY = font.ascender * scale;
  const segsByGlyph = [];
  let penX = 0;
  for (const { g, m } of glyphs) {
    const segs = [];
    const contours = font.glyphPoints(g);
    if (contours) {
      for (const c of contours) {
        const f = flattenContour(c, subdivisions);
        for (let k = 0; k < f.length; k += 2) {
          segs.push(penX + f[k] * scale, baselineY - f[k + 1] * scale);
        }
      }
    }
    segsByGlyph.push(segs);
    penX += m.aw * scale + trackingSS;
  }

  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const segs of segsByGlyph) {
    for (let k = 0; k < segs.length; k += 2) {
      minX = Math.min(minX, segs[k]);
      maxX = Math.max(maxX, segs[k]);
      minY = Math.min(minY, segs[k + 1]);
      maxY = Math.max(maxY, segs[k + 1]);
    }
  }
  if (!isFinite(minX)) {
    minX = 0;
    minY = 0;
    maxX = px * SS;
    maxY = px * SS;
  }
  const pad = Math.ceil(px * SS * 0.06);
  const ox = -Math.floor(minX) + pad;
  const oy = -Math.floor(minY) + pad;
  const W = Math.ceil(maxX - minX) + pad * 2;
  const H = Math.ceil(maxY - minY) + pad * 2;
  const hi = Buffer.alloc(W * H * 4);

  for (let gi = 0; gi < segsByGlyph.length; gi++) {
    const segs = segsByGlyph[gi];
    const [r, g, b] = colorFor(gi, segsByGlyph.length);
    const edges = [];
    for (let k = 0; k + 3 < segs.length; k += 4) {
      const x1 = segs[k] + ox;
      const y1 = segs[k + 1] + oy;
      const x2 = segs[k + 2] + ox;
      const y2 = segs[k + 3] + oy;
      if (y1 < y2) edges.push([y1, y2, x1, x2]);
      else if (y2 < y1) edges.push([y2, y1, x2, x1]);
    }
    if (!edges.length) continue;
    let eMin = Infinity;
    let eMax = -Infinity;
    for (const e of edges) {
      eMin = Math.min(eMin, e[0]);
      eMax = Math.max(eMax, e[1]);
    }
    const y0 = Math.max(0, Math.floor(eMin));
    const y1 = Math.min(H - 1, Math.ceil(eMax));
    for (let y = y0; y <= y1; y++) {
      const cy = y + 0.5;
      const xs = [];
      for (const [my0, my1, x1, x2] of edges) {
        if (cy >= my0 && cy < my1) xs.push(x1 + ((x2 - x1) * (cy - my0)) / (my1 - my0));
      }
      xs.sort((a, b) => a - b);
      for (let j = 0; j + 1 < xs.length; j += 2) {
        const xa = Math.max(0, Math.ceil(xs[j] - 0.5));
        const xb = Math.min(W - 1, Math.floor(xs[j + 1] - 0.5));
        for (let x = xa; x <= xb; x++) {
          const o = (y * W + x) * 4;
          hi[o] = r;
          hi[o + 1] = g;
          hi[o + 2] = b;
          hi[o + 3] = 255;
        }
      }
    }
  }

  // downsample SS -> 1 (coverage-averaged antialiasing)
  const Wf = Math.floor(W / SS);
  const Hf = Math.floor(H / SS);
  const rgba = Buffer.alloc(Wf * Hf * 4);
  for (let y = 0; y < Hf; y++) {
    for (let x = 0; x < Wf; x++) {
      let sr = 0;
      let sg = 0;
      let sb = 0;
      let sa = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const i = ((y * SS + sy) * W + (x * SS + sx)) * 4;
          const a = hi[i + 3] / 255;
          sr += hi[i] * a;
          sg += hi[i + 1] * a;
          sb += hi[i + 2] * a;
          sa += a;
        }
      }
      const o = (y * Wf + x) * 4;
      const n = SS * SS;
      rgba[o] = sa > 0 ? Math.round(sr / sa) : 0;
      rgba[o + 1] = sa > 0 ? Math.round(sg / sa) : 0;
      rgba[o + 2] = sa > 0 ? Math.round(sb / sa) : 0;
      rgba[o + 3] = Math.round((sa / n) * 255);
    }
  }
  return { width: Wf, height: Hf, rgba };
}

module.exports = { rasterizeText };
