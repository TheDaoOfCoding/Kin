# Runs a developer/test Kin.exe BESIDE the user's installed Kin, without ever touching the installed one. Dot-source it (after kin-lock.ps1):
#
#   . (Join-Path $PSScriptRoot 'kin-dev.ps1')
#   Set-KinDev -Exe $Kin -DataDir $Data         # once, when the throwaway data directory is chosen
#   $p = Start-DevKin $KinArgs                  # --data-dir <Data> --dev-bind 127.0.0.1 ... (refuses anything else)
#   Invoke-Kin --send-clipboard                 # & Kin.exe --data-dir <Data> --send-clipboard : a command for OUR instance only
#   Get-DevKinProcesses                         # the processes of our instance (same program file AND same --data-dir)
#   Stop-DevKin                                 # --quit to our instance, then (if it hangs) Stop-Process -Id of OUR processes only
#
# ASCII only (Windows PowerShell 5.1 reads a script with no BOM as ANSI).
#
# The rules this file keeps (docs/results/v0-1-1-desktop.md, "Dev Kin beside the installed Kin"):
#  * An instance is its data directory. Kin.exe keys its single-instance lock, toast identity, port, web view data and mDNS by it (instance.rs),
#    so a Kin with a data directory of its own runs beside the installed one, which uses %APPDATA%\Kin. The installed Kin is never started,
#    stopped, asked or killed from here, and a script that would use %APPDATA%\Kin as its data directory stops with an error.
#  * Every command to Kin.exe carries --data-dir, so it reaches OUR instance. (A launch with no --data-dir is the installed Kin's: Kin.exe itself
#    refuses it from a program file that is not the one that is running, but a script must never depend on that.)
#  * Nothing is ever killed by name. A process is ours if its program file is the one we were given AND its command line names our data directory
#    (Get-DevKinProcesses); the only thing a script stops is such a process, by its ID.
#  * The refusal "another Kin is running" is for a Kin on OUR data directory (or the lock of kin-lock.ps1), never for the installed Kin.
$script:KinInstalledDataDir = Join-Path $env:APPDATA 'Kin'
$script:KinDevExe = $null
$script:KinDevData = $null

