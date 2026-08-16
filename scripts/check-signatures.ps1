param(
    [string[]]$Paths = @(
        "$PSScriptRoot\..\target\release\nextar.exe",
        "$PSScriptRoot\..\target\release\nextar-gui.exe",
        "$PSScriptRoot\..\setup\target\release\nextar-setup.exe"
    )
)
foreach ($p in $Paths) {
    if (-not (Test-Path $p)) { Write-Host "missing: $p"; continue }
    $sig = Get-AuthenticodeSignature $p
    $signer = if ($sig.SignerCertificate) { $sig.SignerCertificate.Subject } else { "(none)" }
    $ts = if ($sig.TimeStamperCertificate) { "timestamped" } else { "no timestamp" }
    Write-Host ("{0}`n  status : {1}`n  signer : {2}`n  {3}" -f (Split-Path $p -Leaf), $sig.Status, $signer, $ts)
}
