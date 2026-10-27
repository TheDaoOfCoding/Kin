# Idle memory of the release Kin.exe, tray only, measured as Task Manager does: private working set from
# the Win32_PerfFormattedData_PerfProc_Process counters (what Task Manager calls "active private working set"), working set and
# private bytes from Get-Process, 60 s after launch. No window is ever opened, unless -OpenWindows is given.
#
# The app runs with the developer network settings (loopback only, no mDNS), so no Windows Firewall prompt appears. That leaves
# out the mDNS daemon of a normal run. It runs with a throwaway profile, so nothing in %APPDATA%\Kin is touched.
#
# -OpenWindows (1.4): every window (pairing, the main window on each of its pages Home, History and Settings, debug) is opened and closed first, and the memory is
# measured after that (the windows are destroyed when closed, so the tray-only number must come back, and no WebView2 process may be left over).
#
# -PageCpu: the main window is opened on Home and its CPU time (Kin.exe and its WebView2 processes) is sampled for -PageSeconds on each page in turn (Home, History,
# Settings), so a hidden page that keeps polling, or a page that costs more than Home, shows up. Needs a phone-less dev Kin only; no phone is paired.
#
#   powershell -ExecutionPolicy Bypass -File desktop\measure-idle.ps1 [-IdleSeconds 60] [-Runs 3] [-OpenWindows] [-PageCpu [-PageSeconds 10]]
param(
    [int]$IdleSeconds = 60,
    [int]$Runs = 3,
    [switch]$OpenWindows,
    [switch]$PageCpu,
    [int]$PageSeconds = 10
)
# Only one of the Kin scripts (e2e, smoke, capture, check) may run at a time: they share the real clipboard, the Kin.exe slot and the emulator (tools\kin-lock.ps1).
. (Join-Path $PSScriptRoot '..\tools\kin-lock.ps1'); Enter-KinLock 'measure-idle'
. (Join-Path $PSScriptRoot '..\tools\kin-dev.ps1')   # a Kin of our own beside the installed one; every command carries --data-dir (kin-dev.ps1)
$ErrorActionPreference = 'Stop'
$exe = Join-Path $PSScriptRoot 'src-tauri\target\release\Kin.exe'
if ($OpenWindows -or $PageCpu) { Assert-NoGameRunning }   # windows are shown

# Waits for a window of OUR Kin with this title, and returns its process (or $null).
function Wait-KinWindow([string]$title, [int]$seconds = 15) {
    $deadline = [DateTime]::Now.AddSeconds($seconds)
    while ([DateTime]::Now -lt $deadline -and -not (Get-DevKinProcesses | Where-Object { $_.MainWindowTitle -eq $title })) { Start-Sleep -Milliseconds 200 }
    Get-DevKinProcesses | Where-Object { $_.MainWindowTitle -eq $title } | Select-Object -First 1   # OUR Kin only
}

function Close-KinWindow([string]$title) {
    $w = Get-DevKinProcesses | Where-Object { $_.MainWindowTitle -eq $title } | Select-Object -First 1
    if ($w) { [void]$w.CloseMainWindow() }
    $deadline = [DateTime]::Now.AddSeconds(10)
    while ([DateTime]::Now -lt $deadline -and (Get-DevKinProcesses | Where-Object { $_.MainWindowTitle -eq $title })) { Start-Sleep -Milliseconds 200 }
}

function Open-AndCloseEveryWindow([string]$exe) {
    # The main window is one window (titled "Kin") with three pages: --drop builds it on Home, --history and --settings tell the open window to switch its page.
    # The other windows are separate: title -> the command that opens it. Each is opened, given time to draw, and closed.
    $steps = @(@('Pair new phone', '--pair'), @('Kin', '--drop'), @('Kin', '--history'), @('Kin', '--settings'), @('Kin', $null), @('Debug', '--debug-window'))
    foreach ($step in $steps) {
        $title = $step[0]
        if ($step[1]) { Invoke-Kin $step[1] }
        if (-not $step[1]) { Close-KinWindow $title; continue }   # (the main window is closed once its three pages have been shown)
        $w = Wait-KinWindow $title
        Start-Sleep -Seconds 2   # let the page load and draw
        if (-not $w) { Write-Warning "the '$title' window did not open ($($step[1]))"; continue }
        if ($title -ne 'Kin') { Close-KinWindow $title }
    }
}

