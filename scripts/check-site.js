'use strict';
// Landing-page smoke check: validates the generated site and its assets
// before the Pages workflow deploys (and after every CI asset regeneration).
// Zero dependencies — exit code 1 on any failure, 0 when everything passes.
//
//   node scripts/check-site.js
//
// Checks:
//   * site/index.html  — inline scripts parse, CSS braces balance, required
//     meta/link tags and brand/structural markers are present.
//   * site/manifest.json — valid JSON with the required PWA fields + icons.
//   * regenerated assets — favicon.ico (ICO magic), og-image.png, the PWA
//     icons and apple-touch-icon exist with the expected pixel dimensions.
const fs = require('fs');
const path = require('path');

const ROOT = path.join(__dirname, '..');
const SITE = path.join(ROOT, 'site');

const errors = [];
const check = (cond, msg) => { if (!cond) errors.push(msg); };

// --- index.html ----------------------------------------------------------
const htmlPath = path.join(SITE, 'index.html');
check(fs.existsSync(htmlPath), 'site/index.html is missing');
let html = '';
if (fs.existsSync(htmlPath)) html = fs.readFileSync(htmlPath, 'utf8');

// Every inline <script> must be valid JavaScript.
const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)];
check(scripts.length > 0, 'index.html has no inline <script> blocks');
scripts.forEach((m, i) => {
  try { new Function(m[1]); }
  catch (e) { errors.push(`script #${i} fails to parse: ${e.message}`); }
});

// The single <style> block must be brace-balanced.
const style = (html.match(/<style>([\s\S]*?)<\/style>/) || [])[1] || '';
const open = (style.match(/{/g) || []).length;
const close = (style.match(/}/g) || []).length;
check(open > 0 && open === close, `CSS brace mismatch (${open} open / ${close} close)`);

// Required head tags.
[
  '<meta name="description"',
  '<meta property="og:image"',
  '<meta name="twitter:image"',
  '<meta name="theme-color"',
  'rel="manifest" href="manifest.json"',
  'rel="icon"',
  'rel="apple-touch-icon"',
].forEach(t => check(html.includes(t), `index.html missing <head> tag: ${t}`));

// Brand / structural markers: nav, hero, download cards, theme toggle, badges.
[
  'id="nav"',
  'class="hero"',
  'data-download="nextar-setup.exe"',
  'data-download="nextar-macos.dmg"',
  'id="theme-toggle"',
  'data-live-badge',
  'data-verify',
].forEach(t => check(html.includes(t), `index.html missing marker: ${t}`));

// --- manifest.json -------------------------------------------------------
const manifestPath = path.join(SITE, 'manifest.json');
check(fs.existsSync(manifestPath), 'site/manifest.json is missing');
let manifest = null;
if (fs.existsSync(manifestPath)) {
  try { manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')); }
  catch (e) { errors.push(`manifest.json is not valid JSON: ${e.message}`); }
}
if (manifest) {
  check(manifest.name && manifest.short_name, 'manifest.json missing name/short_name');
  check(manifest.display === 'standalone', 'manifest.json display must be "standalone"');
  check(manifest.background_color, 'manifest.json missing background_color');
  check(manifest.theme_color, 'manifest.json missing theme_color');
  check(Array.isArray(manifest.icons) && manifest.icons.length >= 3,
    'manifest.json must list at least 3 icons (192/512/touch)');
  (manifest.icons || []).forEach(ic =>
    check(ic && ic.src && ic.sizes && ic.type,
      `manifest icon missing src/sizes/type: ${JSON.stringify(ic)}`));
}

// --- regenerated assets --------------------------------------------------
function pngSize(file) {
  const b = fs.readFileSync(file);
  if (b.length < 24 || b[0] !== 0x89 || b[1] !== 0x50 || b[2] !== 0x4e || b[3] !== 0x47) {
    throw new Error('not a PNG (bad signature)');
  }
  return { w: b.readUInt32BE(16), h: b.readUInt32BE(20) };
}
function isValidIco(file) {
  const b = fs.readFileSync(file);
  // ICO header: reserved(2)=0, type(2)=1, image count(2)>=1.
  return b.length >= 6 && b[0] === 0 && b[1] === 0 && b[2] === 1 && b[3] === 0 && b.readUInt16LE(4) >= 1;
}

const expectedPNG = {
  'og-image.png': [1200, 630],
  'apple-touch-icon.png': [180, 180],
  'icon-192.png': [192, 192],
  'icon-512.png': [512, 512],
};
for (const [name, [w, h]] of Object.entries(expectedPNG)) {
  const p = path.join(SITE, name);
  check(fs.existsSync(p), `site/${name} is missing`);
  if (fs.existsSync(p)) {
    try {
      const s = pngSize(p);
      check(s.w === w && s.h === h, `${name} is ${s.w}x${s.h}, expected ${w}x${h}`);
    } catch (e) { errors.push(`${name}: ${e.message}`); }
  }
}
const fav = path.join(SITE, 'favicon.ico');
check(fs.existsSync(fav), 'site/favicon.ico is missing');
if (fs.existsSync(fav)) check(isValidIco(fav), 'favicon.ico is not a valid multi-image ICO');

// --- report --------------------------------------------------------------
if (errors.length) {
  console.error('site smoke check FAILED:');
  errors.forEach(e => console.error('  ✗ ' + e));
  process.exit(1);
}
console.log('site smoke check OK — index.html, manifest.json, favicon.ico, og-image.png and PWA icons all valid');
