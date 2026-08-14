# nextar - scheduled shell smoke-test runner
#
# Runs scripts/verify-shell.ps1 (registry + Explorer verb resolution; add
# -Deep for the full -Run invocation checks) and reports the result:
#   * appends every run to %LOCALAPPDATA%\nextar\shell-test.log
#   * on failure, writes a Windows Application event-log entry
#   * on failure, raises a desktop toast (best effort)
#
# Exit code: 0 = passed, 1 = failed.

param(
    [switch]$Deep,          # forward to verify-shell.ps1 -Run (brief progress windows)
    [string]$InstallDir = '' # override where the exes live (CI installs to a temp prefix)
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$verify = Join-Path $here 'verify-shell.ps1'
$logDir = Join-Path $env:LOCALAPPDATA 'nextar'
New-Item -ItemType Directory -Path $logDir -Force | Out-Null
$log = Join-Path $logDir 'shell-test.log'
$stamp = Get-Date -Format 'yyyy-MM-dd HH:mm:ss'

Write-Output "== $stamp nextar shell smoke test $(if ($Deep) { '(deep)' } else { '(silent)' }) =="

# Run the verification in a child process so its host output is captured
# (Write-Host cannot be piped from the same process in PowerShell 5.1).
$childArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $verify)
if ($Deep) { $childArgs += '-Run' }
if ($InstallDir) { $childArgs += '-InstallDir'; $childArgs += $InstallDir }
$out = & powershell.exe @childArgs 2>&1 | Out-String
$ok = ($LASTEXITCODE -eq 0)

if ($ok) {
    Add-Content -Path $log -Value "== $stamp PASS ==" -Encoding UTF8
} else {
    Add-Content -Path $log -Value "== $stamp FAIL ==" -Encoding UTF8
}
$out.Trim() | Add-Content -Path $log -Encoding UTF8

if (-not $ok) {
    # 1) Windows Application event log. A dedicated 'nextar' source needs
    #    admin to create once; without it we fall back to the generic
    #    'Application' source, which non-admin users can write to directly.
    $eventSource = 'nextar'
    try {
        try {
            if (-not [System.Diagnostics.EventLog]::SourceExists('nextar')) {
                [System.Diagnostics.EventLog]::CreateEventSource('nextar', 'Application')
            }
        } catch {
            $eventSource = 'Application'   # SourceExists needs admin when the source is new
        }
        Write-EventLog -LogName Application -Source $eventSource -EntryType Error -EventId 1001 `
            -Message "nextar shell smoke test FAILED at $stamp - details in %LOCALAPPDATA%\nextar\shell-test.log" -ErrorAction Stop
    } catch {
        # event log unavailable - the log file still has the result
    }

    # 2) desktop toast (best effort; needs an interactive session)
    try {
        $null = [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType=WindowsRuntime]
        $null = [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType=WindowsRuntime]
        $xml = [Windows.Data.Xml.Dom.XmlDocument]::new()
        $xml.LoadXml("<toast><visual><binding template='ToastGeneric'><text>nextar shell test</text><text>FAILED - see %LOCALAPPDATA%\nextar\shell-test.log</text></binding></visual></toast>")
        $toast = [Windows.UI.Notifications.ToastNotification]::new($xml)
        [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('nextar').Show($toast)
    } catch {
        # toast unavailable (locked session, no toast support) - ignore
    }
}

Write-Output ($out.Trim())
if ($ok) { Write-Output 'RESULT: PASS' } else { Write-Output 'RESULT: FAIL' }
if ($ok) { exit 0 } else { exit 1 }
