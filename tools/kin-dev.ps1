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
# The rules this file keeps ("Dev Kin beside the installed Kin", see desktop/README.md):
#  * An instance is its data directory. Kin.exe keys its single-instance lock, toast identity, port, web view data and mDNS by it (instance.rs),
#    so a Kin with a data directory of its own runs beside the installed one, which uses %APPDATA%\Kin. The installed Kin is never started,
#    stopped, asked or killed from here, and a script that would use %APPDATA%\Kin as its data directory stops with an error.
#  * Every command to Kin.exe carries --data-dir, so it reaches OUR instance. (A launch with no --data-dir is the installed Kin's: Kin.exe itself
#    refuses it from a program file that is not the one that is running, but a script must never depend on that.)
#  * Nothing is ever killed by name. A process is ours if its program file is the one we were given AND its command line names our data directory
#    (Get-DevKinProcesses); the only thing a script stops is such a process, by its ID.
#  * The refusal "another Kin is running" is for a Kin on OUR data directory (or the lock of kin-lock.ps1), never for the installed Kin.
#  * THE REAL CLIPBOARD IS NEVER USED BY DEFAULT (2B.6). Every test Kin gets --dev-clipboard-sink <dir>: Kin.exe then never opens the Windows clipboard, its writes
#    go to files in <dir> (write-<n>.txt / .dib / .png and index.json) and its clipboard reads and copy events come from <dir>\input.* (see
#    desktop\src-tauri\src\clipboard_sink.rs). The helpers below (Get-Clipboard, Set-QuietClipboardText, Get-ClipboardFormats, Save-Clipboard, ...) are the sink's
#    in sink mode, so a script reads and "copies" through files, and the user's clipboard is not touched. A script opts in to the real clipboard only with
#    Set-KinDev -RealClipboard (loud, one script: tools\check-real-clipboard.ps1).
$script:KinInstalledDataDir = Join-Path $env:APPDATA 'Kin'
$script:KinDevExe = $null
$script:KinDevData = $null
$script:KinClipboardSink = $null      # the sink directory of this run; $null in the real-clipboard mode and before Set-KinDev
$script:KinRealClipboard = $false     # Set-KinDev -RealClipboard
$script:KinSinkCounter = 0

