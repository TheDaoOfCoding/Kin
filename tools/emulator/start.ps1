# Boots an Android emulator and waits until it is ready. Prints the adb serial (emulator-5554).
#
#   .\start.ps1                    headless kin_api37 (no window, no audio)
#   .\start.ps1 -Avd kin_api29     the minSdk (API 29) image
#   .\start.ps1 -Window            show the emulator window
#
# Exit code 0 = booted and ready. The last output line is the serial. Stop it with .\stop.ps1.
# Cold boot every time (no snapshots), so each run starts from a clean device.
param(
    [string]$Avd = 'kin_api37',
    [switch]$Window,
    [int]$Port = 5554,
    [int]$TimeoutSec = 300
)
. "$PSScriptRoot\_common.ps1"

$serial = "emulator-$Port"
Initialize-Adb

# Already running? Reuse it if it is the same AVD, otherwise refuse (one emulator at a time keeps RAM sane).
$running = @(Get-EmulatorSerials)
if ($running) {
    $name = (& $Adb -s $running[0] emu avd name 2>$null | Select-Object -First 1)
    if ($name) { $name = $name.Trim() }
    if ($name -eq $Avd) { Write-Host "$Avd is already running."; Write-Output $running[0]; return }
    throw "Emulator $($running -join ', ') ($name) is already running. Run .\stop.ps1 -All first (plain .\stop.ps1 only stops kin_* AVDs)."
}

if (-not (& $Emulator -list-avds | Where-Object { $_ -eq $Avd })) { throw "No AVD named '$Avd'. Existing: $(& $Emulator -list-avds)" }

$log = Join-Path $PSScriptRoot "emulator-$Avd.log"
$emuArgs = @('-avd', $Avd, '-port', $Port, '-no-snapshot', '-no-boot-anim', '-no-metrics', '-gpu', 'swiftshader_indirect')
if (-not $Window) { $emuArgs += '-no-window', '-no-audio' }

$sw = [Diagnostics.Stopwatch]::StartNew()
$env:ANDROID_HOME = $Sdk
Start-Process -FilePath $Emulator -ArgumentList $emuArgs -WindowStyle Hidden `
    -RedirectStandardOutput $log -RedirectStandardError "$log.err" | Out-Null
Write-Host "Starting $Avd (log: $log)..."

# -All is safe here: we only get this far when no other emulator was running, so the only one left is the one we just started
# (stop.ps1 alone would leave it running if $Avd is not a kin_* AVD).
function Fail($msg) { & "$PSScriptRoot\stop.ps1" -All | Out-Null; throw "$msg See $log" }

# 1. device shows up in adb
while ((& $Adb devices | Out-String) -notmatch "$serial\s+device") {
    if ($sw.Elapsed.TotalSeconds -gt $TimeoutSec) { Fail "Timed out waiting for $serial to appear." }
    if (-not (Get-Process -Name 'qemu-system*', 'emulator' -ErrorAction SilentlyContinue)) {
        Fail 'The emulator process exited early (is acceleration available? run: emulator -accel-check).'
    }
    Start-Sleep -Seconds 2
}
# 2. Android finished booting
while ($true) {
    $done = (& $Adb -s $serial shell getprop sys.boot_completed 2>$null | Out-String).Trim()
    if ($done -eq '1') { break }
    if ($sw.Elapsed.TotalSeconds -gt $TimeoutSec) { Fail "Timed out waiting for sys.boot_completed on $serial." }
    Start-Sleep -Seconds 2
}
# 3. package manager answers (boot_completed can come slightly before installs work)
while ((& $Adb -s $serial shell pm path android 2>$null | Out-String) -notmatch 'package:') {
    if ($sw.Elapsed.TotalSeconds -gt $TimeoutSec) { Fail 'Timed out waiting for the package manager.' }
    Start-Sleep -Seconds 1
}

# Quieter, more repeatable UI for scripted runs: no animations, stay awake while plugged in, unlock.
foreach ($s in 'window_animation_scale', 'transition_animation_scale', 'animator_duration_scale') {
    & $Adb -s $serial shell settings put global $s 0 | Out-Null
}
& $Adb -s $serial shell settings put global stay_on_while_plugged_in 7 | Out-Null
& $Adb -s $serial shell input keyevent KEYCODE_WAKEUP | Out-Null
& $Adb -s $serial shell wm dismiss-keyguard | Out-Null

Write-Host ("{0} ready in {1:N0} s" -f $Avd, $sw.Elapsed.TotalSeconds)
Write-Output $serial
