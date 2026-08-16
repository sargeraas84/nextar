<#
.SYNOPSIS
    Export the "Michael Rieger" self-signed code-signing certificate.

.DESCRIPTION
    Writes two files:
      - <OutDir>\nextar-rieger.pfx   (cert + private key, password protected)
      - <OutDir>\nextar-rieger.cer   (public cert only, for trusting on other machines)

    The .pfx lets CI sign builds (decode it to base64 and store in the
    CODE_SIGN_PFX secret with CODE_SIGN_PFX_PASS). The .cer is what other
    users import into their Trusted Root store so Explorer stops warning.

    SECURITY: keep the .pfx private - anyone with it can sign as you.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts/export-signing-cert.ps1
    powershell -ExecutionPolicy Bypass -File scripts/export-signing-cert.ps1 -OutDir .\dist\certs -Password 'change-me'
#>
param(
    [string]$OutDir = "$PSScriptRoot\..\dist\certs",
    [string]$CertSubject = "Michael Rieger",
    $PfxPassword = ""
)

$ErrorActionPreference = "Stop"

$cert = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert -ErrorAction SilentlyContinue |
    Where-Object { $_.Subject -like "*$CertSubject*" } |
    Select-Object -First 1
if (-not $cert) {
    Write-Host "No '$CertSubject' code-signing cert found - run scripts/sign.ps1 first." -ForegroundColor Yellow
    exit 1
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$pfxPath = Join-Path $OutDir "nextar-rieger.pfx"
$cerPath = Join-Path $OutDir "nextar-rieger.cer"

if (-not $PfxPassword) {
    $PfxPassword = Read-Host -AsSecureString "PFX password (protects the private key)"
    if ($PfxPassword.Length -lt 4) { throw "password too short" }
} else {
    $PfxPassword = ConvertTo-SecureString $PfxPassword -AsPlainText -Force
}

Export-PfxCertificate -Cert $cert -FilePath $pfxPath -Password $PfxPassword | Out-Null
Export-Certificate -Cert $cert -FilePath $cerPath -Type CERT | Out-Null

Write-Host ""
Write-Host "Exported:"
Write-Host "  private: $pfxPath"
Write-Host "  public : $cerPath"
Write-Host ""
Write-Host "To sign in CI:"
Write-Host "  `$pfxB64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes('$pfxPath'))"
Write-Host "  gh secret set CODE_SIGN_PFX -b `$pfxB64   (and CODE_SIGN_PFX_PASS with the password)"
Write-Host ""
Write-Host "To trust on another machine (Explorer stops warning there):"
Write-Host ('  Import-Certificate -FilePath ''' + $cerPath + ''' -CertStoreLocation Cert:\CurrentUser\Root')
