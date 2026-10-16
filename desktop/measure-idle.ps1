# Idle memory of the release Kin.exe, tray only, measured the same way as docs/results/phase-0-tauri.md: private working set from
# the Win32_PerfFormattedData_PerfProc_Process counters (what Task Manager calls "active private working set"), working set and
# private bytes from Get-Process, 60 s after launch. No window is ever opened, unless -OpenWindows is given.
#
# The app runs with the developer network settings (loopback only, no mDNS), so no Windows Firewall prompt appears. That leaves
# out the mDNS daemon of a normal run. It runs with a throwaway profile, so nothing in %APPDATA%\Kin is touched.
#
# -OpenWindows (1.4): every window (pairing, drop, history, settings, debug) is opened and closed first, and the memory is measured after that
# (the windows are destroyed when closed, so the tray-only number must come back, and no WebView2 process may be left over).
#
#   powershell -ExecutionPolicy Bypass -File desktop\measure-idle.ps1 [-IdleSeconds 60] [-Runs 3] [-OpenWindows]
param(
    [int]$IdleSeconds = 60,
    [int]$Runs = 3,
    [switch]$OpenWindows
)
# Only one of the Kin scripts (e2e, smoke, capture, check) may run at a time: they share the real clipboard, the Kin.exe slot and the emulator (tools\kin-lock.ps1).
. (Join-Path $PSScriptRoot '..\tools\kin-lock.ps1'); Enter-KinLock 'measure-idle'
. (Join-Path $PSScriptRoot '..\tools\kin-dev.ps1')   # a Kin of our own beside the installed one; every command carries --data-dir (kin-dev.ps1)
$ErrorActionPreference = 'Stop'
$exe = Join-Path $PSScriptRoot 'src-tauri\target\release\Kin.exe'
if ($OpenWindows) { Assert-NoGameRunning }   # windows are shown

function Open-AndCloseEveryWindow([string]$exe) {
    # window title -> the command that opens it
    $windows = [ordered]@{ 'Pair new phone' = '--pair'; 'Send to phone' = '--drop'; 'History' = '--history'; 'Settings' = '--settings'; 'Debug' = '--debug-window' }
    foreach ($title in $windows.Keys) {
        Invoke-Kin $windows[$title]
        $deadline = [DateTime]::Now.AddSeconds(15)
        while ([DateTime]::Now -lt $deadline -and -not (Get-DevKinProcesses | Where-Object { $_.MainWindowTitle -eq $title })) { Start-Sleep -Milliseconds 200 }
        Start-Sleep -Seconds 2   # let the page load and draw
        $w = Get-DevKinProcesses | Where-Object { $_.MainWindowTitle -eq $title } | Select-Object -First 1   # OUR Kin only
        if (-not $w) { Write-Warning "the '$title' window did not open"; continue }
        [void]$w.CloseMainWindow()
        $deadline = [DateTime]::Now.AddSeconds(10)
        while ([DateTime]::Now -lt $deadline -and (Get-DevKinProcesses | Where-Object { $_.MainWindowTitle -eq $title })) { Start-Sleep -Milliseconds 200 }
    }
}

foreach ($run in 1..$Runs) {
    $data = Join-Path ([IO.Path]::GetFullPath($env:TEMP)) "kin-idle-$run"
    Remove-Item $data -Recurse -Force -ErrorAction SilentlyContinue
    $sw = [Diagnostics.Stopwatch]::StartNew()
    # The path is quoted, because Start-Process doesn't quote the arguments of an array for you, and the temp folder can have a space.
    $kinArgs = @('--data-dir', ('"{0}"' -f $data), '--dev-bind', '127.0.0.1', '--dev-received-dir', ('"{0}"' -f (Join-Path $data 'received')))
    $kinArgs += $(if ($OpenWindows) { '--dev-no-toasts' } else { '--dev-no-windows' })
    Set-KinDev -Exe $exe -DataDir $data
    $p = Start-DevKin $kinArgs
    if ($OpenWindows) {
        Start-Sleep -Seconds 3
        Open-AndCloseEveryWindow $exe
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
