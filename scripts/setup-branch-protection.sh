#!/usr/bin/env bash
# Configure branch protection on `master` so merging requires the CI checks
# to pass — not just the release-tag gate. Branch protection is a repository
# setting (it can't live in the tree), so this applies it once via the
# GitHub API. Run it after the repo is pushed and `gh` is authenticated:
#
#   gh auth login
#   bash scripts/setup-branch-protection.sh <owner>/<repo> [branch]
#
# The required status checks are the job names in .github/workflows/ci.yml
# (`build-and-verify` on Windows and `macos-package` on macOS), which already
# run the full shell-verification and installer-E2E suites. This is idempotent
# (the PUT replaces the previous protection config) and leaves PR reviews
# optional so only the CI gates block a merge.
set -euo pipefail

REPO="${1:?usage: setup-branch-protection.sh <owner>/<repo> [branch]}"
BRANCH="${2:-master}"

if ! command -v gh >/dev/null 2>&1; then
  echo "error: gh CLI not found — install it (https://cli.github.com) and run 'gh auth login' first" >&2
  exit 1
fi

if ! gh auth status >/dev/null 2>&1; then
  echo "error: gh is not authenticated — run 'gh auth login' first" >&2
  exit 1
fi

gh api -X PUT "repos/$REPO/branches/$BRANCH/protection" --input - >/dev/null <<JSON
{
  "required_status_checks": {
    "strict": true,
    "contexts": ["build-and-verify", "macos-package"]
  },
  "enforce_admins": false,
  "required_pull_request_reviews": null,
  "restrictions": null,
  "required_linear_history": false,
  "allow_force_pushes": false,
  "allow_deletions": false
}
JSON

echo "branch protection enabled on $REPO@$BRANCH:"
echo "  required checks: build-and-verify, macos-package (strict)"
echo "  force pushes / deletions: disabled"