function ConvertTo-KinNormalPath([string]$Path) {
    # The same idea as instance.rs: absolute, `..` resolved, one kind of separator, no trailing one, lower case.
    ([IO.Path]::GetFullPath($Path)).TrimEnd('\').ToLowerInvariant()
}

function Test-KinInstalledDataDir([string]$Path) {
    (ConvertTo-KinNormalPath $Path) -eq (ConvertTo-KinNormalPath $script:KinInstalledDataDir)
}

function Get-KinSinkDirFor([string]$DataDir) {
    # A SIBLING of the data directory, not inside it: the sink holds the text of the test clipboard, and the privacy greps of the e2e scripts search the data directory.
    ([IO.Path]::GetFullPath($DataDir)).TrimEnd('\') + '-clipboard-sink'
}

function Set-KinDev {
    # -RealClipboard: LOUD opt-in to the user's real Windows clipboard (only tools\check-real-clipboard.ps1 does it). Without it every Kin of this run gets
    # --dev-clipboard-sink <ClipboardSink> (default: <DataDir>-clipboard-sink) and never touches the real clipboard.
    param([Parameter(Mandatory = $true)][string]$Exe, [Parameter(Mandatory = $true)][string]$DataDir, [switch]$RealClipboard, [string]$ClipboardSink = '')
    if (Test-KinInstalledDataDir $DataDir) {
        throw "The data directory $DataDir is the installed Kin's own. A test Kin needs a throwaway directory of its own (for example under %TEMP%); the installed Kin is never touched."
    }
    if ($RealClipboard -and $ClipboardSink) { throw 'Set-KinDev: -RealClipboard and -ClipboardSink exclude each other' }
    $full = (Resolve-Path -LiteralPath $Exe -ErrorAction Stop).Path
    $pf = @($env:ProgramFiles, ${env:ProgramFiles(x86)}) | Where-Object { $_ }
    foreach ($root in $pf) {
        if ($full.StartsWith((Join-Path $root 'Kin') + '\', [StringComparison]::OrdinalIgnoreCase)) {
            throw "$full is the installed Kin. These scripts run a test build (desktop\src-tauri\target\release\Kin.exe), never the installed program."
        }
    }
    $script:KinDevExe = $full
    $script:KinDevData = [IO.Path]::GetFullPath($DataDir)
    $script:KinRealClipboard = [bool]$RealClipboard
    if ($RealClipboard) {
        $script:KinClipboardSink = $null
        $env:KIN_DEV_CLIPBOARD_SINK = ''
        Write-Host '*** REAL CLIPBOARD: this run uses the user''s real Windows clipboard (-RealClipboard). It must have been started on purpose. ***' -ForegroundColor Red
    } else {
        $sink = if ($ClipboardSink) { [IO.Path]::GetFullPath($ClipboardSink) } else { Get-KinSinkDirFor $script:KinDevData }
        # What an earlier run left in a sink of the same name would make the numbers of the writes lie (a stale write-1.txt): start from nothing, but only when no Kin of ours is on it.
        if ([IO.Directory]::Exists($sink) -and ($sink -like '*clipboard-sink*') -and @(Get-DevKinProcesses).Count -eq 0) { try { [IO.Directory]::Delete($sink, $true) } catch { } }
        # Sinks left by runs that did not clean up (a script that deletes its data directory by itself) are swept once they are a day old.
        $parent = Split-Path -Parent $sink
        if ($parent -and [IO.Directory]::Exists($parent)) {
            foreach ($old in @(Get-ChildItem -LiteralPath $parent -Directory -Filter '*-clipboard-sink' -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTime -lt (Get-Date).AddDays(-1) })) {
                try { [IO.Directory]::Delete($old.FullName, $true) } catch { }
            }
        }
        $script:KinClipboardSink = $sink
        # In the environment too: every Kin that a child of this script starts (a bun e2e script, a smoke test, a cargo example) gets the sink without a flag.
        $env:KIN_DEV_CLIPBOARD_SINK = $sink
        Write-Host "Clipboard sink: the real clipboard is not used; Kin writes to $sink"
    }
}

$script:KinSinkCapable = @{}
function Test-KinExeHasSink([string]$Exe) {
    # Whether this Kin.exe knows --dev-clipboard-sink (the flag's text is in the program file). A build from before 2B.6 (the "old PC" of e2e-2b-1, built from the v0.1.1 tag)
    # ignores the flag as an unknown argument and would use the REAL clipboard, so Start-DevKin refuses it in sink mode.
    $item = Get-Item -LiteralPath $Exe -ErrorAction Stop
    $key = '{0}|{1}' -f $item.FullName.ToLowerInvariant(), $item.LastWriteTimeUtc.Ticks
    if (-not $script:KinSinkCapable.ContainsKey($key)) {
        $text = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($item.FullName))
        $script:KinSinkCapable[$key] = $text.Contains('--dev-clipboard-sink')
    }
    return [bool]$script:KinSinkCapable[$key]
}

function Get-KinClipboardSink { $script:KinClipboardSink }
function Test-KinRealClipboard { $script:KinRealClipboard }

function Add-KinSinkArgument([string[]]$Arguments) {
    # The arguments plus --dev-clipboard-sink <dir> for a sink run (unless they already have it); a real-clipboard run must not have it.
    $has = ($Arguments -contains '--dev-clipboard-sink') -or (@($Arguments | Where-Object { $_ -like '--dev-clipboard-sink=*' }).Count -gt 0)
    if ($script:KinRealClipboard) { if ($has) { throw 'a -RealClipboard run must not pass --dev-clipboard-sink' }; return $Arguments }
    if (-not $script:KinClipboardSink) { throw 'the clipboard mode is not set: call Set-KinDev first (Kin never runs from a script without --dev-clipboard-sink unless -RealClipboard was given)' }
    if ($has) { return $Arguments }
    return @($Arguments) + @('--dev-clipboard-sink', ('"{0}"' -f $script:KinClipboardSink))
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
    # Kin of another test run and a Kin of another script are never in it.
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
    # For information only: Kin.exe processes that are not ours (the installed Kin, a Kin of another test run). Never used to decide anything but a message.
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
    # --dev-clipboard-sink too: if no Kin is running on this data directory this launch BECOMES the Kin, and it must not be one that opens the real clipboard.
    if ($script:KinClipboardSink) {
        # The same old-build refusal as Start-DevKin: a Kin without the flag would ignore it and open the user's REAL clipboard.
        if (-not (Test-KinExeHasSink $script:KinDevExe)) { throw "Invoke-Kin: $script:KinDevExe does not know --dev-clipboard-sink (a build from before 2B.6): it would open the user's REAL clipboard. Use a current build, or run on purpose with Set-KinDev -RealClipboard." }
        & $script:KinDevExe '--data-dir' $script:KinDevData '--dev-clipboard-sink' $script:KinClipboardSink @args
    }
    elseif ($script:KinRealClipboard) { & $script:KinDevExe '--data-dir' $script:KinDevData @args }
    else { throw 'Invoke-Kin: the clipboard mode is not set (Set-KinDev)' }
}

function Start-DevKin {
    # Starts OUR Kin. The arguments must name our data directory and --dev-bind (loopback only), or nothing starts.
    param([Parameter(Mandatory = $true)][string[]]$ArgumentList)
    if (-not $script:KinDevExe) { throw 'Start-DevKin: call Set-KinDev first' }
    $text = ($ArgumentList -join ' ')
    if (-not (Test-KinCommandLineHasDataDir $text $script:KinDevData)) { throw "Start-DevKin: the arguments must contain --data-dir for $script:KinDevData" }
    if ($ArgumentList -notcontains '--dev-bind') { throw 'Start-DevKin: a test Kin always runs with --dev-bind 127.0.0.1 (loopback only, no mDNS)' }
    Assert-NoDevKinOnDataDir
    # Every test Kin runs with --dev-clipboard-sink unless this run said -RealClipboard (Set-KinDev): it then never opens the user's clipboard.
    $ArgumentList = Add-KinSinkArgument $ArgumentList
    if ($script:KinClipboardSink -and -not (Test-KinExeHasSink $script:KinDevExe)) {
        throw "Start-DevKin: $script:KinDevExe does not know --dev-clipboard-sink (a build from before 2B.6): it would open the user's REAL clipboard. Use a current build, or run on purpose with Set-KinDev -RealClipboard."
    }
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
    $names = '^EasyAntiCheat(?!_EOS$)', '^BEService$', '^GameOverlayUI$', '^VALORANT-Win64-Shipping$', '^cs2$', '^csgo$', '^RocketLeague$'
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
    # The clipboard sink of that data directory (a sibling: Get-KinSinkDirFor) goes with it.
    $sink = Get-KinSinkDirFor $Path
    if ([IO.Directory]::Exists($sink)) { try { [IO.Directory]::Delete($sink, $true) } catch { Write-Host "could not remove $sink" } }
}

# ---- the clipboard sink: reading what Kin wrote, and "copying" for the PC to phone sync (all of it through files; the real clipboard is never involved) ----

function Read-KinSinkText([string]$Name) {
    # The file's text, or $null if it is not there. Opened so that Kin can replace it while we read (FileShare.Delete), tried again if Kin has it for a moment.
    $sink = $script:KinClipboardSink
    if (-not $sink) { return $null }
    $path = Join-Path $sink $Name
    for ($i = 0; $i -lt 20; $i++) {
        if (-not [IO.File]::Exists($path)) { return $null }
        try {
            $fs = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
            try { $r = New-Object IO.StreamReader($fs, (New-Object Text.UTF8Encoding($false))); return $r.ReadToEnd() } finally { $fs.Dispose() }
        } catch { Start-Sleep -Milliseconds 25 }
    }
    return $null
}

function Read-KinSinkBytes([string]$Name) {
    $sink = $script:KinClipboardSink
    if (-not $sink) { return $null }
    $path = Join-Path $sink $Name
    for ($i = 0; $i -lt 20; $i++) {
        if (-not [IO.File]::Exists($path)) { return $null }
        try {
            $fs = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
            try { $ms = New-Object IO.MemoryStream; $fs.CopyTo($ms); return , $ms.ToArray() } finally { $fs.Dispose() }
        } catch { Start-Sleep -Milliseconds 25 }
    }
    return $null
}

function Get-KinSinkIndex {
    # index.json of the sink: { version, writes: [ { n, kind (text|image|clear), file, formats: [ { name, bytes, dword } ], flags: { sensitive, quiet, excludedFromMonitor, excludedFromHistory, excludedFromCloud, testContent } } ] }
    $t = Read-KinSinkText 'index.json'
    if (-not $t) { return $null }
    try { $t | ConvertFrom-Json } catch { $null }
}

function Get-KinSinkWrites {
    # What Kin wrote so far, in order (an empty array when nothing).
    $i = Get-KinSinkIndex
    if ($null -eq $i -or $null -eq $i.writes) { return @() }
    return @($i.writes)
}

function Get-KinSinkCurrent {
    # current.json: what a read of the clipboard sees now ({ sequence, source (app|input), text, formats }).
    $t = Read-KinSinkText 'current.json'
    if (-not $t) { return $null }
    try { $t | ConvertFrom-Json } catch { $null }
}

function Set-KinSinkInput {
    # The user copies something: input.txt (and input.formats, one opt-out format per line, `name` or `name=dword`), THEN a new value in input.seq. Kin polls
    # input.seq every 100 ms; a new value is one copy event, handled like a change of the real clipboard (debounce, exclusion rules, loop guard).
    param([string]$Text = '', [string[]]$Formats = @(), [switch]$NoText)
    $sink = $script:KinClipboardSink
    if (-not $sink) { throw 'Set-KinSinkInput: this run has no clipboard sink (it is in the real-clipboard mode or Set-KinDev was not called)' }
    [IO.Directory]::CreateDirectory($sink) | Out-Null
    if ($NoText) { [IO.File]::Delete((Join-Path $sink 'input.txt')) } else { [IO.File]::WriteAllText((Join-Path $sink 'input.txt'), $Text, (New-Object Text.UTF8Encoding($false))) }
    if (@($Formats).Count -gt 0) { [IO.File]::WriteAllText((Join-Path $sink 'input.formats'), ((@($Formats) -join "`n") + "`n"), [Text.Encoding]::ASCII) } else { [IO.File]::Delete((Join-Path $sink 'input.formats')) }
    $script:KinSinkCounter++
    [IO.File]::WriteAllText((Join-Path $sink 'input.seq'), ('{0}-{1}-{2}' -f $PID, $script:KinSinkCounter, [Environment]::TickCount), [Text.Encoding]::ASCII)
}

function Get-PcClipboardText {
    # The text on "the PC clipboard" of this run: the sink's (what Kin wrote last, or what Set-KinSinkInput copied), or in a -RealClipboard run the real one. $null if none.
    if ($script:KinClipboardSink) { $c = Get-KinSinkCurrent; if ($c) { return $c.text } else { return $null } }
    if ($script:KinRealClipboard) { try { return (Microsoft.PowerShell.Management\Get-Clipboard -Raw -ErrorAction SilentlyContinue) } catch { return $null } }
    throw 'Get-PcClipboardText: the clipboard mode is not set (Set-KinDev)'
}

function Get-KinSinkLastImageWrite {
    # The newest image write of the sink when it is still what the clipboard holds (CF_DIBV5 in current.json), else $null.
    $c = Get-KinSinkCurrent
    if (-not $c -or -not (@($c.formats | ForEach-Object { $_.name }) -contains 'CF_DIBV5')) { return $null }
    @(Get-KinSinkWrites | Where-Object { $_.kind -eq 'image' }) | Select-Object -Last 1
}

function Convert-KinDibToBitmap([byte[]]$Dib) {
    # The bytes of write-<n>.dib (a BITMAPV5HEADER, then bottom-up 32-bit BGRA pixels) as a System.Drawing.Bitmap (top-down, like Windows.Forms.Clipboard.GetImage()).
    Add-Type -AssemblyName System.Drawing
    $w = [BitConverter]::ToInt32($Dib, 4); $h = [BitConverter]::ToInt32($Dib, 8)
    $fmt = [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
    $bmp = New-Object System.Drawing.Bitmap($w, $h, $fmt)
    $data = $bmp.LockBits((New-Object System.Drawing.Rectangle(0, 0, $w, $h)), [System.Drawing.Imaging.ImageLockMode]::WriteOnly, $fmt)
    try {
        for ($y = 0; $y -lt $h; $y++) {
            [Runtime.InteropServices.Marshal]::Copy($Dib, 124 + ($h - 1 - $y) * $w * 4, [IntPtr]($data.Scan0.ToInt64() + [int64]$y * $data.Stride), $w * 4)
        }
    } finally { $bmp.UnlockBits($data) }
    return , $bmp
}

function Get-PcClipboardImage {
    # The picture on "the PC clipboard" as a Bitmap ($null if there is none): the sink's newest image write, or in a -RealClipboard run Windows.Forms' GetImage.
    if ($script:KinClipboardSink) {
        $w = Get-KinSinkLastImageWrite
        if (-not $w) { return $null }
        $bytes = Read-KinSinkBytes $w.file
        if (-not $bytes) { return $null }
        return (Convert-KinDibToBitmap $bytes)
    }
    if ($script:KinRealClipboard) { Add-Type -AssemblyName System.Windows.Forms; try { return [Windows.Forms.Clipboard]::GetImage() } catch { return $null } }
    throw 'Get-PcClipboardImage: the clipboard mode is not set (Set-KinDev)'
}

function Get-PcClipboardFileDropList {
    # The files on "the PC clipboard" (CF_HDROP): the source file of the sink's newest image write, or the real list in a -RealClipboard run.
    if ($script:KinClipboardSink) {
        $c = Get-KinSinkCurrent
        if ($c -and (@($c.formats | ForEach-Object { $_.name }) -contains 'CF_HDROP')) { $w = Get-KinSinkLastImageWrite; if ($w) { return @($w.source) } }
        return @()
    }
    if ($script:KinRealClipboard) { Add-Type -AssemblyName System.Windows.Forms; try { return @([Windows.Forms.Clipboard]::GetFileDropList()) } catch { return @() } }
    throw 'Get-PcClipboardFileDropList: the clipboard mode is not set (Set-KinDev)'
}

function Get-PcClipboardPngBytes {
    # The bytes of the `PNG` format on "the PC clipboard" ($null if it has none): the sink's write-<n>.png, or the real format in a -RealClipboard run.
    if ($script:KinClipboardSink) {
        $w = Get-KinSinkLastImageWrite
        if (-not $w -or -not $w.png) { return $null }
        return (Read-KinSinkBytes $w.png)
    }
    if ($script:KinRealClipboard) {
        Add-Type -AssemblyName System.Windows.Forms
        $s = $null; try { $s = [Windows.Forms.Clipboard]::GetData('PNG') } catch { }
        if ($s -is [IO.Stream]) { $ms = New-Object IO.MemoryStream; $s.CopyTo($ms); return , $ms.ToArray() }
        return $null
    }
    throw 'Get-PcClipboardPngBytes: the clipboard mode is not set (Set-KinDev)'
}

# The cmdlets Get-Clipboard / Set-Clipboard are shadowed by these functions in every script that loads this file, so a stray `Get-Clipboard -Raw` in a check reads the
# sink and a `Set-Clipboard` "copies" into it. (Functions win over cmdlets. In a -RealClipboard run they call the real cmdlet.)
function Get-Clipboard {
    [CmdletBinding()]
    param([switch]$Raw, [string]$Format, [string]$TextFormatType)
    if ($Format -and $Format -ne 'Text') { throw "Get-Clipboard -Format $Format is not available through the clipboard sink: use Get-PcClipboardImage / Get-PcClipboardFileDropList" }
    if ($script:KinRealClipboard) { return (Microsoft.PowerShell.Management\Get-Clipboard @PSBoundParameters) }
    $t = Get-PcClipboardText
    if ($null -eq $t) { return $null }
    if ($Raw) { return $t }
    return ($t -split "`r?`n")
}

function Set-Clipboard {
    [CmdletBinding()]
    param([Parameter(Position = 0, ValueFromPipeline = $true)]$Value)
    process {
        if ($script:KinRealClipboard) { Microsoft.PowerShell.Management\Set-Clipboard -Value $Value; return }
        # In the sink: a quiet copy (the formats a Kin beside the installed one reads as ordinary text and an installed Kin would skip).
        Set-KinSinkInput ([string]($Value -join "`r`n")) -Formats @('ExcludeClipboardContentFromMonitorProcessing', 'KinRelayDevTestContent')
    }
}
