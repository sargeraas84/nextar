# Build the Windows distribution: nextar.exe + nextar-gui.exe + nextar-setup.exe.
# Run from the project root:  powershell -ExecutionPolicy Bypass -File installers/windows/build.ps1
#
# Optional code signing: if NEXTAR_SIGN_CERTFILE and NEXTAR_SIGN_PASSWORD are
# set, the three dist exes are signed with SHA-256 + an RFC-3161 timestamp
# using signtool.exe from the Windows SDK. Otherwise signing is skipped with a
# note — the installer still works, just unsigned.
$ErrorActionPreference = 'Stop'

Set-Location (Join-Path $PSScriptRoot '..\..')   # project root

function Find-Signtool {
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    if (-not (Test-Path $kits)) { return $null }
    Get-ChildItem -Path $kits -Recurse -Filter 'signtool.exe' -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
}

Write-Host '==> building nextar + nextar-gui (release)'
cargo build --release --bin nextar --bin nextar-gui
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host '==> building nextar-setup (embeds the release exes)'
cargo build --release --manifest-path setup/Cargo.toml
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

New-Item -ItemType Directory -Force -Path 'dist' | Out-Null
Copy-Item 'target/release/nextar.exe'             'dist/'
Copy-Item 'target/release/nextar-gui.exe'         'dist/'
Copy-Item 'setup/target/release/nextar-setup.exe' 'dist/'

# ---- best-effort code signing -------------------------------------------
if ($env:NEXTAR_SIGN_CERTFILE) {
    $signtool = Find-Signtool
    if (-not $signtool) {
        Write-Warning 'code signing skipped: signtool.exe not found (install the Windows SDK)'
    }
    elseif (-not $env:NEXTAR_SIGN_PASSWORD) {
        Write-Warning 'code signing skipped: NEXTAR_SIGN_PASSWORD not set'
    }
    else {
        foreach ($exe in @('dist/nextar.exe', 'dist/nextar-gui.exe', 'dist/nextar-setup.exe')) {
            & $signtool.FullName sign /f $env:NEXTAR_SIGN_CERTFILE /p $env:NEXTAR_SIGN_PASSWORD `
                /fd SHA256 /tr 'http://timestamp.digicert.com' /td SHA256 $exe
            if ($LASTEXITCODE -ne 0) { throw "signtool failed for $exe (code $LASTEXITCODE)" }
        }
        Write-Host '==> code signing complete'
    }
}
else {
    Write-Host '==> code signing skipped (set NEXTAR_SIGN_CERTFILE + NEXTAR_SIGN_PASSWORD to sign)'
}

Write-Host '==> dist/'
Get-ChildItem 'dist' | Select-Object Name, Length
Write-Host ''
Write-Host 'next: run dist/nextar-setup.exe to install (per-user, no admin).'
