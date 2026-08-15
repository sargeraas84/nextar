# nextar - Explorer shell-integration verification
#
# Checks the right-click integration at three levels:
#   1. Registry   - every expected verb key exists with a valid command
#   2. Explorer   - the shell itself resolves the verbs (Shell.Application,
#                   the same COM surface Explorer's menu uses)
#   3. Functional - optionally (-Run) actually invokes the verbs and checks
#                   the produced artifacts
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-shell.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-shell.ps1 -Run
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-shell.ps1 -Installer
#
# Exit code: 0 = all checks passed, 1 = one or more failed.

param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'nextar'),
    [switch]$Run,        # invoke verbs for real (brief progress windows open)
    [switch]$Installer,  # headless installer E2E: install, upgrade, uninstall
    [string]$SetupExe = ''  # explicit nextar-setup.exe path for the installer E2E
)
$ErrorActionPreference = 'Stop'

$script:pass = 0
$script:fail = 0
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    if ($ok) {
        $script:pass++
        Write-Host "  [PASS] $name" -ForegroundColor Green
    } else {
        $script:fail++
        $msg = "  [FAIL] $name"
        if ($detail) { $msg += " - $detail" }
        Write-Host $msg -ForegroundColor Red
    }
}

Write-Host "nextar shell integration - verifying install at $InstallDir"
$gui = Join-Path $InstallDir 'nextar-gui.exe'
$cli = Join-Path $InstallDir 'nextar.exe'
Check "nextar-gui.exe present" (Test-Path $gui)
Check "nextar.exe present" (Test-Path $cli)
if (-not (Test-Path $gui)) {
    Write-Host "nothing to verify - run nextar-setup.exe first" -ForegroundColor Yellow
    exit 1
}

# ---------------------------------------------------------------- registry
function VerbOk([string]$root, [string]$verb) {
    # -LiteralPath: the file root is '*' and would be treated as a wildcard,
    # making Test-Path enumerate the whole Classes hive (near-hang).
    $key = "HKCU:\Software\Classes\$root\shell\$verb"
    $cmd = "$key\command"
    if (-not (Test-Path -LiteralPath $key)) { return $false }
    if (-not (Test-Path -LiteralPath $cmd)) { return $false }
    $c = (Get-Item -LiteralPath $cmd).GetValue('')
    if (-not $c)                 { return $false }
    return ($c -match 'nextar-gui\.exe')
}

Write-Host "`n-- 1) registry" -ForegroundColor Cyan
foreach ($v in 'NextarCompress', 'NextarEmail') {
    Check "file verb '$v' registered with valid command" (VerbOk '*' $v)
}
foreach ($v in 'NextarCompress', 'NextarEmail', 'NextarExtractInto') {
    Check "folder verb '$v' registered with valid command" (VerbOk 'Directory' $v)
}
foreach ($v in 'NextarOpen', 'NextarExtract', 'NextarRepair') {
    Check ".next verb '$v' registered with valid command" (VerbOk 'SystemFileAssociations\.next' $v)
}
$sendto = Join-Path $env:APPDATA 'Microsoft\Windows\SendTo\Compress to .next.lnk'
Check "SendTo shortcut present" (Test-Path $sendto)

