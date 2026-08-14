#!/usr/bin/env node
// Build resources/nextar.icns — the macOS app icon — from the procedural
// logo generator. The icns format embeds PNG data directly (no macOS
// tooling needed, so this runs anywhere), using the modern icon types:
//
//   icp4 16px · icp5 32px · icp6 64px · ic07 128px · ic08 256px ·
//   ic09 512px · ic10 1024px
//
// Usage:  node scripts/build-icns.js
// Output: resources/nextar.icns (used by installers/macos).

'use strict';

const fs = require('fs');
const path = require('path');

const { renderIcon } = require('./generate-icon');

const RES = path.join(__dirname, '..', 'resources');

// (icns type, pixel size, supersample) — supersample drops for the big
// frames (they are dense enough that ss=1 stays crisp at 1024px).
const FRAMES = [
  ['icp4', 16, 4],
  ['icp5', 32, 4],
  ['icp6', 64, 4],
  ['ic07', 128, 3],
  ['ic08', 256, 3],
  ['ic09', 512, 2],
  ['ic10', 1024, 1],
];

function buildIcns() {
  const chunks = [];
  // 'info' chunk: the 4-char icon name hint ("NEXT") — 8-byte header + 4
  // bytes of data, 12 bytes total.
  const info = Buffer.alloc(12);
  info.write('info', 0, 'ascii');
  info.writeUInt32BE(12, 4);
  info.write('NEXT', 8, 'ascii');
  chunks.push(info);

  for (const [type, size, ss] of FRAMES) {
    const { png } = renderIcon(size, ss, false);
    if (png.readUInt32BE(0) !== 0x89504e47) {
      throw new Error(`renderIcon(${size}) did not produce PNG data`);
    }
    const header = Buffer.alloc(8);
    header.write(type, 0, 'ascii');
    header.writeUInt32BE(8 + png.length, 4);
    chunks.push(header, png);
  }

  const total = 8 + chunks.reduce((n, c) => n + c.length, 0);
  const icns = Buffer.alloc(total);
  icns.write('icns', 0, 'ascii');
  icns.writeUInt32BE(total, 4);
  let off = 8;
  for (const c of chunks) {
    c.copy(icns, off);
    off += c.length;
  }

  fs.mkdirSync(RES, { recursive: true });
  const out = path.join(RES, 'nextar.icns');
  fs.writeFileSync(out, icns);
  return { out, total, frames: FRAMES.map((f) => f[1]) };
}

function verifyIcns(file) {
  const buf = fs.readFileSync(file);
  if (buf.toString('ascii', 0, 4) !== 'icns') throw new Error('bad icns magic');
  if (buf.readUInt32BE(4) !== buf.length) throw new Error('bad icns size');
  let off = 8;
  const seen = [];
  while (off < buf.length) {
    const type = buf.toString('ascii', off, off + 4);
    const len = buf.readUInt32BE(off + 4);
    if (off + len > buf.length) throw new Error(`chunk ${type} overruns file`);
    if (type !== 'info') {
      // each image chunk must embed a valid PNG (signature + IHDR size match)
      const pngSig = buf.readUInt32BE(off + 8);
      if (pngSig !== 0x89504e47) throw new Error(`chunk ${type} is not a PNG`);
      const w = buf.readUInt32BE(off + 8 + 16);
      const h = buf.readUInt32BE(off + 8 + 20);
      const expected = { icp4: 16, icp5: 32, icp6: 64, ic07: 128, ic08: 256, ic09: 512, ic10: 1024 }[type];
      if (w !== expected || h !== expected) throw new Error(`chunk ${type} is ${w}x${h}, expected ${expected}`);
      seen.push(`${type}(${w})`);
    }
    off += len;
  }
  return seen;
}

if (require.main === module) {
  const { out, total } = buildIcns();
  const seen = verifyIcns(out);
  console.log(`[build-icns] wrote ${out} (${(total / 1024).toFixed(1)} KiB, chunks: ${seen.join(' ')})`);
} else {
  module.exports = { buildIcns, verifyIcns };
}
