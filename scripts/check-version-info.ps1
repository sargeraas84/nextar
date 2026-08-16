param(
    [string[]]$Paths = @(
        "$PSScriptRoot\..\target\release\nextar.exe",
        "$PSScriptRoot\..\target\release\nextar-gui.exe",
        "$PSScriptRoot\..\setup\target\release\nextar-setup.exe"
    )
)
foreach ($p in $Paths) {
    if (Test-Path $p) {
        $v = (Get-Item $p).VersionInfo
        Write-Host "--- $p"
        Write-Host ("  CompanyName   : " + $v.CompanyName)
        Write-Host ("  ProductName   : " + $v.ProductName)
        Write-Host ("  FileDesc      : " + $v.FileDescription)
        Write-Host ("  Copyright     : " + $v.LegalCopyright)
        Write-Host ("  FileVersion   : " + $v.FileVersion)
    } else {
        Write-Host "--- missing: $p"
    }
}
