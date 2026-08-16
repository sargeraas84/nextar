<#
.SYNOPSIS
    Sign nextar's Windows binaries with a self-signed "Michael Rieger"
    code-signing certificate.

.DESCRIPTION
    Creates (once) a self-signed CodeSigningCert named "Michael Rieger" in
    the current user's Personal store, copies it into TrustedPeople and the
    per-user Trusted Root store, then signs every built exe with SHA-256 +
    RFC3161 timestamp.

    IMPORTANT: a self-signed cert is trusted only on machines that have the
    cert in their root store. Explorer's "unknown publisher" warning goes
    away on THIS machine (and any machine you install the cert on); other
    machines still need the cert imported, or a real CA-signed cert.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts/sign.ps1
#>
param(
    [string[]]$Paths = @(
        "$PSScriptRoot\..\target\release\nextar.exe",
        "$PSScriptRoot\..\target\release\nextar-gui.exe",
        "$PSScriptRoot\..\setup\target\release\nextar-setup.exe",
        "$PSScriptRoot\..\dist\nextar-setup.exe"
    ),
    [string]$CertSubject = "Michael Rieger",
    [string]$TimestampUrl = "http://timestamp.digicert.com"
)

$ErrorActionPreference = "Stop"

# Locate signtool from the newest Windows SDK kit.
$kits = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin" -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '^\d+\.' } |
    Sort-Object { [version]$_.Name } -Descending
$signtool = $null
foreach ($kit in $kits) {
    $cand = Join-Path $kit.FullName "x64\signtool.exe"
    if (Test-Path $cand) { $signtool = $cand; break }
}
if (-not $signtool) {
    throw "signtool.exe not found under Windows Kits\10\bin"
}
Write-Host "signtool: $signtool"

# Reuse an existing cert if one is already in the store.
$cert = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert -ErrorAction SilentlyContinue |
    Where-Object { $_.Subject -like "*$CertSubject*" } |
    Select-Object -First 1
if (-not $cert) {
    Write-Host "Creating self-signed code-signing cert for '$CertSubject' ..."
    $cert = New-SelfSignedCertificate `
        -Type CodeSigningCert `
        -Subject "CN=$CertSubject, O=$CertSubject" `
        -KeyUsage DigitalSignature `
        -KeyExportPolicy Exportable `
        -CertStoreLocation Cert:\CurrentUser\My `
        -NotAfter (Get-Date).AddYears(5)
    # Trust it locally: per-user TrustedPeople + Trusted Root stores (no
    # admin needed). Export the public cert to a temp .cer and import it into
    # both stores (Copy-Item on cert drives is not supported in PS 5.1).
    $cer = Join-Path $env:TEMP "nextar-rieger.cer"
    Export-Certificate -Cert $cert -FilePath $cer -Type CERT | Out-Null
    if (-not (Get-ChildItem Cert:\CurrentUser\TrustedPeople | Where-Object { $_.Thumbprint -eq $cert.Thumbprint })) {
        Import-Certificate -FilePath $cer -CertStoreLocation Cert:\CurrentUser\TrustedPeople | Out-Null
    }
    if (-not (Get-ChildItem Cert:\CurrentUser\Root | Where-Object { $_.Thumbprint -eq $cert.Thumbprint })) {
        Import-Certificate -FilePath $cer -CertStoreLocation Cert:\CurrentUser\Root | Out-Null
    }
    Remove-Item $cer -ErrorAction SilentlyContinue
} else {
    Write-Host "Reusing existing cert: $($cert.Thumbprint)"
}

# Make sure the cert is also trusted in the root store (re-run safety).
if (-not (Get-ChildItem Cert:\CurrentUser\Root | Where-Object { $_.Thumbprint -eq $cert.Thumbprint })) {
    $cer = Join-Path $env:TEMP "nextar-rieger.cer"
    Export-Certificate -Cert $cert -FilePath $cer -Type CERT | Out-Null
    Import-Certificate -FilePath $cer -CertStoreLocation Cert:\CurrentUser\Root | Out-Null
    Remove-Item $cer -ErrorAction SilentlyContinue
}

$existing = @(Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert | Where-Object { $_.Subject -like "*$CertSubject*" } |
    Select-Object -First 1)
if ($existing) { $cert = $existing }

foreach ($p in $Paths | Select-Object -Unique) {
    if (-not (Test-Path $p)) { Write-Host "skip (missing): $p"; continue }
    Write-Host "Signing: $p"
    & $signtool sign /fd SHA256 /sha1 $cert.Thumbprint /tr $TimestampUrl /td SHA256 $p
    if ($LASTEXITCODE -ne 0) { throw "signtool failed for $p" }
}

Write-Host ""
Write-Host "Done. Verify with: Get-AuthenticodeSignature <path>"
