param([string]$Path)
$tokens = $null
$errs = $null
[void][System.Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$errs)
if ($errs.Count -eq 0) {
    Write-Host "PARSE OK: $Path"
} else {
    $errs | ForEach-Object { Write-Host ("LINE {0}: {1}" -f $_.Extent.StartLineNumber, $_.Message) }
    exit 1
}
