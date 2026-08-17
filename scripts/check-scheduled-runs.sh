#!/usr/bin/env bash
# One-command verification for the schedule-fired (no-push) nightly + smoke
# runs. The nightly workflow is scheduled for 04:37 UTC and the smoke for
# 05:47 UTC daily; this checks that BOTH fired via the `schedule` event
# (not a push), completed green, and that the release body carries that
# day's smoke results.
#
#   usage: scripts/check-scheduled-runs.sh [owner/repo] [-n N]
#     -n N    check the last N days instead of today only (default 1; the
#             monthly audit workflow uses -n 31). Range mode asserts each of
#             the last N days had a schedule-event nightly + smoke run that
#             completed success, and skips the day-specific release-body
#             checks.
#
# Exit 0 when everything is green; non-zero with a clear message otherwise.
set -euo pipefail
cd "$(dirname "$0")/.."

RANGE=1
REPO=""
while [ $# -gt 0 ]; do
  case "$1" in
    -n|--range) RANGE="$2"; shift 2 ;;
    *) REPO="$1"; shift ;;
  esac
done
REPO="${REPO:-$(git remote get-url origin 2>/dev/null | sed -E 's#.*[:/]([^/]+/[^/.]+)(\.git)?$#\1#' || true)}"
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

# --- range mode: every one of the last N days must have had a green
# schedule-event run of BOTH workflows. Used by the monthly audit; skips the
# day-specific release-body checks (the body only carries the latest day).
if [ "$RANGE" -gt 1 ]; then
  echo "== nextar schedule verification for the last $RANGE days of $REPO (UTC) =="
  bad=""
  for wf in nightly.yml smoke.yml; do
    echo "-- $wf --"
    # date + conclusion per schedule-event run (100 = ~3 months of dailies).
    RUNS=$(gh run list --repo "$REPO" --workflow "$wf" --event schedule --limit 100 --json conclusion,createdAt --jq '.[] | "\(.createdAt[0:10]) \(.conclusion)"' 2>/dev/null || true)
    # Days before the workflow's first-ever schedule run can't be expected to
    # have one (the repo may not have existed yet) — those days are skipped.
    FIRST=$(printf '%s\n' "$RUNS" | awk 'NF { print $1 }' | sort | head -1)
    ok=0; skipped=0; flagged=""
    for ((i = 0; i < RANGE; i++)); do
      D=$(date -u -d "-$i days" +%F 2>/dev/null || date -u -v-${i}d +%F)
      if [ -n "$FIRST" ] && [[ "$D" < "$FIRST" ]]; then
        skipped=$((skipped + 1))
        continue
      fi
      # green iff at least one success and no failure that day.
      if printf '%s\n' "$RUNS" | awk -v d="$D" '$1 == d && $2 == "success" { s = 1 } $1 == d && $2 == "failure" { f = 1 } END { exit !(s == 1 && f != 1) }'; then
        ok=$((ok + 1))
      else
        flagged="$flagged $D"
      fi
    done
    checked=$((RANGE - skipped))
    echo "  $ok/$checked days green (first run $FIRST)${flagged:+ — flagged:$flagged}"
    [ -n "$flagged" ] && { fail=1; bad="$bad [$wf:$flagged]"; }
  done
  echo
  if [ "$fail" -eq 0 ]; then
    echo "ALL GREEN — every one of the last $RANGE days had a green schedule-fired nightly and smoke."
  else
    echo "SOME CHECKS FAILED — missed or failed schedule slots:$bad"
  fi
  exit "$fail"
fi

echo "== nextar schedule verification for $(date -u +%F) (UTC) =="

# Find the most recent schedule-event run of each workflow. gh run list
# with --event schedule filters to runs that fired from cron (no push).
NIGHTLY_RUN=$(gh run list --repo "$REPO" --workflow nightly.yml --event schedule --limit 1 --json databaseId,conclusion,createdAt --jq '.[0]' 2>/dev/null || echo '')
SMOKE_RUN=$(gh run list --repo "$REPO" --workflow smoke.yml --event schedule --limit 1 --json databaseId,conclusion,createdAt --jq '.[0]' 2>/dev/null || echo '')

# The run JSON is a single line with fields databaseId, conclusion and
# createdAt (ISO 8601). Extract with awk/grep to avoid a python dependency;
# the date comparison slices the ISO timestamp (first 10 chars = YYYY-MM-DD)
# so it works on GNU and BSD date alike.
if [ -n "$NIGHTLY_RUN" ]; then
  NID=$(printf '%s' "$NIGHTLY_RUN" | sed -E 's/.*"databaseId":([0-9]+).*/\1/')
  NC=$(printf '%s' "$NIGHTLY_RUN" | sed -E 's/.*"conclusion":"([^"]*)".*/\1/')
  NT=$(printf '%s' "$NIGHTLY_RUN" | sed -E 's/.*"createdAt":"([^"]*)".*/\1/')
  NDAY=${NT:0:10}
  echo "nightly (schedule): run $NID at $NT -> $NC"
  check "nightly schedule run completed" test "$NC" = success
  check "nightly schedule run is today" test "$NDAY" = "$(date -u +%F)"
else
  echo "  [FAIL] no schedule-event nightly run found"
  fail=1
fi

if [ -n "$SMOKE_RUN" ]; then
  SID=$(printf '%s' "$SMOKE_RUN" | sed -E 's/.*"databaseId":([0-9]+).*/\1/')
  SC=$(printf '%s' "$SMOKE_RUN" | sed -E 's/.*"conclusion":"([^"]*)".*/\1/')
  ST=$(printf '%s' "$SMOKE_RUN" | sed -E 's/.*"createdAt":"([^"]*)".*/\1/')
  SDAY=${ST:0:10}
  echo "smoke (schedule): run $SID at $ST -> $SC"
  check "smoke schedule run completed" test "$SC" = success
  check "smoke schedule run is today" test "$SDAY" = "$(date -u +%F)"
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
