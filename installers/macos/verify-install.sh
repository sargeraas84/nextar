#!/usr/bin/env bash
# macOS installer E2E: mount the .dmg, install nextar.app + the CLI into a
# throwaway "Applications" dir, upgrade in place, then uninstall - proving
# the packaging lifecycle works on macOS without touching the real
# ~/Applications or any user state.
#
#   usage: installers/macos/verify-install.sh [path-to.dmg]
#   (defaults to the newest dist/*.dmg)
set -euo pipefail
cd "$(dirname "$0")/../.."   # project root

DMG="${1:-$(ls -t dist/*.dmg 2>/dev/null | head -1)}"
if [[ -z "$DMG" || ! -f "$DMG" ]]; then
  echo "error: no .dmg found - run installers/macos/build-dmg.sh first" >&2
  exit 1
fi

ROOT="$(mktemp -d "${TMPDIR:-/tmp}/nextar-e2e.XXXXXX")"
MNT="$ROOT/mnt"
APPS="$ROOT/Applications"
mkdir -p "$MNT" "$APPS"

pass=0
fail=0
check() {  # check <name> <cmd> [args...]
  local name="$1"; shift
  if "$@" >/dev/null 2>&1; then
    echo "  [PASS] $name"; pass=$((pass + 1))
  else
    echo "  [FAIL] $name"; fail=$((fail + 1))
  fi
}
cleanup() {
  hdiutil detach "$MNT" -quiet 2>/dev/null || true
  rm -rf "$ROOT"
}
trap cleanup EXIT

echo "==> mounting $(basename "$DMG")"
hdiutil attach "$DMG" -nobrowse -readonly -mountpoint "$MNT" >/dev/null
check "dmg mounts and contains nextar.app" test -d "$MNT/nextar.app"
check "dmg contains the standalone CLI" test -x "$MNT/nextar"

APP="$APPS/nextar.app"

# 1) install: copy the bundle + CLI into the fake Applications dir
cp -R "$MNT/nextar.app" "$APP"
cp "$MNT/nextar" "$APPS/nextar"
check "install copies nextar.app" test -d "$APP"
check "app bundle has the GUI executable" test -x "$APP/Contents/MacOS/nextar-gui"
check "app bundle ships the CLI" test -x "$APP/Contents/MacOS/nextar"
check "app bundle has Info.plist" test -f "$APP/Contents/Info.plist"
check "app bundle has the icon" test -f "$APP/Contents/Resources/nextar.icns"
check "installed CLI runs" "$APPS/nextar" --version

# 2) upgrade in place: copy over the existing bundle + CLI
cp -R "$MNT/nextar.app" "$APPS/"
cp "$MNT/nextar" "$APPS/nextar"
check "upgrade re-copies over the bundle" test -x "$APP/Contents/MacOS/nextar-gui"
bundle_count=$(find "$APPS" -maxdepth 1 -name '*.app' -type d | wc -l | tr -d ' ')
check "upgrade leaves a single bundle" test "$bundle_count" -eq 1
check "upgraded CLI still runs" "$APPS/nextar" --version

# 3) uninstall
rm -rf "$APP" "$APPS/nextar"
check "uninstall removes nextar.app" test ! -e "$APP"
check "uninstall removes the CLI" test ! -e "$APPS/nextar"

echo
echo "$pass passed, $fail failed"
[[ $fail -eq 0 ]]
