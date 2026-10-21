# Milestone 2B.5 integration (Focus/DND sync, alerts, Kin.exe --notify, PC battery on the phone, the extension's download alerts, the 2B.4 review fixes): the real debug APK on an
# emulator against the REAL release desktop build as a test Kin beside the user's installed one (own --data-dir, loopback only, no window, --dev-toast-quiet, --dev-no-open,
# --dev-focus = a FAKE Windows Focus, a FAKE PC battery). ASCII only.
# SILENT: the emulator runs headless with -no-audio (tools\emulator\start.ps1), the PC plays nothing, nothing is typed or clicked on the PC.
# The user's real Windows Focus / quiet time / Do Not Disturb is NEVER touched: the Focus the test Kin sees is the fake source (--dev-focus on|off, `--dev-set devFocus`).
#
#   tools\e2e-2b-5.ps1                                 everything on kin_api37
#   tools\e2e-2b-5.ps1 -Only S1,S4                     some of them (setup, pairing, the privacy grep S7 and the guard S8 always run)
#   tools\e2e-2b-5.ps1 -Avd kin_api29 -Only S1,S2,S3,S4,S5    the short Android 10 pass (no `cmd notification set_dnd` there: the debug DND_USER stands in for the user)
#   tools\e2e-2b-5.ps1 -Only SETUP -KeepRunning        setup only: leaves the emulator and the test Kin up (and the data folder) for manual probes
#
# S1 phone DND -> PC (presence.dnd: status.json, tooltip, the share switch, paused phone), the PC holds Kin's own low-battery toast while the phone is in DND (and the setting)
# S2 PC Focus -> phone DND (design 10.2 item 2): both access routes, Priority and Alarms, release; what zen_mode and dumpsys notification show per API is RECORDED
# S3 safety: the phone already in DND, user override (a stricter filter; the user turning DND off), kill of the PC while holding (the 60 s grace), force-stop of the phone app,
#    pause (phone, PC), switch off, access revoked, unlink
# S4 alerts: Kin.exe --notify to the dev Kin's pipe (exit codes and lines), the kin_alerts notification, the history rows, queued while the phone is away, expiry, the 10-per-60-s
#    limit, a paused phone, stdin, 301 characters, an old phone (fake), a pipe that is not this Kin, the clipboard untouched
# S5 PC battery low -> the phone's notification (14 %, re-armed by charging, none while paused)
# S6 the extension: headless Chromium, throwaway profile, a download alert reaches the phone; PAUSED_SKIPPED; Kin closed = Kin not started (tools\e2e-2b-5-extension.ts)
# S9 the 2B.4 review fixes: the screenshot hold's Cancel without expanding the shade, an auto-sent upload that loses the connection ends "not sent", the ring alarm volume under DND,
#    the link notification gone after unlink
# S7 privacy grep: no alert text / title / download name / Focus or DND word in a log, the phone's app data or the PC's data folder (beyond the text table and status.json by design)
# S8 guard: the installed Kin untouched, the PC clipboard restored, nothing left running
#
# It never touches the installed Kin (process, data, AUMID, pipe, Run key, SendTo, Downloads, firewall), saves the PC clipboard in every format first and restores it, injects no
# OS input on the PC, kills only processes it started and stops at the next boundary if a game or an anti-cheat starts (the scenarios after it are recorded as PENDING).
param(
    [string]$Avd = 'kin_api37',
    [string[]]$Only = @(),
    # Parts of S9 to leave out (R2 = the 300 MB upload, R3 = the ring volume under DND), for a short re-test of R1 and R4 only.
    [string[]]$Skip = @(),
    # How many Wi-Fi-flap-then-unlink cycles S9 R4b runs (B1 reproduced in 5 of 7 runs before the round 2 fix).
    [int]$R4bCycles = 5,
    [string]$Kin = (Join-Path $PSScriptRoot '..\desktop\src-tauri\target\release\Kin.exe'),
    [string]$Apk = (Join-Path $PSScriptRoot '..\android\app\build\outputs\apk\debug\app-debug.apk'),
    [int]$Port = 49391,
    [string]$ScreenshotDir = 'notes\screens\phase-2b-5',
    [switch]$KeepRunning,
    [string]$Work = ''
)
$Skip = @($Skip | ForEach-Object { $_ -split ',' } | Where-Object { $_ })   # -File passes "R2,R3" as one string
. (Join-Path $PSScriptRoot 'kin-lock.ps1'); Enter-KinLock 'e2e-2b-5'
. (Join-Path $PSScriptRoot 'kin-dev.ps1')
. (Join-Path $PSScriptRoot '..\desktop\clipboard-backup.ps1')
if (-not $Work) { $Work = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-e2e-2b-5-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
Remove-Item Alias:sc -Force -ErrorAction SilentlyContinue
$script:OrigComputerName = $env:COMPUTERNAME
$env:COMPUTERNAME = 'DESK-PC'
# The dev pipe of this run (KIN_DEV_PIPE_SUFFIX): the test Kin listens on app.kinrelay.dev.<SID>.<suffix>, and Kin.exe --notify finds it by the same variable. The installed Kin's pipe has
# another name (app.kinrelay.desktop.<SID>) and this script never builds or passes it.
$PipeSuffix = 'e2e2b5'
$script:OrigPipeSuffix = $env:KIN_DEV_PIPE_SUFFIX
$env:KIN_DEV_PIPE_SUFFIX = $PipeSuffix
$ErrorActionPreference = 'Continue'
foreach ($name in 'ANDROID_HOME', 'JAVA_HOME') {
    if (-not (Get-Item "env:$name" -ErrorAction SilentlyContinue)) { $v = [Environment]::GetEnvironmentVariable($name, 'User'); if ($v) { Set-Item "env:$name" $v } }
}
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$Emu = Join-Path $Repo 'tools\emulator'
$Adb = Join-Path $(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }) 'platform-tools\adb.exe'
$Fake = Join-Path $Repo 'desktop\src-tauri\target\debug\examples\fake_phone.exe'
$Pkg = 'app.kinrelay.android'
$Service = "$Pkg/$Pkg.notifmirror.MirrorListenerService"
$Serial = 'emulator-5554'
$Data = Join-Path $Work 'data'
$Received = Join-Path $Data 'received'
$PhoneLog = Join-Path $Work 'phone-kin-log.txt'
$ScreenDir = if ([IO.Path]::IsPathRooted($ScreenshotDir)) { $ScreenshotDir } else { Join-Path $Repo $ScreenshotDir }
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Results = [ordered]@{}
$script:Notes = New-Object System.Collections.Generic.List[string]
$script:Pending = New-Object System.Collections.Generic.List[string]
$script:Zen = New-Object System.Collections.Generic.List[string]
$started = @{ Emulator = $false; Kin = $false }
$script:Sdk = 0; $script:FakeProcs = @(); $script:Backup = $null; $script:GameStop = $false; $script:KinProc = $null
# What must never be found in a log, in the phone's app data (the alert text lives in the phone's history table by design: see S7), or in the PC's data folder (outside the text table).
$TestStrings = @('ZQAlertText', 'ZQAlertTitle', 'ZQStdinText', 'ZQQueuedText', 'ZQExpiringText', 'ZQBurstText', 'ZQPausedText', 'ZQDownloadName', 'ZQBadgeText')

function A { & $Adb -s $Serial @args }
function Say($m) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $m) }
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $script:Results[$name] = if ($ok) { 'PASS' } else { 'FAIL' }
    if ($ok) { Say "PASS  $name $detail" } else { Say "FAIL  $name $detail"; $script:Failures.Add($name) }
}
function Note([string]$m) { $script:Notes.Add($m); Say "NOTE  $m" }
function Want([string]$Id) { $ids = @($Only | ForEach-Object { $_ -split ',' } | Where-Object { $_ }); $ids.Count -eq 0 -or $ids -contains $Id }
function Wait-Until([scriptblock]$Cond, [double]$Seconds, [int]$PollMs = 250) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds $PollMs }
    return $false
}
# seconds until the condition holds (poll 150 ms), or $null
function Time-Until([scriptblock]$Cond, [double]$Seconds) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) { if (& $Cond) { return [math]::Round($sw.Elapsed.TotalSeconds, 1) }; Start-Sleep -Milliseconds 150 }
    return $null
}
function Enter-Scenario([string]$Id, [string]$Title) {
    if (-not (Want $Id)) { return $false }
    if ($script:GameStop) { $script:Pending.Add("$Id $Title"); return $false }
    try { Assert-NoGameRunning } catch { $script:GameStop = $true; Say "GAME GUARD at the start of ${Id}: $_"; $script:Pending.Add("$Id $Title (a game started: stopped at this boundary)"); return $false }
    Say "--- $Id $Title"
    return $true
}
function Invoke-EmuScript([string]$Script, [string[]]$ScriptArgs = @(), [int]$TimeoutSec = 400) {
    $out = Join-Path $env:TEMP "kin-e2e-2b-5-$Script.out"
    $p = Start-Process powershell -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $Emu $Script))) + $ScriptArgs)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; return $false }
    return $true
}
function Read-Shared([string]$Path) {
    try {
        $fs = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
        try { $ms = New-Object IO.MemoryStream; $fs.CopyTo($ms); return [Text.Encoding]::GetEncoding(28591).GetString($ms.ToArray()) } finally { $fs.Dispose() }
    } catch { return '' }
}
function Shot([string]$Name) {
    New-Item -ItemType Directory -Force $ScreenDir | Out-Null
    Invoke-EmuScript 'screenshot.ps1' @('-Out', ('"{0}"' -f (Join-Path $ScreenDir "$Name.png"))) 60 | Out-Null
}

# ---------------------------------------------------------------------------------------------------------------------------------- the phone
function Flush-PhoneLog {
    try {
        $all = @(A logcat -d -v threadtime)
        $kinLines = @($all | Where-Object { $_ -cmatch '\sKinRelay:\s' })
        if ($kinLines.Count -gt 0) { Add-Content -Path $PhoneLog -Value $kinLines -Encoding UTF8 }
        A logcat -c | Out-Null
    } catch { }
}
function Phone-Cmd([string]$Command, [string]$Extras = '', [int]$Wait = 900) {
    Flush-PhoneLog
    A shell "am broadcast -a $Pkg.debug.$Command -n $Pkg/.debug.DebugCommandReceiver $Extras" | Out-Null
    Start-Sleep -Milliseconds $Wait
    @(A logcat -d -s KinRelay | Where-Object { $_ -match 'CMD' } | ForEach-Object { ($_ -replace '^.*CMD ', 'CMD ') })
}
function Phone-One([string]$Command, [string]$Extras = '', [int]$Wait = 900) { (Phone-Cmd $Command $Extras $Wait | Where-Object { $_ -match "CMD $Command" } | Select-Object -Last 1) }
function Val([string]$S, [string]$Name) { if ($S -match ("\b{0}=([^\s{{}}]+)" -f $Name)) { return $Matches[1] } return $null }
function Presence-State { Phone-One 'PRESENCE_STATE' '' 700 }
function Pause-State { Phone-One 'PAUSE_STATE' '' 700 }
function Ring-State { Phone-One 'RING_STATE' '' 700 }
function Shots-State { Phone-One 'SHOTS_STATE' '' 700 }
function Dnd-State { Phone-One 'DND_STATE' '' 700 }
function Alert-State { Phone-One 'ALERT_STATE' '' 700 }
function Notif-Dump {
    $d = (A shell dumpsys notification --noredact | Out-String)
    $i = $d.IndexOf('Notification List:')
    if ($i -lt 0) { return $d }
    $m = [regex]::Match($d.Substring($i), '(?m)^\s*(Archive|mArchive|Snoozed|Archived)')
    if ($m.Success) { return $d.Substring($i, $m.Index) } else { return $d.Substring($i) }
}
function Kin-Notifs { $d = Notif-Dump; (($d -split '(?m)(?=^\s*NotificationRecord)') | Where-Object { $_ -match [regex]::Escape("pkg=$Pkg") }) -join "`n" }
# the records of Kin's alert channel only (kin_alerts), as separate strings
function Alert-Notifs { @((Kin-Notifs) -split '(?m)(?=^\s*NotificationRecord)' | Where-Object { $_ -match 'channel=kin_alerts' }) }
function Ui-Dump { A shell uiautomator dump /sdcard/kin-ui.xml 2>$null | Out-Null; (A shell cat /sdcard/kin-ui.xml | Out-String) }
function Tap-Text([string[]]$Texts, [int]$Tries = 6) {
    for ($try = 0; $try -lt $Tries; $try++) {
        $x = Ui-Dump
        foreach ($t in $Texts) {
            if ($x -match ('text="{0}"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"' -f [regex]::Escape($t))) {
                A shell input tap ([int](([int]$Matches[1] + [int]$Matches[3]) / 2)) ([int](([int]$Matches[2] + [int]$Matches[4]) / 2)) | Out-Null
                return $true
            }
        }
        Start-Sleep -Milliseconds 700
    }
    return $false
}
function Go-Home {
    A shell input keyevent KEYCODE_HOME | Out-Null; Start-Sleep -Milliseconds 500
    A shell am start -n "$Pkg/.MainActivity" | Out-Null; Start-Sleep -Seconds 1
}
function Alarm-Volume {
    $d = (A shell dumpsys audio | Out-String)
    $m = [regex]::Match($d, '(?s)- STREAM_ALARM:.*?Current:\s*([^\r\n]*)')
    if (-not $m.Success) { return -1 }
    $c = [regex]::Match($m.Groups[1].Value, '\(speaker\):\s*(\d+)')
    if ($c.Success) { return [int]$c.Groups[1].Value }
    $c = [regex]::Match($m.Groups[1].Value, ':\s*(\d+)'); if ($c.Success) { return [int]$c.Groups[1].Value }
    return -1
}
function Wifi([bool]$On) { A shell svc wifi $(if ($On) { 'enable' } else { 'disable' }) | Out-Null }
function Power-Awake { ((A shell dumpsys power | Out-String) -match 'mWakefulness=Awake') }
# The text on the phone's clipboard, read by a focused debug activity (the only way an app may read it on Android 10+).
function Phone-Clip {
    A logcat -c | Out-Null
    A shell am start -n "$Pkg/.debug.DebugClipboardActivity" | Out-Null
    Wait-Until { $script:cl = (A logcat -d -s KinRelay) | Where-Object { $_ -match 'CMD CLIPBOARD' } | Select-Object -Last 1; [bool]$script:cl } 8 | Out-Null
    $line = $script:cl
    if ($line -and $line -notmatch 'CMD CLIPBOARD len=(?<len>\d+) sha=(?<sha>\w+)(?: sensitive=(?<sens>\w+))? text="(?s)(?<text>.*)"\s*$') {
        # an empty clipboard (or another wording): compare the line itself, so "unchanged" still means something
        A shell input keyevent KEYCODE_HOME | Out-Null
        return [pscustomobject]@{ Text = $null; Sha = ('line:' + ($line -replace '^.*CMD CLIPBOARD', '').Trim()) }
    }
    if (-not $line) { return [pscustomobject]@{ Text = $null; Sha = $null } }
    $sha = $Matches['sha']
    $t = [regex]::Replace($Matches['text'], '\\(.)', { param($m) switch ($m.Groups[1].Value) { 'n' { "`n" } 'r' { "`r" } 't' { "`t" } default { $m.Groups[1].Value } } })
    A shell input keyevent KEYCODE_HOME | Out-Null
    [pscustomobject]@{ Text = $t; Sha = $sha }
}

