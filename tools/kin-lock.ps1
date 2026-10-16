# A lock for the scripts that use things only one script can use at a time: the PC's real clipboard, the emulator's adb serial (emulator-5554)
# and the data directory of a test Kin (a second Kin.exe on the same data directory forwards its launch to the first and exits; since v0.1.1 a
# test Kin runs beside the installed one with a data directory of its own, tools\kin-dev.ps1). The e2e scripts, the smoke test
# of the release APK, the capture and check scripts of desktop\ and measure-idle.ps1 all take it, so two of them (two agents, or a person and
# an agent) can't run at once and wreck each other's run. Two did in milestone 1.5: one script put the other's emulator out and one overwrote
# the saved clipboard of the other.
#
# Use (ASCII only: Windows PowerShell 5.1 reads a script with no BOM as ANSI):
#
#   . (Join-Path $PSScriptRoot '..\tools\kin-lock.ps1'); Enter-KinLock 'e2e-1-4'     # at the top of the script, after param(...)
#   Exit-KinLock                                                                       # optional: the end of the process does it
#
# How it works. The lock is the file %LOCALAPPDATA%\Temp\kin-scripts.lock, opened by the script that holds it with write access that nobody
# else may share. Windows drops that the moment the process ends, however it ends (a crash, a kill, a closed window), so **a lock that was
# left behind by a script that is gone is never in the way**: the next script opens the same file and carries on. (There is no PID to check
# and no timeout to wait for.) The holder writes who it is into the file (script, process ID, start time), and a script that finds the lock
# held reads that and says so. A script that runs another script in the same process, or starts a child process that takes the lock too,
# is not blocked by itself: the holder's process ID is passed in KIN_LOCK_HOLDER_PID, which children inherit.
#
# Run the scripts as `powershell -File script.ps1` (a process each). If you dot-source one into a PowerShell window that stays open, call
# Exit-KinLock when you are done, or the window keeps the lock.
$script:KinLockPath = Join-Path $env:LOCALAPPDATA 'Temp\kin-scripts.lock'
$script:KinLockStream = $null

function Get-KinLockHolder {
    # Who wrote the lock file last (it may be a script that is gone): a string, or $null.
    try {
        $fs = [IO.File]::Open($script:KinLockPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
        try {
            $text = (New-Object IO.StreamReader($fs)).ReadToEnd().Trim([char]0, ' ', "`r", "`n")
            if ($text) { return $text | ConvertFrom-Json }
        } finally { $fs.Dispose() }
    } catch { }
    return $null
}

function Enter-KinLock {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [int]$WaitSeconds = 0
    )
    # The e2e/smoke/bench scripts use an adb server of their own (port 5038, inherited by the emulator and the scripts they start), so that another
    # program's adb (Unity's Android tooling, an IDE) on the default port 5037 can neither take our emulator "offline" nor be stopped by us.
    if (-not $env:ANDROID_ADB_SERVER_PORT) { $env:ANDROID_ADB_SERVER_PORT = '5038' }
    if ($script:KinLockStream) { return }                       # this script already holds it
    if ($env:KIN_LOCK_HOLDER_PID -eq "$PID") { return }         # another script of this same process holds it
    if ($env:KIN_LOCK_HOLDER_PID -and (Get-Process -Id ([int]$env:KIN_LOCK_HOLDER_PID) -ErrorAction SilentlyContinue)) { return }   # a child of the script that holds it
    New-Item -ItemType Directory -Force (Split-Path $script:KinLockPath) | Out-Null
    $deadline = (Get-Date).AddSeconds($WaitSeconds)
    while ($true) {
        try {
            $fs = [IO.File]::Open($script:KinLockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::Read)
            break
        } catch [IO.IOException] {
            if ((Get-Date) -lt $deadline) { Start-Sleep -Seconds 2; continue }
            $h = Get-KinLockHolder
            $who = if ($h) { "'$($h.script)' (process $($h.pid), started $($h.started))" } else { 'another script' }
            throw ("Another Kin script is running: $who. The e2e, smoke, capture and check scripts share the real clipboard, the single Kin.exe slot and the emulator, " +
                "so only one can run at a time. Wait until it has finished and run this one again. (If that process is stuck, stop it: a lock of a process that has ended is never in the way. " +
                "The lock is the file $script:KinLockPath.)")
        }
    }
    $info = [ordered]@{ script = $Name; pid = $PID; started = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss') } | ConvertTo-Json -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($info)
    $fs.SetLength(0)
    $fs.Write($bytes, 0, $bytes.Length)
    $fs.Flush()
    $script:KinLockStream = $fs
    $env:KIN_LOCK_HOLDER_PID = "$PID"
}

function Exit-KinLock {
    if (-not $script:KinLockStream) { return }
    $path = $script:KinLockPath
    try { $script:KinLockStream.Dispose() } catch { }
    $script:KinLockStream = $null
    $env:KIN_LOCK_HOLDER_PID = $null
    try { [IO.File]::Delete($path) } catch { }                  # a script that opened it in this instant keeps it: Delete fails while it is open
}
