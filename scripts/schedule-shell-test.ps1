# nextar - schedule the daily shell smoke test
#
# Registers a Task Scheduler task that runs scripts/run-shell-test.ps1 every
# day. Runs interactively (only when you're logged on) so a failure can raise
# a desktop toast; results also land in %LOCALAPPDATA%\nextar\shell-test.log
# and the Application event log.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/schedule-shell-test.ps1
#   powershell ... -schedule-shell-test.ps1 -Deep          # include -Run invocation checks
#   powershell ... -schedule-shell-test.ps1 -Time 18:30    # daily at 18:30
#   powershell ... -schedule-shell-test.ps1 -Remove        # unregister the task

param(
    [switch]$Remove,             # unregister the scheduled task
    [switch]$Deep,               # forward -Run to the verifier (brief progress windows flash)
    [string]$Time = '09:00'      # daily run time, HH:MM
)

$task = 'nextar-shell-test'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$runner = Join-Path $here 'run-shell-test.ps1'

if ($Remove) {
    Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue
    Write-Host "removed scheduled task '$task'"
    exit 0
}

if (-not (Test-Path $runner)) {
    Write-Host "runner not found: $runner" -ForegroundColor Red
    exit 1
}

$arg = "-NoProfile -ExecutionPolicy Bypass -File `"$runner`""
if ($Deep) { $arg += ' -Deep' }

$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $arg
$trigger = New-ScheduledTaskTrigger -Daily -At $Time
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive

try {
    Register-ScheduledTask -TaskName $task -Action $action -Trigger $trigger `
        -Principal $principal -Description 'nextar shell integration daily smoke test' -Force | Out-Null
} catch {
    Write-Host "registration failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}

Write-Host "registered '$task': daily at $Time $(if ($Deep) { '(deep checks - windows flash)' } else { '(silent registry + explorer checks)' })"
Write-Host "runner: $runner"
Write-Host "results: %LOCALAPPDATA%\nextar\shell-test.log  (failures also hit the Application event log + a toast)"
Write-Host "remove anytime with: powershell -File $($MyInvocation.MyCommand.Path) -Remove"
exit 0
