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
same recovery headlessly.

## Releasing

Releases are gated on the installer E2E — both `release.yml` jobs run the
install lifecycle before artifacts are attached, so a release only appears
when packaging works on Windows and macOS.

- **Automatic:** push a `v*` tag (`git tag v0.2.0 && git push origin v0.2.0`).
- **Manual:** run the `release` workflow with a `version` input; it bumps
  `Cargo.toml` + `setup/src/main.rs`, commits, tags, and releases.

The `prepare` job updates the version in `Cargo.toml`, `setup/Cargo.toml`,
and the `VERSION` constant in `setup/src/main.rs` — keep all three in sync
when bumping by hand.

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
