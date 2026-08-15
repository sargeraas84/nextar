#!/usr/bin/env bash
# Assemble nextar.app — the macOS application bundle — from the release
# binaries and resources. Called by build-dmg.sh; can also be run on its
# own to produce a runnable .app without the disk image.
#
#   usage: make-app-bundle.sh <release-dir> <output-dir>
#   example: ./installers/macos/make-app-bundle.sh target/release dist/app
set -euo pipefail
cd "$(dirname "$0")/../.."   # project root

RELEASE="${1:-target/release}"
OUT="${2:-dist/app}"

if [[ ! -x "$RELEASE/nextar-gui" ]]; then
  echo "error: $RELEASE/nextar-gui not found — run 'cargo build --release --bin nextar --bin nextar-gui' first" >&2
  exit 1
fi

# fresh icon (regenerates resources/nextar.icns; needs only node)
node scripts/build-icns.js >/dev/null

APP="$OUT/nextar.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$RELEASE/nextar-gui" "$APP/Contents/MacOS/nextar-gui"
cp "$RELEASE/nextar"     "$APP/Contents/MacOS/nextar"      # CLI, available inside the bundle
cp installers/macos/Info.plist "$APP/Contents/Info.plist"
cp resources/nextar.icns "$APP/Contents/Resources/nextar.icns"

# The icns embeds a PNG "nextar" name hint; make sure the plist reference
# matches the resource file (CFBundleIconFile = "nextar").

if command -v codesign >/dev/null 2>&1; then
  # ad-hoc signature: lets the app run locally without a Developer ID;
  # replace with a real identity for public distribution.
  codesign --force --deep -s - "$APP"
fi

echo "[make-app-bundle] wrote $APP ($(du -sh "$APP" | cut -f1))"
