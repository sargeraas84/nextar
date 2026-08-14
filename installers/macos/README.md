# nextar · macOS installer

Builds a professional `nextar-<version>-macos.dmg` disk image containing a
ready-to-run `nextar.app` bundle plus the standalone `nextar` CLI binary.

## Requirements (run on a Mac)

- **Rust toolchain** — `rustup` + stable (the code is fully cross-platform;
  the Windows-only bits — registry theme detection, DWM title-bar styling,
  shell integration — are `#[cfg(windows)]` and compile away on macOS).
- **Xcode Command Line Tools** — `xcode-select --install` (provides `clang`,
  `hdiutil`, `codesign`).
- **Node.js** — only used by the icon generator to produce `nextar.icns`.

## Build

```bash
./installers/macos/build-dmg.sh
```

Output: `dist/nextar-0.1.0-macos.dmg`

The script:

1. `cargo build --release --bin nextar --bin nextar-gui` — release binaries.
2. `node scripts/build-icns.js` — (re)generates `resources/nextar.icns`
   (16 → 1024 px, embedded PNGs, no macOS-only tooling needed).
3. Assembles `nextar.app`:
   ```
   nextar.app/
   └── Contents/
       ├── Info.plist          (bundle metadata from installers/macos)
       ├── MacOS/
       │   ├── nextar-gui      (main executable)
       │   └── nextar          (CLI, reachable via Show Package Contents)
       └── Resources/
           └── nextar.icns
   ```
   An ad-hoc `codesign --force --deep -s -` is applied so the app runs
   locally. For public distribution, replace it with a Developer ID
   identity in `installers/macos/make-app-bundle.sh`.
4. Stages a dmg volume (`nextar.app` + `nextar` CLI + `README.txt`) and
   creates a compressed read-only image with `hdiutil create -format UDZO`.

## First launch

The app is not notarized yet. If Gatekeeper blocks it, right-click
`nextar.app` → **Open**, or run:

```bash
xattr -dr com.apple.quarantine dist/nextar-0.1.0-macos.dmg
```

## Notarization (for public releases)

Ad-hoc signing is fine for local/CI builds. To distribute to other
machines without warnings:

1. Get a Developer ID Application certificate.
2. Sign with `codesign --force --deep -s "Developer ID Application: …"`.
3. `xcrun notarytool submit dist/nextar-0.1.0-macos.dmg --keychain-profile …`
4. `xcrun stapler staple dist/nextar-0.1.0-macos.dmg`
