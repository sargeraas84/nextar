#!/usr/bin/env bash
# Build the macOS installer: release binaries → nextar.app → nextar.dmg.
#
#   usage: ./installers/macos/build-dmg.sh
#   output: dist/nextar-<version>-macos.dmg   (drag nextar.app to Applications)
#
# Requirements (run ON a Mac — the .app/.dmg must be produced on macOS):
#   - Rust toolchain (rustup), Xcode Command Line Tools (clang, hdiutil)
#   - node (for the icon generator)
#   The volume ships nextar.app plus the standalone `nextar` CLI binary.
set -euo pipefail
cd "$(dirname "$0")/../.."   # project root

VERSION="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
DMG="dist/nextar-${VERSION}-macos.dmg"
STAGE="dist/dmg-staging"

echo "==> building release binaries (nextar + nextar-gui)"
cargo build --release --bin nextar --bin nextar-gui

echo "==> assembling nextar.app"
./installers/macos/make-app-bundle.sh target/release dist/app

echo "==> staging dmg volume"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp -R dist/app/nextar.app "$STAGE/"
cp target/release/nextar "$STAGE/nextar"     # CLI for terminal users
cat > "$STAGE/README.txt" <<EOF
nextar $VERSION — macOS

Drag nextar.app to your Applications folder.

Command-line tool: open Terminal, then run
  "$STAGE/nextar" --help
(or copy nextar to a folder on your PATH, e.g. /usr/local/bin).

GitHub-style notes: the app is ad-hoc signed; on first launch right-click
nextar.app > Open if Gatekeeper complains. See docs/ARCHITECTURE.md.
EOF

echo "==> creating $DMG"
rm -f "$DMG"
hdiutil create -volname "nextar $VERSION" -srcfolder "$STAGE" -ov -format UDZO "$DMG"

rm -rf "$STAGE" dist/app
echo "==> done: $DMG ($(du -h "$DMG" | cut -f1))"
