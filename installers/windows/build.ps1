# Build the Windows distribution: nextar.exe + nextar-gui.exe + nextar-setup.exe.
# Run from the project root:  powershell -ExecutionPolicy Bypass -File installers/windows/build.ps1
$ErrorActionPreference = 'Stop'

Set-Location (Join-Path $PSScriptRoot '..\..')   # project root

Write-Host '==> building nextar + nextar-gui (release)'
cargo build --release --bin nextar --bin nextar-gui
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host '==> building nextar-setup (embeds the release exes)'
cargo build --release --manifest-path setup/Cargo.toml
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

New-Item -ItemType Directory -Force -Path 'dist' | Out-Null
Copy-Item 'target/release/nextar.exe'        'dist/'
Copy-Item 'target/release/nextar-gui.exe'    'dist/'
Copy-Item 'setup/target/release/nextar-setup.exe' 'dist/'

Write-Host '==> dist/'
Get-ChildItem 'dist' | Select-Object Name, Length
Write-Host ''
Write-Host 'next: run dist/nextar-setup.exe to install (per-user, no admin).'
