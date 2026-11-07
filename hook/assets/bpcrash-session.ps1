# Mintfixed's native-Windows post-exit helper for Elytra's --once injector.
# This script is launched once per game session and never watches future launches.
# A separate process is required: the game hook cannot run after a fatal crash.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][int]$GameProcessId,
    [Parameter(Mandatory = $true)][int]$InjectorProcessId,
    [Parameter(Mandatory = $true)][string]$GameExe,
    [Parameter(Mandatory = $true)][string]$ReporterExe,
    [Parameter(Mandatory = $true)][long]$StartUnixSeconds
)

$ErrorActionPreference = 'Stop'

try {
    # Obtain the process handle now so PID reuse cannot extend our lifetime.
    $game = Get-Process -Id $GameProcessId -ErrorAction SilentlyContinue
    if ($null -ne $game) {
        $game.WaitForExit()
    }
} catch {
    Write-Warning "BP-Crash-Reporter: could not wait for game PID ${GameProcessId}: $_"
    exit 1
}

# Normally --once has exited long before DRG ends. Allow an in-flight injection
# to finish before attempting triage, but never leave a stuck injector running.
try {
    Wait-Process -Id $InjectorProcessId -Timeout 40 -ErrorAction Stop
} catch {
    try {
        $injector = Get-CimInstance Win32_Process -Filter "ProcessId=$InjectorProcessId" -ErrorAction SilentlyContinue
        if ($null -ne $injector -and
            [string]::Equals($injector.ExecutablePath, $ReporterExe, [StringComparison]::OrdinalIgnoreCase)) {
            Write-Warning "BP-Crash-Reporter: timed out; stopping own one-shot injector PID $InjectorProcessId"
            Stop-Process -Id $InjectorProcessId -Force -ErrorAction Stop
        }
    } catch {
        Write-Warning "BP-Crash-Reporter: could not check/stop injector PID ${InjectorProcessId}: $_"
    }
}

# The game's UE log is flushed on exit; give Windows a moment to release it.
Start-Sleep -Milliseconds 400
$since = [DateTimeOffset]::FromUnixTimeSeconds($StartUnixSeconds).UtcDateTime.AddSeconds(-5)

try {
    # Elytra names its files bpcrash_<date>_<time>_<pid>_<seq>_t<tid>.txt.
    # Use both this session's PID and timestamp; never modify old reports.
    $reports = @(
        Get-ChildItem -LiteralPath $PSScriptRoot -Filter 'bpcrash_*.txt' -File |
            Where-Object {
                $_.LastWriteTimeUtc -ge $since -and
                $_.Name -like "bpcrash_*_${GameProcessId}_*_t*.txt"
            } |
            Sort-Object LastWriteTimeUtc -Descending
    )
    if ($reports.Count -gt 0) {
        # The last report is the crash (or startup self-test on a clean exit),
        # matching Elytra's own normal watcher behavior.
        $report = $reports[0]
        Write-Output "BP-Crash-Reporter: running post-exit triage for $($report.Name)"
        & $ReporterExe --triage $report.FullName $GameExe
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "BP-Crash-Reporter: triage failed with code $LASTEXITCODE"
        }
    } else {
        Write-Output "BP-Crash-Reporter: no report for game PID $GameProcessId"
    }
} catch {
    Write-Warning "BP-Crash-Reporter: post-exit triage failed: $_"
}

# Prevent the old updater's PID file from pointing at a recycled process ID.
try {
    $pidFile = Join-Path $PSScriptRoot 'bpcrash.pid'
    if ((Test-Path -LiteralPath $pidFile) -and
        ((Get-Content -LiteralPath $pidFile -Raw).Trim() -eq [string]$InjectorProcessId)) {
        Remove-Item -LiteralPath $pidFile -Force
    }
} catch {
    Write-Warning "BP-Crash-Reporter: could not remove stale PID file: $_"
}
