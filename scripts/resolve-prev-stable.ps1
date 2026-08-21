# Resolves the previous stable release tag for the cross-version upgrade
# leg, with a test seam so CI can exercise the "no previous stable" branch
# without a real repo.
#
#   usage:
#     scripts/resolve-prev-stable.ps1 -Tag v0.3.7 [-AllowNoPrev] [-PrevTag <tag>]
#
# Always exits 0 and prints exactly one token to stdout:
#   <tag>  - the previous stable tag was resolved.
#   SKIP   - no previous stable, and -AllowNoPrev granted (opt-out).
#   NONE   - no previous stable, and -AllowNoPrev NOT granted.
# The caller decides how to act on SKIP vs NONE.
#
# The stable lookup lists releases and filters to non-draft, non-prerelease,
# non-`nightly` entries (releases/latest would 404 on a brand-new repo).
# Invoke-RestMethod is used instead of `gh api --jq`, which returned empty on
# the Windows runner (native-arg/jq parsing flakiness).
#
# Test seam: pass -PrevTag to bypass the API entirely (an empty -PrevTag ''
# simulates a repo with no stable release yet).
#
# Env: GH_TOKEN + GITHUB_REPOSITORY (used when -PrevTag is not supplied).
param(
  [Parameter(Mandatory = $true)][string]$Tag,
  [switch]$AllowNoPrev,
  [string]$PrevTag
)

if ($PSBoundParameters.ContainsKey('PrevTag')) {
  $prev = $PrevTag
} else {
  $repo = $env:GITHUB_REPOSITORY
  if (-not $repo) { Write-Output "NONE"; return }
  $headers = @{ Authorization = "Bearer $env:GH_TOKEN"; "User-Agent" = "nextar-release-ci" }
  $releases = @(Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/$repo/releases?per_page=100")
  $stable = $releases |
    Where-Object { -not $_.draft -and -not $_.prerelease -and $_.tag_name -ne 'nightly' } |
    Select-Object -First 1
  $prev = if ($stable) { $stable.tag_name } else { $null }
}

if (-not $prev -or $prev -eq $Tag) {
  if ($AllowNoPrev) {
    Write-Output "SKIP"
  } else {
    Write-Output "NONE"
  }
  return
}

Write-Output $prev
