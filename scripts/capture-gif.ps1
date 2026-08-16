# capture-gif.ps1 — record an animated GIF of nextar's boot splash, Home
# hero entrance, and view transitions, then assemble it with ffmpeg.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File capture-gif.ps1 `
#     -Exe C:\path\to\nextar-gui.exe -Out C:\path\to\nextar-demo.gif `
#     [-Ffmpeg C:\path\to\ffmpeg.exe] [-Fps 10]
param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [Parameter(Mandatory = $true)][string]$Out,
  [string]$Ffmpeg = "ffmpeg",
  [int]$Fps = 10
)

# Pin the DARK theme for captures (CI runners default to Windows LIGHT
# mode; NEXTAR_LOGO_THEME is the app's documented dev/CI override and
# beats both the registry and the Follow setting).
$env:NEXTAR_LOGO_THEME = "dark"

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W3 {
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
[W3]::SetProcessDPIAware() | Out-Null

$script:winList = New-Object System.Collections.ArrayList
$script:targetPid = 0
$script:enumCb = [W3+EnumProc]{
  param($h, $lp)
  $wp = 0
  [W3]::GetWindowThreadProcessId($h, [ref]$wp) | Out-Null
  if ($wp -eq $script:targetPid -and [W3]::IsWindowVisible($h)) {
    $r = New-Object W3+RECT
    [W3]::GetWindowRect($h, [ref]$r) | Out-Null
    [void]$script:winList.Add(@($h, ($r.R - $r.L), ($r.B - $r.T)))
  }
  return $true
}
function Get-Windows($proc) {
  $script:targetPid = $proc.Id
  $script:winList.Clear()
  [W3]::EnumWindows($script:enumCb, [IntPtr]::Zero) | Out-Null
  return @($script:winList)
}

function Grab($hwnd, $path) {
  $r = New-Object W3+RECT
  [W3]::GetWindowRect($hwnd, [ref]$r) | Out-Null
  $w = $r.R - $r.L; $h = $r.B - $r.T
  if ($w -lt 10 -or $h -lt 10) { return $false }
  $bmp = New-Object System.Drawing.Bitmap($w, $h)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
  $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
  return $true
}

$frameDir = Join-Path $env:TEMP "nextar-gif-frames"
if (Test-Path $frameDir) { Remove-Item -Recurse -Force $frameDir }
New-Item -ItemType Directory -Force -Path $frameDir | Out-Null

$p = Start-Process -FilePath $Exe -PassThru
$frame = 0

# Phase 1: boot splash (a small undecorated window, always-on-top) — the
# logo entrance plays for SPLASH_DURATION ≈ 1.55s. Grab at ~Fps fps and
# STOP the moment the main window appears (the splash closes right before
# it opens), so no dead-window frames leak into the GIF.
$splashHwnd = [IntPtr]::Zero
$mainHwnd = [IntPtr]::Zero
$splashStart = [DateTime]::Now
while (([DateTime]::Now - $splashStart).TotalSeconds -lt 4.0 -and $mainHwnd -eq [IntPtr]::Zero) {
  $wins = Get-Windows $p
  foreach ($e in $wins) {
    if ($e[1] -ge 900 -and $e[2] -ge 500) { $mainHwnd = $e[0]; break }
    if ($splashHwnd -eq [IntPtr]::Zero -and $e[1] -ge 400 -and $e[1] -le 620 -and $e[2] -ge 200 -and $e[2] -le 400) {
      $splashHwnd = $e[0]
    }
  }
  if ($splashHwnd -ne [IntPtr]::Zero) {
    $f = Join-Path $frameDir ("f{0:D4}.png" -f $frame)
    if (Grab $splashHwnd $f) { $frame++ }
  }
  Start-Sleep -Milliseconds (1000 / $Fps)
}
if ($mainHwnd -eq [IntPtr]::Zero) { Write-Error "main window not found"; Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; exit 1 }

# Phase 2: the main window. Pin it topmost at a fixed spot so it's
# composited above anything else, then record the Home hero entrance.
[W3]::SetWindowPos($mainHwnd, [IntPtr](-1), 40, 40, 0, 0, 0x0001) | Out-Null
[W3]::SetForegroundWindow($mainHwnd) | Out-Null
Start-Sleep -Milliseconds 400

# Home hero entrance (~1.4s) at Fps fps.
$homeStart = [DateTime]::Now
while (([DateTime]::Now - $homeStart).TotalSeconds -lt 1.5) {
  $f = Join-Path $frameDir ("f{0:D4}.png" -f $frame)
  if (Grab $mainHwnd $f) { $frame++ }
  Start-Sleep -Milliseconds (1000 / $Fps)
}

# Phase 3: view transitions — press keys 2-6, grab 3 frames per view.
# Retry each key (up to 3x) until the frame actually changes, mirroring
# capture-screenshots.ps1's nav loop: the first SendKeys is sometimes
# swallowed by window activation, so a 0% diff means the key missed.
Add-Type -AssemblyName System.Windows.Forms
function Diff-Frames([string]$a, [string]$b) {
  if (-not (Test-Path $b)) { return 100.0 }
  $ia = New-Object System.Drawing.Bitmap($a); $ib = New-Object System.Drawing.Bitmap($b)
  $d = 0; $n = 0
  for ($y = 0; $y -lt $ia.Height; $y += 10) {
    for ($x = 0; $x -lt $ia.Width; $x += 10) {
      $ca = $ia.GetPixel($x, $y); $cb = $ib.GetPixel($x, $y)
      if ([math]::Abs($ca.R-$cb.R) + [math]::Abs($ca.G-$cb.G) + [math]::Abs($ca.B-$cb.B) -gt 60) { $d++ }
      $n++
    }
  }
  $ia.Dispose(); $ib.Dispose()
  return ([double]($d * 100 / $n))
}
foreach ($k in @('2', '3', '4', '5', '6')) {
  $probe = Join-Path $frameDir ("f{0:D4}.png" -f ($frame - 1))
  $landed = $false
  for ($try = 0; $try -lt 3 -and -not $landed; $try++) {
    [W3]::SetForegroundWindow($mainHwnd) | Out-Null
    Start-Sleep -Milliseconds 150
    [System.Windows.Forms.SendKeys]::SendWait($k)
    Start-Sleep -Milliseconds 350
    $chk = Join-Path $frameDir ("f{0:D4}.png" -f $frame)
    if (Grab $mainHwnd $chk) { $frame++ }
    $dp = Diff-Frames $probe $chk
    if ($dp -gt 1.5) { $landed = $true }
  }
  # Record the remaining hold frames for this view.
  for ($j = 0; $j -lt 2; $j++) {
    $f = Join-Path $frameDir ("f{0:D4}.png" -f $frame)
    if (Grab $mainHwnd $f) { $frame++ }
    Start-Sleep -Milliseconds (1000 / $Fps)
  }
}

# Hold the final (Settings) view a beat longer so the loop end is stable.
for ($j = 0; $j -lt 4; $j++) {
  $f = Join-Path $frameDir ("f{0:D4}.png" -f $frame)
  if (Grab $mainHwnd $f) { $frame++ }
  Start-Sleep -Milliseconds (1000 / $Fps)
}

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Write-Host "captured $frame frames"

# Assemble with ffmpeg: the splash (480x300) and main window (1020x700)
# frames differ in size, so scale to a FIXED canvas (700x480, centered with
# black letterbox) — GIF needs a constant frame size. Then build a palette
# and loop forever.
$dir = $frameDir.Replace('\', '/')
& $Ffmpeg -y -framerate $Fps -i "$dir/f%04d.png" -vf "scale=700:480:force_original_aspect_ratio=decrease,pad=700:480:(ow-iw)/2:(oh-ih)/2:color=#070A11,split[s0][s1];[s0]palettegen=max_colors=256[p];[s1][p]paletteuse=dither=bayer:bayer_scale=5" -loop 0 $Out 2>&1 | Select-Object -Last 3
if (Test-Path $Out) {
  Write-Host "GIF written: $Out ($((Get-Item $Out).Length / 1KB) KB)"
} else {
  Write-Error "ffmpeg did not produce $Out"
  exit 1
}
