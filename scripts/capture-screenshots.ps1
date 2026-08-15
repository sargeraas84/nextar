# capture-screenshots.ps1 — CI-ready screenshot capture for the nightly
# release gallery. Runs BOTH passes: first the Inspect pass (archive arg,
# no navigation) so shots/1-first.png is the loaded Inspect view, then the
# full navigation pass (Home, Create, Extract, Repair, Settings) with the
# Create view seeded via NEXTAR_TEST_INPUTS. Produces stable output names:
#
#   nextar-home.png  nextar-create.png  nextar-extract.png
#   nextar-inspect.png  nextar-repair.png  nextar-settings.png
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File capture-screenshots.ps1 `
#     -Exe C:\path\to\nextar-gui.exe `
#     -OutDir C:\path\to\staging `
#     -ArchiveArg sample.next `
#     -Seed "fixtures/a.txt;fixtures/b.txt"
param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [Parameter(Mandatory = $true)][string]$OutDir,
  [string]$ArchiveArg = "sample.next",
  [string]$Seed = ""
)

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W2 {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
  public delegate bool EnumProc(IntPtr h, IntPtr lp);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[W2]::SetProcessDPIAware() | Out-Null

$script:winList = New-Object System.Collections.ArrayList
$script:targetPid = 0
$script:enumCb = [W2+EnumProc]{
  param($h, $lp)
  $wp = 0
  [W2]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
  if ($wp -eq $script:targetPid -and [W2]::IsWindowVisible($h)) {
    $r = New-Object W2+RECT
    [W2]::GetWindowRect($h, [ref]$r) | Out-Null
    [void]$script:winList.Add(@($h, ($r.R - $r.L), ($r.B - $r.T)))
  }
  return $true
}
function Get-MainWindow($proc) {
  $script:targetPid = $proc.Id
  $script:winList.Clear()
  [W2]::EnumWindows($script:enumCb, [IntPtr]::Zero) | Out-Null
  foreach ($e in $script:winList) {
    if ($e[1] -ge 900 -and $e[2] -ge 500) { return $e[0] }
  }
  return [IntPtr]::Zero
}

function Save-Window($hwnd, $path) {
  $r = New-Object W2+RECT
  [W2]::GetWindowRect($hwnd, [ref]$r) | Out-Null
  $w = $r.R - $r.L; $h = $r.B - $r.T
  if ($w -lt 10 -or $h -lt 10) { Write-Host "  SKIP $path (bad rect $w x $h)"; return }
  [W2]::SetForegroundWindow($hwnd) | Out-Null
  Start-Sleep -Milliseconds 200
  $bmp = New-Object System.Drawing.Bitmap($w, $h)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
  $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
  Write-Host "  saved $path ($w x $h)"
}

function Wait-Settled($hwnd) {
  $tmp1 = Join-Path $env:TEMP "nx-settle-a.png"
  $tmp2 = Join-Path $env:TEMP "nx-settle-b.png"
  for ($i = 0; $i -lt 12; $i++) {
    Save-Window $hwnd $tmp1
    Start-Sleep -Seconds 1
    Save-Window $hwnd $tmp2
    $a = New-Object System.Drawing.Bitmap($tmp1)
    $b = New-Object System.Drawing.Bitmap($tmp2)
    $diff = 0; $n = 0
    for ($y = 0; $y -lt $a.Height; $y += 10) {
      for ($x = 0; $x -lt $a.Width; $x += 10) {
        $ca = $a.GetPixel($x, $y); $cb = $b.GetPixel($x, $y)
        if ([math]::Abs($ca.R-$cb.R) + [math]::Abs($ca.G-$cb.G) + [math]::Abs($ca.B-$cb.B) -gt 60) { $diff++ }
        $n++
      }
    }
    $a.Dispose(); $b.Dispose()
    Write-Host ("  settle check {0}: {1}% differ" -f ($i + 1), [math]::Round($diff * 100 / $n, 2))
    if ($diff * 100 / $n -lt 1.0) { return }
  }
}

function Get-DiffPercent($p1, $p2) {
  $a = New-Object System.Drawing.Bitmap($p1)
  $b = New-Object System.Drawing.Bitmap($p2)
  $diff = 0; $n = 0
  for ($y = 0; $y -lt $a.Height; $y += 8) {
    for ($x = 0; $x -lt $a.Width; $x += 8) {
      $ca = $a.GetPixel($x, $y); $cb = $b.GetPixel($x, $y)
      if ([math]::Abs($ca.R-$cb.R) + [math]::Abs($ca.G-$cb.G) + [math]::Abs($ca.B-$cb.B) -gt 60) { $diff++ }
      $n++
    }
  }
  $a.Dispose(); $b.Dispose()
  return ($diff * 100 / $n)
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Add-Type -AssemblyName System.Windows.Forms
$navKeys = @('2', '3', '4', '5', '6')
$originX = 40; $originY = 40

function Run-Capture([string]$label, [string[]]$argv, [int]$maxNav) {
  Write-Host "=== pass $label ==="
  $a = @()
  if ($argv) { $a = $argv }
  if ($a.Count -gt 0) {
    $p = Start-Process -FilePath $Exe -ArgumentList $a -PassThru
  } else {
    $p = Start-Process -FilePath $Exe -PassThru
  }
  $hwnd = [IntPtr]::Zero
  for ($i = 0; $i -lt 24; $i++) {
    Start-Sleep -Milliseconds 500
    $hwnd = Get-MainWindow $p
    if ($hwnd -ne [IntPtr]::Zero) { break }
  }
  if ($hwnd -eq [IntPtr]::Zero) { Write-Error "pass $label: main window not found"; Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; exit 1 }
  # Topmost so the app's own content renders above anything on the runner.
  [W2]::SetWindowPos($hwnd, [IntPtr](-1), $originX, $originY, 0, 0, 0x0001) | Out-Null
  [W2]::SetForegroundWindow($hwnd) | Out-Null
  Start-Sleep -Milliseconds 800
  Wait-Settled $hwnd

  $first = Join-Path $OutDir "$label-1-first.png"
  Save-Window $hwnd $first
  # Warm-up click on the title bar so the first nav key isn't eaten by focus.
  [W2]::SetForegroundWindow($hwnd) | Out-Null
  $prev = $first
  for ($i = 0; $i -lt $maxNav; $i++) {
    $out = Join-Path $OutDir ("{0}-nav{1}.png" -f $label, ($i + 1))
    for ($try = 0; $try -lt 4; $try++) {
      [System.Windows.Forms.SendKeys]::SendWait($navKeys[$i])
      Start-Sleep -Milliseconds 900
      Save-Window $hwnd $out
      $dp = Get-DiffPercent $prev $out
      Write-Host ("  nav{0} try{1}: diff {2}%" -f ($i + 1), ($try + 1), [math]::Round($dp, 2))
      if ($dp -gt 1.5) { break }
      Start-Sleep -Milliseconds 400
    }
    $prev = $out
  }
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 600
}

# Pass B: Inspect with the archive loaded (no navigation).
Run-Capture "inspect" @($ArchiveArg) 0
Move-Item -Force (Join-Path $OutDir "inspect-1-first.png") (Join-Path $OutDir "nextar-inspect.png")

# Pass A: full navigation with the Create view seeded.
$oldSeed = $env:NEXTAR_TEST_INPUTS
$env:NEXTAR_TEST_INPUTS = $Seed
Run-Capture "main" @() 5
if ($null -eq $oldSeed) { Remove-Item Env:NEXTAR_TEST_INPUTS -ErrorAction SilentlyContinue } else { $env:NEXTAR_TEST_INPUTS = $oldSeed }

Move-Item -Force (Join-Path $OutDir "main-1-first.png") (Join-Path $OutDir "nextar-home.png")
Move-Item -Force (Join-Path $OutDir "main-nav1.png") (Join-Path $OutDir "nextar-create.png")
Move-Item -Force (Join-Path $OutDir "main-nav2.png") (Join-Path $OutDir "nextar-extract.png")
# main-nav3 is the empty Inspect view — the loaded nextar-inspect.png is used.
Remove-Item -Force (Join-Path $OutDir "main-nav3.png") -ErrorAction SilentlyContinue
Move-Item -Force (Join-Path $OutDir "main-nav4.png") (Join-Path $OutDir "nextar-repair.png")
Move-Item -Force (Join-Path $OutDir "main-nav5.png") (Join-Path $OutDir "nextar-settings.png")

Write-Host "--- produced ---"
Get-ChildItem $OutDir -Filter *.png | Select-Object -ExpandProperty Name
Write-Host "done"
