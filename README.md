# nextar

**A next-generation archiver: fast, secure, self-healing.**

`nextar` is a from-scratch Rust archiver designed to take on WinRAR and 7-Zip.
It combines the best modern open-source technologies:

* ⚡ **Zstandard** — the default "fast" compression tier (~GB/s on commodity
  hardware), with **LZMA2** (the 7-Zip engine) as the "ultra" tier.
* 🔐 **Argon2id + XChaCha20-Poly1305** — military-grade password-based
  key derivation and *authenticated* encryption (privacy *and* integrity).
* 🛡️ **Reed-Solomon recovery volumes** — the archive heals itself when bits
  flip or the file is only partially downloaded.
* 🧵 **Fully multi-threaded pipeline** — read ‖ compress ‖ encrypt ‖ write
  across every core.

```
$ nextar create photos -o photos.next -r 8 -p 'secret'
$ nextar list photos.next
$ nextar extract photos.next -p 'secret'
$ nextar repair damaged.next --volumes photos.next.nvol
```

## Desktop app (nextar-gui)

The repo also ships a modern desktop front-end (`cargo run --release --bin
nextar-gui`), sharing the same engine and brand identity:

* **Create** — drop files/folders anywhere (or native pickers), choose codec,
  level, block size, optional password and Reed-Solomon parity, then run with
  a live progress bar.
* **Extract** — pick or drop an archive, choose an output folder, extract.
* **Inspect** — preview an archive's header, metadata and file tree, verify its
  integrity, and jump straight into extraction.
* **Repair** — drop a corrupted archive plus its `.nvol` volume and rebuild it.
* The gradient logo is drawn procedurally (no image assets) and the app icon
  is embedded into both executables at build time.

## Installer & Explorer integration

`scripts/package.sh` assembles a complete `dist/` with three executables:
`nextar.exe`, `nextar-gui.exe`, and **`nextar-setup.exe`** — a full GUI
installer wizard (built with eframe, no admin needed). Double-click it to
walk through:

  **Welcome → Destination folder (with Browse…) → Options → Install → Finished**

* **Destination** — defaults to `%LOCALAPPDATA%\nextar`, changeable with the
  Browse… button; per-user, no elevation required.
* **Options** — toggle right-click menus, the `.next` file association, a
  Start Menu shortcut, and a desktop shortcut (all on by default except
  desktop).
* **Install** — live progress, then “Launch nextar-gui” when finished.

What it registers:

* right-click **any file/folder** → “Compress to .next”
* right-click a **.next archive** → “Extract here” (into a folder named after
  the archive, stripping the shared root — 7-Zip style) and
  “Repair with .nvol…”
* double-click a **.next archive** → opens in nextar-gui's inspect view
* a proper uninstall entry (Settings → Apps → nextar); uninstalling opens a
  GUI uninstaller that removes files, shortcuts, and registry entries
  including the folder.

The menu commands run `nextar-gui --run <create|extract|repair> <path>` — a
quiet always-on-top progress window (no console flash) that auto-closes on
success and shows the error inline on failure.

For scripts: `nextar-setup --quiet [--prefix <dir>]` installs headlessly,
`--dry-run` previews, and `--uninstall` removes. The setup crate is standalone
at `setup/`; rebuild after changing the engine with `cargo build --release
--manifest-path setup/Cargo.toml`.

## Branding

nextar ships with a brand identity:

* A **clean vector chrome mark on glass** — a heavy chrome " >> " double
  chevron on a rounded glass tile with a smooth gradient and a crisp
  hairline bezel. The chevrons are **exact mitered vector polygons**
  (flat-cut bar ends meeting at a true mitered point, like an SVG stroke
  with `stroke-linejoin="miter"`), each bar filled with one smooth linear
  gradient (steel base → bright tip). The receding back chevron is muted
  cyan-tinted steel, the front chevron is hero chrome with a subtle cyan
  lit-chrome edge along its upper bars. No texture, no noise — pure
  geometry, so it looks like a proper vector logo at every size. In **light mode** the tile is frosted white with dark
  steel chrome; in **dark mode** the app swaps to a **deep-navy glass tile
  with white-hot chrome** automatically, following the Windows apps theme
  (read from `HKCU\...\Themes\Personalize`, polled live while the app
  runs). No text in the icon — text lives in the app — so it stays crisp
  and professional from 16px to 256px. It is embedded into all three
  Windows executables at build time (`build.rs` + `embed-resource`), so
  `nextar.exe`, `nextar-gui.exe` and `nextar-setup.exe` show it in
  Explorer and the taskbar, and the same theme-aware painter renders it in
  the GUI, the boot splash, the shell progress window, and the installer
  wizard.
