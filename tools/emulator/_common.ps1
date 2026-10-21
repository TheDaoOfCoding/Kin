# Shared helpers for the emulator scripts. Dot-source it: . "$PSScriptRoot\_common.ps1"
# Uses OUR Android SDK adb by full path, never whatever is first on PATH (Unity bundles another adb).

# Not 'Stop': Windows PowerShell 5.1 turns any native stderr text (adb prints progress there) into a
# terminating error. Scripts check exit codes and output explicitly and `throw` instead.
$ErrorActionPreference = 'Continue'

$script:Sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
$script:Adb = Join-Path $Sdk 'platform-tools\adb.exe'
$script:Emulator = Join-Path $Sdk 'emulator\emulator.exe'
$script:RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

foreach ($p in $Adb, $Emulator) {
    if (-not (Test-Path $p)) { throw "Missing $p. Install with: sdkmanager platform-tools emulator" }
}

# Make sure the adb server on port 5037 is OURS. A server from another adb version (for example
# Unity's) is killed and replaced. Unity may restart its own server later and take the port back;
# running any of these scripts again takes it over again.
function Initialize-Adb {
    # Our own adb server port, even when a script runs without tools\kin-lock.ps1: another program's adb (Unity's) on 5037 is never ours to stop.
    if (-not $env:ANDROID_ADB_SERVER_PORT) { $env:ANDROID_ADB_SERVER_PORT = '5038' }
    $ours = (Resolve-Path $Adb).Path
    $foreign = @(Get-Process adb -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -ne $ours })
    # With an adb server port of our own (ANDROID_ADB_SERVER_PORT, set by tools\kin-lock.ps1) another adb belongs to somebody else's server on the default
    # port: it is none of our business and is never stopped.
    if ($env:ANDROID_ADB_SERVER_PORT -and $env:ANDROID_ADB_SERVER_PORT -ne '5037') { $foreign = @() }
    if ($foreign) {
        & $Adb kill-server 2>$null | Out-Null   # works across versions: it just asks whoever listens on 5037 to quit
        Start-Sleep -Milliseconds 500
        foreach ($p in @(Get-Process adb -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -ne $ours })) {
            Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        }
    }
    & $Adb start-server 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'adb start-server failed' }
}

# Serials of emulators adb currently lists (any state).
function Get-EmulatorSerials {
    @(& $Adb devices | Select-String '^(emulator-\d+)\s' | ForEach-Object { $_.Matches[0].Groups[1].Value })
}

# Pick the serial to talk to: the one given, else the only running emulator.
function Resolve-Serial([string]$Serial) {
    if ($Serial) { return $Serial }
    $all = @(Get-EmulatorSerials)
    if ($all.Count -eq 0) { throw 'No emulator is running. Start one with tools\emulator\start.ps1' }
    if ($all.Count -gt 1) { throw "More than one emulator is running ($($all -join ', ')). Pass -Serial." }
    return $all[0]
}
