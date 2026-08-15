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

# launch the GUI briefly to prove nextar-gui actually starts on macOS.
# Time-bounded: poll up to ~12s for the process to stay alive; if it exits
# early, dump the captured log + a screenshot for diagnosis.
GUI_LOG="$ROOT/gui.log"
"$APP/Contents/MacOS/nextar-gui" >"$GUI_LOG" 2>&1 &
GUI_PID=$!

gui_alive=0
for ((i = 0; i < 12; i++)); do
  if ! kill -0 "$GUI_PID" 2>/dev/null; then
    break
  fi
  sleep 1
done
if kill -0 "$GUI_PID" 2>/dev/null; then
  gui_alive=1
  kill "$GUI_PID" 2>/dev/null || true
  wait "$GUI_PID" 2>/dev/null || true
else
  echo "    --- nextar-gui exited early; last 40 log lines ---"
  tail -n 40 "$GUI_LOG" 2>/dev/null || true
  if command -v screencapture >/dev/null 2>&1; then
    screencapture -x "$ROOT/gui-crash.png" 2>/dev/null || true
    echo "    --- screenshot: $ROOT/gui-crash.png ---"
  fi
fi
check "GUI launches and stays alive" test "$gui_alive" -eq 1

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