* A **synthwave retro GUI** — deep-purple CRT-styled surfaces with neon
  cyan/hot-pink accents, a subtle scanline + grid overlay, segmented LED
  progress bars, and tactile neon keycap buttons. The whole UI follows the
  Windows light/dark theme: in light mode the surfaces flip to
  lavender-white with darker neon accents (same eased 450 ms cross-fade as
  the logo tile, in both the GUI and the installer wizard). A **Settings
  view** (⚙ in the sidebar) lets you pin the appearance — Follow Windows /
  Always dark / Always light — independent of the OS, with a live logo
  preview that morphs as you pick, and set **create defaults** (codec,
  compression level, block size, thread count, recovery parity) that seed
  the Create view on the next launch; everything is persisted to
  `%LOCALAPPDATA%\nextar\settings.json` and honored by the GUI, the boot
  splash, the shell progress window, and the installer wizard on relaunch
  (`NEXTAR_LOGO_THEME=dark|light` still pins the theme for scripts/CI).
  The native window title bar and decorations follow the same palette via
  DWM attributes (immersive dark mode + caption/text/border colors on
  Win11), so even the OS chrome cross-fades with the theme, and every
  window gets rounded corners (Win11; square corners on Win10). The logo
  tile is a perfect circle in both the icon and the in-app painter, with
  the chrome chevrons scaled to fill it (the front tip lands ~2/3 of the
  way to the rim; the tile gradient is a per-vertex mesh so the circle
  stays clean at every size) and a thin neon-cyan ring around the edge,
  matching the lit-chrome accent on both tiles. It boots with an
  animated splash (chrome " >> " mark over a neon sun, CRT sweep, boot bar)
  that fades into the app, and it is context-aware: dropping a folder routes
  to the Create view, dropping a `.next` archive routes to Inspect with a
  one-click "Extract here" banner.
* **Colored terminal output** — truecolor status lines, success/error
  indicators, progress bars, and per-letter gradient headers. Color is
  enabled only when output is a terminal, disabled by `NO_COLOR` or
  `--no-color`, and forced on with `CLICOLOR_FORCE=1`.
* The logo is generated by `scripts/generate-icon.js` (zero dependencies)
  into `resources/nextar.ico` + `resources/nextar.png`; `--dark` writes the
  deep-navy variant to `nextar-dark.ico`/`nextar-dark.png` (the app painter
  picks between the two palettes at runtime, so the shipped icon is the
  light glass tile that contrasts on both taskbars).

## Build

Requires **Rust 1.75+** and a C compiler for the bundled LZMA2 library
(MSVC on Windows, GCC/Clang elsewhere — `xz2` builds liblzma from source).

```bash
cd nextar
cargo build --release          # → target/release/nextar(.exe) + nextar-gui(.exe)
cargo test                     # integration tests: roundtrips, crypto, repair
```

On Windows, the MSVC toolchain is auto-discovered by `cc`; on Linux/macOS any
`cc` will do.

## Usage

```
nextar create <path>... [-o OUT] [-c zstd|lzma2|store] [-l LEVEL]
                       [-p PASSWORD] [-b SIZE] [-r PARITY] [-s SEGMENT]
                       [-t THREADS] [-f] [-q]
nextar extract <archive> [-o DIR] [-p PASSWORD] [-t THREADS] [-q]
nextar list <archive> [-l]
nextar info <archive>
nextar verify <archive> [-p PASSWORD] [-q]
nextar repair <archive> [--volumes FILE] [-o OUT] [-f] [-q]
```

