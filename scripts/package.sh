#!/usr/bin/env bash
# Build the full nextar distribution:
#   dist/nextar.exe        command-line tool
#   dist/nextar-gui.exe    desktop app
#   dist/nextar-setup.exe  installer (bundles the two exes + Explorer integration)
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> regenerating icons + site assets (mark is the source of truth)"
node scripts/generate-icon.js
node scripts/generate-icon.js --dark
node scripts/build-site-assets.js

echo "==> building nextar + nextar-gui (release)"
cargo build --release --bin nextar --bin nextar-gui

echo "==> building nextar-setup (embeds the release exes)"
cargo build --release --manifest-path setup/Cargo.toml

mkdir -p dist
cp target/release/nextar.exe dist/
cp target/release/nextar-gui.exe dist/
cp setup/target/release/nextar-setup.exe dist/

# Sign the payload with the Michael Rieger code-signing cert when available
# (scripts/sign.ps1 creates it on first run). Best effort: if signtool or the
# cert is missing, the build still succeeds — signing is a distribution nicety.
if command -v powershell >/dev/null 2>&1; then
    echo "==> signing dist/ payload (best effort)"
    # sign.ps1's default path list covers target/release + dist/.
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/sign.ps1 2>&1 | sed 's/^/    /' || echo "    (signing skipped - install Windows SDK signtool or run scripts/sign.ps1)"
fi

echo "==> dist/"
ls -la dist/
echo
echo "next: run dist/nextar-setup.exe to install (per-user, no admin)."