# ---- Do Not Disturb of the emulator (the USER's side of the tests) and what Android shows
# `cmd notification set_dnd <priority|alarms|none|off|all>` exists from API 30; on API 29 the debug DND_USER stands in for the user (it needs Do Not Disturb access).
function Set-UserDnd([string]$Mode) {
    if ($script:Sdk -ge 30) { A shell cmd notification set_dnd $Mode | Out-Null }
    else {
        $f = switch ($Mode) { 'priority' { 'priority' } 'alarms' { 'alarms' } 'none' { 'none' } default { 'all' } }
        Phone-Cmd 'DND_USER' "--es filter $f" 600 | Out-Null
    }
}
function Zen-Mode { (A shell settings get global zen_mode | Out-String).Trim() }
function Kin-Rule {
    # "STATE_TRUE ZEN_MODE_IMPORTANT_INTERRUPTIONS" for Kin's own rule on API 35+, 'none' when there is none (and always on API 29: a global filter)
    $d = (A shell dumpsys notification | Out-String)
    $m = [regex]::Match($d, 'ZenRule\[id=implicit_app\.kinrelay\.android[^\r\n]*')
    if (-not $m.Success) { return 'none' }
    $s = [regex]::Match($m.Value, 'state=(STATE_\w+)'); $z = [regex]::Match($m.Value, 'zenMode=(ZEN_MODE_\w+)'); $n = [regex]::Match($m.Value, 'name=([^,\]]+)')
    return ('{0} {1} "{2}"' -f $s.Groups[1].Value, $z.Groups[1].Value, $n.Groups[1].Value)
}
function Rule-Active { (Kin-Rule) -match 'STATE_TRUE' }
# Records exactly what Android shows (zen_mode, Kin's rule, the zen lines of dumpsys notification, Kin's DND_STATE), for the results file.
function Observe([string]$Label) {
    $z = Zen-Mode; $r = Kin-Rule
    $d = (A shell dumpsys notification | Out-String)
    $lines = @(([regex]::Matches($d, '(?m)^\s*(mZenMode=[^\r\n]*|mInterruptionFilter=[^\r\n]*|ZenRule\[id=implicit_app\.kinrelay[^\r\n]*|ZenRule\[id=MANUAL_RULE[^\r\n]*)') | ForEach-Object { $_.Groups[1].Value.Trim() }) | ForEach-Object { if ($_.Length -gt 260) { $_.Substring(0, 260) + '...' } else { $_ } })
    $s = Dnd-State
    $line = ("[API {0}] {1} | zen_mode={2} | kin rule: {3} | DND_STATE weHold={4} heldFilter={5} route={6} filter={7} path={8} | {9}" -f $script:Sdk, $Label, $z, $r, (Val $s 'weHold'), (Val $s 'heldFilter'), (Val $s 'route'), (Val $s 'filter'), (Val $s 'path'), ($lines -join ' || '))
    $script:Zen.Add($line); Say "ZEN   $line"
}
function Dnd-Wait([scriptblock]$Pred, [double]$Sec = 12) { Wait-Until { $l = Dnd-State; [bool]($l -and (& $Pred $l)) } $Sec 300 }
function Hold-Wait([bool]$Held, [double]$Sec = 12) { Wait-Until { (Val (Dnd-State) 'weHold') -eq $(if ($Held) { 'true' } else { 'false' }) } $Sec 300 }
function Grant-Dnd { A shell cmd notification allow_dnd $Pkg | Out-Null }
function Revoke-Access { A shell cmd notification disallow_dnd $Pkg 2>$null | Out-Null; A shell cmd notification disallow_listener $Service 2>$null | Out-Null }
# The fake Windows Focus of the test Kin (`--dev-set devFocus on|off`), then wait until the PC's OWN stable value (status.json focus.on) has followed: a replaced fake source is read at the
# next 15 s poll tick (it does not mark itself dirty like the real Windows event does), and then the 3 s filter runs. So this takes 3 to 18 s on the PC; the phone's part is timed by the caller
# from the moment this returns. The seconds it took are kept in $script:FocusLatency.
function Set-PcFocus([bool]$On) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    Invoke-Kin --dev-set devFocus $(if ($On) { 'on' } else { 'off' }) | Out-Null
    $ok = Wait-Until { (Get-Status).focus.on -eq $On } 40 200
    $script:FocusLatency = [math]::Round($sw.Elapsed.TotalSeconds, 1)
    return $ok
}
# Back to a quiet starting point: the PC out of Focus, nothing held, the user's DND off, the follow switch off (-KeepFollow leaves it as it was).
function Reset-Dnd([switch]$KeepFollow) {
    Set-PcFocus $false | Out-Null
    if (-not $KeepFollow) { Phone-Cmd 'DND_SET' '--ez follow false' 700 | Out-Null }
    if ((Val (Dnd-State) 'weHold') -eq 'true') { Phone-Cmd 'DND_RELEASE_NOW' '' 700 | Out-Null }
    Set-UserDnd 'off'
    if ($script:Sdk -ge 30) { A shell cmd notification set_dnd all | Out-Null }
    Start-Sleep -Seconds 1
}

# ---------------------------------------------------------------------------------------------------------------------------------- the PC
function Kin-Log { Read-Shared (Join-Path $Data 'logs\kin.log') }
function Log-Count([string]$Pattern) { ([regex]::Matches((Kin-Log), $Pattern)).Count }
function Get-Status { try { (Read-Shared (Join-Path $Data 'status.json')) | ConvertFrom-Json } catch { $null } }
function View([string]$Name) {
    Remove-Item (Join-Path $Data "view-$Name.json") -ErrorAction SilentlyContinue
    Invoke-Kin --dev-dump-view $Name | Out-Null
    Wait-Until { Test-Path (Join-Path $Data "view-$Name.json") } 6 | Out-Null
    try { (Read-Shared (Join-Path $Data "view-$Name.json")) | ConvertFrom-Json } catch { $null }
}
function Clip-Signature { ((Get-ClipboardFormats | ForEach-Object { "$($_.Name):$($_.Bytes)" }) -join ',') }
function Pc-Wait([scriptblock]$Pred, [double]$Sec = 20) { Wait-Until { $s = Get-Status; [bool]($s -and (& $Pred $s)) } $Sec 300 }
function Received-Files { @(Get-ChildItem $Received -File -Recurse -ErrorAction SilentlyContinue) }
function Start-TestKin([string[]]$Extra = @()) {
    $KinArgs = @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-port', "$Port",
        '--dev-received-dir', ('"{0}"' -f $Received), '--dev-print-pair-url', '--dev-no-windows', '--dev-toast-quiet', '--dev-no-open', '--dev-battery', '77:charging', '--dev-focus', 'off') + $Extra
    $script:KinProc = Start-DevKin $KinArgs
}
# Starts the test Kin again (after it was stopped or killed) and waits until a status.json NEWER than the start says the phone is connected (the old file of the dead process still says so).
# The phone's back-off (up to 120 s) is skipped with the debug CONNECT.
function Start-TestKinAndWait([string[]]$Extra = @(), [double]$Sec = 200) {
    $since = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    Start-TestKin $Extra
    Wait-Until { $s = Get-Status; $s -and $s.updatedAtMs -gt $since } 40 300 | Out-Null
    A shell am start -n "$Pkg/.MainActivity" | Out-Null
    Phone-Cmd 'CONNECT' '' 400 | Out-Null
    return (Wait-Until { $s = Get-Status; $s -and $s.updatedAtMs -gt $since -and $s.state -eq 'phone_connected' } $Sec 500)
}
function Pair-Phone {
    Remove-Item (Join-Path $Data 'pair-url.txt') -ErrorAction SilentlyContinue
    Invoke-Kin --dev-start-pairing | Out-Null
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    $paired = $false
    for ($try = 1; $try -le 3 -and -not $paired; $try++) {
        A shell am start -a android.intent.action.VIEW -p $Pkg -d "'$url'" --ez app.kinrelay.android.AUTO_CONFIRM true 2>$null | Out-Null
        $paired = Wait-Until { (Get-Status).state -eq 'phone_connected' } 25
    }
    return $paired
}
function Start-Fake([string[]]$ArgList, [string]$OutName) {
    $out = Join-Path $Work $OutName
    $args2 = ($ArgList | ForEach-Object { if ($_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ } }) -join ' '
    $p = Start-Process $Fake -ArgumentList $args2 -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" -WindowStyle Hidden
    $script:FakeProcs += $p
    return $p
}
function Fake-Text([string]$OutName) { Read-Shared (Join-Path $Work $OutName) }
# Kin.exe --notify against the TEST Kin's pipe. A GUI-subsystem program: it is waited for (Start-Process -Wait) so the exit code is real, stdout/stderr are redirected to files.
# Always `--data-dir <ours>` before --notify (that makes it a dev instance: the dev pipe + KIN_DEV_PIPE_SUFFIX), never without it: without it Kin.exe would speak to the INSTALLED Kin.
function Notify([string[]]$NotifyArgs, [string]$StdinText = $null, [string]$Exe = '') {
    if (-not $env:KIN_DEV_PIPE_SUFFIX) { throw 'Notify: KIN_DEV_PIPE_SUFFIX is not set: that would be the installed Kin''s pipe' }
    if (-not $Exe) { $Exe = $script:KinDevExe }
    $q = { param($s) if ($s -match '[\s"]' -or $s -eq '') { '"' + ($s -replace '"', '\"') + '"' } else { $s } }
    $line = ('--data-dir "{0}" --notify ' -f $Data) + (($NotifyArgs | ForEach-Object { & $q $_ }) -join ' ')
    if ($line -match 'app\.kinrelay\.desktop') { throw 'Notify: the installed Kin''s pipe name must never appear in a test command line' }
    $id = [guid]::NewGuid().ToString('N').Substring(0, 8)
    $out = Join-Path $Work "notify-$id.out"
    $sp = @{ FilePath = $Exe; ArgumentList = $line; Wait = $true; PassThru = $true; RedirectStandardOutput = $out; RedirectStandardError = "$out.err"; WindowStyle = 'Hidden' }
    if ($null -ne $StdinText) { $in = Join-Path $Work "notify-$id.in"; [IO.File]::WriteAllBytes($in, [Text.Encoding]::UTF8.GetBytes($StdinText)); $sp.RedirectStandardInput = $in }
    $p = Start-Process @sp
    $o = (Read-Shared $out).Trim(); $e = (Read-Shared "$out.err").Trim()
    [pscustomobject]@{ Exit = $p.ExitCode; Out = $o; Err = $e; Text = ($o + ' ' + $e).Trim() }
}

# ---- alerts on the phone: the notification record and the history rows
function NField([string]$Block, [string]$Name) { $m = [regex]::Match($Block, ('android\.{0}=\w+ \(([^\r\n]*)\)\r?\n' -f $Name)); if ($m.Success) { $m.Groups[1].Value } else { $null } }
function Unhex([string]$H) { if ($H) { [Text.Encoding]::UTF8.GetString(([byte[]]($H -split '(?<=\G..)(?!$)' | ForEach-Object { [Convert]::ToByte($_, 16) }))) } else { '' } }
# the rows of the phone's `texts` table (kind alert or all): id, direction, status, kind, title, body
function Phone-Rows([string]$Kind = '') {
    $out = (A shell "run-as $Pkg sqlite3 databases/kin.db 'select id,direction,status,kind,hex(title),hex(body) from texts order by createdAtMillis'" | Out-String)
    @($out -split "`r?`n" | Where-Object { $_ -match '\|' } | ForEach-Object { $f = $_ -split '\|'; if ($f.Count -ge 6) { [pscustomobject]@{ Id = $f[0]; Dir = $f[1]; Status = $f[2]; Kind = $f[3]; Title = (Unhex $f[4]); Body = (Unhex $f[5]) } } } | Where-Object { -not $Kind -or $_.Kind -eq $Kind })
}
function Alert-Rows { Phone-Rows 'alert' }
# Records what the phone's alert notification looks like (for the results file); nothing is asserted here
function Observe-Alert([string]$Label, [string]$Block) {
    $line = ("[API {0}] ALERT {1} | channel={2} title='{3}' text='{4}' subText='{5}' vis={6} publicTitle='{7}' actions={8}" -f $script:Sdk, $Label,
        ([regex]::Match($Block, 'channel=(\w+)').Groups[1].Value), (NField $Block 'title'), (NField $Block 'text'), (NField $Block 'subText'), ([regex]::Match($Block, 'vis=(\w+)').Groups[1].Value),
        ([regex]::Match($Block, 'publicNotification=[\s\S]*?android\.title=String \(([^\r\n]*)\)').Groups[1].Value), ($Block -match 'actions='))
    $script:Zen.Add($line); Say "ZEN   $line"
}
function Alert-Counts { $s = Alert-State; $r = if ($s -match 'received (\d+)') { [int]$Matches[1] } else { -1 }; $sh = if ($s -match 'shown (\d+)') { [int]$Matches[1] } else { -1 }; $q = if ($s -match 'quiet (\d+)') { [int]$Matches[1] } else { -1 }; [pscustomobject]@{ Received = $r; Shown = $sh; Quiet = $q } }
function Pc-History { try { @((View 'history').items) } catch { @() } }
function Pc-Sid { [Security.Principal.WindowsIdentity]::GetCurrent().User.Value }

