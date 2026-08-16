# Contributing to nextar

nextar is a Rust archiver (Zstd / LZMA2, Argon2id + XChaCha20-Poly1305,
Reed-Solomon recovery volumes) with an `eframe` GUI, a per-user Windows
installer, a macOS `.dmg` pipeline, and a static landing page. This file
covers the workflow maintainers need most; the full architecture lives in
`docs/ARCHITECTURE.md`, the binary format in `docs/FORMAT.md`, and the user
guide in `README.md`.

## Quick start

```bash
cargo test                     # engine + GUI + integration tests
cargo clippy --all-targets     # lint
cargo run --release --bin nextar-gui   # launch the GUI
bash scripts/package.sh        # rebuild dist/ (icons + site assets + installers)
```

## Tests that matter before you ship

- `cargo test` — unit + integration tests (engine, crypto, recovery, GUI,
  and the settings migration tests).
- `cargo test --bin nextar-gui settings` — settings migration coverage only
  (fast feedback when touching the schema).
- `scripts/verify-shell.ps1 -Run` — Explorer right-click verbs, invoked for
  real (compress / extract / repair, including encrypted repair).
- `scripts/verify-shell.ps1 -Installer` — payload-only installer E2E.
- `scripts/verify-shell.ps1 -Installer -Full` — full install → upgrade
  (settings.json preserved) → uninstall. **Ephemeral runner only**: it
  clobbers the global shell verbs and restores any pre-existing install.

## Settings schema & migration

User settings live in `%LOCALAPPDATA%\nextar\settings.json` (see
`Settings` in `src/bin/nextar-gui.rs`). Every field must carry
`#[serde(default)]` so an older file still parses. When you add, rename, or
remove a field:

1. Add a fixture to `tests/fixtures/settings/` representing the old schema
   (named by version, e.g. `v4-<what-changed>.json`).
2. Extend `settings_fixtures_migrate_and_preserve_values` with the expected
   migrated `Settings`.
3. Run `cargo test --bin nextar-gui settings`.

A corrupt settings file is detected at startup and the Settings view offers a
one-click reset (the unreadable file is backed up first as
`settings.json.corrupt-<timestamp>`). `nextar-gui --reset-settings` does the
same recovery headlessly. Every reset appends an audit line to
`%LOCALAPPDATA%\nextar\recovery.log` so backups stay traceable across
launches; `nextar-gui --check-settings [path]` validates a settings file for
scripts/CI (exit 0 clean/absent, exit 1 corrupt) and checks the live location
when no path is given.

## Releasing

Releases are gated on the installer E2E — both `release.yml` jobs run the
install lifecycle before artifacts are attached, so a release only appears
when packaging works on Windows and macOS.

- **Automatic:** push a `v*` tag (`git tag v0.3.0 && git push origin v0.3.0`).
- **Manual:** run the `release` workflow with a `version` input; it bumps
  `Cargo.toml` + `setup/src/main.rs`, commits, tags, and releases. The
  remaining version-bearing files below are NOT touched by the workflow —
  bump them by hand.

### Release checklist (every cut)

1. **Bump the version in every version-bearing file.** The `prepare` job
   only updates `Cargo.toml`, `setup/Cargo.toml`, and the `VERSION`
   constant in `setup/src/main.rs`; the rest are manual:

   - `Cargo.toml` + `setup/Cargo.toml` (`version = "..."`)
   - `VERSION` const in `setup/src/main.rs`
   - `nextar.rc` + `setup/nextar.rc` (`FileVersion` + `ProductVersion`)
   - `installers/macos/Info.plist` (both version strings)
   - **NOT yet:** `site/index.html` — the stable card bump is a separate
     step 5 below.

   Sanity-check nothing was missed:
   `grep -rn "<old-version>" --include='*.rs' --include='*.toml' --include='*.rc' --include='*.plist' .`

2. **Build + test locally:**
   `cargo test --bin nextar-gui` and `cargo test --manifest-path setup/Cargo.toml`.

3. **Commit, push master, tag, push tag:**
   ```bash
   git commit ...  # app-version bump only — site card still on the OLD tag
   git push origin master
   git tag vX.Y.Z && git push origin vX.Y.Z
   ```

4. **Watch the release run green and verify it published** (`windows`,
   `macos`, `release` jobs all success):
   ```bash
   gh run list --workflow=release.yml --limit 1
   gh release view vX.Y.Z --json assets --jq '.assets[].name'
   ```
   Confirm the release body carries the `**Checksums (SHA-256)**` block
   (the stable card reads it).

5. **Bump the stable card — as a SEPARATE commit AFTER the release is
   published.** The nightly staleness gate (`scripts/check-stable-card.js`,
   wired into `nightly.yml` as `stable-card-check`) fails if the deployed
   card's `STABLE_TAG` differs from the latest stable release. Bumping the
   card before the release exists (or in the same commit as the tag) makes
   a nightly running during that window trip a false positive; bumping it
   after the release is live keeps every nightly green:
   - `site/index.html`: `STABLE_TAG`, the `<h3>Stable · vX.Y.Z</h3>`, both
     `checking vX.Y.Z…` verify placeholders, the `shasum -a 256
     nextar-X.Y.Z-macos.dmg` line, the dl-note, and `STABLE_ASSETS`.
   - Commit + push (`git push origin master`), then verify the gate:
     ```bash
     node scripts/check-stable-card.js        # must exit 0
     # optional real-browser check of the deployed card:
     NEXTAR_CHECK_TAG=vX.Y.Z node scripts/verify-stable-card.cjs
     ```

   If `check-stable-card.js` fails with *"stable card is stale: site pins
   …"* the bump above was missed; if it fails while the card already
   points at the new tag, the release hasn't published yet — wait for step
   4 and re-run.

### Moving a tag after a fix (used for v0.2.0)

The tag-push trigger builds the workflow code **at the tagged commit** — a
release whose build fails needs its tag moved to the fix commit and
re-pushed, not just a master push (master pushes don't retrigger `release`).
This is safe **only if no release exists yet** for that tag (check with
`gh release view <tag>`; if it 404s, move it):

```bash
# 1. push the fix to master (also re-triggers nightly)
git push origin master
# 2. delete + recreate the tag at the fix commit
git tag -d v0.2.0
git push origin :refs/tags/v0.2.0
git tag v0.2.0 <fix-sha>
git push origin v0.2.0
# 3. watch the fresh release run
git rev-parse v0.2.0   # confirm it points at the fix
gh run list --workflow=release.yml --limit 1
```

During v0.2.0 this moved the tag three times (CI hang → signing locator →
signature gate) until the release went green. If a release **already exists**
at that tag, do NOT move it — cut a new patch tag (e.g. `v0.2.1`) instead.

## Branch protection

Merging to `master` should require CI. Branch protection is a repository
setting (not in the tree); apply it once:

```bash
gh auth login
bash scripts/setup-branch-protection.sh <owner>/<repo>
```

This requires the `build-and-verify` (Windows) and `macos-package` (macOS)
checks with `strict` up-to-date enforcement. Add CODEOWNERS entries in
`.github/CODEOWNERS` as the maintainer set changes.

## Landing page & brand assets

The logo is procedural: `scripts/generate-icon.js` produces the `.ico`/`.png`/
`.icns`, and `scripts/build-site-assets.js` regenerates the favicon, `og:image`
and PWA icons from the same mark. `scripts/check-site.js` validates the page
and assets; it runs in CI before the Pages deploy. If you change the mark,
run `bash scripts/package.sh` (or `node scripts/generate-icon.js` +
`node scripts/build-site-assets.js`) and commit the regenerated files.