# The CPU seconds that Kin.exe and the processes below it (WebView2 starts its own) have used so far.
function Get-KinTreeCpuSeconds {
    $root = @(Get-DevKinProcesses | Select-Object -First 1)
    if (-not $root) { return 0.0 }
    $all = @(Get-CimInstance Win32_Process)
    $ids = New-Object System.Collections.Generic.List[int]; $ids.Add([int]$root[0].Id)
    for ($i = 0; $i -lt $ids.Count; $i++) { foreach ($c in ($all | Where-Object { $_.ParentProcessId -eq $ids[$i] })) { if (-not $ids.Contains([int]$c.ProcessId)) { $ids.Add([int]$c.ProcessId) } } }
    $sum = 0.0
    foreach ($id in $ids) { $p = Get-Process -Id $id -ErrorAction SilentlyContinue; if ($p) { $sum += $p.TotalProcessorTime.TotalSeconds } }
    return $sum
}

# CPU per page: the main window on Home, then History, then Settings, each sampled for $PageSeconds (average share of one core).
function Measure-PageCpu([int]$seconds) {
    Invoke-Kin --drop
    $w = Wait-KinWindow 'Kin'
    if (-not $w) { Write-Warning 'the main window did not open'; return }
    Start-Sleep -Seconds 3   # let the page load and draw
    foreach ($page in @(@('Home', '--drop'), @('History', '--history'), @('Settings', '--settings'))) {
        Invoke-Kin $page[1]
        Start-Sleep -Seconds 2   # the page switches and draws
        $a = Get-KinTreeCpuSeconds
        Start-Sleep -Seconds $seconds
        $b = Get-KinTreeCpuSeconds
        Write-Host ("PAGE_CPU {0}: {1}% of one core over {2} s" -f $page[0], [math]::Round(($b - $a) / $seconds * 100, 2), $seconds)
    }
    Close-KinWindow 'Kin'
}

foreach ($run in 1..$Runs) {
    $data = Join-Path ([IO.Path]::GetFullPath($env:TEMP)) "kin-idle-$run"
    Remove-Item $data -Recurse -Force -ErrorAction SilentlyContinue
    $sw = [Diagnostics.Stopwatch]::StartNew()
    # The path is quoted, because Start-Process doesn't quote the arguments of an array for you, and the temp folder can have a space.
    $kinArgs = @('--data-dir', ('"{0}"' -f $data), '--dev-bind', '127.0.0.1', '--dev-received-dir', ('"{0}"' -f (Join-Path $data 'received')))
    $kinArgs += $(if ($OpenWindows -or $PageCpu) { '--dev-no-toasts' } else { '--dev-no-windows' })
    Set-KinDev -Exe $exe -DataDir $data
    $p = Start-DevKin $kinArgs
    if ($OpenWindows) {
        Start-Sleep -Seconds 3
        Open-AndCloseEveryWindow $exe
    }
    if ($PageCpu) {
        Start-Sleep -Seconds 3
        Measure-PageCpu $PageSeconds
    }
    Start-Sleep -Seconds ([math]::Max(0, $IdleSeconds - $sw.Elapsed.TotalSeconds))
    $p.Refresh()
    $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object { $_.IDProcess -eq $p.Id }
    $children = @(Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $p.Id })
    [pscustomobject]@{
        Run = $run
        AfterSeconds = [math]::Round($sw.Elapsed.TotalSeconds)
        WindowsOpenedAndClosed = $OpenWindows.IsPresent
        Processes = 1 + $children.Count
        WorkingSetMB = [math]::Round($p.WorkingSet64 / 1MB, 1)
        PrivateWorkingSetMB = [math]::Round($perf.WorkingSetPrivate / 1MB, 1)
        PrivateBytesMB = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
        Threads = $p.Threads.Count
        Handles = $p.HandleCount
    } | Format-Table -AutoSize | Out-String
    Stop-DevKin 8   # --quit to our data directory (never to the installed Kin); a hung process of ours is stopped by its ID
    Remove-DevDataDir $data
}
