# Shuts down the Kin emulators (AVDs named kin_*) and makes sure none of their processes is left.
# Emulators of other AVDs on this PC are left alone.
#
#   .\stop.ps1        stops only emulators whose AVD name starts with kin_
#   .\stop.ps1 -All   stops every emulator and kills every emulator process of this SDK
#
# The snapshot cleanup at the end only ever touches kin_*.avd folders, with or without -All.
param([switch]$All)
. "$PSScriptRoot\_common.ps1"

Initialize-Adb

function Test-KinAvd([string]$Name) { $Name -like 'kin_*' }

# Which AVD an emulator runs, asked over adb. Empty when it can not answer (still booting, offline).
function Get-AvdName([string]$Serial) {
    $n = & $Adb -s $Serial emu avd name 2>$null | Select-Object -First 1
    if ($n) { $n.Trim() } else { '' }
}

# Processes that belong to this SDK's emulator (the launcher and the qemu child). In Kin-only mode: only those started
# with -avd kin_*. crashpad_handler is handled separately, because its command line does not name the AVD.
function Get-EmulatorProcesses([bool]$KinOnly) {
    $root = "$Sdk\emulator\"
    Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath.StartsWith($root, [StringComparison]::OrdinalIgnoreCase) -and $_.Name -ne 'crashpad_handler.exe' -and
        (-not $KinOnly -or $_.CommandLine -match '(^|\s)-avd\s+"?kin_')
    }
}

$kinOnly = -not $All
foreach ($s in Get-EmulatorSerials) {
    $name = Get-AvdName $s
    if ($All -or (Test-KinAvd $name)) {
        Write-Host "Killing $s ($name)"
        & $Adb -s $s emu kill 2>$null | Out-Null
    } elseif ($name) {
        Write-Host "Leaving $s ($name) running. Use -All to stop it too."
    } else {
        Write-Host "Could not read the AVD name of $s. It is only stopped below if its process was started with -avd kin_*."
    }
}

# Give them a few seconds to exit cleanly, then force whatever is left.
for ($i = 0; $i -lt 25; $i++) {
    if (-not (Get-EmulatorProcesses $kinOnly)) { break }
    Start-Sleep -Seconds 1
}
Get-EmulatorProcesses $kinOnly | ForEach-Object {
    Write-Host "Force-killing $($_.Name) ($($_.ProcessId))"
    Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
}
# crashpad_handler stays around after its emulator is gone. Kill this SDK's handlers only when no emulator process is left at all.
if (-not (Get-EmulatorProcesses $false)) {
    Get-CimInstance Win32_Process -Filter "Name='crashpad_handler.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith("$Sdk\emulator\", [StringComparison]::OrdinalIgnoreCase) } |
        ForEach-Object { Write-Host "Force-killing crashpad_handler ($($_.ProcessId))"; Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
}

# Leave no adb server behind, unless an emulator we were told to keep still needs it.
$stillRunning = @(Get-EmulatorSerials)
if ($stillRunning.Count -eq 0) {
    & $Adb kill-server 2>$null | Out-Null
    Write-Host 'No emulator running.'
} else {
    Write-Host "Still running: $($stillRunning -join ', '). The adb server stays up for it."
}

# The emulator writes a multi-GB RAM snapshot on exit even with -no-snapshot. We always cold boot, so it is never
# used: delete it to save disk. ONLY the kin_*.avd folders: other AVDs on this PC may rely on their snapshots.
$avdHome = if ($env:ANDROID_AVD_HOME) { $env:ANDROID_AVD_HOME } else { Join-Path $env:USERPROFILE '.android\avd' }
Get-ChildItem $avdHome -Directory -Filter 'kin_*.avd' -ErrorAction SilentlyContinue |
    ForEach-Object { Remove-Item (Join-Path $_.FullName 'snapshots') -Recurse -Force -ErrorAction SilentlyContinue }