function ConvertTo-KinNormalPath([string]$Path) {
    # The same idea as instance.rs: absolute, `..` resolved, one kind of separator, no trailing one, lower case.
    ([IO.Path]::GetFullPath($Path)).TrimEnd('\').ToLowerInvariant()
}

function Test-KinInstalledDataDir([string]$Path) {
    (ConvertTo-KinNormalPath $Path) -eq (ConvertTo-KinNormalPath $script:KinInstalledDataDir)
}

function Set-KinDev {
    param([Parameter(Mandatory = $true)][string]$Exe, [Parameter(Mandatory = $true)][string]$DataDir)
    if (Test-KinInstalledDataDir $DataDir) {
        throw "The data directory $DataDir is the installed Kin's own. A test Kin needs a throwaway directory of its own (for example under %TEMP%); the installed Kin is never touched."
    }
    $full = (Resolve-Path -LiteralPath $Exe -ErrorAction Stop).Path
    $pf = @($env:ProgramFiles, ${env:ProgramFiles(x86)}) | Where-Object { $_ }
    foreach ($root in $pf) {
        if ($full.StartsWith((Join-Path $root 'Kin') + '\', [StringComparison]::OrdinalIgnoreCase)) {
            throw "$full is the installed Kin. These scripts run a test build (desktop\src-tauri\target\release\Kin.exe), never the installed program."
        }
    }
    $script:KinDevExe = $full
    $script:KinDevData = [IO.Path]::GetFullPath($DataDir)
}

function Get-KinCommandLineDataDir([string]$CommandLine) {
    if ($CommandLine -and $CommandLine -match '(?i)--data-dir(?:=|\s+)(?:"([^"]*)"|(\S+))') { if ($Matches[1]) { return $Matches[1] } else { return $Matches[2] } }
    return $null
}

function Test-KinCommandLineHasDataDir([string]$CommandLine, [string]$DataDir) {
    # Compared as normalised paths, not as text: [IO.Path]::GetFullPath turns an 8.3 name in an existing part of a path ($env:TEMP is
    # C:\Users\USERNA~1\... on some PCs) into the long name, so the directory a script was given and the one on Kin's command line can be spelled differently.
    $d = Get-KinCommandLineDataDir $CommandLine
    if (-not $d) { return $false }
    return (ConvertTo-KinNormalPath $d) -eq (ConvertTo-KinNormalPath $DataDir)
}

function Get-DevKinProcesses {
    # The Kin.exe processes of OUR instance: this program file and this data directory. The installed Kin (another file, another directory), a
    # Kin of another agent and a Kin of another script are never in it.
    if (-not $script:KinDevExe -or -not $script:KinDevData) { return @() }
    $found = @()
    foreach ($c in @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" -ErrorAction SilentlyContinue)) {
        if ($c.ExecutablePath -and ($c.ExecutablePath -ieq $script:KinDevExe) -and (Test-KinCommandLineHasDataDir $c.CommandLine $script:KinDevData)) {
            $p = Get-Process -Id $c.ProcessId -ErrorAction SilentlyContinue
            if ($p) { $found += $p }
        }
    }
    return $found
}

function Get-OtherKinProcesses {
    # For information only: Kin.exe processes that are not ours (the installed Kin, a Kin of another agent). Never used to decide anything but a message.
    $mine = @(Get-DevKinProcesses | ForEach-Object { $_.Id })
    @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" -ErrorAction SilentlyContinue | Where-Object { $mine -notcontains $_.ProcessId })
}

function Assert-NoDevKinOnDataDir {
    # The refusal that replaces "Kin.exe is already running": only a Kin on OUR data directory is in the way.
    $p = @(Get-DevKinProcesses)
    if ($p.Count -gt 0) { throw "A Kin is already running on the data directory $script:KinDevData (process $($p[0].Id))." }
    $others = @(Get-OtherKinProcesses)
    if ($others.Count -gt 0) {
        Write-Host ("NOTE: {0} other Kin.exe process(es) are running ({1}). They are not touched: this run has its own data directory, port, single-instance lock and toast identity." -f $others.Count, (($others | ForEach-Object { $_.ProcessId }) -join ', '))
    }
}

function Invoke-Kin {
    # A command for OUR instance: Kin.exe --data-dir <ours> <arguments>. (A simple function on purpose: --dev-... tokens stay plain strings in $args.)
    if (-not $script:KinDevExe) { throw 'Invoke-Kin: call Set-KinDev first' }
    if ($args -contains '--data-dir') { throw 'Invoke-Kin adds --data-dir itself' }
    & $script:KinDevExe '--data-dir' $script:KinDevData @args
}

function Start-DevKin {
    # Starts OUR Kin. The arguments must name our data directory and --dev-bind (loopback only), or nothing starts.
    param([Parameter(Mandatory = $true)][string[]]$ArgumentList)
    if (-not $script:KinDevExe) { throw 'Start-DevKin: call Set-KinDev first' }
    $text = ($ArgumentList -join ' ')
    if (-not (Test-KinCommandLineHasDataDir $text $script:KinDevData)) { throw "Start-DevKin: the arguments must contain --data-dir for $script:KinDevData" }
    if ($ArgumentList -notcontains '--dev-bind') { throw 'Start-DevKin: a test Kin always runs with --dev-bind 127.0.0.1 (loopback only, no mDNS)' }
    Assert-NoDevKinOnDataDir
    Start-Process $script:KinDevExe -ArgumentList $ArgumentList -PassThru
}

function Stop-DevKin {
    # Quit OUR Kin (the --quit goes to our data directory), wait for it, and if it hangs stop OUR process by its ID (path and data directory verified).
    param([int]$TimeoutSec = 10)
    if (-not $script:KinDevExe -or @(Get-DevKinProcesses).Count -eq 0) { return }
    try { Invoke-Kin --quit 2>$null | Out-Null } catch { }
    $end = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $end -and @(Get-DevKinProcesses).Count -gt 0) { Start-Sleep -Milliseconds 250 }
    foreach ($p in @(Get-DevKinProcesses)) {
        Write-Host "Kin (process $($p.Id)) did not quit in $TimeoutSec s: stopping that process"
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }
}

function Stop-OwnProcess {
    # Stops a process that THIS script started and still holds ($Process from Start-Process -PassThru), after checking that it is still the program
    # we started (its ID can be reused). Never by name.
    param($Process, [string]$ExpectedPath = '')
    if (-not $Process) { return }
    try {
        $p = Get-Process -Id $Process.Id -ErrorAction Stop
        if ($p.StartTime -ne $Process.StartTime) { return }
        if ($ExpectedPath -and $p.Path -and ($p.Path -ine $ExpectedPath)) { return }
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    } catch { }
}

function Get-RunningGames {
    # A GAME that is running now (a heuristic): a process of a known game or of an anti-cheat that only runs with a game, or a process that runs from a
    # game library folder. The rule behind it is "no disturbance while the user PLAYS", so an idle launcher or service does NOT count: the Epic launcher
    # and its helpers (EpicGamesLauncher, EpicWebHelper, EpicOnlineServicesUserHelper, EOSOverlayRenderer), the Steam client, the Riot client and the
    # EasyAntiCheat_EOS service (which stays up for hours after the game closes) are ignored. To add a game, add a pattern to $names or $paths.
    $names = '^FortniteClient-Win64-Shipping', '^FortniteLauncher$', '^FortniteBootstrapper$', '^EasyAntiCheat(?!_EOS$)', '^BEService$', '^GameOverlayUI$', '^VALORANT-Win64-Shipping$', '^cs2$', '^csgo$', '^RocketLeague$'
    $paths = '\\steamapps\\common\\', '\\Epic Games\\(?!Launcher\\|Epic Online Services\\)[^\\]+\\', '\\Riot Games\\(VALORANT|League of Legends)\\', '\\XboxGames\\', '\\Origin Games\\', '\\EA Games\\', '\\Ubisoft Game Launcher\\games\\', '\\GOG Galaxy\\Games\\'
    $pathRx = '(?i)' + ($paths -join '|')
    $found = @()
    foreach ($p in @(Get-Process -ErrorAction SilentlyContinue)) {
        $hit = $false
        foreach ($n in $names) { if ($p.ProcessName -match $n) { $hit = $true; break } }
        if ($hit) { $found += $p.ProcessName; continue }
        $path = $null; try { $path = $p.Path } catch { }
        if ($path -and ($path -match $pathRx)) { $found += $p.ProcessName }
    }
    return @($found | Sort-Object -Unique)
}

function Get-UserNotificationState {
    # Windows' own answer to "is the user in a game or a full-screen program?": SHQueryUserNotificationState. Returns the name of the state, or $null if
    # the call failed (no interactive desktop). Works for any game, not only the ones in Get-RunningGames.
    if (-not ('KinDevShell.Quns' -as [type])) {
        Add-Type -Namespace KinDevShell -Name Quns -MemberDefinition '[System.Runtime.InteropServices.DllImport("shell32.dll")] public static extern int SHQueryUserNotificationState(out int state);'
    }
    $s = 0
    $hr = [KinDevShell.Quns]::SHQueryUserNotificationState([ref]$s)
    if ($hr -ne 0) { return $null }
    switch ($s) {
        1 { 'QUNS_NOT_PRESENT' } 2 { 'QUNS_BUSY' } 3 { 'QUNS_RUNNING_D3D_FULL_SCREEN' } 4 { 'QUNS_PRESENTATION_MODE' }
        5 { 'QUNS_ACCEPTS_NOTIFICATIONS' } 6 { 'QUNS_QUIET_TIME' } 7 { 'QUNS_APP' } default { "QUNS_$s" }
    }
}

function Assert-NoGameRunning {
    # Stops (throws) when a game runs (Get-RunningGames) OR Windows reports a full-screen / game / presentation state. Idle launchers and services alone do
    # not stop anything. Says what it saw, so a run's log shows why it went on. Cheap (one process list and one call): call it again at the start of every
    # visible step.
    $g = @(Get-RunningGames)
    $state = $null; try { $state = Get-UserNotificationState } catch { }
    $busy = @('QUNS_BUSY', 'QUNS_RUNNING_D3D_FULL_SCREEN', 'QUNS_PRESENTATION_MODE') -contains $state
    if ($g.Count -gt 0 -or $busy) {
        $why = @()
        if ($g.Count -gt 0) { $why += ('game process(es): ' + ($g -join ', ')) }
        if ($busy) { $why += "Windows says $state" }
        throw ("A game seems to be running ({0}): not showing anything now. Close the game and run this again." -f ($why -join '; '))
    }
    Write-Host ("Game guard: no game running (Windows notification state: {0}; idle launchers and services do not count)." -f $(if ($state) { $state } else { 'unknown' }))
}

function Remove-DevDataDir {
    # Deletes a throwaway directory of a test Kin. A Kin that showed a window has a web view whose helper processes (msedgewebview2) hold files in
    # <data dir>\webview for a moment after Kin has quit, so the delete is tried again for a few seconds. Never fails the caller.
    param([Parameter(Mandatory = $true)][string]$Path, [int]$Seconds = 20)
    $end = (Get-Date).AddSeconds($Seconds)
    while ([IO.Directory]::Exists($Path)) {
        try { [IO.Directory]::Delete($Path, $true) } catch { }
        if (-not [IO.Directory]::Exists($Path) -or (Get-Date) -ge $end) { break }
        Start-Sleep -Milliseconds 500
    }
    if ([IO.Directory]::Exists($Path)) { Write-Host "could not remove $Path (a web view process still holds it)" }
}
