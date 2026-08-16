'use strict';
// Builds the marketing-site assets from the procedural convergence-core mark:
//   site/favicon.ico          (copy of the multi-size Windows icon)
//   site/apple-touch-icon.png (180px mark on deep navy — no transparency)
//   site/icon-192.png         (PWA, maskable-safe)
//   site/icon-512.png         (PWA, maskable-safe)
//   site/og-image.png         (1200x630 social card: glow + mark + NEXTAR wordmark)
//   site/manifest.json        (web-app manifest)
// Zero dependencies, like generate-icon.js. The wordmark is rasterized from
// the bundled Space Grotesk Bold (resources/fonts/) via font-raster.js.
// Re-run whenever the mark, colors, or name change: `node scripts/build-site-assets.js`.
const fs = require('fs');
const path = require('path');
const { renderIcon, encodePNG } = require('./generate-icon.js');
const { rasterizeText } = require('./font-raster.js');

const ROOT = path.join(__dirname, '..');
const SITE = path.join(ROOT, 'site');

// Brand gradient for the wordmark (matches the app's dark-mode grad_text:
// ice cyan → violet → pink).
const GRAD = [
  [0x5f, 0xf2, 0xff],
  [0x8b, 0x7b, 0xff],
  [0xff, 0x5f, 0xd7],
];
const wordmarkColor = (i, n) => {
  const t = n > 1 ? i / (n - 1) : 0;
  const [a, b] = t < 0.5 ? [GRAD[0], GRAD[1]] : [GRAD[1], GRAD[2]];
  const u = t < 0.5 ? t * 2 : (t - 0.5) * 2;
  return [
    Math.round(a[0] + (b[0] - a[0]) * u),
    Math.round(a[1] + (b[1] - a[1]) * u),
    Math.round(a[2] + (b[2] - a[2]) * u),
  ];
};

// --- favicon: reuse the multi-size Windows icon (16..256) ---
fs.copyFileSync(path.join(ROOT, 'resources', 'nextar.ico'), path.join(SITE, 'favicon.ico'));

// The mark composited on the deep-navy brand background (#0a0f1e), scaled to
// `scale` so maskable icons keep the ring inside the safe zone.
function markOnNavy(size, scale = 1) {
  const mark = renderIcon(size, 2, true);
  const out = Buffer.alloc(size * size * 4);
  const m = (size * (1 - scale)) / 2;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = (y * size + x) * 4;
      out[i] = 0x0a;
      out[i + 1] = 0x0f;
      out[i + 2] = 0x1e;
      out[i + 3] = 255;
      const sx = (x - m) / scale;
      const sy = (y - m) / scale;
      if (sx >= 0 && sx < size && sy >= 0 && sy < size) {
        const si = (Math.floor(sy) * size + Math.floor(sx)) * 4;
        const a = mark.rgba[si + 3] / 255;
        out[i] = Math.round(mark.rgba[si] * a + out[i] * (1 - a));
        out[i + 1] = Math.round(mark.rgba[si + 1] * a + out[i + 1] * (1 - a));
        out[i + 2] = Math.round(mark.rgba[si + 2] * a + out[i + 2] * (1 - a));
      }
    }
  }
  return out;
}

// Source-over blit of a straight-alpha RGBA image onto a buffer.
function blit(dst, dw, dh, img, dx, dy) {
  for (let y = 0; y < img.height; y++) {
    const ty = dy + y;
    if (ty < 0 || ty >= dh) continue;
    for (let x = 0; x < img.width; x++) {
      const tx = dx + x;
      if (tx < 0 || tx >= dw) continue;
      const si = (y * img.width + x) * 4;
      const di = (ty * dw + tx) * 4;
      const a = img.rgba[si + 3] / 255;
      dst[di] = Math.round(img.rgba[si] * a + dst[di] * (1 - a));
      dst[di + 1] = Math.round(img.rgba[si + 1] * a + dst[di + 1] * (1 - a));
      dst[di + 2] = Math.round(img.rgba[si + 2] * a + dst[di + 2] * (1 - a));
    }
  }
}