| Option | Meaning |
|--------|---------|
| `-c`   | Codec: `zstd` (fast, default), `lzma2` (ultra), `store` |
| `-l`   | Level — zstd 1..22 (default 3), lzma2 0..9 (default 6) |
| `-b`   | Chunk size, e.g. `1M`, `256K` (default `1M`) |
| `-r N` | Reed-Solomon parity blocks per segment → writes `<archive>.nvol` (0 = off) |
| `-s`   | Data blocks per RS segment (default 128; `k + r ≤ 256`) |
| `-p`   | Password (Argon2id + XChaCha20-Poly1305) |
| `-t`   | Worker threads (default: all cores) |
| `-f`   | Overwrite existing outputs |

### Examples

```bash
# Fast backup of a directory
nextar create ~/Documents -o docs.next

# Maximum compression
nextar create ~/Documents -o docs.next -c lzma2 -l 9

# Encrypted + self-healing backup
nextar create ~/Documents -o docs.next -r 8 -p 'correct horse battery staple'

# List contents (works without the password — only payloads are encrypted)
nextar list docs.next

# Verify integrity
nextar verify docs.next -p 'correct horse battery staple'

# Heal a corrupted or partially downloaded archive
nextar repair docs.partial.next --volumes docs.next.nvol -o docs.fixed.next
nextar extract docs.fixed.next -p 'correct horse battery staple'
```

## How it works

* Files are split into 1 MiB chunks, and a bounded pipeline of worker threads
  reads, compresses (zstd/lzma2), encrypts (XChaCha20-Poly1305) and writes
  them in parallel. Block ids are deterministic, so the archive layout is
  fully sequential and seekable.
* Passwords are stretched with **Argon2id** (64 MiB, t=3, p=1); each block's
  nonce is derived from the archive salt + block id, and each ciphertext is
  bound to its position and codec via the AEAD's AAD — reordering or
  tampering fails decryption.
* With `-r N`, every 128-block segment gets `N` Reed-Solomon parity shards in
  a sibling `.nvol` volume. `nextar repair` rebuilds up to `N` corrupted or
  missing blocks per segment — from bit flips or a partial download — and the
  volume even carries a copy of the index, so a truncated archive can be
  fully reconstructed.
* File permissions, timestamps, symlinks and empty directories are preserved
  (Unix modes; best-effort mapping on Windows).

## Project structure

```
nextar/
├── Cargo.toml               # package: engine lib + nextar (CLI) + nextar-gui
├── src/                     # engine (lib) + CLI + GUI binary
│   ├── lib.rs · archive.rs · format.rs · pipeline.rs · compress.rs
│   ├── crypto.rs · recovery.rs · progress.rs · term.rs
│   ├── main.rs              # nextar CLI (create/extract/list/repair…)
│   └── bin/nextar-gui.rs    # desktop app (synthwave UI, theme-aware)
├── setup/                   # Windows installer wizard crate (→ nextar-setup.exe)
├── installers/
│   ├── windows/             # build.ps1 + docs for the Windows installer
│   └── macos/               # Info.plist, make-app-bundle.sh, build-dmg.sh → .dmg
├── scripts/                 # generate-icon.js, build-icns.js, package.sh
├── resources/               # nextar.ico/.png/.icns (light + dark variants)
├── docs/                    # ARCHITECTURE.md, FORMAT.md
├── dist/                    # built artifacts + installers output
└── tests/                   # integration tests
```

**Installers** — Windows: `installers/windows/build.ps1` (or
`scripts/package.sh`) produces `dist/nextar-setup.exe`, a per-user wizard
with Explorer right-click integration. macOS: run
`installers/macos/build-dmg.sh` **on a Mac** to produce
`dist/nextar-<version>-macos.dmg` (`.app` bundle + CLI, ad-hoc signed; see
`installers/macos/README.md` for notarization steps).

See **`docs/FORMAT.md`** for the complete binary layout of the `.NEXT`
archive and `.nvol` volume, and **`docs/ARCHITECTURE.md`** for the module
breakdown, pipeline design and threat model.

## Status

Working v0.1: create / extract / list / info / verify / repair, zstd + lzma2
tiers, encryption, and Reed-Solomon recovery are implemented and covered by
integration tests (`cargo test`). The format is versioned (v1) and documented.