# ================================================================================================================================== run
$installedBefore = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
try {
    Assert-NoGameRunning
    $Kin = (Resolve-Path $Kin -ErrorAction Stop).Path
    foreach ($f in $Apk, $Fake) { if (-not (Test-Path $f)) { throw "missing: $f" } }
    New-Item -ItemType Directory -Force $Data | Out-Null
    $KinCopy = Join-Path $Work 'Kin.exe'
    Copy-Item $Kin $KinCopy
    Set-KinDev -Exe $KinCopy -DataDir $Data
    Assert-NoDevKinOnDataDir
    $script:Backup = Save-Clipboard | Select-Object -Last 1
    $script:ClipAtStart = Clip-Signature
    Say 'the PC clipboard was saved in every format; it is put back at the end'
    '{ "hotkey": "", "autoClipboardSync": false, "mirrorNotifications": false, "focusShare": true }' | Set-Content (Join-Path $Data 'settings.json') -Encoding ASCII

    Say "starting $Avd (headless, -no-audio)"
    $started.Emulator = $true
    Invoke-EmuScript 'start.ps1' @('-Avd', $Avd) | Out-Null
    $isBooted = { ((A shell getprop sys.boot_completed 2>$null | Out-String).Trim() -eq '1') }
    if (-not (& $isBooted)) { & $Adb kill-server 2>$null | Out-Null; Start-Sleep -Seconds 2; & $Adb start-server 2>$null | Out-Null; Wait-Until $isBooted 90 | Out-Null }
    if (-not (& $isBooted)) { throw "emulator $Avd did not start" }
    A shell svc wifi enable | Out-Null
    $runningAvd = ((A emu avd name | Out-String) -split "`r?`n" | Select-Object -First 1).Trim()
    if ($runningAvd -ne $Avd) { throw "the emulator that is running is '$runningAvd', not '$Avd' (start.ps1 reuses a running one: run tools\emulator\stop.ps1 -All first)" }
    $script:Sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    $expectSdk = if ($Avd -match 'api(\d+)$') { [int]$Matches[1] } else { 0 }
    if ($expectSdk -and $script:Sdk -ne $expectSdk) { throw "the emulator reports API $($script:Sdk) but the AVD name says API ${expectSdk}, refusing to label this run" }
    Say ("the emulator: {0}, Android {1} (API {2})" -f $runningAvd, (A shell getprop ro.build.version.release | Out-String).Trim(), $script:Sdk)
    A shell dumpsys battery unplug | Out-Null; A shell dumpsys battery set level 100 | Out-Null
    Revoke-Access
    if ($script:Sdk -ge 30) { A shell cmd notification set_dnd all | Out-Null }
    A uninstall $Pkg 2>&1 | Out-Null
    $out = (& $Adb -s $Serial install -r (Resolve-Path $Apk).Path 2>&1 | Out-String)
    Check 'setup: install Kin (debug build)' ($out -match 'Success')
    Revoke-Access   # a stale approval can survive an uninstall (the API 37 image kept Kin in the allowed list of an earlier run)
    if ($script:Sdk -ge 37) { A shell pm grant $Pkg android.permission.ACCESS_LOCAL_NETWORK | Out-Null }
    if ($script:Sdk -ge 33) { A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null }
    A shell am start -n "$Pkg/.MainActivity" | Out-Null

    $started.Kin = $true
    Start-TestKin
    Check 'setup: the test Kin started' (Wait-Until { (Get-Status).state -eq 'not_paired' } 30)
    $paired = Pair-Phone
    Check 'setup: paired and connected to the real test Kin' $paired
    if (-not $paired) { throw 'not paired' }
    A logcat -G 16M | Out-Null
    for ($i = 0; $i -lt 4; $i++) { if (Tap-Text @('Done', 'Finish', 'Skip', 'Continue') 1) { break }; A shell input swipe 540 1900 540 700 300 | Out-Null; Start-Sleep -Milliseconds 600 }
    Start-Sleep -Seconds 2
    if ((Ui-Dump) -match 'Show notifications on your PC') { Tap-Text @('Not now') 3 | Out-Null; Start-Sleep -Seconds 1 }
    A shell input keyevent KEYCODE_HOME | Out-Null
    A shell dumpsys battery unplug | Out-Null
    $PcId = (Get-Status).devices[0].deviceId
    $PhoneName = (Get-Status).devices[0].name
    Say "phone name on the PC: $PhoneName"

    # ============================================================================================================================ scenarios
    # === SCENARIOS-START ===

    # ============================================================================================================================ S1 phone DND -> PC
    if (Enter-Scenario 'S1' 'phone Do Not Disturb -> the PC; the PC holds its own toast') {
        Go-Home
        Grant-Dnd   # API 29: the debug DND_USER needs it. (API 37: the shell's set_dnd needs none.) The follow switch is off here, so Kin changes nothing.
        $r = Phone-One 'DND_SET' '--ez share true --ez follow false' 900
        Check 'S1 the share switch is on (DND_STATE share=true, follow=false)' ((Val $r 'share') -eq 'true' -and (Val $r 'follow') -eq 'false') $r
        Check 'S1 the filter receiver is registered while the session is ready, and only then' ((Val (Dnd-State) 'receiver') -eq 'true' -and (Val (Dnd-State) 'sessionReady') -eq 'true') (Dnd-State)
        Check 'S1 baseline: the PC shows no phone dnd' (Pc-Wait { param($s) $null -eq $s.presence.phone.dnd } 8) ((Get-Status).presence.phone | ConvertTo-Json -Compress)
        Set-UserDnd 'priority'
        $t = Time-Until { (Get-Status).presence.phone.dnd -eq 'priority' } 8
        Check 'S1 phone DND priority reaches the PC (status.json presence.phone.dnd = priority) within 3 s' ($null -ne $t -and $t -le 3.5) "$t s"
        Check 'S1 the PC tooltip says "phone: Do Not Disturb" (the window chip is in the Settings/drop view, which a dump redacts: unit-tested)' ((Get-Status).tray.tooltip -match 'phone: Do Not Disturb') "$((Get-Status).tray.tooltip)"
        Observe 'S1 user DND = priority, share on (phone side)'
        Set-UserDnd 'alarms'
        $t = Time-Until { (Get-Status).presence.phone.dnd -eq 'alarms' } 8
        Check 'S1 alarms only -> dnd = alarms within 3 s' ($null -ne $t -and $t -le 3.5) "$t s"
        Set-UserDnd 'none'
        $t = Time-Until { (Get-Status).presence.phone.dnd -eq 'total' } 8
        Check 'S1 total silence (none) -> dnd = total within 3 s; the tooltip stays the short word' ($null -ne $t -and $t -le 3.5 -and (Get-Status).tray.tooltip -match 'Do Not Disturb') "$t s | $((Get-Status).tray.tooltip)"
        Observe 'S1 user DND = none (total silence)'
        Set-UserDnd 'off'
        $t = Time-Until { $null -eq (Get-Status).presence.phone.dnd } 8
        Check 'S1 DND off -> the PC shows no dnd within 3 s, no chip in the tooltip' ($null -ne $t -and $t -le 3.5 -and (Get-Status).tray.tooltip -notmatch 'Do Not Disturb') "$t s | $((Get-Status).tray.tooltip)"
        # the switch off: nothing is sent
        Phone-Cmd 'DND_SET' '--ez share false' 700 | Out-Null
        Set-UserDnd 'priority'; Start-Sleep -Seconds 4
        Check 'S1 share switch off: the PC is told nothing (no dnd 4 s after the user turned DND on)' ($null -eq (Get-Status).presence.phone.dnd) ((Get-Status).presence.phone | ConvertTo-Json -Compress)
        Phone-Cmd 'DND_SET' '--ez share true' 700 | Out-Null
        Check 'S1 share switched on while the phone is already in DND: the PC learns it (new snapshot)' (Pc-Wait { param($s) $s.presence.phone.dnd -eq 'priority' } 8)
        # a paused phone leaves dnd out of its snapshot
        Phone-Cmd 'PAUSE_SET' '--es mode 1h' | Out-Null
        Check 'S1 phone paused: the PC no longer sees dnd (a paused phone omits it)' (Pc-Wait { param($s) $s.presence.phone.paused -eq $true -and $null -eq $s.presence.phone.dnd } 8) ((Get-Status).presence.phone | ConvertTo-Json -Compress)
        Phone-Cmd 'PAUSE_SET' '--es mode resume' | Out-Null
        Check 'S1 phone resumed: dnd is back' (Pc-Wait { param($s) $s.presence.phone.paused -eq $false -and $s.presence.phone.dnd -eq 'priority' } 8) ((Get-Status).presence.phone | ConvertTo-Json -Compress)
        Set-UserDnd 'off'
        Pc-Wait { param($s) $null -eq $s.presence.phone.dnd } 8 | Out-Null

        # --- Kin's own optional toast is held while the phone is in DND (design 3.6): the low-battery nudge of 2B.4
        $lowRx = 'low battery warning for'; $heldRx = "toast of Kin's own was held"
        $w0 = Log-Count $lowRx; $h0 = Log-Count $heldRx
        Set-UserDnd 'priority'
        Pc-Wait { param($s) $s.presence.phone.dnd -eq 'priority' } 8 | Out-Null
        A shell dumpsys battery set level 14 | Out-Null
        Check 'S1 (hold) level 14, not charging, reaches the PC while the phone is in DND' (Pc-Wait { param($s) $s.presence.phone.level -eq 14 -and $s.presence.phone.charging -eq $false } 60) ((Get-Status).presence.phone | ConvertTo-Json -Compress)
        Start-Sleep -Seconds 3
        Check 'S1 (hold) phone in DND + the setting on: NO low-battery warning, one "held" line' ((Log-Count $lowRx) -eq $w0 -and (Log-Count $heldRx) -eq ($h0 + 1)) "warnings $w0 -> $(Log-Count $lowRx); held $h0 -> $(Log-Count $heldRx)"
        Check 'S1 (hold) status.json counts the held toast' ((Get-Status).focus.counts.heldToasts -ge 1) ((Get-Status).focus | ConvertTo-Json -Compress -Depth 4)
        Set-UserDnd 'off'
        Pc-Wait { param($s) $null -eq $s.presence.phone.dnd } 8 | Out-Null
        Start-Sleep -Seconds 4
        Check 'S1 (hold) DND is over: the held warning does not come later (the machine stays disarmed)' ((Log-Count $lowRx) -eq $w0) "warnings $(Log-Count $lowRx)"
        A shell dumpsys battery set ac 1 | Out-Null; A shell dumpsys battery set level 80 | Out-Null
        Check 'S1 (hold) plugged in at 80 %: the PC sees it (re-arms the nudge)' (Pc-Wait { param($s) $s.presence.phone.level -eq 80 -and $s.presence.phone.charging -eq $true } 60)
        A shell dumpsys battery unplug | Out-Null; A shell dumpsys battery set level 14 | Out-Null
        Check 'S1 (hold) unplugged at 14 % again, DND off' (Pc-Wait { param($s) $s.presence.phone.level -eq 14 -and $s.presence.phone.charging -eq $false } 60)
        Wait-Until { (Log-Count $lowRx) -gt $w0 } 5 | Out-Null
        Check 'S1 (hold) re-armed and DND off: the warning is shown now (one more)' ((Log-Count $lowRx) -eq ($w0 + 1) -and (Log-Count $heldRx) -eq ($h0 + 1)) "warnings $w0 -> $(Log-Count $lowRx); held $h0 -> $(Log-Count $heldRx)"
        # the setting off: shown even in DND
        A shell dumpsys battery set ac 1 | Out-Null; A shell dumpsys battery set level 80 | Out-Null
        Pc-Wait { param($s) $s.presence.phone.level -eq 80 -and $s.presence.phone.charging -eq $true } 60 | Out-Null
        Invoke-Kin --dev-set holdToastsOnPhoneDnd false | Out-Null
        Set-UserDnd 'priority'
        Pc-Wait { param($s) $s.presence.phone.dnd -eq 'priority' } 8 | Out-Null
        $w1 = Log-Count $lowRx
        A shell dumpsys battery unplug | Out-Null; A shell dumpsys battery set level 14 | Out-Null
        Pc-Wait { param($s) $s.presence.phone.level -eq 14 -and $s.presence.phone.charging -eq $false } 60 | Out-Null
        Wait-Until { (Log-Count $lowRx) -gt $w1 } 5 | Out-Null
        Check 'S1 (hold) the setting off + phone in DND: the warning IS shown (not held)' ((Log-Count $lowRx) -eq ($w1 + 1) -and (Log-Count $heldRx) -eq ($h0 + 1)) "warnings $w1 -> $(Log-Count $lowRx)"
        Invoke-Kin --dev-set holdToastsOnPhoneDnd true | Out-Null
        Set-UserDnd 'off'
        A shell dumpsys battery set level 100 | Out-Null
        Pc-Wait { param($s) $s.presence.phone.level -eq 100 -and $null -eq $s.presence.phone.dnd } 60 | Out-Null
        Phone-Cmd 'DND_SET' '--ez share false' 500 | Out-Null
    }

    # ============================================================================================================================ S2 PC Focus -> phone DND
    if (Enter-Scenario 'S2' 'PC Focus -> the phone''s Do Not Disturb (both access routes, Priority and Alarms, release)') {
        Go-Home
        Reset-Dnd
        Revoke-Access
        # Round 2 (review item 4): NO DND_REFRESH here any more. The access is revoked while both switches are off (no receiver listens), and the follower must read the access again at evaluation time
        # (before: it believed it still had access, spent the rising edge on a request that could not be made, and granting later did not start the hold).
        Phone-Cmd 'DND_SET' '--ez share true --ez follow false --es filter priority' 700 | Out-Null
        $s0 = Dnd-State
        Check 'S2 start: no access (path=NONE, access=false), nothing held, the filter is ALL (1)' ((Val $s0 'access') -eq 'false' -and (Val $s0 'path') -eq 'NONE' -and (Val $s0 'weHold') -eq 'false' -and (Val $s0 'filter') -eq '1') $s0
        Check 'S2 the PC says Focus is off (fake source off)' ((Get-Status).focus.on -eq $false -and (Get-Status).focus.dev -eq 'off') ((Get-Status).focus | ConvertTo-Json -Compress -Depth 4)
        Observe 'S2 start (no access, no focus)'

        # ---- no access: the switch is on, the PC is in Focus: Kin can do nothing and says nothing is held
        Phone-Cmd 'DND_SET' '--ez follow true' 700 | Out-Null
        Set-PcFocus $true | Out-Null
        Start-Sleep -Seconds 4
        $s = Dnd-State
        Check 'S2 follow on, NO access, PC in Focus: nothing is held and the filter is untouched (zen_mode 0)' ((Val $s 'weHold') -eq 'false' -and (Val $s 'pcFocus') -eq 'true' -and (Zen-Mode) -eq '0') "$s | zen=$(Zen-Mode)"
        Observe 'S2 PC in Focus, follow on, no access'
        # ---- granting the access while the PC is already in Focus is a fresh edge (the access broadcast wakes the follower): Do Not Disturb access = route 1
        $tg = Get-Date
        Grant-Dnd
        $held = Hold-Wait $true 15
        Check 'S2 route 1 (Do Not Disturb access, allow_dnd): granting access while the PC is in Focus starts the hold (weHold=true)' $held (Dnd-State)
        $s = Dnd-State
        Check 'S2 route 1: path=POLICY, heldFilter=2 (priority), route=rule on API 35+ / global below' ((Val $s 'path') -eq 'POLICY' -and (Val $s 'heldFilter') -eq '2' -and (Val $s 'route') -eq $(if ($script:Sdk -ge 35) { 'rule' } else { 'global' })) $s
        Check 'S2 route 1: zen_mode is 1 (priority)' ((Zen-Mode) -eq '1') "zen=$(Zen-Mode)"
        if ($script:Sdk -ge 35) { Check 'S2 route 1: Android shows Kin''s own rule "Do Not Disturb (Kin)" active (STATE_TRUE), not a change of the global filter' ((Rule-Active) -and (Kin-Rule) -match 'Do Not Disturb \(Kin\)') (Kin-Rule) }
        else { Check 'S2 route 1 (API < 35): no rule exists, the global filter changed' ((Kin-Rule) -eq 'none') (Kin-Rule) }
        Observe 'S2 route 1 (allow_dnd): PC in Focus, Priority'
        Check 'S2 the PC chip tells the truth while Kin holds the phone silent (phone dnd = priority in status.json)' (Pc-Wait { param($s) $s.presence.phone.dnd -eq 'priority' } 8) ((Get-Status).presence.phone | ConvertTo-Json -Compress)
        # release
        $tr = Get-Date
        Set-PcFocus $false | Out-Null
        $t = Time-Until { (Zen-Mode) -eq '0' } 15
        Check 'S2 PC leaves Focus: the phone releases within 6 s of the PC''s stable value changing, and zen_mode is 0 again' ($null -ne $t -and $t -le 6 -and (Hold-Wait $false 3)) "$t s after the PC's stable value changed (PC side took $($script:FocusLatency) s) | $(Dnd-State)"
        if ($script:Sdk -ge 35) { Check 'S2 released: Kin''s rule is STATE_FALSE (not removed, deactivated)' (-not (Rule-Active)) (Kin-Rule) }
        Observe 'S2 route 1: PC left Focus (released)'
        Check 'S2 the PC chip clears again' (Pc-Wait { param($s) $null -eq $s.presence.phone.dnd } 8)

        # ---- Alarms only
        Phone-Cmd 'DND_SET' '--es filter alarms' 700 | Out-Null
        Set-PcFocus $true | Out-Null
        $t = Time-Until { (Zen-Mode) -eq '3' } 15
        Check 'S2 "How much" = Alarms only: zen_mode 3 (alarms) within 8 s of the PC''s stable Focus' ($null -ne $t -and $t -le 8) "$t s | zen=$(Zen-Mode) | $(Dnd-State)"
        Check 'S2 Alarms only: heldFilter=4' ((Val (Dnd-State) 'heldFilter') -eq '4') (Dnd-State)
        Observe 'S2 route 1: Alarms only'
        Set-PcFocus $false | Out-Null
        Check 'S2 Alarms only released' ((Time-Until { (Zen-Mode) -eq '0' } 15) -ne $null) "zen=$(Zen-Mode)"
        Phone-Cmd 'DND_SET' '--es filter priority' 700 | Out-Null

        # ---- route 2: notification access only (the listener); on the emulator this also reports policy access, so the order of the design picks route 1 (record it)
        A shell cmd notification disallow_dnd $Pkg | Out-Null
        Check 'S2 access revoked again: path=NONE' (Wait-Until { (Val (Dnd-State) 'path') -eq 'NONE' } 8 500) (Dnd-State)
        A shell cmd notification allow_listener $Service | Out-Null
        Phone-Cmd 'DND_REFRESH' '' 1500 | Out-Null
        $s = Dnd-State
        Note ("API {0}: with notification access only (allow_listener): access={1} path={2}" -f $script:Sdk, (Val $s 'access'), (Val $s 'path'))
        Check 'S2 route 2 (notification access only, allow_listener): Kin sees access (the emulator reports policy access as granted with it)' ((Val $s 'access') -eq 'true') $s
        Set-PcFocus $true | Out-Null
        $t = Time-Until { (Zen-Mode) -eq '1' } 15
        Check 'S2 route 2: the PC in Focus -> zen_mode 1 within ~8 s' ($null -ne $t) "$t s | zen=$(Zen-Mode) | $(Dnd-State)"
        Observe 'S2 route 2 (allow_listener only): PC in Focus, Priority'
        Set-PcFocus $false | Out-Null
        Check 'S2 route 2: released' ((Time-Until { (Zen-Mode) -eq '0' } 15) -ne $null -and (Hold-Wait $false 4)) "zen=$(Zen-Mode) | $(Dnd-State)"
        A shell cmd notification disallow_listener $Service | Out-Null

        # ---- Settings screen of the phone (the explanation, the hold row)
        Grant-Dnd
        Phone-Cmd 'DND_REFRESH' '' 1000 | Out-Null
        Set-PcFocus $true | Out-Null
        Hold-Wait $true 15 | Out-Null
        Go-Home
        Shot '01-s2-phone-home-while-silenced-by-kin'
        $ui = Ui-Dump
        Check 'S2 the phone says it is holding ("Silenced by Kin" on Home or in Settings)' ($ui -match 'Silenced by Kin' -or (Kin-Notifs) -match 'Silenced by Kin') (([regex]::Matches($ui, 'text="([^"]*Silenced[^"]*)"') | ForEach-Object { $_.Groups[1].Value }) -join ' | ')
        Set-PcFocus $false | Out-Null
        Hold-Wait $false 15 | Out-Null
        Reset-Dnd
        Phone-Cmd 'DND_SET' '--ez share false' 500 | Out-Null
    }

    # ============================================================================================================================ S3 safety
    if (Enter-Scenario 'S3' 'safety: already in DND, user override, PC killed, phone app killed, pause, switch off, access lost, unlink') {
        Go-Home
        Reset-Dnd
        Revoke-Access; Grant-Dnd
        Phone-Cmd 'DND_SET' '--ez follow true --ez share true --es filter priority' 700 | Out-Null
        Phone-Cmd 'DND_REFRESH' '' 800 | Out-Null
        $s = Dnd-State
        Check 'S3 setup: follow on, access granted (path=POLICY), PC not in Focus, nothing held' ((Val $s 'follow') -eq 'true' -and (Val $s 'path') -eq 'POLICY' -and (Val $s 'weHold') -eq 'false' -and (Zen-Mode) -eq '0') "$s | zen=$(Zen-Mode)"

        # ---- (1) the phone is already in DND: Kin changes nothing and holds nothing (design 4.2 step 2)
        Set-UserDnd 'alarms'; Start-Sleep -Seconds 1
        Set-PcFocus $true | Out-Null
        Start-Sleep -Seconds 4
        $s = Dnd-State
        Check 'S3 (1) the phone already in alarms-only: Kin holds nothing, the filter stays alarms (zen_mode 3)' ((Val $s 'weHold') -eq 'false' -and (Val $s 'pcFocus') -eq 'true' -and (Zen-Mode) -eq '3') "$s | zen=$(Zen-Mode)"
        if ($script:Sdk -ge 35) { Check 'S3 (1) Kin made no rule of its own (none, or STATE_FALSE)' (-not (Rule-Active)) (Kin-Rule) }
        Observe 'S3 (1) user already in alarms, PC enters Focus'
        Set-PcFocus $false | Out-Null
        Start-Sleep -Seconds 5
        Check 'S3 (1) the PC leaves Focus: the user''s alarms-only is still on (Kin never lifts what it did not set)' ((Zen-Mode) -eq '3') "zen=$(Zen-Mode)"
        Set-UserDnd 'off'; Start-Sleep -Seconds 2
        Check 'S3 (1) the user turns DND off: zen_mode 0' ((Zen-Mode) -eq '0') "zen=$(Zen-Mode)"

        # ---- (2) user override: the user changes DND while Kin holds -> Kin lets go, deactivates ITS OWN rule (API 35+), does not re-apply until the next rising edge
        Set-PcFocus $true | Out-Null
        Check 'S3 (2) setup: Kin holds (priority)' (Hold-Wait $true 15) (Dnd-State)
        Set-UserDnd 'alarms'
        $t = Time-Until { (Val (Dnd-State) 'weHold') -eq 'false' } 8
        Check 'S3 (2) the user sets alarms-only while Kin holds: override within 8 s (weHold=false)' ($null -ne $t) "$t s | $(Dnd-State)"
        if ($script:Sdk -ge 35) { Check 'S3 (2) ANDROID 15+ (cross-team point a): Kin''s own rule is deactivated too, so it does not stay active behind the user''s state' (-not (Rule-Active)) (Kin-Rule) }
        Check 'S3 (2) the user''s alarms-only stays (zen_mode 3): Kin did not fight it' ((Zen-Mode) -eq '3') "zen=$(Zen-Mode)"
        Observe 'S3 (2) override: user alarms-only on top of Kin priority'
        Set-UserDnd 'off'; Start-Sleep -Seconds 5
        Check 'S3 (2) the user then turns DND OFF with the PC still in Focus: zen_mode 0, NOT re-applied (no re-apply until the next rising edge)' ((Zen-Mode) -eq '0' -and (Val (Dnd-State) 'weHold') -eq 'false' -and (Val (Dnd-State) 'pcFocus') -eq 'true') "zen=$(Zen-Mode) | $(Dnd-State)"
        if ($script:Sdk -ge 35) { Check 'S3 (2) and the user''s off also left no active rule of Kin''s behind (the user''s off is respected)' (-not (Rule-Active)) (Kin-Rule) }
        Set-PcFocus $false | Out-Null; Start-Sleep -Seconds 5
        Set-PcFocus $true | Out-Null
        Check 'S3 (2) the NEXT rising edge of Focus holds again' (Hold-Wait $true 15) (Dnd-State)
        if ($script:Sdk -ge 35) {
            # the user's "off" while Kin's rule is on: the shell's set_dnd off only changes the manual rule (spike S5 f) -- recorded, not asserted
            Set-UserDnd 'off'; Start-Sleep -Seconds 3
            Observe 'S3 (2b) set_dnd off while Kin''s own rule is active (the shell''s off touches only the manual rule)'
            Note 'API 35+: `cmd notification set_dnd off` while Kin''s rule is active leaves Kin''s rule (and zen_mode 1) as it is: the shell''s "off" does not end an app-owned rule. What the real Quick Settings tile does is for a real phone.'
        }
        Set-PcFocus $false | Out-Null
        Check 'S3 (2) setup for the next case: released' (Hold-Wait $false 15) (Dnd-State)
        Reset-Dnd -KeepFollow

        # ---- (3) pause on the phone releases; resume is a fresh edge
        Set-PcFocus $true | Out-Null
        Check 'S3 (3) setup: Kin holds' (Hold-Wait $true 15) (Dnd-State)
        Phone-Cmd 'PAUSE_SET' '--es mode 1h' 700 | Out-Null
        Check 'S3 (3) the phone is paused: the hold is released (weHold=false, zen_mode 0)' ((Hold-Wait $false 8) -and (Zen-Mode) -eq '0') "$(Dnd-State) | zen=$(Zen-Mode)"
        Start-Sleep -Seconds 4
        Check 'S3 (3) a paused phone does not hold even though the PC is in Focus' ((Val (Dnd-State) 'weHold') -eq 'false' -and (Zen-Mode) -eq '0') (Dnd-State)
        Phone-Cmd 'PAUSE_SET' '--es mode resume' 700 | Out-Null
        Check 'S3 (3) resumed while the PC is still in Focus: a fresh edge holds again' (Hold-Wait $true 15) (Dnd-State)
        # the PC pauses: a paused PC puts no focus in its snapshot
        Invoke-Kin --pause 1h | Out-Null
        Check 'S3 (3) the PC is paused: it sends no focus, the phone releases' ((Hold-Wait $false 12) -and (Zen-Mode) -eq '0') "$(Dnd-State) | zen=$(Zen-Mode)"
        Invoke-Kin --pause resume | Out-Null
        Check 'S3 (3) the PC resumes while in Focus: it sends focus again and the phone holds again' (Hold-Wait $true 15) (Dnd-State)

        # ---- (4) the switch turned off while held releases
        Phone-Cmd 'DND_SET' '--ez follow false' 700 | Out-Null
        Check 'S3 (4) the "Silence this phone" switch turned off while held: released at once' ((Hold-Wait $false 6) -and (Zen-Mode) -eq '0') "$(Dnd-State) | zen=$(Zen-Mode)"
        Phone-Cmd 'DND_SET' '--ez follow true' 700 | Out-Null
        Check 'S3 (4) turned on again while the PC is in Focus: held again' (Hold-Wait $true 15) (Dnd-State)

        # ---- (5) access revoked while held (recorded; the design says the hold stays recorded and is released at the next start)
        Revoke-Access
        Start-Sleep -Seconds 4
        $s = Dnd-State
        Observe 'S3 (5) access revoked while Kin holds'
        Note ("API {0}: after `disallow_dnd` while holding: weHold={1} access={2} zen_mode={3} kin rule={4}" -f $script:Sdk, (Val $s 'weHold'), (Val $s 'access'), (Zen-Mode), (Kin-Rule))
        # the PC leaves Focus WHILE the access is gone: the release cannot be made (no access); the record stays (design edge case)
        Set-PcFocus $false | Out-Null
        Wait-Until { (Val (Dnd-State) 'pcFocus') -eq 'false' } 10 700 | Out-Null
        Start-Sleep -Seconds 2
        $s = Dnd-State
        Observe 'S3 (5) the PC left Focus while the access is gone'
        Check 'S3 (5) with no access the release cannot be made: the record stays (weHold=true) and the silence stays (the design: released at the next start; "Access lost" row)' ((Val $s 'weHold') -eq 'true' -and (Zen-Mode) -eq '1') "$s | zen=$(Zen-Mode)"
        Grant-Dnd
        $relAfterGrant = Hold-Wait $false 15
        Observe 'S3 (5) access given back, PC out of Focus'
        Check 'S3 (5) round 2 (R44): access given back: the leftover hold is released by itself (weHold=false, zen_mode 0), no "Release now" needed' ($relAfterGrant -and (Zen-Mode) -eq '0') "$(Dnd-State) | zen=$(Zen-Mode)"
        if (-not $relAfterGrant) {
            Note 'a hold whose release failed for lack of access was NOT retried when the access came back'
            Phone-Cmd 'DND_RELEASE_NOW' '' 1200 | Out-Null
            Check 'S3 (5) "Release now" ends it (weHold=false, zen_mode 0)' ((Val (Dnd-State) 'weHold') -eq 'false' -and (Zen-Mode) -eq '0') "$(Dnd-State) | zen=$(Zen-Mode)"
        }
        Reset-Dnd -KeepFollow
        Phone-Cmd 'DND_SET' '--ez follow true' 500 | Out-Null

        # ---- (6) the phone app is force-stopped while holding: the filter stays (nothing else ends it), the restart releases it
        Set-PcFocus $true | Out-Null
        Check 'S3 (6) setup: Kin holds' (Hold-Wait $true 15) (Dnd-State)
        A shell am force-stop $Pkg | Out-Null
        Start-Sleep -Seconds 3
        Check 'S3 (6) after force-stop the phone is STILL silenced (stuck: zen_mode 1; the record is the only thing that can end it)' ((Zen-Mode) -eq '1') "zen=$(Zen-Mode) | rule: $(Kin-Rule)"
        Observe 'S3 (6) after am force-stop while holding (stuck)'
        Set-PcFocus $false | Out-Null
        Start-Sleep -Seconds 4
        A shell am start -n "$Pkg/.MainActivity" | Out-Null
        $t = Time-Until { (Zen-Mode) -eq '0' } 40
        Check 'S3 (6) the app is started again (the PC is out of Focus): the persisted hold is reconciled at start, zen_mode 0' ($null -ne $t) "$t s | zen=$(Zen-Mode)"
        Check 'S3 (6) the phone reconnects to the PC' (Pc-Wait { param($s) $s.state -eq 'phone_connected' } 60)
        Check 'S3 (6) and the record is clear' ((Val (Dnd-State) 'weHold') -eq 'false') (Dnd-State)
        Observe 'S3 (6) after the restart'

        # ---- (7) the PC dev Kin is killed while the phone holds: released ~60 s after the close
        Phone-Cmd 'DND_SET' '--ez follow true' 500 | Out-Null
        Set-PcFocus $true | Out-Null
        Check 'S3 (7) setup: Kin holds' (Hold-Wait $true 15) (Dnd-State)
        $kp = @(Get-DevKinProcesses)
        foreach ($p in $kp) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }   # OUR test Kin only (program file and data directory verified)
        $tk = Get-Date
        Check 'S3 (7) the PC process is gone (the test Kin)' (Wait-Until { @(Get-DevKinProcesses).Count -eq 0 } 10)
        Check 'S3 (7) the phone notices the close and starts the 60 s grace' (Wait-Until { $g = Val (Dnd-State) 'graceMs'; $g -match '^\d+$' -and [int]$g -gt 0 } 20 700) (Dnd-State)
        Start-Sleep -Seconds 25
        Check 'S3 (7) 25 s after the close: still held (a Wi-Fi blip must not release)' ((Val (Dnd-State) 'weHold') -eq 'true' -and (Zen-Mode) -eq '1') "$(Dnd-State) | zen=$(Zen-Mode)"
        $t = Time-Until { (Zen-Mode) -eq '0' } 60
        $el = ((Get-Date) - $tk).TotalSeconds
        Check 'S3 (7) released about 60 s after the close (55 to 75 s after the kill)' ($null -ne $t -and $el -ge 50 -and $el -le 80) ("{0:N1} s after the kill | zen=$(Zen-Mode) | $(Dnd-State)" -f $el)
        Check 'S3 (7) weHold=false afterwards' ((Val (Dnd-State) 'weHold') -eq 'false') (Dnd-State)
        Observe 'S3 (7) 60 s after the PC vanished'
        Check 'S3 (7) the PC is started again and the phone reconnects (a status.json newer than the restart says so)' (Start-TestKinAndWait)

        # ---- (8) unlink from the PC releases
        Set-PcFocus $true | Out-Null
        Check 'S3 (8) setup: Kin holds again after the reconnect' (Hold-Wait $true 20) (Dnd-State)
        $PcId = (Get-Status).devices[0].deviceId
        Invoke-Kin --dev-set unlinkDevice $PcId | Out-Null
        Check 'S3 (8) the PC unlinks the phone while Kin holds: the hold is released at once (weHold=false, zen_mode 0)' ((Hold-Wait $false 10) -and (Time-Until { (Zen-Mode) -eq '0' } 10) -ne $null) "$(Dnd-State) | zen=$(Zen-Mode)"
        Set-PcFocus $false | Out-Null
        Check 'S3 (8) setup: pair again' (Pair-Phone)
        $PcId = (Get-Status).devices[0].deviceId
        Reset-Dnd
        Phone-Cmd 'DND_SET' '--ez follow false --ez share false' 500 | Out-Null
        Revoke-Access
    }

    # ============================================================================================================================ S4 alerts
    if (Enter-Scenario 'S4' 'alerts: Kin.exe --notify to the phone') {
        Go-Home
        Reset-Dnd
        Phone-Cmd 'DND_SET' '--ez follow false --ez share false' 500 | Out-Null
        Phone-Cmd 'PAUSE_SET' '--es mode resume' 500 | Out-Null
        $pcClip0 = Clip-Signature
        $phClip0 = (Phone-Clip).Sha
        Go-Home
        $rows0 = @(Alert-Rows).Count
        # ---- 1. the plain form
        $r = Notify @('ZQAlertText one')
        Check 'S4 Kin.exe --notify "text": exit code 0 and the line "Sent to <phone>."' ($r.Exit -eq 0 -and $r.Out -match '^Sent to .+\.$') "exit $($r.Exit): $($r.Text)"
        Check 'S4 the phone shows a notification with the text' (Wait-Until { (@(Alert-Notifs) -join "`n") -match 'ZQAlertText one' } 10 500) ''
        $blk = @(Alert-Notifs)[0]
        Check 'S4 channel kin_alerts, title = the PC name, text = the alert, category msg' ($blk -match 'channel=kin_alerts' -and (NField $blk 'title') -eq 'DESK-PC' -and (NField $blk 'text') -eq 'ZQAlertText one' -and $blk -match 'category=msg') "title=$(NField $blk 'title') text=$(NField $blk 'text')"
        Check 'S4 private on the lock screen with a public version "Alert from DESK-PC"' ($blk -match 'vis=PRIVATE' -and ($blk -match 'android\.title=String \(Alert from DESK-PC\)' -or ($script:Sdk -lt 30 -and $blk -match 'publicVersion=Notification\('))) $(if ($script:Sdk -lt 30) { 'API 29: dumpsys prints no extras of the public version; its presence (publicVersion=) is what is seen' } else { '' })
        Check 'S4 it has a tap target and NO action buttons, and does not use any other app''s name' ($blk -match 'contentIntent=PendingIntent' -and $blk -notmatch 'actions=' -and $blk -notmatch 'Windows|bank') ''
        $rows = @(Alert-Rows)
        Check 'S4 a history row of kind "alert" (down, done) with the text on the phone' ($rows.Count -eq $rows0 + 1 -and $rows[-1].Body -eq 'ZQAlertText one' -and $rows[-1].Dir -eq 'down' -and $rows[-1].Status -eq 'done') "rows $rows0 -> $($rows.Count)"
        $item = @(Pc-History | Where-Object { $_.text -eq 'ZQAlertText one' })[0]
        Check 'S4 and one on the PC: an alert row (isAlert) with the text, done' ($item -and $item.isAlert -eq $true -and $item.status -eq 'done' -and $item.direction -eq 'out') ($item | ConvertTo-Json -Compress)
        Check 'S4 the PC log records it by counts only (no text)' ((Log-Count 'extension: alert \(cli, ttl (24h|1d)\): acked') -ge 1 -and (Kin-Log) -notmatch 'ZQAlertText') ''
        $pcLogClip0 = Log-Count '(on the clipboard|copied to the clipboard|send clipboard|text .* copied|clipboard changed)'
        Check 'S4 NO clipboard change: the PC clipboard is as it was (every format and size) and so is the phone''s' (((Clip-Signature) -eq $pcClip0) -and ((Phone-Clip).Sha -eq $phClip0)) "phone clipboard $phClip0"
        Go-Home
        # ---- 2. title, ttl, "--"
        $r = Notify @('--title', 'ZQAlertTitle', '--ttl', '2h', '--', 'ZQAlertText two')
        Check 'S4 --title --ttl 2h -- text: exit 0, "Sent to"' ($r.Exit -eq 0 -and $r.Out -match '^Sent to ') "exit $($r.Exit): $($r.Text)"
        Check 'S4 the phone has the row with the title' (Wait-Until { @(Alert-Rows | Where-Object { $_.Title -eq 'ZQAlertTitle' -and $_.Body -eq 'ZQAlertText two' }).Count -eq 1 } 10 500) ''
        # ---- 3. several alerts share ONE updating notification
        Notify @('ZQAlertText three') | Out-Null; Notify @('ZQAlertText four') | Out-Null
        Start-Sleep -Seconds 3
        $an = @(Alert-Notifs)
        Check 'S4 several alerts share ONE updating notification (not one each)' ($an.Count -eq 1) "kin_alerts records: $($an.Count); title='$(NField $an[0] 'title')'"
        Observe-Alert 'after 4 alerts: the one notification' $an[0]
        Shot '02-s4-alerts-notification-shade'
        # ---- 4. stdin
        $r = Notify @('-') 'ZQStdinText hello from stdin'
        Check 'S4 the text from stdin (--notify -): exit 0, the phone gets it trimmed' ($r.Exit -eq 0 -and (Wait-Until { @(Alert-Rows | Where-Object { $_.Body -eq 'ZQStdinText hello from stdin' }).Count -eq 1 } 10 500)) "exit $($r.Exit): $($r.Text)"
        # ---- 5. limits and usage errors: nothing is sent, nothing is cut silently
        $before = @(Alert-Rows).Count
        $long = 'ZQAlertText' + ('x' * 290)
        $r = Notify @($long)
        Check 'S4 301 characters: exit code 5 and the count is named (nothing is cut silently)' ($r.Exit -eq 5 -and $r.Err -match '301') "exit $($r.Exit): $($r.Text)"
        $r300 = Notify @('ZQAlertText' + ('y' * 289))
        Check 'S4 exactly 300 characters is accepted (exit 0)' ($r300.Exit -eq 0) "exit $($r300.Exit): $($r300.Text)"
        $r = Notify @()
        Check 'S4 no text: exit code 2 (usage)' ($r.Exit -eq 2) "exit $($r.Exit): $($r.Text)"
        $r = Notify @('--bogus', 'x')
        Check 'S4 an unknown option: exit code 2' ($r.Exit -eq 2) "exit $($r.Exit): $($r.Text)"
        $r = Notify @('--ttl', '30s', 'x')
        Check 'S4 --ttl 30s (under a minute): exit code 2' ($r.Exit -eq 2) "exit $($r.Exit): $($r.Text)"
        $r = Notify @('--title', ('t' * 61), 'x')
        Check 'S4 a title of 61 characters: exit code 5' ($r.Exit -eq 5) "exit $($r.Exit): $($r.Text)"
        Start-Sleep -Seconds 2
        Check 'S4 none of those reached the phone (only the 300-character one did)' (@(Alert-Rows).Count -eq $before + 1) "rows $before -> $(@(Alert-Rows).Count)"
        # ---- 6. --device
        $r = Notify @('--device', 'zq-nobody', 'x')
        Check 'S4 --device with a name that is not paired: exit code 4' ($r.Exit -eq 4) "exit $($r.Exit): $($r.Text)"
        $r = Notify @('--device', $PhoneName, 'ZQAlertText device')
        Check 'S4 --device <the phone''s name>: exit 0' ($r.Exit -eq 0 -and $r.Out -match 'Sent to') "exit $($r.Exit): $($r.Text)"

        # ---- 7. the phone's limit: 10 per 60 s (capacity 10, one back per 6 s); the rest FAIL on the PC with rate_limited -> exit 5
        Say 'waiting 65 s so the phone''s alert bucket is full again'
        Start-Sleep -Seconds 65
        $ok = 0; $limited = 0; $other = @()
        for ($i = 1; $i -le 14; $i++) {
            $r = Notify @("ZQBurstText $i")
            if ($r.Exit -eq 0) { $ok++ } elseif ($r.Exit -eq 5) { $limited++ } else { $other += "$($r.Exit):$($r.Text)" }
        }
        Check 'S4 14 alerts in a burst: 10 to 12 are accepted (the bucket of 10 plus what refilled meanwhile), the rest exit 5 (rate limited)' ($ok -ge 10 -and $ok -le 12 -and $limited -ge 2 -and $other.Count -eq 0) "ok $ok, rate limited $limited, other: $($other -join ' | ')"
        Start-Sleep -Seconds 3
        $failedOnPc = @(Pc-History | Where-Object { $_.text -match '^ZQBurstText' -and $_.status -eq 'failed' })
        Check 'S4 the limited ones show in the PC history as failed' ($failedOnPc.Count -ge 2 -and $failedOnPc.Count -eq $limited) "failed on the PC: $($failedOnPc.Count)"
        $burstRows = @(Alert-Rows | Where-Object { $_.Body -match '^ZQBurstText' })
        Check 'S4 and only the accepted ones reached the phone (as rows)' ($burstRows.Count -eq $ok) "rows $($burstRows.Count), accepted $ok"
        Check 'S4 still ONE alert notification after the burst (never more than one of Kin''s alerts)' (@(Alert-Notifs).Count -eq 1) "records $(@(Alert-Notifs).Count)"
        Say 'waiting 65 s for the bucket again'
        Start-Sleep -Seconds 65

        # ---- 8. a paused phone: the history row, no notification
        Phone-Cmd 'PAUSE_SET' '--es mode 1h' 700 | Out-Null
        $c0 = Alert-Counts
        $wBefore = ((@(Alert-Notifs) | ForEach-Object { (([regex]::Matches($_, '(when|posttimeElapsedMs)=\d+') | ForEach-Object { $_.Value }) -join '/') }) -join ',')
        $r = Notify @('ZQPausedText one')
        Check 'S4 phone paused: --notify still exits 0 (the PC''s pause is what stops scripts, not the phone''s)' ($r.Exit -eq 0) "exit $($r.Exit): $($r.Text)"
        Start-Sleep -Seconds 3
        $c1 = Alert-Counts
        $wAfter = ((@(Alert-Notifs) | ForEach-Object { (([regex]::Matches($_, '(when|posttimeElapsedMs)=\d+') | ForEach-Object { $_.Value }) -join '/') }) -join ',')
        Check 'S4 phone paused: history row only (quiet +1, shown unchanged), the notification was not touched' ($c1.Quiet -eq $c0.Quiet + 1 -and $c1.Shown -eq $c0.Shown -and $wBefore -eq $wAfter -and (@(Alert-Notifs) -join "`n") -notmatch 'ZQPausedText') "quiet $($c0.Quiet)->$($c1.Quiet) shown $($c0.Shown)->$($c1.Shown)"
        Check 'S4 phone paused: the row exists (history only)' (@(Alert-Rows | Where-Object { $_.Body -eq 'ZQPausedText one' }).Count -eq 1) ''
        Phone-Cmd 'PAUSE_SET' '--es mode resume' 700 | Out-Null

        # ---- 9. the PC paused: --notify is a script's own command and is sent anyway
        Invoke-Kin --pause 1h | Out-Null
        Pc-Wait { param($s) $s.pause.on -eq $true } 8 | Out-Null
        $r = Notify @('ZQAlertText pc paused')
        Check 'S4 the PC paused: Kin.exe --notify (auto:false) is still sent (exit 0, arrives)' ($r.Exit -eq 0 -and (Wait-Until { @(Alert-Rows | Where-Object { $_.Body -eq 'ZQAlertText pc paused' }).Count -eq 1 } 10 500)) "exit $($r.Exit): $($r.Text)"
        Invoke-Kin --pause resume | Out-Null
        Pc-Wait { param($s) $s.pause.on -eq $false } 8 | Out-Null

        # ---- 10. the phone is away: queued, expires locally (ttl 1m + a clock 10 minutes ahead), arrives with "10 min ago"
        A shell am force-stop $Pkg | Out-Null
        Check 'S4 setup: the phone app is stopped, the PC sees the link close' (Pc-Wait { param($s) $s.state -ne 'phone_connected' } 15)
        $r1 = Notify @('--ttl', '1m', 'ZQExpiringText')
        Check 'S4 phone away: exit 0 and "Queued for <phone> (offline), expires in 1m."' ($r1.Exit -eq 0 -and $r1.Out -match '^Queued for .+ \(offline\), expires in 1m\.$') "exit $($r1.Exit): $($r1.Text)"
        $r2 = Notify @('ZQQueuedText')
        Check 'S4 phone away: "Queued ... expires in 24h." (the default 24 h, as the design example says; round 2)' ($r2.Exit -eq 0 -and $r2.Out -match 'Queued for .+ \(offline\), expires in 24h\.') "exit $($r2.Exit): $($r2.Text)"
        Check 'S4 the PC queue holds both' (Pc-Wait { param($s) $s.queue.count -ge 2 } 10) ((Get-Status).queue | ConvertTo-Json -Compress -Depth 4)
        Stop-DevKin 15
        Check 'S4 the test Kin is back with its clock 10 minutes ahead, and the phone connects again' (Start-TestKinAndWait @('--dev-clock-offset', '600'))
        Check 'S4 the queued alert arrives on the phone' (Wait-Until { @(Alert-Rows | Where-Object { $_.Body -eq 'ZQQueuedText' }).Count -eq 1 } 30 1000) ''
        Check 'S4 and it says how long it waited: "9 min ago" or "10 min ago" in the notification' (Wait-Until { (@(Alert-Notifs) -join "`n") -match '\b(9|10) min ago' } 20 1000) ((@(Alert-Notifs) | ForEach-Object { NField $_ 'subText' }) -join ' | ')
        Check 'S4 the alert with ttl 1m EXPIRED locally: never sent (no row on the phone)' (@(Alert-Rows | Where-Object { $_.Body -eq 'ZQExpiringText' }).Count -eq 0) ''
        $exp = @(Pc-History | Where-Object { $_.text -eq 'ZQExpiringText' })[0]
        Check 'S4 and the PC history shows it as failed, expired' ($exp -and $exp.status -eq 'failed' -and ($exp.error -match 'expired')) ($exp | ConvertTo-Json -Compress)
        Stop-DevKin 15
        Check 'S4 setup: the test Kin restarted on the real clock, the phone is back' (Start-TestKinAndWait)

        # ---- 11. a pipe that is not this Kin, and no Kin at all
        $kinOther = $Kin   # the build output Kin.exe: another program file than the copy that serves the pipe
        $beforeRows = @(Alert-Rows).Count
        $rO = Notify @('ZQAlertText wrong image') $null $kinOther
        Check 'S4 (d) Kin.exe --notify of ANOTHER Kin.exe file against the test Kin''s pipe: refused (exit 6, "not this Kin"), nothing sent' ($rO.Exit -eq 6 -and $rO.Text -match 'not this Kin') "exit $($rO.Exit): $($rO.Text)"
        # a pipe whose server is not Kin.exe at all (a throwaway server of this script, in a dev pipe name of its own)
        $sid = Pc-Sid
        $fakePipe = "app.kinrelay.dev.$sid.zqfake"
        $srv = New-Object System.IO.Pipes.NamedPipeServerStream($fakePipe, [System.IO.Pipes.PipeDirection]::InOut, 1, [System.IO.Pipes.PipeTransmissionMode]::Byte, [System.IO.Pipes.PipeOptions]::Asynchronous)
        $wait = $srv.BeginWaitForConnection($null, $null)
        $env:KIN_DEV_PIPE_SUFFIX = 'zqfake'
        $rF = Notify @('ZQAlertText fake server')
        $env:KIN_DEV_PIPE_SUFFIX = $PipeSuffix
        $gotBytes = $false
        try {
            if ($wait.AsyncWaitHandle.WaitOne(1000)) { $srv.EndWaitForConnection($wait); $buf = New-Object byte[] 64; $rd = $srv.BeginRead($buf, 0, 64, $null, $null); if ($rd.AsyncWaitHandle.WaitOne(1500)) { $n = $srv.EndRead($rd); if ($n -gt 0) { $gotBytes = $true } } }
        } catch { }
        try { $srv.Dispose() } catch { }
        Check 'S4 (d) a pipe whose server is not Kin.exe (a PowerShell stand-in): refused (exit 6), and NOT ONE byte was written to it' ($rF.Exit -eq 6 -and -not $gotBytes) "exit $($rF.Exit): $($rF.Text); bytes received by the stand-in: $gotBytes"
        $env:KIN_DEV_PIPE_SUFFIX = 'zqnobody'
        $rN = Notify @('--no-start', 'ZQAlertText nobody')
        $env:KIN_DEV_PIPE_SUFFIX = $PipeSuffix
        Check 'S4 no Kin on the pipe: exit code 3, nothing started' ($rN.Exit -eq 3 -and @(Get-DevKinProcesses).Count -eq 1) "exit $($rN.Exit): $($rN.Text)"
        Start-Sleep -Seconds 2
        Check 'S4 (d) none of those three reached the phone' (@(Alert-Rows).Count -eq $beforeRows) "rows $beforeRows -> $(@(Alert-Rows).Count)"
        Check 'S4 the installed Kin''s pipe was never opened by this script (no test command line carried its name; the dev pipe is app.kinrelay.dev.*.e2e2b5)' ((Kin-Log) -match 'extension pipe: listening as app\.kinrelay\.dev\.S-1-5-[\d-]+\.e2e2b5') ''

        # ---- 12. the clipboards after all of it
        Check 'S4 after all the alerts the phone clipboard is as it was, and Kin wrote nothing to the PC clipboard (its log has no clipboard write; the PC clipboard itself is the USER''S and may have changed during a long run: see the note)' (((Phone-Clip).Sha -eq $phClip0) -and ((Log-Count '(on the clipboard|copied to the clipboard|send clipboard|text .* copied|clipboard changed)') -eq $pcLogClip0) -and ((Get-Clipboard -Raw -ErrorAction SilentlyContinue | Out-String) -notmatch 'ZQ(Alert|Stdin|Queued|Expiring|Burst|Paused)')) "phone $phClip0; PC clipboard signature same as at the start of S4: $((Clip-Signature) -eq $pcClip0)"
        if ((Clip-Signature) -ne $pcClip0) { Note 'S4: the PC clipboard is not what it was at the start of S4 and Kin''s log shows no clipboard write and no alert text on it: it is what the USER copied meanwhile (the PC is in daily use)' }
        Go-Home

        # ---- 13. a phone that cannot take alerts (fake, old), and no phone at all
        $PcId = (Get-Status).devices[0].deviceId
        Invoke-Kin --dev-set unlinkDevice $PcId | Out-Null
        Check 'S4 setup: the emulator phone is unlinked on the PC' (Pc-Wait { param($s) @($s.devices).Count -eq 0 } 15)
        $rNP = Notify @('ZQAlertText no phone')
        Check 'S4 no phone paired: exit code 4' ($rNP.Exit -eq 4) "exit $($rNP.Exit): $($rNP.Text)"
        Remove-Item (Join-Path $Data 'pair-url.txt') -ErrorAction SilentlyContinue
        Invoke-Kin --dev-start-pairing | Out-Null
        Wait-Until { Test-Path (Join-Path $Data 'pair-url.txt') } 10 | Out-Null
        $url = (Get-Content (Join-Path $Data 'pair-url.txt') -Raw).Trim()
        $ident = Join-Path $Work 'fake-identity.json'
        $fp = Start-Fake @($url, '--host', '127.0.0.1', '--name', 'ZQ Old Phone', '--identity-file', $ident, '--seconds', '40', '--old-phone') 'fake-old.txt'
        Check 'S4 setup: the old fake phone (no alert capability) paired and connected' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 25) ((Fake-Text 'fake-old.txt').Substring(0, [Math]::Min(200, (Fake-Text 'fake-old.txt').Length)))
        $rOld = Notify @('--device', 'ZQ Old Phone', 'ZQAlertText old phone')
        Check 'S4 a connected phone that does not list "alert": exit code 6 (no fallback to a text: nothing reached its clipboard)' ($rOld.Exit -eq 6 -and (Fake-Text 'fake-old.txt') -notmatch 'ZQAlertText') "exit $($rOld.Exit): $($rOld.Text)"
        Stop-OwnProcess $fp $Fake
        Wait-Until { (Get-Status).state -ne 'phone_connected' } 20 | Out-Null
        Invoke-Kin --dev-set unlinkDevice (Get-Status).devices[0].deviceId | Out-Null
        Pc-Wait { param($s) @($s.devices).Count -eq 0 } 15 | Out-Null
        Check 'S4 setup: pair the emulator phone again' (Pair-Phone)
        $PcId = (Get-Status).devices[0].deviceId
    }

    # ============================================================================================================================ S5 PC battery low -> phone
    if (Enter-Scenario 'S5' 'the PC''s low battery on the phone') {
        Go-Home
        Reset-Dnd
        Phone-Cmd 'PAUSE_SET' '--es mode resume' 500 | Out-Null
        Invoke-Kin --dev-set devBattery 77:charging | Out-Null
        Check 'S5 setup: the PC''s fake battery (77 %, charging) is on the phone (PRESENCE_STATE pcBattery)' (Wait-Until { (Val (Presence-State) 'pcBattery') -eq '77/true' } 60 2000) (Presence-State)
        $stamp0 = ((@(Alert-Notifs) | ForEach-Object { (([regex]::Matches($_, '(when|posttimeElapsedMs)=\d+') | ForEach-Object { $_.Value }) -join '/') }) -join ',')
        Invoke-Kin --dev-set devBattery 14:unplugged | Out-Null
        Check 'S5 the PC at 14 %, not charging: the phone notifies "<PC> battery is 14%" on kin_alerts' (Wait-Until { (@(Alert-Notifs) -join "`n") -match 'DESK-PC battery is 14%' } 75 2000) ((@(Alert-Notifs) | ForEach-Object { NField $_ 'text' }) -join ' | ')
        $blk = @(Alert-Notifs | Where-Object { $_ -match 'battery is 14%' })[0]
        Check 'S5 it is on the alert channel, private on the lock screen, no buttons' ($blk -match 'channel=kin_alerts' -and $blk -match 'vis=PRIVATE' -and $blk -notmatch 'actions=') ''
        Observe-Alert 'the PC battery nudge' $blk
        Shot '03-s5-pc-battery-nudge'
        $w1 = ((@(Alert-Notifs) | ForEach-Object { (([regex]::Matches($_, '(when|posttimeElapsedMs)=\d+') | ForEach-Object { $_.Value }) -join '/') }) -join ',')
        Invoke-Kin --dev-set devBattery 12:unplugged | Out-Null   # (a change of 1 point is not sent: the battery step is 2)
        Check 'S5 the phone sees 12 %' (Wait-Until { (Val (Presence-State) 'pcBattery') -eq '12/false' } 60 2000) (Presence-State)
        Start-Sleep -Seconds 3
        Check 'S5 one per discharge episode: 12 % gives no second notification' (((@(Alert-Notifs) -join "`n") -notmatch 'battery is 12%') -and $w1 -eq ((@(Alert-Notifs) | ForEach-Object { (([regex]::Matches($_, '(when|posttimeElapsedMs)=\d+') | ForEach-Object { $_.Value }) -join '/') }) -join ',')) ((@(Alert-Notifs) | ForEach-Object { NField $_ 'text' }) -join ' | ')
        Invoke-Kin --dev-set devBattery 80:charging | Out-Null
        Check 'S5 charging at 80 %: the phone sees it (re-arms the nudge)' (Wait-Until { (Val (Presence-State) 'pcBattery') -eq '80/true' } 60 2000) (Presence-State)
        Invoke-Kin --dev-set devBattery 11:unplugged | Out-Null   # another level than the first episode, so the text itself proves a new post
        Check 'S5 unplugged at 11 % (a second discharge episode, re-armed by charging): a new notification "battery is 11%"' (Wait-Until { (Val (Presence-State) 'pcBattery') -eq '11/false' -and ((@(Alert-Notifs) -join "`n") -match 'DESK-PC battery is 11%') } 75 2000) "$((@(Alert-Notifs) | ForEach-Object { NField $_ 'title' }) -join ' | ')"
        # a paused phone: none
        Invoke-Kin --dev-set devBattery 80:charging | Out-Null
        Wait-Until { (Val (Presence-State) 'pcBattery') -eq '80/true' } 60 2000 | Out-Null
        Phone-Cmd 'PAUSE_SET' '--es mode 1h' 700 | Out-Null
        $w2 = ((@(Alert-Notifs) | ForEach-Object { (([regex]::Matches($_, '(when|posttimeElapsedMs)=\d+') | ForEach-Object { $_.Value }) -join '/') }) -join ',')
        Invoke-Kin --dev-set devBattery 10:unplugged | Out-Null
        Check 'S5 (paused phone) the PC at 10 %: the phone sees it ...' (Wait-Until { (Val (Presence-State) 'pcBattery') -match '^10/false' } 60 2000) (Presence-State)
        Start-Sleep -Seconds 3
        Check 'S5 ... and shows NO notification while paused' (((@(Alert-Notifs) -join "`n") -notmatch 'battery is 10%') -and $w2 -eq ((@(Alert-Notifs) | ForEach-Object { (([regex]::Matches($_, '(when|posttimeElapsedMs)=\d+') | ForEach-Object { $_.Value }) -join '/') }) -join ',')) ((@(Alert-Notifs) | ForEach-Object { NField $_ 'text' }) -join ' | ')
        Phone-Cmd 'PAUSE_SET' '--es mode resume' 700 | Out-Null
        Invoke-Kin --dev-set devBattery 77:charging | Out-Null
    }

    # ============================================================================================================================ S6 the extension
    if (Enter-Scenario 'S6' 'the browser extension: download alerts (headless Chromium, throwaway profile, the dev host)') {
        Go-Home
        Reset-Dnd
        $keyPath = 'HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev'
        & reg query $keyPath /ve 2>$null | Out-Null
        $keyThere = ($LASTEXITCODE -eq 0)
        $extManifest = Join-Path $Repo 'extension\dist\chromium-test\manifest.json'
        $hostExe = Join-Path $Repo 'desktop\native-host\target\dev\debug\kin-native-host.exe'
        $mockExe = Join-Path $Repo 'desktop\native-host\target\dev\debug\mock-kin.exe'
        if ($keyThere) { Check 'S6 the dev host registry key does not exist yet (refusing to touch one that does)' $false $keyPath }
        elseif (-not (Test-Path $extManifest)) { Check 'S6 the extension test build exists (bun run build:test in extension\)' $false }
        elseif (-not (Test-Path $hostExe) -or -not (Test-Path $mockExe)) { Check 'S6 the dev native host and the stand-in Kin exist (desktop\native-host\target\dev\debug)' $false }
        else {
            $bun = (Get-Command bun -ErrorAction Stop).Source
            $script:ScenarioStart = Get-Date
            $log = Join-Path $Work 's6-extension.log'
            $env:E2E_DATA = $Data; $env:E2E_SUFFIX = $PipeSuffix; $env:E2E_KIN = $KinCopy; $env:E2E_HOST_EXE = (Resolve-Path $hostExe).Path; $env:E2E_MOCK_EXE = (Resolve-Path $mockExe).Path
            $env:E2E_ADB = $Adb; $env:E2E_SERIAL = $Serial; $env:E2E_PKG = $Pkg; $env:E2E_OUT = (Join-Path $Work 's6-results.json'); $env:E2E_WORK = $Work; $env:E2E_SHOTS = $ScreenDir
            $p = Start-Process $bun -ArgumentList @((Join-Path $PSScriptRoot 'e2e-2b-5-extension.ts')) -PassThru -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError "$log.err" -WorkingDirectory $Repo
            if (-not $p.WaitForExit(600000)) { Stop-OwnProcess $p $bun; Check 'S6 the extension run finished in 10 minutes' $false }
            foreach ($k in 'E2E_DATA', 'E2E_SUFFIX', 'E2E_KIN', 'E2E_HOST_EXE', 'E2E_MOCK_EXE', 'E2E_ADB', 'E2E_SERIAL', 'E2E_PKG', 'E2E_OUT', 'E2E_WORK', 'E2E_SHOTS') { Remove-Item "Env:\$k" -ErrorAction SilentlyContinue }
            $lines = @(Get-Content $log -ErrorAction SilentlyContinue)
            foreach ($l in $lines) {
                if ($l -match '^\s+ok\s+(.*)$') { Check "S6 $($Matches[1])" $true }
                elseif ($l -match '^\s+FAIL\s+(.*)$') { Check "S6 $($Matches[1])" $false (($lines | Where-Object { $_ -match '^\s{6,}' } | Select-Object -First 3) -join ' | ') }
            }
            if (-not ($lines -match 'steps passed')) { Check 'S6 the extension run printed its summary' $false (($lines | Select-Object -Last 6) -join ' | ') }
            try { $script:DownloadNames = @((Get-Content (Join-Path $Work 's6-results.json') -Raw | ConvertFrom-Json).names) } catch { $script:DownloadNames = @() }
            & reg query $keyPath /ve 2>$null | Out-Null
            if ($LASTEXITCODE -eq 0) { Check 'S6 the dev host registry key is gone after the run' $false; & reg delete $keyPath /f | Out-Null } else { Check 'S6 the dev host registry key is gone after the run' $true }
            Check 'S6 the stand-in Kin the host started is gone' (@(Get-Process mock-kin -ErrorAction SilentlyContinue | Where-Object { $_.Path -ieq (Resolve-Path $mockExe).Path -and $_.StartTime -gt $script:ScenarioStart }).Count -eq 0) ''
            Check 'S6 the real test Kin was not started or stopped by the extension (one process, as before)' (@(Get-DevKinProcesses).Count -eq 1) ''
        }
    }

    # ============================================================================================================================ S9 the 2B.4 review fixes
    if (Enter-Scenario 'S9' 'the 2B.4 review fixes: hold Cancel, auto-sent upload on a lost connection, ring volume under DND, link notification after unlink') {
        Go-Home
        Reset-Dnd
        Phone-Cmd 'PAUSE_SET' '--es mode resume' 500 | Out-Null
        Invoke-Kin --pause resume | Out-Null   # a paused PC makes the phone skip auto-sent screenshots (counted "paused")
        Pc-Wait { param($s) $s.pause.on -eq $false } 8 | Out-Null
        $perm = if ($script:Sdk -ge 33) { 'android.permission.READ_MEDIA_IMAGES' } else { 'android.permission.READ_EXTERNAL_STORAGE' }
        A shell pm grant $Pkg $perm | Out-Null
        function Shot-Files { @(Received-Files | Where-Object { $_.Name -match '^(Screenshot|Photo) ' }) }
        function Insert-Shot([string]$Kind) { $l = Phone-One 'SHOTS_INSERT' "--es kind $Kind" 900; if ($l -match ' id=(\d+)') { [long]$Matches[1] } else { -1 } }
        function SC([string]$S, [string]$Key) { if ($S -match ("\b{0}[: ]+(\d+)" -f $Key)) { [int]$Matches[1] } else { -1 } }

        # ---- R1: the hold notification has its own group, so its Cancel button is visible in the shade WITHOUT expanding anything
        $r = Phone-One 'SHOTS_SET' '--ez screenshots true' 1500
        Check 'S9 R1 setup: auto-send of screenshots is on and watching' ($r -match 'refused=false' -and $r -match 'watching=true') $r
        Start-Sleep -Seconds 3
        # The hold lasts only 5 s: the shade is opened and read at once (one uiautomator dump, up to 3 tries); the group key and the picture are a second trial.
        # Round 2 (B2): the hold is a silent HEADS-UP (its own high-importance channel) with the Cancel button, the line "Tap to cancel", and a tap on the notification itself cancels.
        $cancelVisible = $false; $holdRow = $false; $expander = 0; $huCancel = $false; $huTap = $false; $shadeTap = $false
        for ($try = 1; $try -le 3 -and -not $holdRow; $try++) {
            $idT = Insert-Shot 'screenshot'
            Start-Sleep -Milliseconds 900
            # The shade is NOT open. uiautomator does not see the heads-up window, so what is checked is the record (high importance, so Android shows it as a heads-up) and a picture of the screen.
            $huRec = (Notif-Dump); $huCancel = ($huRec -match 'tag=kin-shot importance=4'); $huTap = ($huRec -match 'android\.text=String \(Tap to cancel\)')
            if (-not $script:HuShot) { Shot '05-s9-b2-hold-heads-up'; $script:HuShot = $true }
            A shell cmd statusbar expand-notifications | Out-Null; Start-Sleep -Milliseconds 700
            $ui = Ui-Dump
            $holdRow = $ui -match 'Sending'
            $shadeTap = $ui -match 'text="Tap to cancel"'
            $cancelVisible = $ui -match 'text="Cancel"'
            $expander = ([regex]::Matches($ui, 'content-desc="Expand[^"]*"')).Count
            A shell cmd statusbar collapse | Out-Null
            if (-not $holdRow) { Say "S9 R1 try ${try}: the hold was not on screen yet/any more (it lasts 5 s)"; Start-Sleep -Seconds 8 } else { Start-Sleep -Seconds 7 }
        }
        Check 'S9 R1 the hold is a row of its own in the shade (not folded under Kin''s persistent notification: no "2" group expander on it)' $holdRow "sending row on screen=$holdRow, expand buttons in the shade=$expander"
        Check 'S9 R1/B2 (round 2) on arrival (shade not open) the hold is a high-importance record titled "...", text "Tap to cancel" (a heads-up; picture 05)' ($huCancel -and $huTap) "importance=4: $huCancel, tap-line: $huTap"
        Check 'S9 R1/B2 (round 2) in the open shade the row shows "Tap to cancel" without expanding anything (the Cancel button too when Android expands it)' $shadeTap "tap-line=$shadeTap cancel=$cancelVisible expand buttons=$expander"
        $id2 = Insert-Shot 'screenshot'
        Check 'S9 R1 a new screenshot: the hold notification appears' (Wait-Until { (Notif-Dump) -match 'tag=kin-shot' } 3 100) ''
        $recs = @(((Notif-Dump) -split '(?m)(?=^\s*NotificationRecord)') | Where-Object { $_ -match 'tag=kin-shot' })
        $rec = if ($recs.Count) { $recs[0] } else { '' }
        $grp = [regex]::Match($rec, '(?m)^\s*groupKey=([^\r\n]*)').Groups[1].Value
        Check 'S9 R1 the hold notification is its own group (a group key of its own, not Kin''s persistent notification)' ($grp -match 'kin-shot' -or $rec -match 'group=kin-shot') "groupKey=$grp"
        $chan = [regex]::Match($rec, 'channel=(\S+)').Groups[1].Value
        $imp = [regex]::Match($rec, 'tag=kin-shot importance=(\d+)').Groups[1].Value
        $txt = [regex]::Match($rec, 'android\.text=String \(([^\r\n]*)\)').Groups[1].Value
        $hasActions = $rec -match 'actions=\{[^}]*Cancel' -or $rec -match '"Cancel"' -or $rec -match 'Cancel'
        $hasContent = $rec -match 'contentIntent=PendingIntent'
        $silentOk = $rec -notmatch 'sound=Uri' -and $rec -notmatch 'vibrate=true'
        Check 'S9 R1/B2 the hold record: channel kin_shot_hold, importance HIGH (4), text "Tap to cancel", a content intent, the Cancel action, no sound of its own' ($chan -eq 'kin_shot_hold' -and $imp -eq '4' -and $txt -eq 'Tap to cancel' -and $hasContent -and $hasActions -and $silentOk) "channel=$chan importance=$imp text='$txt' content=$hasContent actions=$hasActions silent=$silentOk"
        $chanDump = (A shell dumpsys notification --noredact | Out-String)
        $cm = [regex]::Match($chanDump, '(?s)NotificationChannel\{mId=''kin_shot_hold''.*?\}')
        Check 'S9 R1/B2 the channel kin_shot_hold has no sound and no vibration (the silent heads-up)' ($cm.Success -and $cm.Value -match 'mImportance=4' -and $cm.Value -notmatch 'mSound=content' -and $cm.Value -notmatch 'mVibrationEnabled=true') ($cm.Value)
        A shell cmd statusbar expand-notifications | Out-Null; Start-Sleep -Milliseconds 700
        New-Item -ItemType Directory -Force $ScreenDir | Out-Null
        cmd /c "`"$Adb`" -s $Serial exec-out screencap -p > `"$(Join-Path $ScreenDir '04-s9-r1-hold-in-the-shade-no-expanding.png')`"" 2>$null
        Phone-Cmd 'SHOTS_CANCEL' "--el id $id2" 500 | Out-Null
        A shell cmd statusbar collapse | Out-Null
        Start-Sleep -Seconds 8
        $st = Shots-State
        Check 'S9 R1 a cancel (debug SHOTS_CANCEL) stops the send: counted cancelled, nothing sent' ((SC $st 'cancelled') -ge 1) $st
        # ---- B2 (round 2): ONE TAP on the notification (the heads-up, or the row in the open shade) cancels the send; the same single-owner cancel as the button
        $c1 = SC $st 'cancelled'; $sent1 = SC $st 'sent'; $files1 = (Shot-Files).Count
        # One tap on the heads-up itself (the title line; the emulator screen is 1080 wide, the heads-up card is at the top), nothing expanded. The insert, the wait and the tap are ONE shell call:
        # with host-side latency between them the 5 s hold was over before the tap landed. Fallback: the row in the open shade.
        A shell "am broadcast -a $Pkg.debug.SHOTS_INSERT -n $Pkg/.debug.DebugCommandReceiver --es kind screenshot >/dev/null; sleep 1.6; input tap 540 220" | Out-Null
        Start-Sleep -Milliseconds 700
        $how = 'the heads-up'
        $tapped = ((SC (Shots-State) 'cancelled') -gt $c1)
        if (-not $tapped) {
            # The tap missed (or the heads-up was gone): wait this hold out, then a second attempt on the row in the open shade.
            $how = 'the row in the open shade (the heads-up tap did not cancel)'
            Start-Sleep -Seconds 9
            $st = Shots-State; $c1 = SC $st 'cancelled'; $sent1 = SC $st 'sent'; $files1 = (Shot-Files).Count
            $id3 = Insert-Shot 'screenshot'; Start-Sleep -Milliseconds 900
            A shell cmd statusbar expand-notifications | Out-Null; Start-Sleep -Milliseconds 700
            [void](Tap-Text @('Tap to cancel') 2)
            Start-Sleep -Milliseconds 700
            $tapped = ((SC (Shots-State) 'cancelled') -gt $c1)
        }
        A shell cmd statusbar collapse | Out-Null
        Check 'S9 R1/B2 one tap on the notification (no Cancel button, no expanding) cancels the hold' $tapped "tapped $how"
        Start-Sleep -Seconds 8
        $st3 = Shots-State
        Check 'S9 R1/B2 the tap cancelled the send: counted cancelled (+1), nothing sent, no file on the PC, no hold or "Sending" notification left' (((SC $st3 'cancelled') -eq ($c1 + 1)) -and ((SC $st3 'sent') -eq $sent1) -and ((Shot-Files).Count -eq $files1) -and ((Notif-Dump) -notmatch 'tag=kin-shot')) $st3
        Phone-Cmd 'SHOTS_SET' '--ez screenshots false' 800 | Out-Null

        if ($Skip -notcontains 'R3') {
        # ---- R3: ring alarm volume under Do Not Disturb (restored; nothing saved or raised where the alarm is silenced)
        Phone-Cmd 'RING_SET' '--ez allow true --ez silent false --ez loud true' 800 | Out-Null
        Check 'S9 R3 setup: the PC offers ringing' (Wait-Until { (View 'settings').presence.ring.can -eq $true } 15 1000) ''
        $vol0 = Alarm-Volume
        function Ring-Once([string]$Label) {
            Invoke-Kin --ring | Out-Null
            Wait-Until { (Ring-State) -match 'state=ringing' } 10 500 | Out-Null
            $rs = Ring-State; $vNow = Alarm-Volume
            Invoke-Kin --ring-stop | Out-Null
            Wait-Until { (Ring-State) -match 'state=idle' } 10 500 | Out-Null
            $after = Ring-State; $vAfter = Alarm-Volume
            Start-Sleep -Seconds 22   # the PC allows one press per 10 s, and the phone 3 starts per 60 s
            return [pscustomobject]@{ During = $rs; Vol = $vNow; After = $after; VolAfter = $vAfter }
        }
        if ($script:Sdk -ge 30) {
            $g = Ring-Once 'no dnd'
            Check 'S9 R3 no DND: audible, the alarm volume raised while ringing, put back and nothing saved afterwards' ($g.During -match 'audible=true' -and $g.Vol -gt $vol0 -and $g.VolAfter -eq $vol0 -and (Val $g.After 'savedAlarmIndex') -eq 'null') "during: $($g.During) vol $vol0 -> $($g.Vol) -> $($g.VolAfter) | $($g.After)"
            A shell cmd notification set_dnd none | Out-Null; Start-Sleep -Seconds 1
            $g = Ring-Once 'total silence'
            Check 'S9 R3 DND total silence: audible=false, NOTHING saved or raised (volume unchanged, savedAlarmIndex null), unchanged afterwards' ($g.During -match 'audible=false' -and (Val $g.During 'savedAlarmIndex') -eq 'null' -and $g.Vol -eq $vol0 -and $g.VolAfter -eq $vol0) "during: $($g.During) vol $vol0 -> $($g.Vol) -> $($g.VolAfter)"
            A shell cmd notification set_dnd alarms | Out-Null; Start-Sleep -Seconds 1
            $g = Ring-Once 'alarms only'
            Check 'S9 R3 DND alarms-only: the alarm still plays (audible=true), volume raised and restored' ($g.During -match 'audible=true' -and $g.Vol -gt $vol0 -and $g.VolAfter -eq $vol0 -and (Val $g.After 'savedAlarmIndex') -eq 'null') "during: $($g.During) vol $vol0 -> $($g.Vol) -> $($g.VolAfter)"
            A shell cmd notification set_dnd priority | Out-Null; Start-Sleep -Seconds 1
            $g = Ring-Once 'priority'
            Check 'S9 R3 DND priority only (the default policy lets alarms through): audible=true, volume raised and restored' ($g.During -match 'audible=true' -and $g.Vol -gt $vol0 -and $g.VolAfter -eq $vol0 -and (Val $g.After 'savedAlarmIndex') -eq 'null') "during: $($g.During) vol $vol0 -> $($g.Vol) -> $($g.VolAfter)"
            A shell cmd notification set_dnd off | Out-Null; A shell cmd notification set_dnd all | Out-Null
            Check 'S9 R3 after all of it the alarm volume is what it was' ((Alarm-Volume) -eq $vol0) "now $(Alarm-Volume), before $vol0"
        } else {
            Note 'API 29: no `cmd notification set_dnd`: the ring under DND is checked on API 37 (the debug DND_USER could set it, but the PC-side ring needs the same path as 2B.4 S2, which skipped it on API 29 too)'
            $g = Ring-Once 'no dnd'
            Check 'S9 R3 no DND: audible, volume raised and restored, nothing saved afterwards' ($g.During -match 'audible=true' -and $g.Vol -gt $vol0 -and $g.VolAfter -eq $vol0 -and (Val $g.After 'savedAlarmIndex') -eq 'null') "during: $($g.During) vol $vol0 -> $($g.Vol) -> $($g.VolAfter)"
            Grant-Dnd
            Set-UserDnd 'none'; Start-Sleep -Seconds 1
            $g = Ring-Once 'total silence'
            Check 'S9 R3 (API 29, DND_USER total silence) audible=false, nothing saved or raised' ($g.During -match 'audible=false' -and (Val $g.During 'savedAlarmIndex') -eq 'null' -and $g.Vol -eq $vol0 -and $g.VolAfter -eq $vol0) "during: $($g.During) vol $vol0 -> $($g.Vol) -> $($g.VolAfter)"
            Set-UserDnd 'off'; Revoke-Access
        }

        }
        if ($Skip -notcontains 'R2') {
        # ---- R2: an auto-sent upload that loses its connection ends "not sent" at once and is NOT resumed later
        $perm2 = $perm
        $big = 'Screenshot_zqbig.png'
        A shell "rm -f /sdcard/Pictures/Screenshots/$big" | Out-Null
        Phone-Cmd 'SHOTS_SET' '--ez screenshots true' 1500 | Out-Null
        Start-Sleep -Seconds 3
        A shell "mkdir -p /sdcard/Pictures/Screenshots && head -c 314572800 /dev/urandom > /sdcard/Pictures/Screenshots/$big" | Out-Null
        A shell am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE -d "file:///sdcard/Pictures/Screenshots/$big" | Out-Null
        Check 'S9 R2 setup: a 300 MB "screenshot" from another app is in the MediaStore, the hold starts' (Wait-Until { (Notif-Dump) -match 'tag=kin-shot' } 20 500) (Shots-State)
        $filesBefore = @(Shot-Files | Where-Object { $_.Name -notmatch '\.part$' }).Count
        $flying =Pc-Wait { param($s) @($s.transfers.active).Count -gt 0 } 40
        Check 'S9 R2 the upload is in flight on the PC (after the 5 s hold)' $flying ((Get-Status).transfers.active | ConvertTo-Json -Compress -Depth 3)
        Start-Sleep -Seconds 2
        Wifi $false
        Check 'S9 R2 setup: the phone''s link is down (Wi-Fi off in the middle of the upload)' (Wait-Until { (Phone-One 'STATE' '' 700) -notmatch 'link=Connected' } 40 1000) (Phone-One 'STATE' '' 700)
        $tr = @(Phone-Cmd 'TRANSFERS' '' 1200)
        Say ("phone transfers right after the drop: " + ($tr -join ' | '))
        Wifi $true
        Check 'S9 R2 the phone reconnects' (Wait-Until { (Phone-One 'STATE' '' 700) -match 'link=Connected' } 90 2000) (Phone-One 'STATE' '' 700)
        Start-Sleep -Seconds 45
        $tr2 = @(Phone-Cmd 'TRANSFERS' '' 1200)
        Say ("phone transfers 45 s after the reconnect: " + ($tr2 -join ' | '))
        $upLines = @($tr2 | Where-Object { $_ -match 'CMD TRANSFER ' -and $_ -match '\bup\b' })
        $activeUp = @($upLines | Where-Object { $_ -match '\b(active|queued|running|waiting|retry)\b' })
        $pcDone = @(Shot-Files | Where-Object { $_.Name -notmatch '\.part$' }).Count
        $pcParts = @(Get-ChildItem $Received -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '\.part$' })
        Check 'S9 R2 45 s after the PC is reachable again the upload is NOT resumed: no active or queued upload on the phone and no completed file on the PC' ($activeUp.Count -eq 0 -and $pcDone -eq $filesBefore) "phone: $($upLines -join ' | ') | PC completed files: $pcDone (before $filesBefore)"
        if ($pcParts.Count -gt 0) { Note ("S9 R2: the PC keeps the partial file of the abandoned upload ({0} .part file(s), {1} bytes) until its own cleanup (the PC forgets an upload with no progress after 24 hours); the phone will never resume it" -f $pcParts.Count, (($pcParts | Measure-Object -Property Length -Sum).Sum)) }
        Check 'S9 R2 the transfer is recorded as failed, not sent (the phone says so)' (@($upLines | Where-Object { $_ -match 'fail|offline|not sent|error' }).Count -ge 1) ($upLines -join ' | ')
        A shell "rm -f /sdcard/Pictures/Screenshots/$big" | Out-Null
        Phone-Cmd 'SHOTS_SET' '--ez screenshots false' 800 | Out-Null

        }
        # ---- R4: the link notification does not come back after unlinking
        Check 'S9 R4 setup: the persistent link notification is up ("Connected to")' (Wait-Until { (Kin-Notifs) -match 'Connected to' } 20 1000) ''
        $PcId = (Get-Status).devices[0].deviceId
        Invoke-Kin --dev-set unlinkDevice $PcId | Out-Null
        Check 'S9 R4 the PC unlinked the phone; the phone is no longer paired' (Wait-Until { (Phone-One 'STATE' '' 700) -match 'paired=null' } 20 1500) (Phone-One 'STATE' '' 700)
        $gone = @(); $lingering = ''
        foreach ($wait in 1, 5, 12, 25) {
            Start-Sleep -Seconds $wait
            $kn = Kin-Notifs
            $gone += [bool]($kn -notmatch 'Connected to')
            if ($kn -match 'Connected to' -and -not $lingering) { $lingering = ((($kn -split '(?m)(?=^\s*NotificationRecord)') | Where-Object { $_ -match 'Connected to' } | Select-Object -First 1) -split "`r?`n" | Where-Object { $_ -match 'NotificationRecord|channel=|flags=|android\.title|android\.text|when=|stats=' } | Select-Object -First 8) -join ' // ' }
        }
        Check 'S9 R4 the link notification is gone at 1, 6, 18 and 43 s after the unlink and does not come back' (($gone -notcontains $false)) "(gone: $($gone -join ',')) $lingering"
        Check 'S9 R4 the foreground service is not running any more' ((A shell dumpsys activity services $Pkg | Out-String) -notmatch 'isForeground=true') ''
        Check 'S9 R4 setup: pair again for what follows' (Pair-Phone)
        $PcId = (Get-Status).devices[0].deviceId
        # ---- R4b: the same, but the link flapped shortly before (Wi-Fi off and on): the stop of the service races with the last notification update. A record without the foreground-service flag
        # after the service is gone is a stale notification (a post that came after the service left the foreground). The designed "DESK-PC no longer trusts this phone. Pair again" notification
        # (the phone found out by a 401 while the PC's message could not reach it) is a foreground-service one and is not counted.
        $stale = @()
        for ($n = 1; $n -le $R4bCycles; $n++) {
            Wifi $false; Start-Sleep -Seconds (4 + ($n % 4)); Wifi $true
            Wait-Until { (Get-Status).state -eq 'phone_connected' } 60 1000 | Out-Null
            Start-Sleep -Seconds (3 * ($n % 4))
            Invoke-Kin --dev-set unlinkDevice ((Get-Status).devices[0].deviceId) | Out-Null
            Start-Sleep -Seconds 12
            foreach ($r in @((Kin-Notifs) -split '(?m)(?=^\s*NotificationRecord)' | Where-Object { $_ -match 'channel=link' })) {
                $fl = [regex]::Match($r, 'flags=([A-Z_|]+)\r?\n').Groups[1].Value; $tx = [regex]::Match($r, 'android\.text=String \(([^\r\n]*)\)').Groups[1].Value
                Say "S9 R4b cycle ${n}: a link notification is on the shade after the unlink: flags=$fl text='$tx'"
                $svcRunning = [bool]((A shell dumpsys activity services $Pkg | Out-String) -match 'ServiceRecord')
                # stale = the record is there but the service is gone (the service is what owns it; the PairAgain notification belongs to a running service)
                if (-not $svcRunning) { $stale += "cycle $n flags=$fl text='$tx' (service running: $svcRunning)" }
            }
            Pair-Phone | Out-Null
        }
        Check 'S9 R4b after a Wi-Fi flap, unlinking from the PC leaves NO stale link notification (R4bCycles cycles; a record without the foreground-service flag counts as stale)' ($stale.Count -eq 0) ($stale -join ' | ')
        $PcId = (Get-Status).devices[0].deviceId
    }

    # ============================================================================================================================ S7 privacy
    if (Want 'S7') {
        Remove-Item (Join-Path $Data 'view-*.json') -ErrorAction SilentlyContinue
        Flush-PhoneLog
        $names = @($script:DownloadNames | Where-Object { $_ })
        $all = @($TestStrings + $names)
        $pcLog = Kin-Log
        $leak = @($all | Where-Object { $pcLog.Contains($_) })
        Check 'S7 the PC log holds no alert text, title or download name' ($leak.Count -eq 0 -and $pcLog.Length -gt 1000) "log $($pcLog.Length) bytes; $($leak -join ',')"
        $dndLines = @(([regex]::Matches($pcLog, '(?m)^[^\r\n]*(focus|Focus|dnd|Do Not Disturb)[^\r\n]*$')) | ForEach-Object { $_.Value })
        $ringToasts = @($dndLines | Where-Object { $_ -match 'toast: Ringing, but Do Not Disturb' })
        if ($ringToasts.Count -gt 0) { Note ("S7: the PC log holds {0} line(s) of the 2B.4 ring toast `"Ringing, but Do Not Disturb is on total silence`" (the toast text is logged, as for every PC toast): it says the phone was in total silence at the moment of a Ring press, and nothing else; not a Focus/DND history" -f $ringToasts.Count) }
        $dndLines = @($dndLines | Where-Object { $_ -notmatch 'toast: Ringing, but Do Not Disturb' })
        Check 'S7 the PC log has no Focus/DND history beyond counts and enums (no line names the phone''s filter word)' (@($dndLines | Where-Object { $_ -match '(?i)\b(priority|alarms|total)\b' -and $_ -match '(?i)dnd|disturb' }).Count -eq 0) "($($dndLines.Count) focus/dnd lines; offenders: $((@($dndLines | Where-Object { $_ -match '(?i)\b(priority|alarms|total)\b' -and $_ -match '(?i)dnd|disturb' } | Select-Object -First 3 | ForEach-Object { $_ -replace '\d{4}-\d\d-\d\dT[\d:.]+Z ','' }) -join ' | '))); distinct lines: $((@($dndLines | ForEach-Object { ($_ -replace '^\S+\s+\w+\s+','') -replace '\d+','N' } | Sort-Object -Unique) -join ' | '))"
        $phoneLogText = if (Test-Path $PhoneLog) { (Get-Content $PhoneLog) | Where-Object { $_ -notmatch 'CMD ' } } else { @() }
        $phoneJoined = ($phoneLogText -join "`n")
        $leakPhone = @($all | Where-Object { $phoneJoined.Contains($_) })
        Check 'S7 the phone''s Kin log (every KinRelay line except the debug CMD echoes) holds no alert text, title or download name' ($leakPhone.Count -eq 0 -and $phoneJoined.Length -gt 1000) "log $($phoneJoined.Length) bytes; $($leakPhone -join ',')"
        $phDnd = @($phoneLogText | Where-Object { $_ -match 'dnd:' })
        Check 'S7 the phone''s DND log lines are enums and counts only (priority/alarms words as requests, no times of the PC''s Focus)' (@($phDnd | Where-Object { $_ -match 'pcFocus|focus at|since \d' }).Count -eq 0) "($($phDnd.Count) lines, e.g.: $((($phDnd | Select-Object -First 2 | ForEach-Object { $_ -replace '^\S+\s+\S+\s+\d+\s+\d+\s+\w\s+', '' }) -join ' | ')))"
        # the phone's app data: the alert texts are in the history table by design (the text table); nowhere else
        $hitFiles = @()
        foreach ($s in $all) {
            $o = (A shell "run-as $Pkg sh -c 'grep -rl -F -e `"$s`" . 2>/dev/null'" | Out-String)
            foreach ($l in ($o -split "`r?`n" | Where-Object { $_ })) { $hitFiles += [pscustomobject]@{ S = $s; File = $l.Trim() } }
        }
        $outside = @($hitFiles | Where-Object { $_.File -notmatch '^\./databases/kin\.db' })
        Check 'S7 on the phone the alert texts are only in the history database (databases/kin.db*) and in no log, DataStore, cache or other file' ($outside.Count -eq 0) "found in: $((($hitFiles | ForEach-Object { $_.File } | Sort-Object -Unique) -join ', ')); outside the database: $((($outside | ForEach-Object { $_.File } | Sort-Object -Unique) -join ', '))"
        $dndFiles = (A shell "run-as $Pkg sh -c 'find . -iname \`"*dnd*\`" 2>/dev/null'" | Out-String).Trim()
        Say "phone DND store files: $dndFiles"
        foreach ($f in ($dndFiles -split "`r?`n" | Where-Object { $_ -match 'preferences_pb' })) {
            $w = (A shell "run-as $Pkg sh -c 'strings $f 2>/dev/null || cat $f'" | Out-String)
            $words = @(([regex]::Matches($w, '[a-z_]{4,}') | ForEach-Object { $_.Value } | Sort-Object -Unique) | Where-Object { $_ -notmatch '^(true|false)$' })
            Check 'S7 the phone''s DND store holds only the allowed keys (switches, how much, we_hold, held_filter, held_route: no history)' (@($words | Where-Object { $_ -notmatch '^(dnd_\w+|follow|share|follow_filter|filter|we_hold|held_filter|held_route|route|rule|global|priority|alarms|dnd|kin)$' }).Count -eq 0) ("words: " + ($words -join ','))
        }
        # the PC data folder: the alert text is in the text table (the database) by design; everywhere else it must not be
        $files = @(Get-ChildItem $Data -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch '\\received\\' -and $_.Extension -notin '.db', '.sqlite', '.sqlite3', '.db-wal', '.db-shm' -and $_.Name -notmatch '\.db(-wal|-shm)?$' })
        $hitsPc = @($files | Where-Object { $c = Read-Shared $_.FullName; ($all | Where-Object { $c.Contains($_) }).Count -gt 0 })
        Check 'S7 no file of the PC data folder outside the text database (log, settings.json, status.json, ...) holds an alert text, title or download name' ($hitsPc.Count -eq 0) (($hitsPc | ForEach-Object { $_.Name }) -join ',')
        $st = Read-Shared (Join-Path $Data 'status.json')
        Check 'S7 status.json holds the focus state as words and counts only (no history), and no alert text' ($st -match '"focus"' -and ($all | Where-Object { $st.Contains($_) }).Count -eq 0) ''
        $settingsText = Read-Shared (Join-Path $Data 'settings.json')
        Check 'S7 settings.json has the three switches and no Focus or DND state of the moment' ($settingsText -match 'focusShare' -and $settingsText -notmatch '(?i)"dnd"|focusState|"on"\s*:') (($settingsText -replace '\s+', ' ').Substring(0, [Math]::Min(300, $settingsText.Length)))
        $dbs = @(Get-ChildItem $Data -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '\.db$' })
        $dbText = ($dbs | ForEach-Object { Read-Shared $_.FullName }) -join ''
        Check 'S7 the PC database has no phone DND or Focus history (no word of the filter next to a time)' ($dbText -notmatch '(?i)dnd[_ ]?(history|log|state)|focus[_ ]?(history|log)') ''
        Note ("S7: the alert texts are in the phone's history table and in the PC's text table by design (ZQAlertText found in the PC database: {0})" -f ($dbText.Contains('ZQAlertText')))
    }

    # === SCENARIOS-END ===
}
catch {
    Say "ERROR: $_ ($($_.ScriptStackTrace))"
    $script:Failures.Add("exception: $_")
}
finally {
    foreach ($p in $script:FakeProcs) { if ($p -and -not $p.HasExited) { Stop-OwnProcess $p $Fake } }
    if ($started.Emulator -and ((A get-state | Out-String) -match 'device') -and -not ($KeepRunning -and (Want 'SETUP') -and $Only.Count -eq 1)) {
        try { A shell svc wifi enable | Out-Null } catch {}
        try { A shell dumpsys battery reset | Out-Null } catch {}
        try { Phone-Cmd 'DND_SET' '--ez follow false --ez share false' 300 | Out-Null } catch {}
        try { Phone-Cmd 'DND_RELEASE_NOW' '' 300 | Out-Null } catch {}
        try { if ($script:Sdk -ge 30) { A shell cmd notification set_dnd off | Out-Null; A shell cmd notification set_dnd all | Out-Null } else { Phone-Cmd 'DND_USER' '--es filter all' 300 | Out-Null } } catch {}
        try { Revoke-Access } catch {}
        try { A shell cmd appops reset $Pkg | Out-Null } catch {}
        try { Phone-Cmd 'PAUSE_SET' '--es mode resume' 300 | Out-Null } catch {}
        try { Phone-Cmd 'SHOTS_SET' '--ez screenshots false --ez camera false' 300 | Out-Null } catch {}
        try { Phone-Cmd 'RING_STOP_LOCAL' '' 300 | Out-Null } catch {}
    }
    if ($script:Backup) {
        try {
            # The PC is in daily use: if the user copied something while this ran, that is theirs and is NOT replaced by the backup. What this run can leave on the clipboard is a
            # screenshot image Kin wrote (CF_DIBV5 plus the opt-out marker) or what it was at the start; anything else (text, HTML, files) is the user's own newer copy.
            # desktop\clipboard-backup.ps1 decides (the same rule as e2e-2b-4.ps1): the backup is put back only if the clipboard holds nothing, what it was at the start or a screenshot Kin wrote.
            if (Restore-ClipboardIfNotUsers $script:Backup $script:ClipAtStart) { Say 'the PC clipboard was restored' }
            else { Say 'the PC clipboard was left alone at the end: the user copied something during the run'; Note 'the PC clipboard was left alone at the end: the user copied something during the run' }
            $script:Backup = $null
        } catch { Say "restoring the clipboard failed: $_" }
    }
    if ($started.Kin -and -not $KeepRunning) { Stop-DevKin 15 }
}
# ---------------------------------------------------------------------------------------------------------------------------------- S8 guard
try {
    $installedAfter = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
    Check 'S8 the installed Kin was not started, stopped or restarted (same PIDs)' (($installedBefore -join ',') -eq ($installedAfter -join ',')) "$($installedBefore -join ',') -> $($installedAfter -join ',')"
    if (-not $KeepRunning) { Check 'S8 no Kin process of this run is left' (@(Get-DevKinProcesses).Count -eq 0) }
    Check 'S8 every fake phone this run started is gone' (@($script:FakeProcs | Where-Object { $_ -and -not $_.HasExited }).Count -eq 0)
    Check 'S8 the PC clipboard was restored' ($null -eq $script:Backup)
    $hkcu = (reg query 'HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev' 2>$null | Out-String)
    Check 'S8 the dev native-messaging host key (HKCU, app.kinrelay.dev) is not left behind' ([string]::IsNullOrWhiteSpace($hkcu)) ''
} catch { Say "ERROR in the guard checks: $_"; $script:Failures.Add("exception: $_") }
if (-not $KeepRunning -and $started.Emulator) { Invoke-EmuScript 'stop.ps1' @('-All') 120 | Out-Null }
$env:COMPUTERNAME = $script:OrigComputerName
if ($null -eq $script:OrigPipeSuffix) { Remove-Item Env:\KIN_DEV_PIPE_SUFFIX -ErrorAction SilentlyContinue } else { $env:KIN_DEV_PIPE_SUFFIX = $script:OrigPipeSuffix }
if (-not $KeepRunning) { Remove-DevDataDir $Data }
Exit-KinLock
Write-Host ''
$script:Results.GetEnumerator() | ForEach-Object { Write-Host ("{0,-5} {1}" -f $_.Value, $_.Key) }
if ($script:Zen.Count -gt 0) {
    Write-Host "`nWHAT ANDROID SHOWED (zen_mode, Kin's own rule, dumpsys notification) on ${Avd}:"
    $script:Zen | ForEach-Object { Write-Host " $_" }
    try { $script:Zen | Set-Content (Join-Path $Work "zen-observations-$Avd.txt") -Encoding UTF8 } catch {}
}
if ($script:Notes.Count -gt 0) { Write-Host "`nNotes:"; $script:Notes | ForEach-Object { Write-Host " - $_" } }
if ($script:Pending.Count -gt 0) { Write-Host "`nPENDING (not run):"; $script:Pending | ForEach-Object { Write-Host " - $_" } }
Say "work folder (logs of the phone): $Work"
$passN = @($script:Results.Values | Where-Object { $_ -eq 'PASS' }).Count
if ($script:Failures.Count -gt 0) { Write-Host "`nFAILED: $($script:Failures -join '; ')"; Write-Host "$passN of $($script:Results.Count) checks passed on $Avd."; exit 1 }
if ($script:Pending.Count -gt 0) { Write-Host "`nINCOMPLETE: $($script:Pending.Count) scenario(s) pending."; exit 2 }
Write-Host "`nOK: all $($script:Results.Count) checks passed on $Avd."
exit 0