// --- og:image: 1200x630, navy gradient + cyan/violet glows + mark + wordmark ---
function ogImage() {
  const W = 1200;
  const H = 630;
  const S = 300;
  const mark = renderIcon(S, 2, true);
  const cx = W / 2;
  const cy = H / 2 - 70;
  const out = Buffer.alloc(W * H * 4);
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      const i = (y * W + x) * 4;
      const t = y / H;
      // deep navy vertical gradient (site --bg #050505 -> #0a0f1e)
      let r = 0x05 + (0x0a - 0x05) * (1 - t);
      let g = 0x05 + (0x0f - 0x05) * (1 - t);
      let b = 0x05 + (0x1e - 0x05) * (1 - t);
      // cyan glow behind the mark
      const d = Math.hypot(x - cx, y - cy);
      const glowC = Math.exp(-((d / 280) ** 2)) * 0.32;
      r += 0x5f * glowC;
      g += 0xf2 * glowC;
      b += 0xff * glowC;
      // violet glow, top-right (site accent)
      const d2 = Math.hypot(x - W * 0.86, y - H * 0.22);
      const glowV = Math.exp(-((d2 / 260) ** 2)) * 0.3;
      r += 0x8b * glowV;
      g += 0x7b * glowV;
      b += 0xff * glowV;
      // blit the mark (source-over)
      const mx = x - Math.round(cx - S / 2);
      const my = y - Math.round(cy - S / 2);
      let ar = r;
      let ag = g;
      let ab = b;
      if (mx >= 0 && mx < S && my >= 0 && my < S) {
        const mi = (my * S + mx) * 4;
        const a = mark.rgba[mi + 3] / 255;
        ar = mark.rgba[mi] * a + r * (1 - a);
        ag = mark.rgba[mi + 1] * a + g * (1 - a);
        ab = mark.rgba[mi + 2] * a + b * (1 - a);
      }
      out[i] = Math.min(255, Math.round(ar));
      out[i + 1] = Math.min(255, Math.round(ag));
      out[i + 2] = Math.min(255, Math.round(ab));
      out[i + 3] = 255;
    }
  }
  // NEXTAR wordmark in Space Grotesk Bold, gradient, centered under the mark
  const font = fs.readFileSync(path.join(ROOT, 'resources', 'fonts', 'SpaceGrotesk-Bold.ttf'));
  const word = rasterizeText(font, 'NEXTAR', 64, wordmarkColor);
  blit(out, W, H, word, Math.round((W - word.width) / 2), Math.round(cy + S / 2 + 34));
  return encodePNG(W, H, out);
}

fs.writeFileSync(path.join(SITE, 'apple-touch-icon.png'), encodePNG(180, 180, markOnNavy(180)));
fs.writeFileSync(path.join(SITE, 'icon-192.png'), encodePNG(192, 192, markOnNavy(192, 0.85)));
fs.writeFileSync(path.join(SITE, 'icon-512.png'), encodePNG(512, 512, markOnNavy(512, 0.85)));
fs.writeFileSync(path.join(SITE, 'og-image.png'), ogImage());
// The hero + brand-mark logos reference this file (they used to inline the
// mark as a giant base64 blob; a file keeps the HTML lean and always in
// sync with the raster logo).
fs.writeFileSync(path.join(SITE, 'logo.png'), renderIcon(512, 2, false).png);

const manifest = {
  name: 'nextar',
  short_name: 'nextar',
  description: 'The next-generation archiver — zstd + lzma2, Argon2id + XChaCha20-Poly1305, Reed-Solomon self-healing archives.',
  start_url: '.',
  display: 'standalone',
  background_color: '#050505',
  theme_color: '#37e6ff',
  icons: [
    { src: 'icon-192.png', sizes: '192x192', type: 'image/png', purpose: 'any maskable' },
    { src: 'icon-512.png', sizes: '512x512', type: 'image/png', purpose: 'any maskable' },
    { src: 'apple-touch-icon.png', sizes: '180x180', type: 'image/png' },
  ],
};
fs.writeFileSync(path.join(SITE, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');

console.log(
  'wrote site/favicon.ico, apple-touch-icon.png, icon-192.png, icon-512.png, og-image.png, logo.png, manifest.json'
);
