#!/usr/bin/env node
// Self-verifying check for the landing page's stable download card.
//
// The card is pinned to a versioned release (STABLE_TAG in site/index.html).
// It can silently go stale in three ways, all of which this script fails on:
//
//   1. A NEWER stable release exists but nobody bumped STABLE_TAG, so the
//      card keeps pointing at an old build (and old checksums).
//   2. The pinned release was deleted / renamed, so the card's fetch 404s
//      and falls back to "unavailable — stable release not found".
//   3. The pinned release exists but lost its assets or its checksum body,
//      so the badge shows sizes/ages but the verify blocks lie.
//
// Dependency-free (Node 18+ global fetch) so it runs anywhere: a nightly CI
// job, a pre-push hook, or a maintainer's laptop.
//
//   usage: node scripts/check-stable-card.js [--repo owner/repo] [--site URL]
//
// Env overrides: REPO, SITE_URL, GH_TOKEN (optional, avoids API rate limits).

const https = require('https');

const REPO = process.env.REPO || arg('--repo') || 'sargeraas84/nextar';
const SITE_URL = process.env.SITE_URL || arg('--site') || `https://${REPO.split('/')[0]}.github.io/${REPO.split('/')[1]}/`;
const TOKEN = process.env.GH_TOKEN || '';

function arg(name) {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

function fail(msg) {
  console.error('::error::' + msg);
  console.error('STABLE CARD CHECK FAILED: ' + msg);
  process.exit(1);
}

function get(url, headers = {}) {
  return new Promise((resolve, reject) => {
    const req = https.get(url, { headers: { 'User-Agent': 'nextar-stable-check', ...headers } }, (res) => {
      let body = '';
      res.on('data', (d) => (body += d));
      res.on('end', () => resolve({ status: res.statusCode, body }));
    });
    req.on('error', reject);
    req.setTimeout(30000, () => req.destroy(new Error('timeout: ' + url)));
  });
}

async function api(path) {
  const headers = {};
  if (TOKEN) headers.Authorization = 'Bearer ' + TOKEN;
  const res = await get(`https://api.github.com/repos/${REPO}${path}`, headers);
  if (res.status === 403) fail(`GitHub API rate-limited (add GH_TOKEN): ${path}`);
  if (res.status === 404) return null;
  if (res.status !== 200) fail(`GitHub API ${res.status} for ${path}`);
  return JSON.parse(res.body);
}

(async () => {
  // 1. What is the site pinned to right now (the deployed HTML)?
  const site = await get(SITE_URL);
  if (site.status !== 200) fail(`landing page returned HTTP ${site.status} at ${SITE_URL}`);
  const m = site.body.match(/var STABLE_TAG = '([^']+)'/);
  if (!m) fail(`could not find 'var STABLE_TAG' in the deployed page — card wiring broken`);
  const pinned = m[1];
  console.log(`deployed card pinned to: ${pinned}`);

  // 2. Latest real stable release (excludes the prerelease 'nightly' tag).
  const releases = await api('/releases?per_page=30');
  if (!releases) fail('could not list releases');
  const stables = releases
    .filter((r) => !r.prerelease && !r.draft && /^v\d+\.\d+\.\d+$/.test(r.tag_name))
    .map((r) => r.tag_name);
  if (stables.length === 0) fail('no versioned stable releases found');
  const latest = stables[0];
  console.log(`latest stable release:  ${latest}`);

  // 3. Staleness: a newer stable exists but the card wasn't bumped.
  if (latest !== pinned) {
    fail(
      `stable card is stale: site pins ${pinned} but the latest stable release is ${latest}. ` +
        `Bump STABLE_TAG in site/index.html (and the h3/verify placeholders) to ${latest}.`
    );
  }

  // 4. The pinned release must exist and carry the full payload.
  const rel = await api(`/releases/tags/${encodeURIComponent(pinned)}`);
  if (!rel) fail(`pinned release ${pinned} does not exist (deleted or renamed) — the card now shows 'unavailable'`);
  const assetNames = rel.assets.map((a) => a.name);
  const dmgName = `${pinned.replace(/^v/, 'nextar-')}-macos.dmg`;
  const expected = ['nextar-setup.exe', 'nextar-gui.exe', 'nextar.exe', dmgName];
  const missing = expected.filter((n) => !assetNames.includes(n));
  if (missing.length) fail(`release ${pinned} is missing assets: ${missing.join(', ')}`);
  const zero = rel.assets.filter((a) => a.size === 0).map((a) => a.name);
  if (zero.length) fail(`release ${pinned} has zero-byte assets: ${zero.join(', ')}`);

  // 5. The body must carry a checksum for every shipped asset, else the
  //    card's verify blocks show hashes that don't match the bytes.
  const body = rel.body || '';
  if (!body.includes('**Checksums (SHA-256)**')) {
    fail(`release ${pinned} body has no '**Checksums (SHA-256)**' section — verify blocks will be empty`);
  }
  for (const name of expected) {
    const line = body.split('\n').find((l) => l.includes(`${name}:`));
    if (!line) fail(`release ${pinned} body has no checksum line for ${name}`);
    const hash = (line.split(':')[1] || '').trim();
    if (!/^[0-9a-f]{64}$/.test(hash)) fail(`release ${pinned} body checksum for ${name} is not a SHA-256: '${hash}'`);
    console.log(`  ${name}: ${hash}`);
  }

  console.log('✅ stable card verified: pinned to latest release, 4 assets, all checksums present');
  console.log(JSON.stringify({ pinned, latest, assets: expected.length, status: 'ok' }, null, 2));
})().catch((e) => fail(e.message));