# ---------------------------------------------------------------- explorer
Write-Host "`n-- 2) explorer verb resolution" -ForegroundColor Cyan
$work = Join-Path $env:TEMP ("nextar-verify-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
$sub = Join-Path $work 'folder'
New-Item -ItemType Directory -Path $sub | Out-Null
$file = Join-Path $work 'note.txt'
Set-Content -Path $file -Value 'nextar shell verify' -Encoding UTF8

$shell = New-Object -ComObject Shell.Application
function Get-Verbs([string]$path) {
    $ns = $shell.NameSpace((Split-Path $path -Parent))
    $item = $ns.ParseName((Split-Path $path -Leaf))
    $names = @()
    if ($null -ne $item) {
        foreach ($v in $item.Verbs()) { $names += $v.Name }
    }
    ,$names
}

$fileVerbs = Get-Verbs $file
$hasFile = ($fileVerbs -contains 'Compress to .next') -or ($fileVerbs -contains 'NextarCompress')
Check "Explorer shows Compress on a FILE" $hasFile ("verbs: " + ($fileVerbs -join ', '))

$folderVerbs = Get-Verbs $sub
$hasFolder = ($folderVerbs -contains 'Compress to .next') -or ($folderVerbs -contains 'NextarCompress')
Check "Explorer shows Compress on a FOLDER" $hasFolder ("verbs: " + ($folderVerbs -join ', '))

# real .next archive, via the CLI, for the .next-file checks
$arch = Join-Path $work 'sample.next'
& $cli create $file -o $arch -f -q 2>&1 | Out-Null
Check "CLI created a test .next archive" (Test-Path $arch)
if (Test-Path $arch) {
    $nextVerbs = Get-Verbs $arch
    $hasOpen = ($nextVerbs -contains 'Open in nextar') -or ($nextVerbs -contains 'NextarOpen')
    $hasExtract = ($nextVerbs -contains 'Extract here') -or ($nextVerbs -contains 'NextarExtract')
    # display name ends with the unicode ellipsis - match by prefix
    $hasRepair = ($nextVerbs -contains 'NextarRepair') -or [bool]($nextVerbs | Where-Object { $_ -like 'Repair with .nvol*' })
    Check "Explorer shows Open in nextar on a .next" $hasOpen ("verbs: " + ($nextVerbs -join ', '))
    Check "Explorer shows Extract here on a .next" $hasExtract
    Check "Explorer shows Repair with .nvol on a .next" $hasRepair
}

# ---------------------------------------------------------------- functional
if ($Run) {
    Write-Host "`n-- 3) invoking verbs" -ForegroundColor Cyan
    $ns = $shell.NameSpace($work)

    # right-click file -> Compress to .next
    $item = $ns.ParseName('note.txt')
    $item.InvokeVerb('NextarCompress')
    $outFile = Join-Path $work 'note.txt.next'
    for ($i = 0; $i -lt 30 -and -not (Test-Path $outFile); $i++) { Start-Sleep -Milliseconds 500 }
    Check "InvokeVerb(Compress) on file produced note.txt.next" (Test-Path $outFile)

    # right-click folder -> Compress to .next
    $item = $ns.ParseName('folder')
    $item.InvokeVerb('NextarCompress')
    $outFolder = Join-Path $work 'folder.next'
    for ($i = 0; $i -lt 30 -and -not (Test-Path $outFolder); $i++) { Start-Sleep -Milliseconds 500 }
    Check "InvokeVerb(Compress) on folder produced folder.next" (Test-Path $outFolder)

    # right-click .next -> Extract here. The output folder takes the archive
    # stem, so use a stem that can't collide with the source file
    # (extracting note.txt.next with note.txt present would - correctly -
    # refuse to overwrite and show an error).
    $sample = Join-Path $work 'sample.next'
    & $cli create $file -o $sample -f -q 2>&1 | Out-Null
    if (Test-Path $sample) {
        $item = $ns.ParseName('sample.next')
        $item.InvokeVerb('NextarExtract')
        $extracted = Join-Path $work 'sample\note.txt'
        for ($i = 0; $i -lt 30 -and -not (Test-Path $extracted); $i++) { Start-Sleep -Milliseconds 500 }
        Check "InvokeVerb(Extract here) produced sample\note.txt" (Test-Path $extracted)
    }

    # right-click .next -> Repair with .nvol: build a recovery archive,
    # corrupt it, invoke the Repair verb, and re-verify the repaired output.
    $rsrc = Join-Path $work 'repair-src'
    New-Item -ItemType Directory -Path $rsrc | Out-Null
    $rand = Join-Path $rsrc 'rand.bin'
    $bytes = New-Object byte[] (400 * 1024)   # incompressible -> archive stays big
    (New-Object Random 12345).NextBytes($bytes)
    [IO.File]::WriteAllBytes($rand, $bytes)
    $rarch = Join-Path $work 'repair.next'
    $rvol = Join-Path $work 'repair.next.nvol'
    & $cli create $rsrc -o $rarch -b 128K -r 8 -f -q 2>&1 | Out-Null
    Check "CLI created recovery archive + .nvol" ((Test-Path $rarch) -and (Test-Path $rvol))
    if (Test-Path $rarch) {
        # flip bytes in two data regions (past the header)
        $data = [IO.File]::ReadAllBytes($rarch)
        $data[100000] = 0xDE; $data[100001] = 0xAD; $data[100002] = 0xBE; $data[100003] = 0xEF
        $data[250000] = 0xFF; $data[250001] = 0xFF; $data[250002] = 0xFF; $data[250003] = 0xFF
        [IO.File]::WriteAllBytes($rarch, $data)

        # verify *should* fail here - an expected non-zero exit becomes a
        # terminating error under ErrorActionPreference=Stop, so drop it
        # for this call and rely on the captured output instead.
        $old = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        $v = & $cli verify $rarch -q 2>&1 | Out-String
        $ErrorActionPreference = $old
        Check "verify detects the corruption" ($v -match 'corrupt')

        $item = $ns.ParseName('repair.next')
        $item.InvokeVerb('NextarRepair')
        $repaired = Join-Path $work 'repair.repaired.next'
        for ($i = 0; $i -lt 30 -and -not (Test-Path $repaired); $i++) { Start-Sleep -Milliseconds 500 }
        Check "InvokeVerb(Repair) produced repair.repaired.next" (Test-Path $repaired)
        if (Test-Path $repaired) {
            $old = $ErrorActionPreference
            $ErrorActionPreference = 'Continue'
            $rv = & $cli verify $repaired -q 2>&1 | Out-String
            $ErrorActionPreference = $old
            Check "repaired archive verifies clean" ($rv -match 'all ok')
        }
    }

    # encrypted variant: password-protected archive + .nvol. Repair is
    # password-agnostic (it fixes the encrypted bytes with parity); the
    # password is only needed afterwards to extract.
    $ersrc = Join-Path $work 'enc-repair-src'
    New-Item -ItemType Directory -Path $ersrc | Out-Null
    $erand = Join-Path $ersrc 'rand.bin'
    $ebytes = New-Object byte[] (400 * 1024)
    (New-Object Random 6789).NextBytes($ebytes)
    [IO.File]::WriteAllBytes($erand, $ebytes)
    $earch = Join-Path $work 'enc.next'
    $evol = Join-Path $work 'enc.next.nvol'
    & $cli create $ersrc -o $earch -b 128K -r 8 -p secret -f -q 2>&1 | Out-Null
    Check "CLI created encrypted recovery archive + .nvol" ((Test-Path $earch) -and (Test-Path $evol))
    if (Test-Path $earch) {
        $data = [IO.File]::ReadAllBytes($earch)
        $data[100000] = 0xAA; $data[100001] = 0xBB; $data[100002] = 0xCC; $data[100003] = 0xDD
        $data[250000] = 0x00; $data[250001] = 0x00; $data[250002] = 0x00; $data[250003] = 0x00
        [IO.File]::WriteAllBytes($earch, $data)

        # encrypted blocks are only fully checkable with the password (the
        # GCM tag catches the flipped bytes), so pass it to verify.
        $old = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        $v = & $cli verify $earch -p secret -q 2>&1 | Out-String
        $ErrorActionPreference = $old
        Check "encrypted verify detects the corruption" ($v -match 'corrupt')

        $item = $ns.ParseName('enc.next')
        $item.InvokeVerb('NextarRepair')
        $repaired = Join-Path $work 'enc.repaired.next'
        for ($i = 0; $i -lt 30 -and -not (Test-Path $repaired); $i++) { Start-Sleep -Milliseconds 500 }
        Check "InvokeVerb(Repair) produced enc.repaired.next" (Test-Path $repaired)
        if (Test-Path $repaired) {
            $old = $ErrorActionPreference
            $ErrorActionPreference = 'Continue'
            $rv = & $cli verify $repaired -p secret -q 2>&1 | Out-String
            $ErrorActionPreference = $old
            Check "encrypted repaired archive verifies clean" ($rv -match 'all ok')

            # the encryption survived the repair: no password -> refused
            $old = $ErrorActionPreference
            $ErrorActionPreference = 'Continue'
            $bad = & $cli extract $repaired -o (Join-Path $work 'enc-out-bad') -q 2>&1 | Out-String
            $ErrorActionPreference = $old
            Check "encrypted repaired archive still requires the password" ($bad -match 'encrypted|password')

            # right password -> content matches the original byte-for-byte
            $okout = Join-Path $work 'enc-out-ok'
            & $cli extract $repaired -o $okout -p secret -q 2>&1 | Out-Null
            $extracted = Join-Path $okout 'enc-repair-src\rand.bin'
            Check "encrypted repaired archive extracts with the password" (Test-Path $extracted)
            if (Test-Path $extracted) {
                $h1 = (Get-FileHash $erand -Algorithm SHA256).Hash
                $h2 = (Get-FileHash $extracted -Algorithm SHA256).Hash
                Check "extracted content matches the original" ($h1 -eq $h2)
            }
        }
    }
}

# ---------------------------------------------------------------- installer E2E
if ($Installer) {
    Write-Host "`n-- 4) headless installer E2E (install -> upgrade -> uninstall)" -ForegroundColor Cyan

    # Locate a release setup exe (it embeds the nextar payload). Prefer an
    # explicit -SetupExe, then the installed copy, then the dev dist build.
    $setup = $null
    $candidates = @()
    if ($SetupExe) { $candidates += $SetupExe }
    $candidates += (Join-Path $InstallDir 'nextar-setup.exe')
    $candidates += (Join-Path (Split-Path $PSScriptRoot -Parent) 'dist\nextar-setup.exe')
    foreach ($c in $candidates) {
        if ($c -and (Test-Path -LiteralPath $c)) { $setup = $c; break }
    }
    Check "nextar-setup.exe located for E2E" ($null -ne $setup)

    # If a real install exists, the E2E temporarily re-points the global
    # shell verbs at the temp prefix, so remember to restore it afterwards.
    $realSetup = Join-Path $InstallDir 'nextar-setup.exe'
    $hadRealInstall = (Test-Path -LiteralPath $realSetup)
    $e2e = Join-Path $env:TEMP ("nextar-e2e-" + [guid]::NewGuid().ToString('N'))

    try {
        if ($null -ne $setup) {
            $exe = Join-Path $e2e 'nextar.exe'
            $guiexe = Join-Path $e2e 'nextar-gui.exe'
            $setupexe = Join-Path $e2e 'nextar-setup.exe'
            $unkey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\nextar'
            $sm = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\nextar.lnk'

            # 1) install into a fresh temp prefix
            & $setup --prefix $e2e --quiet 2>&1 | Out-Null
            Check "install exited 0" ($LASTEXITCODE -eq 0)
            Check "install wrote the three payload exes" ((Test-Path -LiteralPath $exe) -and (Test-Path -LiteralPath $guiexe) -and (Test-Path -LiteralPath $setupexe))
            Check "install registered the uninstall entry" (Test-Path -LiteralPath $unkey)
            if (Test-Path -LiteralPath $unkey) {
                $loc = (Get-ItemProperty -LiteralPath $unkey).InstallLocation
                Check "uninstall InstallLocation points at the prefix" ($loc -eq $e2e)
            }
            Check "install created the Start Menu shortcut" (Test-Path -LiteralPath $sm)

            # 2) upgrade in place: re-run install into the same prefix
            & $setup --prefix $e2e --quiet 2>&1 | Out-Null
            Check "upgrade exited 0" ($LASTEXITCODE -eq 0)
            Check "upgrade kept the payload" ((Test-Path -LiteralPath $exe) -and (Test-Path -LiteralPath $guiexe))
            $lnkCount = @(Get-ChildItem -LiteralPath (Split-Path $sm -Parent) -Filter 'nextar.lnk' -ErrorAction SilentlyContinue).Count
            Check "upgrade produced no duplicate Start Menu shortcut" ($lnkCount -le 1)
            if (Test-Path -LiteralPath $unkey) {
                Check "upgrade kept a single uninstall entry" ((Get-ItemProperty -LiteralPath $unkey).InstallLocation -eq $e2e)
            }

            # 3) uninstall (run from outside the prefix so removal is synchronous)
            & $setup --prefix $e2e --quiet --uninstall 2>&1 | Out-Null
            Check "uninstall exited 0" ($LASTEXITCODE -eq 0)
            Start-Sleep -Milliseconds 500
            Check "uninstall removed the payload exes" ((-not (Test-Path -LiteralPath $exe)) -and (-not (Test-Path -LiteralPath $guiexe)))
            Check "uninstall removed the uninstall entry" (-not (Test-Path -LiteralPath $unkey))
            Check "uninstall removed the Start Menu shortcut" (-not (Test-Path -LiteralPath $sm))
        }
    } finally {
        Remove-Item -LiteralPath $e2e -Recurse -Force -ErrorAction SilentlyContinue
        if ($hadRealInstall) {
            & $realSetup --prefix $InstallDir --quiet 2>&1 | Out-Null
            Write-Host "  (restored shell integration for $InstallDir; desktop-shortcut opt-ins are not restored by the headless defaults)" -ForegroundColor DarkGray
        }
    }
}

# ---------------------------------------------------------------- summary
Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
Write-Host "`n$($script:pass) passed, $($script:fail) failed" -ForegroundColor $(if ($script:fail -eq 0) { 'Green' } else { 'Red' })
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
