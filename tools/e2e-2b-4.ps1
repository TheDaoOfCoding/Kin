# Milestone 2B.4 integration (presence, pause, ring, auto-sent screenshots, unlinking): the real debug APK on an emulator against the REAL release desktop build as a
# test Kin beside the user's installed one (own --data-dir, loopback only, no window, --dev-toast-quiet, --dev-no-open, a FAKE PC battery). ASCII only.
# SILENT: the emulator runs headless with -no-audio (tools\emulator\start.ps1), the PC never plays anything, nothing is typed or clicked on the PC.
#
#   tools\e2e-2b-4.ps1                              S1 .. S6 on kin_api37
#   tools\e2e-2b-4.ps1 -Only S1,S3                  some of them (setup, pairing, the privacy grep S7 and the guard S8 always run)
#   tools\e2e-2b-4.ps1 -Avd kin_api29 -Only S1,S2,S5    the short Android 10 pass (the photo permission is READ_EXTERNAL_STORAGE there)
#   tools\e2e-2b-4.ps1 -Only SETUP -KeepRunning     setup only: leaves the emulator and the test Kin up (and the data folder) for manual spikes
#
# S1 presence both ways: the emulator's fake battery (dumpsys battery) -> the PC (status.json, tooltip) and exactly ONE low-battery warning per episode (re-armed by charging);
#    the PC's fake battery (--dev-set devBattery) -> the phone (PRESENCE_STATE and the persistent notification text); the switches
# S2 ring from the PC (a real MediaPlayer on the -no-audio emulator): ringing + the alarm volume raised, the Stop notification and (screen locked) the Stop screen with the
#    full-screen intent allowed and denied (B1), a second press while ringing is refused by the PC (B2 and the 3-per-60-s limit are unit tests: the PC cannot restart a ring),
#    auto-stop after 30 s with the alarm volume put back, `--ring-stop`, Stop on the phone, the switch off (the PC says why), Do Not Disturb "total silence" (audible false)
# S3 pause both ways: the PC (CLI = the tray's action) and the phone (the quick settings tile, the debug command): the peer shows it, quiet arrivals (no notification, no clipboard,
#    no browser, the sensitive-text count), resume, until-tomorrow / until-resumed / a past end / the 8-day guard (the 08:00 logic itself is unit-tested)
# S4 auto-sent screenshots: enable (permission by pm grant), the baseline (an earlier screenshot is not sent), a new one waits 5 s with a Cancel and is then sent and lands on the PC
#    clipboard with CF_DIBV5 and the three sensitive formats, Cancel, other / own / stale / camera off, camera on (saved, never on the clipboard), no visible hold (B3), paused,
#    the PC away (dropped, not queued), the permission revoked
# S5 unlinking: from the PC with an upload in flight (it stops, no reconnect), from the PC while the phone is offline (queued items peer_changed; the phone finds out by a 401 and
#    stops), from the phone (the Unlink confirmation text)
# S6 old peers and the privacy of a free-text ring reason (fake phone)
# S7 privacy grep: no screenshot name, sensitive text, ring reason or battery level in a log, in the phone's app data or in the PC's data folder (status.json holds levels by design)
# S8 guard: the installed Kin untouched, the PC clipboard restored, nothing left running
#
# (The regressions e2e-2b-3 -Only S1,S2 / e2e-2b-2 -Only S1 / e2e-1-3 -Only S1,S4 are their own scripts.)
# It never touches the installed Kin (process, data, AUMID, Run key, SendTo, Downloads, firewall), saves the PC clipboard in every format first and restores it, injects no OS
# input on the PC, kills only processes it started and stops at the next boundary if a game or an anti-cheat starts (the scenarios after it are recorded as PENDING).
param(
    [string]$Avd = 'kin_api37',
    [string[]]$Only = @(),
    [string]$Kin = (Join-Path $PSScriptRoot '..\desktop\src-tauri\target\release\Kin.exe'),
    [string]$Apk = (Join-Path $PSScriptRoot '..\android\app\build\outputs\apk\debug\app-debug.apk'),
    [int]$Port = 49381,
    [string]$ScreenshotDir = 'docs\results\phase-2b-4-integration-screens',
    [switch]$KeepRunning,
    [string]$Work = ''
)
. (Join-Path $PSScriptRoot 'kin-lock.ps1'); Enter-KinLock 'e2e-2b-4'
. (Join-Path $PSScriptRoot 'kin-dev.ps1')
. (Join-Path $PSScriptRoot '..\desktop\clipboard-backup.ps1')
if (-not $Work) { $Work = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-e2e-2b-4-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
Remove-Item Alias:sc -Force -ErrorAction SilentlyContinue   # `sc` is Set-Content here: the S4 helper below is called SC
$script:OrigComputerName = $env:COMPUTERNAME
$env:COMPUTERNAME = 'DESK-PC'
$ErrorActionPreference = 'Continue'
foreach ($name in 'ANDROID_HOME', 'JAVA_HOME') {
    if (-not (Get-Item "env:$name" -ErrorAction SilentlyContinue)) { $v = [Environment]::GetEnvironmentVariable($name, 'User'); if ($v) { Set-Item "env:$name" $v } }
}
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$Emu = Join-Path $Repo 'tools\emulator'
$Adb = Join-Path $(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }) 'platform-tools\adb.exe'
$Fake = Join-Path $Repo 'desktop\src-tauri\target\debug\examples\fake_phone.exe'
$Pkg = 'app.kinrelay.android'
$Serial = 'emulator-5554'
$Data = Join-Path $Work 'data'
$Received = Join-Path $Data 'received'
$PhoneLog = Join-Path $Work 'phone-kin-log.txt'
$ScreenDir = if ([IO.Path]::IsPathRooted($ScreenshotDir)) { $ScreenshotDir } else { Join-Path $Repo $ScreenshotDir }
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Results = [ordered]@{}
$script:Notes = New-Object System.Collections.Generic.List[string]
$script:Pending = New-Object System.Collections.Generic.List[string]
$started = @{ Emulator = $false; Kin = $false }
$script:Sdk = 0; $script:FakeProcs = @(); $script:Backup = $null; $script:GameStop = $false
# What must never be found in a log, in the phone's app data or in the PC's data folder (status.json may hold battery levels by design: it is skipped for those).
$TestStrings = @('kin-debug-screenshot', 'kin-debug-camera', 'kin-debug-other', 'kin-debug-own', 'ZQSecretReason', 'ZQSensitiveWords')

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
# At the start of each scenario: a game may have started since the last one. Then every later scenario is PENDING, not failed.
function Enter-Scenario([string]$Id, [string]$Title) {
    if (-not (Want $Id)) { return $false }
    if ($script:GameStop) { $script:Pending.Add("$Id $Title"); return $false }
    try { Assert-NoGameRunning } catch { $script:GameStop = $true; Say "GAME GUARD at the start of ${Id}: $_"; $script:Pending.Add("$Id $Title (a game started: stopped at this boundary)"); return $false }
    Say "--- $Id $Title"
    return $true
}
function Invoke-EmuScript([string]$Script, [string[]]$ScriptArgs = @(), [int]$TimeoutSec = 400) {
    $out = Join-Path $env:TEMP "kin-e2e-2b-4-$Script.out"
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
    # the phone's Kin log is saved to a file before every command (and the buffer cleared), so nothing rolls over and S7 reads all of it
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
function Ring-State { Phone-One 'RING_STATE' '' 700 }
function Presence-State { Phone-One 'PRESENCE_STATE' '' 700 }
function Pause-State { Phone-One 'PAUSE_STATE' '' 700 }
function Shots-State { Phone-One 'SHOTS_STATE' '' 700 }
function Notif-Dump {
    $d = (A shell dumpsys notification --noredact | Out-String)
    $i = $d.IndexOf('Notification List:')
    if ($i -lt 0) { return $d }
    $m = [regex]::Match($d.Substring($i), '(?m)^\s*(Archive|mArchive|Snoozed|Archived)')
    if ($m.Success) { return $d.Substring($i, $m.Index) } else { return $d.Substring($i) }
}
# the part of the notification list that belongs to Kin
function Kin-Notifs { $d = Notif-Dump; (($d -split '(?m)(?=^\s*NotificationRecord)') | Where-Object { $_ -match [regex]::Escape("pkg=$Pkg") }) -join "`n" }
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
    # STREAM_ALARM current index from dumpsys audio ("- STREAM_ALARM:" block, the "Current:" line); -1 if unreadable
    $d = (A shell dumpsys audio | Out-String)
    $m = [regex]::Match($d, '(?s)- STREAM_ALARM:.*?Current:\s*([^\r\n]*)')
    if (-not $m.Success) { return -1 }
    $c = [regex]::Match($m.Groups[1].Value, '\(speaker\):\s*(\d+)')
    if ($c.Success) { return [int]$c.Groups[1].Value }
    $c = [regex]::Match($m.Groups[1].Value, ':\s*(\d+)'); if ($c.Success) { return [int]$c.Groups[1].Value }
    return -1
}
function Wifi([bool]$On) { A shell svc wifi $(if ($On) { 'enable' } else { 'disable' }) | Out-Null }
# Do Not Disturb "total silence" on (-On) or off, on the emulator only: `cmd notification set_dnd` exists from API 30, below it the zen_mode setting (2 = no interruptions)
function Set-Dnd([bool]$TotalSilence) {
    if ($script:Sdk -ge 30) { A shell cmd notification set_dnd $(if ($TotalSilence) { 'none' } else { 'off' }) | Out-Null }
    else { A shell settings put global zen_mode $(if ($TotalSilence) { 2 } else { 0 }) | Out-Null }
}
function Power-Awake { ((A shell dumpsys power | Out-String) -match 'mWakefulness=Awake') }
function Focus-Window { $d = (A shell dumpsys window | Out-String); ([regex]::Match($d, 'mCurrentFocus=Window\{[^}]*\}')).Value }
# The text on the phone's clipboard, read by a focused debug activity (the only way an app may read it on Android 10+).
function Phone-Clip {
    A logcat -c | Out-Null
    A shell am start -n "$Pkg/.debug.DebugClipboardActivity" | Out-Null
    $line = $null
    Wait-Until { $script:cl = (A logcat -d -s KinRelay) | Where-Object { $_ -match 'CMD CLIPBOARD' } | Select-Object -Last 1; [bool]$script:cl } 8 | Out-Null
    $line = $script:cl
    if (-not $line -or $line -notmatch 'CMD CLIPBOARD len=(?<len>\d+) sha=(?<sha>\w+)(?: sensitive=(?<sens>\w+))? text="(?s)(?<text>.*)"\s*$') { return [pscustomobject]@{ Text = $null } }
    $t = [regex]::Replace($Matches['text'], '\\(.)', { param($m) switch ($m.Groups[1].Value) { 'n' { "`n" } 'r' { "`r" } 't' { "`t" } default { $m.Groups[1].Value } } })
    A shell input keyevent KEYCODE_HOME | Out-Null
    [pscustomobject]@{ Text = $t }
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
function Clip-Names { @(Get-ClipboardFormats | ForEach-Object { $_.Name }) }
# waits until a predicate over status.json holds: Pc-Wait { param($s) $s.pause.on -eq $true } 8
function Pc-Wait([scriptblock]$Pred, [double]$Sec = 20) { Wait-Until { $s = Get-Status; [bool]($s -and (& $Pred $s)) } $Sec 500 }
function Received-Files { @(Get-ChildItem $Received -File -Recurse -ErrorAction SilentlyContinue) }
function Start-TestKin([string[]]$Extra = @()) {
    $KinArgs = @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-port', "$Port",
        '--dev-received-dir', ('"{0}"' -f $Received), '--dev-print-pair-url', '--dev-no-windows', '--dev-toast-quiet', '--dev-no-open', '--dev-battery', '77:charging') + $Extra
    $script:KinProc = Start-DevKin $KinArgs
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

# ================================================================================================================================== run
try {
    Assert-NoGameRunning
    $Kin = (Resolve-Path $Kin -ErrorAction Stop).Path
    foreach ($f in $Apk, $Fake) { if (-not (Test-Path $f)) { throw "missing: $f" } }
    New-Item -ItemType Directory -Force $Data | Out-Null
    $installedBefore = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
    $KinCopy = Join-Path $Work 'Kin.exe'
    Copy-Item $Kin $KinCopy
    Set-KinDev -Exe $KinCopy -DataDir $Data
    Assert-NoDevKinOnDataDir
    $script:Backup = Save-Clipboard | Select-Object -Last 1
    Say 'the PC clipboard was saved in every format; it is put back at the end'
    '{ "hotkey": "", "autoClipboardSync": false, "mirrorNotifications": false }' | Set-Content (Join-Path $Data 'settings.json') -Encoding ASCII

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
    # a known battery before Kin starts: unplugged at 100 % (the emulator keeps what an earlier run left)
    A shell dumpsys battery unplug | Out-Null; A shell dumpsys battery set level 100 | Out-Null
    Set-Dnd $false
    A uninstall $Pkg 2>&1 | Out-Null
    $out = (& $Adb -s $Serial install -r (Resolve-Path $Apk).Path 2>&1 | Out-String)
    Check 'setup: install Kin (debug build)' ($out -match 'Success')
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
    # the first-run reliability setup comes up after pairing: finish it, and say "Not now" to the mirroring offer
    for ($i = 0; $i -lt 4; $i++) { if (Tap-Text @('Done', 'Finish', 'Skip', 'Continue') 1) { break }; A shell input swipe 540 1900 540 700 300 | Out-Null; Start-Sleep -Milliseconds 600 }
    Start-Sleep -Seconds 2
    if ((Ui-Dump) -match 'Show notifications on your PC') { Tap-Text @('Not now') 3 | Out-Null; Start-Sleep -Seconds 1 }
    A shell input keyevent KEYCODE_HOME | Out-Null
    # a known battery on the emulator: unplugged and a level the tests move from
    A shell dumpsys battery unplug | Out-Null

    # ============================================================================================================================ scenarios
    # === SCENARIOS-START ===
    $PcId = (Get-Status).devices[0].deviceId

    # ============================================================================================================================ S1 presence
    if (Enter-Scenario 'S1' 'presence both ways') {
        Check 'S1 the PC shows the phone''s battery from the first snapshot (emulator unplugged at 100 %)' (Pc-Wait { param($s) $s.presence.phone.level -eq 100 -and $s.presence.phone.charging -eq $false } 45) ((Get-Status).presence.phone | ConvertTo-Json -Compress)
        $w0 = Log-Count 'low battery warning for'
        A shell dumpsys battery set level 60 | Out-Null
        Check 'S1 level 60 reaches the PC (status.json; the sender waits up to 30 s)' (Pc-Wait { param($s) $s.presence.phone.level -eq 60 } 55) ((Get-Status).presence.phone | ConvertTo-Json -Compress)
        Check 'S1 the PC tooltip carries it' ((Get-Status).tray.tooltip -match '60%') (Get-Status).tray.tooltip
        Check 'S1 no low-battery warning at 60 %' ((Log-Count 'low battery warning for') -eq $w0)
        A shell dumpsys battery set level 14 | Out-Null
        Check 'S1 level 14, not charging, reaches the PC' (Pc-Wait { param($s) $s.presence.phone.level -eq 14 -and $s.presence.phone.charging -eq $false } 55)
        Wait-Until { (Log-Count 'low battery warning for') -gt $w0 } 5 | Out-Null
        Check 'S1 exactly ONE low-battery warning at 14 %' ((Log-Count 'low battery warning for') -eq ($w0 + 1)) "count $w0 -> $(Log-Count 'low battery warning for')"
        A shell dumpsys battery set level 12 | Out-Null
        Check 'S1 level 12 reaches the PC' (Pc-Wait { param($s) $s.presence.phone.level -eq 12 } 55)
        Start-Sleep -Seconds 2
        Check 'S1 still ONE warning for the episode (14 % then 12 %)' ((Log-Count 'low battery warning for') -eq ($w0 + 1)) "count $(Log-Count 'low battery warning for')"
        A shell dumpsys battery set ac 1 | Out-Null; A shell dumpsys battery set level 80 | Out-Null
        Check 'S1 plugged in at 80 % reaches the PC (re-arms the nudge)' (Pc-Wait { param($s) $s.presence.phone.level -eq 80 -and $s.presence.phone.charging -eq $true } 55) ((Get-Status).presence.phone | ConvertTo-Json -Compress)
        A shell dumpsys battery unplug | Out-Null; A shell dumpsys battery set level 14 | Out-Null
        Check 'S1 unplugged at 14 % again reaches the PC' (Pc-Wait { param($s) $s.presence.phone.level -eq 14 -and $s.presence.phone.charging -eq $false } 55)
        Wait-Until { (Log-Count 'low battery warning for') -gt ($w0 + 1) } 5 | Out-Null
        Check 'S1 a second warning after charging re-armed it (two in all)' ((Log-Count 'low battery warning for') -eq ($w0 + 2)) "count $(Log-Count 'low battery warning for')"
        $ps = Presence-State
        Check 'S1 the phone''s battery receiver is registered while a session is ready (no polling)' ((Val $ps 'receiver') -eq 'true') $ps
        Shot '01-s1-phone-battery-on-pc-status'
        # PC -> phone
        Invoke-Kin --dev-set devBattery 9:unplugged | Out-Null
        Check 'S1 the PC''s fake battery (9 %, unplugged) reaches the phone (PRESENCE_STATE)' (Wait-Until { (Val (Presence-State) 'pcBattery') -eq '9/false' } 55 2000) (Presence-State)
        Check 'S1 and the phone''s persistent notification says it' (Wait-Until { (Kin-Notifs) -match '\b9%' } 75 3000) (((Kin-Notifs) -split "`n" | Select-String 'Connected to' | Select-Object -First 1))
        A shell cmd statusbar expand-notifications | Out-Null; Start-Sleep -Seconds 2; Shot '02-s1-pc-battery-in-phone-notification'; A shell cmd statusbar collapse | Out-Null
        Phone-Cmd 'PRESENCE_SET' '--ez show false' | Out-Null
        Check 'S1 phone "show" off: the PC''s battery leaves the notification' (Wait-Until { (Kin-Notifs) -notmatch '\b9%' } 75 3000)
        Phone-Cmd 'PRESENCE_SET' '--ez show true' | Out-Null
        Invoke-Kin --dev-set presenceShowPhone false | Out-Null
        Check 'S1 PC "show phone battery" off: status.json and the tooltip stop showing the level' (Pc-Wait { param($s) $null -eq $s.presence.phone.level -and $s.tray.tooltip -notmatch '%' } 15) ((Get-Status).tray.tooltip)
        Invoke-Kin --dev-set presenceShowPhone true | Out-Null
        Check 'S1 and back on' (Pc-Wait { param($s) $s.presence.phone.level -eq 14 } 15)
        Invoke-Kin --dev-set devBattery 77:charging | Out-Null
    }

    # ============================================================================================================================ S2 ring
    if (Enter-Scenario 'S2' 'ring my phone from the PC (silent emulator, real MediaPlayer)') {
        Go-Home
        Phone-Cmd 'RING_SET' '--ez allow true --ez silent false --ez loud true' | Out-Null
        $vol0 = Alarm-Volume
        Say "alarm volume before: $vol0"
        Check 'S2 the PC offers ringing (phone listed the ring capability)' ((View 'settings').presence.ring.can -eq $true) ((View 'settings').presence.ring | ConvertTo-Json -Compress)
        Invoke-Kin --ring | Out-Null; $t0 = Get-Date
        Check 'S2 Kin.exe --ring: the phone is ringing, audible' (Wait-Until { (Ring-State) -match 'state=ringing audible=true' } 10 400) (Ring-State)
        $rs = Ring-State
        Check 'S2 the alarm volume was saved, then raised above its value' ((Val $rs 'savedAlarmIndex') -eq "$vol0" -and (Alarm-Volume) -gt $vol0) "saved=$(Val $rs 'savedAlarmIndex') now=$(Alarm-Volume) before=$vol0"
        Check 'S2 the PC says ringing' (Pc-Wait { param($s) $s.ring.state -eq 'ringing' } 10)
        $kn = Kin-Notifs
        Check 'S2 the ring notification: channel kin_ring, category alarm, a Stop action, no full-screen-denied flag' ($kn -match 'channel=kin_ring' -and $kn -match 'category=alarm' -and $kn -match '"Stop"' -and $kn -notmatch 'FSI_REQUESTED_BUT_DENIED') (($kn -split "`n" | Select-String 'flags=' | Select-Object -First 1))
        Shot '03-s2-ring-heads-up-unlocked'
        # The real PC refuses a second press while a ring is going (ring.rs AlreadyRinging), so a phone-side restart (B2) and the phone's 3-per-60-s limit cannot be driven from the
        # PC: they are unit tests (RingMachineTest, ring[] vectors). What can be shown here: the second press changes nothing.
        while (((Get-Date) - $t0).TotalSeconds -lt 11) { Start-Sleep -Milliseconds 300 }
        Invoke-Kin --ring | Out-Null; $tLast = $t0
        Start-Sleep -Seconds 3
        $rs = Ring-State
        Check 'S2 a second Ring press while ringing is refused by the PC: no second ring.start (started stays 1)' ((Val $rs 'state') -eq 'ringing' -and $rs -match 'started 1,') $rs
        while (((Get-Date) - $t0).TotalSeconds -lt 25) { Start-Sleep -Milliseconds 300 }
        Check 'S2 25 s in: still ringing, the Stop notification is still there' ((Kin-Notifs) -match '"Stop"' -and (Ring-State) -match 'state=ringing') ''
        Check 'S2 the ring stops by itself 30 s after the start (timeout)' (Wait-Until { (Ring-State) -match 'state=idle' } 45 1000) (Ring-State)
        $secs = ((Get-Date) - $tLast).TotalSeconds
        $rs = Ring-State
        Check 'S2 auto-stop came 27 to 38 s after the start, reason timeout' ($secs -ge 27 -and $secs -le 38 -and (Val $rs 'lastReason') -eq 'timeout') ("{0:N1} s, {1}" -f $secs, $rs)
        Check 'S2 the alarm volume is back to its start value and nothing is saved' ((Alarm-Volume) -eq $vol0 -and (Val $rs 'savedAlarmIndex') -eq 'null') "now=$(Alarm-Volume) before=$vol0 | $rs"
        Check 'S2 the PC says the ring stopped' (Pc-Wait { param($s) $s.ring.state -eq 'stopped' } 10) ((Get-Status).ring | ConvertTo-Json -Compress)
        Check 'S2 the Stop notification is gone' ((Kin-Notifs) -notmatch 'channel=kin_ring')
        Start-Sleep -Seconds 11

        # --- locked screen, full-screen intent allowed (B1): the Stop screen over the lock screen, stopped with the real Stop button
        A shell cmd appops set $Pkg USE_FULL_SCREEN_INTENT allow 2>$null | Out-Null
        A shell input keyevent KEYCODE_SLEEP | Out-Null; Start-Sleep -Seconds 3
        Check 'S2 setup: the screen is off' (-not (Power-Awake))
        Invoke-Kin --ring | Out-Null; $tL1 = Get-Date
        Wait-Until { Power-Awake } 8 | Out-Null; Start-Sleep -Seconds 1
        $rs = Ring-State
        Check 'S2 B1: screen locked + full-screen intent allowed: the screen came on with RingActivity' ((Power-Awake) -and (Focus-Window) -match 'RingActivity' -and (Val $rs 'canFullScreen') -eq 'true') "$(Focus-Window) | $rs"
        Shot '05-s2-ring-stop-screen-locked-fsi-allowed'
        $tapped = Tap-Text @('Stop') 3
        Check 'S2 the real Stop button of the Stop screen stops the ring (reason user), volume restored' ($tapped -and (Wait-Until { (Ring-State) -match 'state=idle' } 8 500) -and (Val (Ring-State) 'lastReason') -eq 'user' -and (Alarm-Volume) -eq $vol0) "$(Ring-State) vol=$(Alarm-Volume)"
        Check 'S2 the PC says stopped' (Pc-Wait { param($s) $s.ring.state -eq 'stopped' } 10)
        A shell input keyevent KEYCODE_WAKEUP | Out-Null; A shell input keyevent KEYCODE_HOME | Out-Null

        # --- full-screen intent denied (Android 14+): ring keeps going, the notification says FSI_REQUESTED_BUT_DENIED, the screen stays off
        if ($script:Sdk -ge 34) {
            while (((Get-Date) - $tL1).TotalSeconds -lt 12) { Start-Sleep -Milliseconds 500 }
            A shell cmd appops set $Pkg USE_FULL_SCREEN_INTENT deny | Out-Null
            A shell input keyevent KEYCODE_SLEEP | Out-Null; Start-Sleep -Seconds 3
            Invoke-Kin --ring | Out-Null; $tL2 = Get-Date
            Check 'S2 FSI denied: the phone rings' (Wait-Until { (Ring-State) -match 'state=ringing' } 10 500) (Ring-State)
            Start-Sleep -Seconds 3
            $rs = Ring-State; $kn = Kin-Notifs
            Check 'S2 B1 (denied): canFullScreen false, the screen stays off (no Stop screen), the notification is posted with its Stop and FSI_REQUESTED_BUT_DENIED' ((Val $rs 'canFullScreen') -eq 'false' -and -not (Power-Awake) -and $kn -match '"Stop"' -and $kn -match 'FSI_REQUESTED_BUT_DENIED') "$(Val $rs 'canFullScreen') awake=$(Power-Awake)"
            A shell input keyevent KEYCODE_WAKEUP | Out-Null; Start-Sleep -Seconds 2
            Shot '06-s2-ring-after-wake-fsi-denied'
            Invoke-Kin --ring-stop | Out-Null
            Check 'S2 --ring-stop stops it with reason pc, volume restored' ((Wait-Until { (Ring-State) -match 'state=idle' } 8 500) -and (Val (Ring-State) 'lastReason') -eq 'pc' -and (Alarm-Volume) -eq $vol0) "$(Ring-State) vol=$(Alarm-Volume)"
            A shell cmd appops set $Pkg USE_FULL_SCREEN_INTENT allow | Out-Null
            A shell input keyevent KEYCODE_HOME | Out-Null
            while (((Get-Date) - $tL2).TotalSeconds -lt 12) { Start-Sleep -Milliseconds 500 }
        } else {
            while (((Get-Date) - $tL1).TotalSeconds -lt 12) { Start-Sleep -Milliseconds 500 }   # the PC allows one press per 10 s
            Invoke-Kin --ring | Out-Null; Start-Sleep -Seconds 3; Invoke-Kin --ring-stop | Out-Null
            Check 'S2 (API < 34) --ring-stop stops it with reason pc' ((Wait-Until { (Ring-State) -match 'state=idle' } 8 500) -and (Val (Ring-State) 'lastReason') -eq 'pc' -and (Alarm-Volume) -eq $vol0) (Ring-State)
            Start-Sleep -Seconds 11
        }

        # --- Do Not Disturb "total silence": nothing will be heard, the PC is told
        if ($script:Sdk -ge 30) {
            Set-Dnd $true; Start-Sleep -Seconds 1
            Invoke-Kin --ring | Out-Null
            Check 'S2 DND total silence: ringing with audible=false' (Wait-Until { (Ring-State) -match 'state=ringing audible=false' } 10 500) (Ring-State)
            Check 'S2 the PC is told (ring.audible false)' (Pc-Wait { param($s) $s.ring.state -eq 'ringing' -and $s.ring.audible -eq $false } 10) ((Get-Status).ring | ConvertTo-Json -Compress)
            Invoke-Kin --ring-stop | Out-Null; Wait-Until { (Ring-State) -match 'state=idle' } 8 500 | Out-Null
            Set-Dnd $false; Start-Sleep -Seconds 1
            Check 'S2 after total silence the alarm volume is what it was' ((Alarm-Volume) -eq $vol0) "now=$(Alarm-Volume) before=$vol0"
        } else {
            Note 'API 29: the emulator has no shell way to set Do Not Disturb total silence (cmd notification set_dnd is API 30+, and `settings put global zen_mode` does not change the interruption filter): audible=false is checked on API 37 only'
        }

        # --- the phone's switch
        Phone-Cmd 'RING_SET' '--ez allow false' 1500 | Out-Null
        Check 'S2 ring switch off on the phone: the PC no longer offers it and says why' (Wait-Until { $r = (View 'settings').presence.ring; $r.can -eq $false -and $r.reason -match 'turn it on' } 15 1000) ((View 'settings').presence.ring | ConvertTo-Json -Compress)
        $started0 = Val (Ring-State) 'started'
        $l0 = Log-Count "Ringing isn't available"
        Invoke-Kin --ring | Out-Null; Start-Sleep -Seconds 2
        Check 'S2 Kin.exe --ring with the switch off: the PC says why and nothing rings' ((Log-Count "Ringing isn't available") -gt $l0 -and (Ring-State) -match 'state=idle') (Ring-State)
        Phone-Cmd 'RING_SET' '--ez allow true' 1500 | Out-Null
        Check 'S2 switch back on: offered again' (Wait-Until { (View 'settings').presence.ring.can -eq $true } 15 1000)
    }

    # ============================================================================================================================ S3 pause
    if (Enter-Scenario 'S3' 'pause both ways, quiet arrivals') {
        Go-Home
        $nowMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        # ---- the PC pauses (the CLI is what the tray's "Pause Kin" runs; the tray menu itself cannot be clicked without OS input)
        $sig0 = Clip-Signature
        Invoke-Kin --pause 1h | Out-Null
        Check 'S3 PC: Kin.exe --pause 1h: status.json and the tooltip say paused' (Pc-Wait { param($s) $s.pause.on -eq $true -and $s.tray.tooltip -match 'paused' } 8) ((Get-Status).tray.tooltip)
        Check 'S3 PC pause: the phone shows "paused on the PC" within 2 s (PRESENCE_STATE and the notification)' ((Wait-Until { (Presence-State) -match 'peerPaused=(?!false|null)' } 6 700) -and (Wait-Until { (Kin-Notifs) -match 'paused on the PC' } 40 2000)) ((Presence-State))
        $toasts0 = Log-Count 'toast:'
        Phone-Cmd 'SEND_TEXT' "--es text 'ZQ quiet words'" 1200 | Out-Null
        Phone-Cmd 'SEND_LINK' '--es url https://example.com/zq-quiet' 1200 | Out-Null
        Phone-Cmd 'SEND_TEXT' '--es text ZQSensitiveWords --ez sensitive true' 1200 | Out-Null
        Check 'S3 PC paused: a text, a link and a sensitive text from the phone arrive quietly (counted)' (Pc-Wait { param($s) $s.pause.quietArrivals -ge 2 -and $s.pause.quietSensitive -ge 1 } 25) ((Get-Status).pause | ConvertTo-Json -Compress)
        Check 'S3 the log says: handled quietly (history only), sensitive one not kept' ((Log-Count 'arrived while paused, handled quietly') -ge 2 -and (Log-Count 'sensitive, arrived while paused \(not kept\)') -ge 1) ''
        Check 'S3 PC paused: no toast was requested for them' ((Log-Count 'toast:') -eq $toasts0) "toasts $toasts0 -> $(Log-Count 'toast:')"
        Check 'S3 PC paused: the PC clipboard is untouched (every format and size as before); no browser (--dev-no-open, and the log says quiet)' ((Clip-Signature) -eq $sig0 -and (Kin-Log) -notmatch 'link .*opened') ''
        $hist = (View 'history' | ConvertTo-Json -Depth 8 -Compress)
        Check 'S3 PC paused: the link is in the PC history (stored, quiet)' ($hist -match 'example\.com') ''
        Remove-Item (Join-Path $Data 'view-*.json') -ErrorAction SilentlyContinue
        # a manual send from the PC while the PC is paused still goes (manual sends are never blocked)
        Invoke-Kin --dev-set-clipboard 'ZQ pc words' | Out-Null; Start-Sleep -Milliseconds 700; Invoke-Kin --send-clipboard | Out-Null
        Check 'S3 PC paused: a manual send PC -> phone still arrives (phone clipboard has it)' (Wait-Until { (Phone-Clip).Text -eq 'ZQ pc words' } 15 1500) ''
        # resume
        Invoke-Kin --pause resume | Out-Null
        Check 'S3 PC resume: status.json not paused, the phone stops showing it' ((Pc-Wait { param($s) $s.pause.on -eq $false } 8) -and (Wait-Until { (Presence-State) -match 'peerPaused=(false|null)' } 8 700)) (Presence-State)
        $sig1 = Clip-Signature
        Phone-Cmd 'SEND_TEXT' "--es text 'ZQ after resume'" 1200 | Out-Null
        Check 'S3 PC resumed: a text from the phone reaches the PC clipboard again' (Wait-Until { (Clip-Signature) -ne $sig1 } 10 500) ''
        Check 'S3 and the quiet counter is where it was (it resets on resume)' ((Get-Status).pause.quietArrivals -eq 0) ((Get-Status).pause | ConvertTo-Json -Compress)
        # until resumed, until tomorrow
        Invoke-Kin --pause resumed | Out-Null
        Check 'S3 PC: until resumed (status.json untilResumed; the phone sees it)' ((Pc-Wait { param($s) $s.pause.on -eq $true -and $s.pause.untilResumed -eq $true } 8) -and (Wait-Until { (Presence-State) -match 'peerPaused=(?!false|null)' } 6 700)) ((Get-Status).pause | ConvertTo-Json -Compress)
        Invoke-Kin --pause tomorrow | Out-Null
        Pc-Wait { param($s) $s.pause.on -eq $true -and $null -ne $s.pause.untilMs } 8 | Out-Null
        $p = (Get-Status).pause; $nowMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        Check 'S3 PC: until tomorrow = the next 08:00, 2 to 26 hours ahead (the 08:00 rule itself is a unit test)' ($p.on -eq $true -and $p.untilMs -gt ($nowMs + 2 * 3600000 - 60000) -and $p.untilMs -le ($nowMs + 26 * 3600000)) ("{0:N2} h ahead" -f (($p.untilMs - $nowMs) / 3600000.0))
        Invoke-Kin --pause resume | Out-Null
        Check 'S3 PC: resumed again' (Pc-Wait { param($s) $s.pause.on -eq $false } 8)

        # ---- the phone pauses: the tile (quick settings, `cmd statusbar click-tile` = a tap on it), then the debug command
        $tile = "$Pkg/$Pkg.pause.PauseTile"
        A shell cmd statusbar add-tile $tile | Out-Null; Start-Sleep -Seconds 1
        A shell cmd statusbar click-tile $tile | Out-Null; Start-Sleep -Seconds 2
        Check 'S3 phone: tapping the Pause tile pauses for 1 h' ((Pause-State) -match 'paused=true') (Pause-State)
        Check 'S3 phone paused: the PC shows "phone paused" (status.json and the tooltip)' (Pc-Wait { param($s) $s.presence.phone.paused -eq $true -and $s.tray.tooltip -match 'phone paused' } 8) ((Get-Status).tray.tooltip)
        Shot '07-s3-phone-paused-home'
        $clip0 = (Phone-Clip).Text
        Invoke-Kin --dev-set-clipboard 'ZQ words to paused phone' | Out-Null; Start-Sleep -Milliseconds 700; Invoke-Kin --send-clipboard | Out-Null
        Start-Sleep -Seconds 4
        $ps = Pause-State
        Check 'S3 phone paused: a manual PC text arrives quietly (counted as received while paused)' ((Val $ps 'quiet') -ge 1) $ps
        Check 'S3 phone paused: no notification with the text and the phone clipboard is unchanged' ((Kin-Notifs) -notmatch 'ZQ words to paused' -and (Phone-Clip).Text -eq $clip0) ''
        A shell cmd statusbar click-tile $tile | Out-Null; Start-Sleep -Seconds 2
        Check 'S3 phone: tapping the tile again resumes' ((Pause-State) -match 'paused=false') (Pause-State)
        Check 'S3 phone resumed: the PC says so' (Pc-Wait { param($s) $s.presence.phone.paused -eq $false } 8)
        Invoke-Kin --dev-set-clipboard 'ZQ words after phone resume' | Out-Null; Start-Sleep -Milliseconds 700; Invoke-Kin --send-clipboard | Out-Null
        Check 'S3 phone resumed: a PC text reaches the phone clipboard again' (Wait-Until { (Phone-Clip).Text -eq 'ZQ words after phone resume' } 15 1500) ''
        A shell cmd statusbar remove-tile $tile | Out-Null
        Phone-Cmd 'PAUSE_SET' '--es mode untilresumed' | Out-Null
        Check 'S3 phone: until resumed, the PC sees it (pausedUntilResumed)' (Pc-Wait { param($s) $s.presence.phone.paused -eq $true -and $s.presence.phone.pausedUntilResumed -eq $true } 8) ((Get-Status).presence.phone | ConvertTo-Json -Compress)
        Phone-Cmd 'PAUSE_SET' '--es mode tomorrow' | Out-Null
        $ps = Pause-State; $nowMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $until = if ($ps -match 'untilMs=(\d+)') { [long]$Matches[1] } else { 0 }
        Check 'S3 phone: until tomorrow = 2 to 26 hours ahead' ($until -gt ($nowMs + 2 * 3600000 - 60000) -and $until -le ($nowMs + 26 * 3600000)) ("{0:N2} h" -f (($until - $nowMs) / 3600000.0))
        Phone-Cmd 'PAUSE_SET' "--es mode off --el untilMs $($nowMs - 5000)" | Out-Null
        Check 'S3 phone: a pause that ended in the past is no pause' ((Pause-State) -match 'paused=false') (Pause-State)
        Phone-Cmd 'PAUSE_SET' "--es mode off --el untilMs $($nowMs + 9 * 86400000)" | Out-Null
        Check 'S3 phone: an end 9 days ahead (clock set wrong) is treated as expired (8-day guard)' ((Pause-State) -match 'paused=false') (Pause-State)
        Phone-Cmd 'PAUSE_SET' '--es mode resume' | Out-Null
        Check 'S3 phone resumed; the PC says not paused' (Pc-Wait { param($s) $s.presence.phone.paused -eq $false } 8)
    }

    # ============================================================================================================================ S4 auto-sent screenshots
    if (Enter-Scenario 'S4' 'auto-sent screenshots and camera photos') {
        Go-Home
        $perm = if ($script:Sdk -ge 33) { 'android.permission.READ_MEDIA_IMAGES' } else { 'android.permission.READ_EXTERNAL_STORAGE' }
        A shell pm grant $Pkg $perm | Out-Null
        function SC([string]$S, [string]$Key) { if ($S -match ("\b{0}[: ]+(\d+)" -f $Key)) { [int]$Matches[1] } else { -1 } }
        function Insert-Shot([string]$Kind, [string]$Extra = '') { $l = Phone-One 'SHOTS_INSERT' "--es kind $Kind $Extra" 900; if ($l -match ' id=(\d+)') { [long]$Matches[1] } else { -1 } }
        function Shots-On { Phone-One 'SHOTS_SET' '--ez screenshots true' 1500 }
        function Shot-Files { @(Received-Files | Where-Object { $_.Name -match '^(Screenshot|Photo) ' }) }
        function Hold-Visible { (Notif-Dump) -match 'tag=kin-shot' }
        $clipBefore = Clip-Signature
        $preId = Insert-Shot 'screenshot'
        $r = Shots-On
        Check 'S4 turning it on: refused=false, watching, permission held, a baseline taken' ($r -match 'refused=false' -and $r -match 'watching=true' -and $r -match 'permission=true' -and $r -match 'baseline=\d+') $r
        Start-Sleep -Seconds 8
        Check 'S4 baseline: the screenshot that existed before turning it on is NOT sent' ((Shot-Files).Count -eq 0 -and (SC (Shots-State) 'sent') -eq 0) (Shots-State)
        # a new one: hold 5 s, then sent
        $f0 = (Shot-Files).Count
        $id1 = Insert-Shot 'screenshot'; $tIns = Get-Date
        Check 'S4 a new screenshot: the hold notification (Sending screenshot to DESK-PC in 5 s, Cancel) appears at once' (Wait-Until { Hold-Visible } 3 300) ''
        $hn = (Notif-Dump); $hi = $hn.IndexOf('tag=kin-shot'); $holdText = if ($hi -ge 0) { $hn.Substring($hi, [Math]::Min(2500, $hn.Length - $hi)) } else { '' }
        Check 'S4 the hold says "Sending screenshot to DESK-PC" and has a Cancel action' ($holdText -match 'Sending screenshot to' -and $holdText -match '"Cancel"') ''
        A shell cmd statusbar expand-notifications | Out-Null; Start-Sleep -Milliseconds 800; Shot '08-s4-screenshot-hold-notification'; A shell cmd statusbar collapse | Out-Null
        $early = (Shot-Files).Count
        Wait-Until { (Shot-Files).Count -gt $f0 } 20 300 | Out-Null
        $el = ((Get-Date) - $tIns).TotalSeconds
        Check 'S4 not sent during the hold (nothing on the PC at ~2.5 s), sent after it (5 to 14 s)' ($early -eq $f0 -and (Shot-Files).Count -eq ($f0 + 1) -and $el -ge 4.5 -and $el -le 14) ("{0:N1} s" -f $el)
        $newest = Shot-Files | Sort-Object LastWriteTime | Select-Object -Last 1
        Check 'S4 the file on the PC has a generic name (Screenshot <date time>.png), not the phone''s file name' ($newest.Name -match '^Screenshot \d{4}-\d{2}-\d{2} \d{2}\.\d{2}\.\d{2}( \(\d+\))?\.png$') $newest.Name
        $names = Clip-Names
        Check 'S4 the PC clipboard holds the image (CF_DIBV5) with the three sensitive formats' (($names -contains 'CF_DIBV5') -and ($names -contains 'ExcludeClipboardContentFromMonitorProcessing') -and ($names -contains 'CanIncludeInClipboardHistory') -and ($names -contains 'CanUploadToCloudClipboard')) ($names -join ',')
        Check 'S4 the PC toast says Screenshot from the phone (log)' ((Log-Count 'toast: Screenshot from') -ge 1) ''
        Check 'S4 the phone counted it (sent 1, seen 1) and the baseline moved to the new id' ((SC (Shots-State) 'sent') -eq 1 -and (Shots-State) -match "baseline=$id1") (Shots-State)
        # a real system screenshot (the emulator's own, owner com.android.systemui)
        $f1 = (Shot-Files).Count
        A shell input keyevent KEYCODE_SYSRQ | Out-Null
        $got = Wait-Until { (Shot-Files).Count -gt $f1 } 25 500
        if ($script:Sdk -ge 33) { Check 'S4 a real system screenshot (Pictures/Screenshots, owner com.android.systemui) is sent too' $got (Shots-State) } else { if ($got) { Say 'a real system screenshot was sent (API 29)' } else { Note 'API 29: the emulator''s KEYCODE_SYSRQ screenshot did not appear as a file (not a product finding)' } }
        # cancel
        $f2 = (Shot-Files).Count; $c0 = SC (Shots-State) 'cancelled'
        $id2 = Insert-Shot 'screenshot'
        Wait-Until { Hold-Visible } 3 300 | Out-Null
        Phone-Cmd 'SHOTS_CANCEL' "--el id $id2" 500 | Out-Null
        Start-Sleep -Seconds 9
        $st = Shots-State
        Check 'S4 Cancel within 5 s: not sent, counted as cancelled, the baseline moved past it' ((Shot-Files).Count -eq $f2 -and (SC $st 'cancelled') -eq ($c0 + 1) -and $st -match 'baseline=(\d+)' -and [long]$Matches[1] -ge $id2) $st
        # skipped kinds
        $o0 = SC (Shots-State) 'other'; $w0 = SC (Shots-State) 'own'; $x0 = SC (Shots-State) 'off'
        Insert-Shot 'other' | Out-Null; Insert-Shot 'own' | Out-Null; Insert-Shot 'camera' | Out-Null
        Start-Sleep -Seconds 8
        $st = Shots-State
        Check 'S4 other folders, Kin''s own folder and a camera photo (camera switch off) are never sent (counted other, own, off)' ((Shot-Files).Count -eq $f2 -and (SC $st 'other') -eq ($o0 + 1) -and (SC $st 'own') -eq ($w0 + 1) -and (SC $st 'off') -eq ($x0 + 1)) $st
        $sx = SC (Shots-State) 'stale'
        Insert-Shot 'screenshot' '--ei ageSec 60' | Out-Null
        Start-Sleep -Seconds 3
        Check 'S4 an image older than 15 s when it appears is stale: not sent' ((Shot-Files).Count -eq $f2 -and (SC (Shots-State) 'stale') -eq ($sx + 1)) (Shots-State)
        # camera on
        Phone-Cmd 'SHOTS_SET' '--ez camera true' 1500 | Out-Null
        $clipPre = Clip-Signature
        Insert-Shot 'camera' | Out-Null
        Check 'S4 camera on: a camera photo is saved on the PC (Photo <date>.png)' (Wait-Until { @(Shot-Files | Where-Object { $_.Name -match '^Photo ' }).Count -ge 1 } 20 500) ((Shot-Files | ForEach-Object { $_.Name }) -join ',')
        Start-Sleep -Seconds 1
        Check 'S4 and it NEVER goes to the PC clipboard (every format and size as it was)' ((Clip-Signature) -eq $clipPre) ''
        Check 'S4 the PC toast says Photo from the phone (log)' ((Log-Count 'toast: Photo from') -ge 1) ''
        Phone-Cmd 'SHOTS_SET' '--ez camera false' 800 | Out-Null
        # B3: no visible hold, no sending
        $f3 = (Shot-Files).Count
        if ($script:Sdk -ge 33) {
            A shell pm revoke $Pkg android.permission.POST_NOTIFICATIONS | Out-Null; Start-Sleep -Seconds 3
            A shell am start -n "$Pkg/.MainActivity" | Out-Null; Start-Sleep -Seconds 3
            $st = Shots-State
            Check 'S4 B3: notifications revoked -> the phone says notifications=false (hold impossible)' ($st -match 'notifications=false') $st
            $nh0 = SC $st 'no_hold'
            Insert-Shot 'screenshot' | Out-Null
            Start-Sleep -Seconds 9
            $st = Shots-State
            Check 'S4 B3: with no visible hold the screenshot is NOT sent (the observer is not even registered: watching=false; or skipped no_hold)' ((Shot-Files).Count -eq $f3 -and ($st -match 'watching=false' -or (SC $st 'no_hold') -gt $nh0)) $st
            A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null; Start-Sleep -Seconds 2
            A shell am start -n "$Pkg/.MainActivity" | Out-Null; Start-Sleep -Seconds 3
            Phone-Cmd 'SHOTS_SET' '--ez screenshots true' 1500 | Out-Null
            Check 'S4 B3: notifications back: watching again' ((Shots-State) -match 'notifications=true' -and (Shots-State) -match 'watching=true') (Shots-State)
        } else {
            Note 'API 29: no POST_NOTIFICATIONS runtime permission; B3 (no visible hold) has no shell mechanism here: unit-tested only (ShotWatcherCoreTest) and on the TESTING.md real-phone list'
        }
        # paused: skipped and the baseline moves, so nothing comes back after the pause
        $f4 = (Shot-Files).Count; $p0 = SC (Shots-State) 'paused'
        Phone-Cmd 'PAUSE_SET' '--es mode 1h' | Out-Null
        Insert-Shot 'screenshot' | Out-Null
        Start-Sleep -Seconds 8
        Check 'S4 phone paused: a screenshot is not sent (counted paused)' ((Shot-Files).Count -eq $f4 -and (SC (Shots-State) 'paused') -eq ($p0 + 1)) (Shots-State)
        Phone-Cmd 'PAUSE_SET' '--es mode resume' | Out-Null
        Start-Sleep -Seconds 8
        Check 'S4 after the resume the one taken while paused does not appear' ((Shot-Files).Count -eq $f4) ''
        # the PC is away: dropped, never queued
        $f5 = (Shot-Files).Count; $of0 = SC (Shots-State) 'offline'
        Wifi $false
        Check 'S4 setup: the phone''s link is down (Wi-Fi off; the PC itself only notices after its ping timeout)' (Wait-Until { (Phone-One 'STATE' '' 700) -notmatch 'link=Connected' } 40 1000) (Phone-One 'STATE' '' 700)
        Insert-Shot 'screenshot' | Out-Null
        Start-Sleep -Seconds 9
        Check 'S4 the PC away at the end of the hold: dropped and counted (offline), not queued' ((SC (Shots-State) 'offline') -eq ($of0 + 1) -and (Shots-State) -notmatch 'queued') (Shots-State)
        Wifi $true
        Check 'S4 setup: the phone reconnects' (Wait-Until { (Phone-One 'STATE' '' 700) -match 'link=Connected' } 90 2000) (Phone-One 'STATE' '' 700)
        Start-Sleep -Seconds 10
        Check 'S4 after reconnecting the dropped screenshot is NOT sent later' ((Shot-Files).Count -eq $f5) ''
        # permission withdrawn
        A shell pm revoke $Pkg $perm | Out-Null; Start-Sleep -Seconds 3
        A shell am start -n "$Pkg/.MainActivity" | Out-Null; Start-Sleep -Seconds 4
        $st = Shots-State
        Check 'S4 photo permission revoked: permission=false, the observer is not registered (watching=false)' ($st -match 'permission=false' -and $st -match 'watching=false') $st
        Go-Home
        A shell input swipe 540 1700 540 600 300 | Out-Null; Start-Sleep -Seconds 1
        Check 'S4 and Home says it needs access to all photos' ((Ui-Dump) -match 'Needs access to all photos') ''
        Shot '09-s4-needs-access-to-photos'
        Phone-Cmd 'SHOTS_SET' '--ez screenshots false --ez camera false' 800 | Out-Null
    }

    # ============================================================================================================================ S5 unlinking
    if (Enter-Scenario 'S5' 'unlinking: from the PC (in flight, offline phone), from the phone') {
        function Phone-Transfers-Active { @(Phone-Cmd 'TRANSFERS' '' 1500 | Where-Object { $_ -match 'CMD TRANSFER \S+ up (active|queued|running)' }).Count }
        $PcId = (Get-Status).devices[0].deviceId
        # --- (a) an upload in flight, unlink from the PC
        $sizeMb = 500
        A shell "run-as $Pkg sh -c 'mkdir -p files/debug-share && head -c $($sizeMb * 1048576) /dev/urandom > files/debug-share/zqbig.bin'" | Out-Null
        A shell am start -n "$Pkg/.debug.DebugShareActivity" --es files zqbig.bin | Out-Null
        $flying = Pc-Wait { param($s) @($s.transfers.active).Count -gt 0 } 40
        Check 'S5a an upload of 500 MB from the phone is in flight on the PC' $flying ((Get-Status).transfers.active | ConvertTo-Json -Compress -Depth 3)
        $conn0 = Log-Count 'phone connected:'
        Remove-Item (Join-Path $Data 'dev-set-result.json') -ErrorAction SilentlyContinue
        Invoke-Kin --dev-set unlinkDevice $PcId | Out-Null
        $tUn = Get-Date
        Wait-Until { Test-Path (Join-Path $Data 'dev-set-result.json') } 10 | Out-Null
        $res = (Read-Shared (Join-Path $Data 'dev-set-result.json')) | ConvertFrom-Json
        Check 'S5a the PC removed the device' ($res.ok -eq $true -and $res.message -match 'removed: true') ($res | ConvertTo-Json -Compress)
        Check 'S5a the upload stops: no active transfer on the PC' (Pc-Wait { param($s) @($s.transfers.active).Count -eq 0 } 10) ((Get-Status).transfers.active | ConvertTo-Json -Compress -Depth 3)
        Start-Sleep -Seconds 3
        $left = @(Get-ChildItem $Received -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -match 'zqbig' })
        Check 'S5a no part of the file and no file is left on the PC' ($left.Count -eq 0) (($left | ForEach-Object { "$($_.Name) $($_.Length)" }) -join ',')
        $st = Phone-One 'STATE' '' 800
        Check 'S5a the phone found out (no longer paired) and has no transfer still running' ($st -match 'paired=null' -and (Phone-Transfers-Active) -eq 0) $st
        Start-Sleep -Seconds 15
        Check 'S5a no reconnect: the PC has no device and is not connected, no new connection in the log' ((Get-Status).state -ne 'phone_connected' -and @((Get-Status).devices).Count -eq 0 -and (Log-Count 'phone connected:') -eq $conn0) ''
        Shot '10-s5-phone-after-pc-unlink'
        Phone-Cmd 'UNPAIR' '' 500 | Out-Null

        # --- (b) the lost phone: the PC unlinks while the phone is OFFLINE; queued items fail as peer_changed; the phone finds out by a 401 when it comes back
        Check 'S5b setup: pair again' (Pair-Phone)
        $PcId = (Get-Status).devices[0].deviceId
        Wifi $false
        Check 'S5b setup: the phone''s link is down (the PC only notices after its ping timeout, so it may still think the phone is connected)' (Wait-Until { (Phone-One 'STATE' '' 700) -notmatch 'link=Connected' } 40 1000)
        $qf = Join-Path $Work 'zq-queued.txt'; 'queued for an offline phone' | Set-Content $qf -Encoding ASCII
        Invoke-Kin --send $qf | Out-Null
        Check 'S5b a send to the offline phone is queued on the PC' (Pc-Wait { param($s) $s.queue.count -ge 1 } 15) ((Get-Status).queue | ConvertTo-Json -Compress -Depth 4)
        Invoke-Kin --dev-set unlinkDevice $PcId | Out-Null
        Check 'S5b unlinking the offline phone: the queue is empty and the item failed as peer_changed' ((Pc-Wait { param($s) $s.queue.count -eq 0 } 10) -and ((View 'history' | ConvertTo-Json -Depth 8 -Compress) -match 'peer_changed')) ((Get-Status).queue | ConvertTo-Json -Compress -Depth 4)
        Remove-Item (Join-Path $Data 'view-*.json') -ErrorAction SilentlyContinue
        $conn0 = Log-Count 'phone connected:'
        Wifi $true
        Start-Sleep -Seconds 40
        $st = Phone-One 'STATE' '' 800
        Check 'S5b the phone came back with its old key: the PC refused it (no connection) and the phone stopped (not an endless retry)' ((Log-Count 'phone connected:') -eq $conn0 -and (Get-Status).state -ne 'phone_connected') $st
        Check 'S5b the phone knows it was unlinked: its state is PairAgain (a 401), and it stopped retrying' (Wait-Until { (Phone-One 'STATE' '' 700) -match 'link=PairAgain' } 20 2000) $st
        A shell am start -n "$Pkg/.MainActivity" | Out-Null; Start-Sleep -Seconds 2
        Shot '11-s5-phone-pair-again'
        Phone-Cmd 'UNPAIR' '' 500 | Out-Null

        # --- (c) unlink from the phone, through the screen
        Check 'S5c setup: pair again' (Pair-Phone)
        Go-Home
        $tapped = $false
        for ($i = 0; $i -lt 6 -and -not $tapped; $i++) {
            $tapped = Tap-Text @('Unlink') 1
            if (-not $tapped) { A shell input swipe 540 1900 540 500 300 | Out-Null; Start-Sleep -Milliseconds 700 }
        }
        Start-Sleep -Seconds 1
        $ui = Ui-Dump
        Check 'S5c the confirmation says what happens (Unlink from DESK-PC? ... you can pair again with a new code)' ($tapped -and $ui -match 'Unlink from' -and $ui -match 'pair again') (([regex]::Matches($ui, 'text="([^"]*(Unlink|pair again)[^"]*)"') | ForEach-Object { $_.Groups[1].Value }) -join ' | ')
        Shot '12-s5-phone-unlink-confirmation'
        $x = Ui-Dump
        $btns = [regex]::Matches($x, 'text="Unlink"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"')
        if ($btns.Count -gt 0) { $b = $btns[$btns.Count - 1]; A shell input tap ([int](([int]$b.Groups[1].Value + [int]$b.Groups[3].Value) / 2)) ([int](([int]$b.Groups[2].Value + [int]$b.Groups[4].Value) / 2)) | Out-Null }
        $gone = Pc-Wait { param($s) @($s.devices).Count -eq 0 } 15
        if (-not $gone) { Note 'S5c the Unlink confirm tap did not land; used the debug UNPAIR instead'; Phone-Cmd 'UNPAIR' '' 800 | Out-Null; $gone = Pc-Wait { param($s) @($s.devices).Count -eq 0 } 15 }
        Check 'S5c unlinking on the phone removes the phone from the PC (device list empty)' $gone ((Get-Status).state)
        Check 'S5c the phone is not paired any more' ((Phone-One 'STATE' '' 800) -match 'paired=null') ''
    }

    # ============================================================================================================================ S6 old peers, free-text ring reason
    if (Enter-Scenario 'S6' 'old peers and a hostile ring reason (fake phone)') {
        Invoke-Kin --pause resume | Out-Null
        Remove-Item (Join-Path $Data 'pair-url.txt') -ErrorAction SilentlyContinue
        Invoke-Kin --dev-start-pairing | Out-Null
        Wait-Until { Test-Path (Join-Path $Data 'pair-url.txt') } 10 | Out-Null
        $url = (Get-Content (Join-Path $Data 'pair-url.txt') -Raw).Trim()
        $ident = Join-Path $Work 'fake-identity.json'
        $fp = Start-Fake @($url, '--host', '127.0.0.1', '--name', 'ZQ Old Phone', '--identity-file', $ident, '--seconds', '14', '--old-phone') 'fake-old.txt'
        Wait-Until { (Get-Status).state -eq 'phone_connected' } 20 | Out-Null
        Start-Sleep -Seconds 4
        $t = Fake-Text 'fake-old.txt'
        Check 'S6 setup: the old phone paired and connected' ((Get-Status).state -eq 'phone_connected') ("$($t.Substring(0, [Math]::Min(300, $t.Length))) | " + (Read-Shared (Join-Path $Work 'fake-old.txt.err')))
        Check 'S6 a phone without presence/ring in its hello is sent no presence message' ($t -notmatch 'received presence') $t
        $r = (View 'settings').presence.ring
        Check 'S6 "Ring my phone" is disabled for it with the reason text' ($r.can -eq $false -and $r.reason -match 'Kin 0\.2\.0') ($r | ConvertTo-Json -Compress)
        Remove-Item (Join-Path $Data 'view-*.json') -ErrorAction SilentlyContinue
        Stop-OwnProcess $fp $Fake
        Wait-Until { (Get-Status).state -ne 'phone_connected' } 20 | Out-Null
        $fp = Start-Fake @($url, '--host', '127.0.0.1', '--name', 'ZQ Old Phone', '--identity-file', $ident, '--skip-pairing', '--seconds', '25', '--presence', '--ring', '--ring-status', 'ringing', '--ring-status', 'stopped,ZQSecretReason') 'fake-ring.txt'
        Wait-Until { (Get-Status).state -eq 'phone_connected' } 20 | Out-Null
        Start-Sleep -Seconds 2
        Invoke-Kin --ring | Out-Null
        Check 'S6 a ring.status with a free-text reason: the PC ends the ring' (Pc-Wait { param($s) $s.ring.state -eq 'stopped' } 25) ((Get-Status).ring | ConvertTo-Json -Compress)
        Check 'S6 the free-text reason is in no PC log line' ((Kin-Log) -notmatch 'ZQSecretReason') ''
        Stop-OwnProcess $fp $Fake
    }

    # ============================================================================================================================ S7 privacy
    if (Want 'S7') {
        Remove-Item (Join-Path $Data 'view-*.json') -ErrorAction SilentlyContinue
        Flush-PhoneLog
        # a unique-enough pattern for the levels this run used (60, 14, 13, 12, 25, 80, 9) as a battery value in a log line
        $levelRx = '(?i)\b(60|14|12|80|9)\s?%|\b(level|battery|pcBattery)["=: ]+(60|14|12|80|9)\b'
        $pcLog = Kin-Log
        Check 'S7 the PC log has no battery level (the phone''s or the fake PC''s)' ($pcLog -notmatch $levelRx) (([regex]::Matches($pcLog, $levelRx) | Select-Object -First 3 | ForEach-Object { $_.Value }) -join ',')
        $leakPc = @($TestStrings | Where-Object { $pcLog.Contains($_) })
        Check 'S7 the PC log has no screenshot name, sensitive text or ring reason' ($leakPc.Count -eq 0) ($leakPc -join ',')
        $phoneLogText = if (Test-Path $PhoneLog) { (Get-Content $PhoneLog) | Where-Object { $_ -notmatch 'CMD ' } } else { @() }
        $phoneJoined = ($phoneLogText -join "`n")
        $badLines = @($phoneLogText | Where-Object { $_ -match $levelRx })
        Check 'S7 the phone''s Kin log (every KinRelay line of the run except the debug CMD echo lines) has no battery level' ($badLines.Count -eq 0 -and $phoneJoined.Length -gt 1000) ("$($badLines.Count) lines; first: " + (($badLines | Select-Object -First 1) -replace '^\S+\s+\S+\s+\d+\s+\d+\s+\w\s+', ''))
        $leakPhone = @(($TestStrings + @('ZQ quiet words', 'ZQ pc words', 'ZQ after resume', 'ZQ words', 'Pictures/Screenshots', 'DCIM/Camera', 'systemui', 'Screenshot_2026')) | Where-Object { $phoneJoined.Contains($_) })
        Check 'S7 and none of: a screenshot or file name, a path, sensitive or sent text, a ring reason' ($leakPhone.Count -eq 0) ($leakPhone -join ',')
        $phoneAll = if (Test-Path $PhoneLog) { Get-Content $PhoneLog -Raw } else { '' }
        Check 'S7 the counts of the scenarios that ran exist as numbers only (the debug CMD echo lines: the app itself logs no count line outside them)' ((-not (Want 'S1') -and -not (Want 'S2') -or ($phoneAll -match 'presence: sent \d+' -and $phoneAll -match 'ring: started \d+')) -and (-not (Want 'S4') -or $phoneAll -match 'shots: seen \d+')) ''
        $tar = Join-Path $Work 'phone-data.tar'
        cmd /c "`"$Adb`" -s $Serial exec-out run-as $Pkg tar cf - . > `"$tar`" 2>nul"
        $tarText = ''
        if (Test-Path $tar) { $tarText = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($tar)) }
        # (An ordinary text the user sends or receives is in the phone's history by design (v0.1.x), so 'ZQ quiet words' etc. are expected in the database: not checked here. The sensitive
        # text, the screenshot names and the ring reason must not be.)
        $leakDb = @($TestStrings | Where-Object { $tarText.Contains($_) })
        Check 'S7 the phone''s app data (one tar of databases, DataStore, files, caches) has no sensitive text, no screenshot name, no ring reason' ($tarText.Length -gt 100000 -and $leakDb.Count -eq 0) ("tar $($tarText.Length) bytes; $($leakDb -join ',')")
        Say ("(ordinary sent/received texts in the phone's history, as designed: " + (@('ZQ quiet words', 'ZQ pc words', 'ZQ after resume') | Where-Object { $tarText.Contains($_) }).Count + ' of 3 found)')
        Remove-Item $tar -Force -ErrorAction SilentlyContinue
        $files = @(Get-ChildItem $Data -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch '\\received\\' })
        $hitsPc = @($files | Where-Object { $c = Read-Shared $_.FullName; ($TestStrings | Where-Object { $c.Contains($_) }).Count -gt 0 })
        Check 'S7 no file of the PC data folder holds a test string' ($hitsPc.Count -eq 0) (($hitsPc | ForEach-Object { $_.Name }) -join ',')
        $hitsLevel = @($files | Where-Object { $_.Name -ne 'status.json' -and $_.Extension -in '.log', '.json', '.txt' -and ((Read-Shared $_.FullName) -match $levelRx) })
        Check 'S7 no log or settings file of the PC data folder (status.json skipped: it holds levels by design) has a battery level' ($hitsLevel.Count -eq 0) (($hitsLevel | ForEach-Object { $_.Name }) -join ',')
        $db = @($files | Where-Object { $_.Extension -in '.db', '.sqlite', '.sqlite3' })
        Check 'S7 the PC database holds no battery level, screenshot name or ring reason' (@($db | Where-Object { $c = Read-Shared $_.FullName; $c -match 'ZQSecretReason' -or $c -match 'kin-debug-' }).Count -eq 0) ($db | ForEach-Object { $_.Name })
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
        try { Set-Dnd $false } catch {}
        try { A shell cmd appops reset $Pkg | Out-Null } catch {}
        try { Phone-Cmd 'RING_STOP_LOCAL' '' 300 | Out-Null } catch {}
        try { Phone-Cmd 'RING_SET' '--ez allow true --ez silent false' 300 | Out-Null } catch {}
        try { Phone-Cmd 'SHOTS_SET' '--ez screenshots false --ez camera false' 300 | Out-Null } catch {}
        try { Phone-Cmd 'PAUSE_SET' '--es mode resume' 300 | Out-Null } catch {}
    }
    if ($script:Backup) { try { Restore-Clipboard $script:Backup; $script:Backup = $null; Say 'the PC clipboard was restored' } catch { Say "restoring the clipboard failed: $_" } }
    if ($started.Kin -and -not $KeepRunning) { Stop-DevKin 15 }
}
# ---------------------------------------------------------------------------------------------------------------------------------- S8 guard
try {
    $installedAfter = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
    Check 'S8 the installed Kin was not started, stopped or restarted (same PIDs)' (($installedBefore -join ',') -eq ($installedAfter -join ',')) "$($installedBefore -join ',') -> $($installedAfter -join ',')"
    if (-not $KeepRunning) { Check 'S8 no Kin process of this run is left' (@(Get-DevKinProcesses).Count -eq 0) }
    Check 'S8 every fake phone this run started is gone' (@($script:FakeProcs | Where-Object { $_ -and -not $_.HasExited }).Count -eq 0)
    Check 'S8 the PC clipboard was restored' ($null -eq $script:Backup)
} catch { Say "ERROR in the guard checks: $_"; $script:Failures.Add("exception: $_") }
if (-not $KeepRunning -and $started.Emulator) { Invoke-EmuScript 'stop.ps1' @('-All') 120 | Out-Null }
$env:COMPUTERNAME = $script:OrigComputerName
if (-not $KeepRunning) { Remove-DevDataDir $Data }
Exit-KinLock
Write-Host ''
$script:Results.GetEnumerator() | ForEach-Object { Write-Host ("{0,-5} {1}" -f $_.Value, $_.Key) }
if ($script:Notes.Count -gt 0) { Write-Host "`nNotes:"; $script:Notes | ForEach-Object { Write-Host " - $_" } }
if ($script:Pending.Count -gt 0) { Write-Host "`nPENDING (not run):"; $script:Pending | ForEach-Object { Write-Host " - $_" } }
Say "work folder (logs of the phone): $Work"
if ($script:Failures.Count -gt 0) { Write-Host "`nFAILED: $($script:Failures -join '; ')"; exit 1 }
if ($script:Pending.Count -gt 0) { Write-Host "`nINCOMPLETE: $($script:Pending.Count) scenario(s) pending."; exit 2 }
Write-Host "`nOK: all $($script:Results.Count) checks passed on $Avd."
exit 0
