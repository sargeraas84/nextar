// Headless (offscreen) Electron check: load the DEPLOYED landing page and
// read the stable card's live DOM state — badge text, both SHA-256 hashes,
// and whether the copy buttons got enabled by the JS that fetches the
// pinned release. This exercises the exact same code path a real browser
// runs (isLocalServe() false on the https origin, fetch to api.github.com).
//
// Config comes via env (NEXTAR_CHECK_URL / NEXTAR_CHECK_TAG / NEXTAR_CHECK_OUT):
// Windows Electron silently drops extra argv after the script path, so argv
// is not usable. All progress + the result are written to OUT (JSON) and
// the exit code carries the verdict.
const fs = require('fs');
const path = require('path');

const URL = process.env.NEXTAR_CHECK_URL || 'https://sargeraas84.github.io/nextar/';
const TAG = process.env.NEXTAR_CHECK_TAG || 'v0.2.0';
const OUT = process.env.NEXTAR_CHECK_OUT || path.join(__dirname, 'stable-card-check.json');

function log(msg) {
  try { fs.appendFileSync(OUT + '.log', msg + '\n'); } catch (e) {}
}
function done(code, obj) {
  try { fs.writeFileSync(OUT, JSON.stringify({ tag: TAG, ...obj }, null, 2)); } catch (e) {}
  log('done code=' + code);
  app.exit(code);
}

const { app, BrowserWindow } = require('electron');
log('starting URL=' + URL + ' TAG=' + TAG);
app.disableHardwareAcceleration();

try {
  app.whenReady().then(() => {
    log('app ready');
    const win = new BrowserWindow({
      show: false,
      width: 1280,
      height: 2400,
      webPreferences: { offscreen: true },
    });
    log('window created');

    const timeout = setTimeout(() => {
      done(2, { error: 'timeout waiting for stable card data' });
    }, 40000);

    win.webContents.on('did-finish-load', async () => {
      log('did-finish-load');
      try {
        const started = Date.now();
        let txt = '';
        while (Date.now() - started < 20000) {
          txt = await win.webContents.executeJavaScript(
            "(function(){var b=document.querySelector('[data-stable-badge] .bt');return b?b.textContent:''})()"
          );
          log('poll badge: ' + txt);
          if (txt && !/checking/.test(txt)) break;
          await new Promise(r => setTimeout(r, 500));
        }
        const result = await win.webContents.executeJavaScript(`(function(){
          var badge = document.querySelector('[data-stable-badge] .bt');
          var verifies = Array.prototype.map.call(
            document.querySelectorAll('[data-stable-verify]'), function(v){
              var code = v.querySelector('.verify-hash');
              var btn = v.querySelector('.copy-btn');
              return {
                key: v.getAttribute('data-stable-verify'),
                hash: code ? code.textContent.trim() : null,
                copyEnabled: btn ? !btn.disabled : null,
                copyCmd: v.querySelector('.verify-cmd') ? v.querySelector('.verify-cmd').textContent.trim() : null
              };
            });
          var dl = Array.prototype.map.call(
            document.querySelectorAll('[data-stable-download]'), function(a){
              return { label: a.textContent.trim().slice(0, 40), href: a.getAttribute('href') };
            });
          return { title: document.title, stableBadge: badge ? badge.textContent : null, verifies: verifies, downloads: dl };
        })()`);
        log('result captured');
        clearTimeout(timeout);
        const hashes = (result.verifies || []).map(v => v.hash || '');
        const ok = result.stableBadge && result.stableBadge.indexOf(TAG) !== -1 &&
          hashes.length >= 2 && hashes.every(h => /^[0-9a-f]{64}$/.test(h)) &&
          (result.verifies || []).every(v => v.copyEnabled === true);
        done(ok ? 0 : 1, result);
      } catch (e) {
        clearTimeout(timeout);
        done(1, { error: 'eval failed: ' + e.message });
      }
    });
    win.loadURL(URL).catch(e => { clearTimeout(timeout); done(1, { error: 'load failed: ' + e.message }); });
  });
} catch (e) {
  log('outer catch: ' + e.message);
  done(1, { error: 'outer: ' + e.message });
}
