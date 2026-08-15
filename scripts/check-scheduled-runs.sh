#!/usr/bin/env bash
# One-command verification for the schedule-fired (no-push) nightly + smoke
# runs. The nightly workflow is scheduled for 04:37 UTC and the smoke for
# 05:47 UTC daily; this checks that BOTH fired via the `schedule` event
# (not a push), completed green, and that the release body carries that
# day's smoke results.
#
#   usage: scripts/check-scheduled-runs.sh [owner/repo]   (default: from git)
#
# Exit 0 when everything is green; non-zero with a clear message otherwise.
set -euo pipefail
cd "$(dirname "$0")/.."

REPO="${1:-$(git remote get-url origin 2>/dev/null | sed -E 's#.*[:/]([^/]+/[^/.]+)(\.git)?$#\1#' || true)}"
REPO="${REPO:-sargeraas84/nextar}"

command -v gh >/dev/null 2>&1 || { echo "error: gh CLI not found" >&2; exit 2; }

fail=0
check() {  # check <name> <cond...>
  local name="$1"; shift
  if "$@" >/dev/null 2>&1; then
    echo "  [PASS] $name"
  else
    echo "  [FAIL] $name"
    fail=1
  fi
}

echo "== nextar schedule verification for $(date -u +%F) (UTC) =="

# Find the most recent schedule-event run of each workflow. gh run list
# with --event schedule filters to runs that fired from cron (no push).
NIGHTLY_RUN=$(gh run list --repo "$REPO" --workflow nightly.yml --event schedule --limit 1 --json databaseId,conclusion,createdAt --jq '.[0]' 2>/dev/null || echo '')
SMOKE_RUN=$(gh run list --repo "$REPO" --workflow smoke.yml --event schedule --limit 1 --json databaseId,conclusion,createdAt --jq '.[0]' 2>/dev/null || echo '')

if [ -n "$NIGHTLY_RUN" ]; then
  NID=$(printf '%s' "$NIGHTLY_RUN" | python -c "import json,sys; d=json.load(sys.stdin); print(d['databaseId'])")
  NC=$(printf '%s' "$NIGHTLY_RUN" | python -c "import json,sys; print(json.load(sys.stdin)['conclusion'])")
  NT=$(printf '%s' "$NIGHTLY_RUN" | python -c "import json,sys; print(json.load(sys.stdin)['createdAt'])")
  echo "nightly (schedule): run $NID at $NT -> $NC"
  check "nightly schedule run completed" test "$NC" = success
  check "nightly schedule run is today" bash -c "test \"\$(date -u -d '$NT' +%F)\" = \"\$(date -u +%F)\""
else
  echo "  [FAIL] no schedule-event nightly run found"
  fail=1
fi

if [ -n "$SMOKE_RUN" ]; then
  SID=$(printf '%s' "$SMOKE_RUN" | python -c "import json,sys; d=json.load(sys.stdin); print(d['databaseId'])")
  SC=$(printf '%s' "$SMOKE_RUN" | python -c "import json,sys; print(json.load(sys.stdin)['conclusion'])")
  ST=$(printf '%s' "$SMOKE_RUN" | python -c "import json,sys; print(json.load(sys.stdin)['createdAt'])")
  echo "smoke (schedule): run $SID at $ST -> $SC"
  check "smoke schedule run completed" test "$SC" = success
  check "smoke schedule run is today" bash -c "test \"\$(date -u -d '$ST' +%F)\" = \"\$(date -u +%F)\""
else
  echo "  [FAIL] no schedule-event smoke run found"
  fail=1
fi

# The release body should carry today's smoke results section.
BODY=$(gh api "repos/$REPO/releases/tags/nightly" --jq '.body' 2>/dev/null || true)
TODAY=$(date -u +%F)
check "release body has a Daily smoke results section" bash -c "printf '%s' \"\$1\" | grep -q '^## Daily smoke results'" _ "$BODY"
check "release body smoke section is for today" bash -c "printf '%s' \"\$1\" | grep -q '$TODAY'" _ "$BODY"

echo
if [ "$fail" -eq 0 ]; then
  echo "ALL GREEN — the schedule-fired nightly and smoke both ran today."
else
  echo "SOME CHECKS FAILED — see above."
fi
exit "$fail"
