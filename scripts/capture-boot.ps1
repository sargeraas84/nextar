# capture-boot.ps1 — capture the two brand surfaces the main pipeline skips:
# the boot splash (a small 480x300 undecorated window) and the installer
# wizard Welcome page (560x480). Produces stable names:
#
#   nextar-splash.png   nextar-wizard.png
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File capture-boot.ps1 `
#     -GuiExe C:\path\to\nextar-gui.exe `
#     -SetupExe C:\path\to\nextar-setup.exe `
#     -OutDir C:\path\to\staging
param(
  [Parameter(Mandatory = $true)][string]$GuiExe,
  [Parameter(Mandatory = $true)][string]$SetupExe,
  [Parameter(Mandatory = $true)][string]$OutDir
)

# Pin the DARK theme for captures (same rationale as capture-screenshots.ps1).
$env:NEXTAR_LOGO_THEME = "dark"

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class B2 {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
  public delegate bool EnumProc(IntPtr h, IntPtr lp);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[B2]::SetProcessDPIAware() | Out-Null

$script:winList = New-Object System.Collections.ArrayList
$script:targetPid = 0
$script:enumCb = [B2+EnumProc]{
  param($h, $lp)
  $wp = 0
  [B2]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
  if ($wp -eq $script:targetPid -and [B2]::IsWindowVisible($h)) {
    $r = New-Object B2+RECT
    [B2]::GetWindowRect($h, [ref]$r) | Out-Null
    [void]$script:winList.Add(@($h, ($r.R - $r.L), ($r.B - $r.T)))
  }
  return $true
}
function Get-WindowBySize($proc, [int]$minW, [int]$minH) {
  $script:targetPid = $proc.Id
  $script:winList.Clear()
  [B2]::EnumWindows($script:enumCb, [IntPtr]::Zero) | Out-Null
  foreach ($e in $script:winList) {
    if ($e[1] -ge $minW -and $e[2] -ge $minH) { return $e[0] }
  }
  return [IntPtr]::Zero
}
function Save-Window($hwnd, $path) {
  $r = New-Object B2+RECT
  [B2]::GetWindowRect($hwnd, [ref]$r) | Out-Null
  $w = $r.R - $r.L; $h = $r.B - $r.T
  if ($w -lt 10 -or $h -lt 10) { Write-Host "  SKIP $path (bad rect $w x $h)"; return $false }
  [B2]::SetForegroundWindow($hwnd) | Out-Null
  Start-Sleep -Milliseconds 300
  $bmp = New-Object System.Drawing.Bitmap($w, $h)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
  $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
  Write-Host "  saved $path ($w x $h)"
  return $true
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# --- Boot splash: launch the GUI and grab the small undecorated splash
# window (480x300) BEFORE the main window (>=900x500) appears. The splash
# closes as the main window opens, so poll fast and stop on first match.
Write-Host "=== capture splash ==="
$p = Start-Process -FilePath $GuiExe -PassThru
$splashHwnd = [IntPtr]::Zero
for ($i = 0; $i -lt 40; $i++) {
  Start-Sleep -Milliseconds 150
  $h = Get-WindowBySize $p 380 240
  if ($h -ne [IntPtr]::Zero) { $splashHwnd = $h; break }
}
if ($splashHwnd -eq [IntPtr]::Zero) {
  Write-Host "  splash window not found (may have booted too fast); killing"
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
} else {
  Start-Sleep -Milliseconds 1200  # let the logo entrance play partway
  [void](Save-Window $splashHwnd (Join-Path $OutDir "nextar-splash.png"))
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 500
}

# --- Installer wizard: the Welcome page sits until the user clicks Next,
# so it is easy to capture deterministically.
Write-Host "=== capture wizard ==="
$w = Start-Process -FilePath $SetupExe -PassThru
$wizardHwnd = [IntPtr]::Zero
for ($i = 0; $i -lt 24; $i++) {
  Start-Sleep -Milliseconds 500
  $h = Get-WindowBySize $w 500 420
  if ($h -ne [IntPtr]::Zero) { $wizardHwnd = $h; break }
}
if ($wizardHwnd -eq [IntPtr]::Zero) {
  Write-Host "  wizard window not found"
  Stop-Process -Id $w.Id -Force -ErrorAction SilentlyContinue
  exit 1
}
Start-Sleep -Milliseconds 1500  # let the wizard's logo entrance settle
[void](Save-Window $wizardHwnd (Join-Path $OutDir "nextar-wizard.png"))
Stop-Process -Id $w.Id -Force -ErrorAction SilentlyContinue

Write-Host "--- produced ---"
Get-ChildItem $OutDir -Filter nextar-*.png | Select-Object -ExpandProperty Name
Write-Host "done"
