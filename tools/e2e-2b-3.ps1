# Milestone 2B.3 integration (media controls in both directions): the real debug APK on an emulator against the REAL desktop build as a test Kin beside the
# user's installed one (own --data-dir, loopback only, no window, --dev-toast-quiet). ASCII only. SILENT: nothing plays, no sound anywhere.
#
#   tools\e2e-2b-3.ps1                          S1 .. S4 and S6 on kin_api37
#   tools\e2e-2b-3.ps1 -Only S1,S2              some of them (setup, pairing, S4 and S6 always run)
#   tools\e2e-2b-3.ps1 -Avd kin_api29 -Only S1,S2   the short Android 10 pass
#
# S1 phone -> PC: a silent test MediaSession of Kin's own on the phone (DebugMediaSession, no audio), share on (phone) + show on (PC): state, art, controls reach the
#    PC; the test Kin's flyout session (SystemMediaTransportControls, read back through GSMTC by its own AUMID only) shows it as Paused (the dev guard); the flyout's
#    buttons and seek (media_test_session control <dev Kin's AUMID>) reach the phone as media.cmd and its test session; share off -> one clear; access revoked
#    while sharing -> the PC's card is cleared (review fix 2)
# S2 PC -> phone: a SILENT test session of the PC's own (examples\media_test_session.exe serve: hidden window, Paused, no audio); the test Kin reads ONLY it
#    (--dev-media-aumid); the phone shows kin-pc-media (MediaStyle notification, the carousel on API 37: screenshot); the phone's buttons -> media.cmd -> the PC test
#    session's own handler; the PC volume through the FAKE endpoint (starts at 50, 2 % steps), the slider's final value, the volume shown with no PC session
#    (fix 4); a fast reconnect clears the old card (fix 8)
# S3 gates: PC show off / PC share off / phone share off / phone show off -> nothing sent; a peer without `media` or with no status (fake_phone) gets nothing;
#    the rate limits (200 phone states, 120 PC changes, a flood of commands)
# S4 privacy: the test titles, artists, app names, AUMIDs of this run are absent from the test Kin's data directory, log and status.json, and from the phone's
#    Kin log and app data
# S6 guard: the REAL master volume is read (read only) before and after: unchanged; the installed Kin's PIDs unchanged; the user's real media sessions are not
#    readable by this test Kin by construction (mode test, --dev-media-aumid names only the test session), and no process of this run is left
#
# (S0 is tools\check-2b3-desktop.ps1, S5 is tools\e2e-2b-2.ps1 -Only S1 and tools\e2e-1-3.ps1 -Only S1,S4: run them as their own scripts.)
# It never touches the installed Kin (process, data, AUMID app.kinrelay.desktop, Run key, SendTo, Downloads, firewall), NEVER sends a media command to a session it
# did not create, NEVER writes the real master volume or mute, saves nothing of the clipboard because it never touches it, injects no OS input on the PC and
# refuses to run while a game or an anti-cheat is running. The emulator is started headless with no audio.
param(
    [string]$Avd = 'kin_api37',
    [string[]]$Only = @(),
    [string]$Kin = (Join-Path $PSScriptRoot '..\desktop\src-tauri\target\release\Kin.exe'),
    [string]$Apk = (Join-Path $PSScriptRoot '..\android\app\build\outputs\apk\debug\app-debug.apk'),
    [int]$Port = 49371,
    [string]$ScreenshotDir = 'docs\results\phase-2b-3-integration-screens',
    [switch]$KeepRunning,
    [string]$Work = ''
)
. (Join-Path $PSScriptRoot 'kin-lock.ps1'); Enter-KinLock 'e2e-2b-3'
. (Join-Path $PSScriptRoot 'kin-dev.ps1')
if (-not $Work) { $Work = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-e2e-2b-3-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
$script:OrigComputerName = $env:COMPUTERNAME
$env:COMPUTERNAME = 'DESK-PC'
$ErrorActionPreference = 'Continue'
foreach ($name in 'ANDROID_HOME', 'JAVA_HOME') {
    if (-not (Get-Item "env:$name" -ErrorAction SilentlyContinue)) { $v = [Environment]::GetEnvironmentVariable($name, 'User'); if ($v) { Set-Item "env:$name" $v } }
}
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $cargoBin })) { $env:Path = "$cargoBin;$env:Path" }
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$Emu = Join-Path $Repo 'tools\emulator'
$Adb = Join-Path $(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }) 'platform-tools\adb.exe'
$Target = Join-Path $Repo 'desktop\src-tauri\target\debug'
$Session = Join-Path $Target 'examples\media_test_session.exe'
$Fake = Join-Path $Target 'examples\fake_phone.exe'
$Pkg = 'app.kinrelay.android'
$Service = "$Pkg/$Pkg.notifmirror.MirrorListenerService"
$Serial = 'emulator-5554'
$Data = Join-Path $Work 'data'
$PhoneLog = Join-Path $Work 'phone-kin-log.txt'
$SystemHits = Join-Path $Work 'phone-system-log-hits.txt'
$Events = Join-Path $Work 'session-events.txt'
$ScreenDir = if ([IO.Path]::IsPathRooted($ScreenshotDir)) { $ScreenshotDir } else { Join-Path $Repo $ScreenshotDir }
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Results = [ordered]@{}
$script:Notes = New-Object System.Collections.Generic.List[string]
$started = @{ Emulator = $false; Kin = $false }
$script:KinAumid = $null; $script:TestAumid = $null; $script:Nonce = $null; $script:Sess = $null; $script:Sdk = 0; $script:FakeProcs = @()
# What must never be found in a log or a data file. Long enough not to turn up by chance in binary data.
$TestStrings = @('ZQPhoneSong', 'ZQPhoneTwo', 'ZQPhoneArtist', 'ZQPhoneArtist2', 'ZQGateSong', 'ZQBurstSong', 'ZQPcSong', 'ZQPcTwo', 'ZQPcArtist', 'ZQPcAlbum', 'ZQPcBurst', 'app.kinrelay.dev.mediatest', 'mediatest-', 'Kin test artist')

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
function Invoke-EmuScript([string]$Script, [string[]]$ScriptArgs = @(), [int]$TimeoutSec = 400) {
    $out = Join-Path $env:TEMP "kin-e2e-2b-3-$Script.out"
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
function Get-Sid([string]$Identity) {
    $sha = [Security.Cryptography.SHA256]::Create()
    $hash = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Identity))
    return (($hash[0..15] | ForEach-Object { $_.ToString('x2') }) -join '')
}
function Shot([string]$Name) {
    New-Item -ItemType Directory -Force $ScreenDir | Out-Null
    Invoke-EmuScript 'screenshot.ps1' @('-Out', ('"{0}"' -f (Join-Path $ScreenDir "$Name.png"))) 60 | Out-Null
}

# ---------------------------------------------------------------------------------------------------------------------------------- the phone
function Flush-PhoneLog {
    # the phone's Kin log is saved to a file before every command (and the buffer cleared), so nothing rolls over and S4 reads all of it
    try {
        $all = @(A logcat -d -v threadtime)
        $kinLines = @($all | Where-Object { $_ -cmatch '\sKinRelay:\s' })   # the app's own tag (not adbd lines that quote our package name, nor system services)
        if ($kinLines.Count -gt 0) { Add-Content -Path $PhoneLog -Value $kinLines -Encoding UTF8 }
        $hits = @($all | Where-Object { $l = $_; $l -cnotmatch '\sKinRelay:\s' -and $l -notmatch '\sadbd\s*:' -and ($TestStrings | Where-Object { $l.Contains($_) }) })
        if ($hits.Count -gt 0) { Add-Content -Path $SystemHits -Value ($hits | ForEach-Object { ($_ -replace '^\S+\s+\S+\s+\d+\s+\d+\s+\w\s+', '') -replace '(.{0,160}).*', '$1' }) -Encoding UTF8 }
        A logcat -c | Out-Null
    } catch { }
}
function Phone-Cmd([string]$Command, [string]$Extras = '', [int]$Wait = 900) {
    Flush-PhoneLog
    A shell "am broadcast -a $Pkg.debug.$Command -n $Pkg/.debug.DebugCommandReceiver $Extras" | Out-Null
    Start-Sleep -Milliseconds $Wait
    @(A logcat -d -s KinRelay | Where-Object { $_ -match 'CMD' } | ForEach-Object { ($_ -replace '^.*CMD ', 'CMD ') })
}
function Media-State { (Phone-Cmd 'MEDIA_STATE' '' 700 | Where-Object { $_ -match 'CMD MEDIA_STATE' } | Select-Object -Last 1) }
function Mval([string]$S, [string]$Name) { if ($S -match ("\b{0}=([^\s{{}}]+)" -f $Name)) { return $Matches[1] } return $null }
function Mint([string]$S, [string]$Name) { $v = Mval $S $Name; if ($null -ne $v -and $v -match '^\d+$') { return [int]$v } return -1 }
function MBrace([string]$S, [string]$Name) { if ($S -match ("\b{0}=\{{([^}}]*)\}}" -f $Name)) { return $Matches[1] } return '' }
function MKey([string]$Brace, [string]$Key) { $m = [regex]::Match($Brace, ("\b{0}[:=]\s*(\d+)" -f [regex]::Escape($Key))); if ($m.Success) { return [int]$m.Groups[1].Value } return 0 }
function Session-Dump { (A shell dumpsys media_session | Out-String) }
function Pc-Session-On-Phone { (Session-Dump) -match 'kin-pc-media' }
function Pc-Card-Text { $d = Session-Dump; $i = $d.IndexOf('kin-pc-media'); if ($i -lt 0) { return '' } return $d.Substring($i, [Math]::Min(2500, $d.Length - $i)) }
function Notif-Dump {
    $d = (A shell dumpsys notification --noredact | Out-String)
    $i = $d.IndexOf('Notification List:')
    if ($i -lt 0) { return $d }
    $m = [regex]::Match($d.Substring($i), '(?m)^\s*(Archive|mArchive|Snoozed|Archived)')
    if ($m.Success) { return $d.Substring($i, $m.Index) } else { return $d.Substring($i) }
}
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
function Find-Slider {
    # the PC volume slider on the Home card: its bounds (a SeekBar node, else just under the "<PC> volume" text)
    $x = Ui-Dump
    if ($x -match 'class="android\.widget\.SeekBar"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"') { return @([int]$Matches[1], [int]$Matches[2], [int]$Matches[3], [int]$Matches[4]) }
    if ($x -match 'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"[^>]*text="[^"]* volume[^"]*"|text="[^"]* volume[^"]*"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"') {
        $g = $Matches; $l = if ($g[1]) { [int]$g[1] } else { [int]$g[5] }; $b = if ($g[4]) { [int]$g[4] } else { [int]$g[8] }; $r = if ($g[3]) { [int]$g[3] } else { [int]$g[7] }
        return @($l, $b, $r, ($b + 90))
    }
    return $null
}
function Go-Home {
    A shell input keyevent KEYCODE_HOME | Out-Null; Start-Sleep -Milliseconds 500
    A shell am start -n "$Pkg/.MainActivity" | Out-Null; Start-Sleep -Seconds 1
    Tap-Text @('Home') 2 | Out-Null; Start-Sleep -Milliseconds 700
}

# ---------------------------------------------------------------------------------------------------------------------------------- the PC
function Kin-Log { Read-Shared (Join-Path $Data 'logs\kin.log') }
function Get-Status { try { Get-Content (Join-Path $Data 'status.json') -Raw -ErrorAction Stop | ConvertFrom-Json } catch { $null } }
function Kc([string]$Path) {
    # The counters come from --dev-dump-view media (current): status.json is written only on a state change, so a command or a state that changes no state lags in it.
    $v = (View-Media).counts; foreach ($p in ($Path -split '\.')) { $v = $v.$p }; if ($null -eq $v) { return -1 }; [int]$v }
function Kv { (Get-Status).media.view }
function View-Media {
    Remove-Item (Join-Path $Data 'view-media.json') -ErrorAction SilentlyContinue
    Invoke-Kin --dev-dump-view media | Out-Null
    Wait-Until { Test-Path (Join-Path $Data 'view-media.json') } 6 | Out-Null
    try { Get-Content (Join-Path $Data 'view-media.json') -Raw | ConvertFrom-Json } catch { $null }
}
function Pc-Volume { $v = (View-Media).panel.volume; if ($v) { return [int]$v.level } return -1 }
function Pc-Read { (& $Session read $script:KinAumid 2>$null) -join ' ' }
function Pc-Flyout-Title([string]$Title) { (Pc-Read) -match ("title={0} " -f [regex]::Escape($Title)) }
function Send-Session([string]$Line) {
    # a command to the test session's stdin; waits for its OK or ERROR (EVENT lines of presses may come in between)
    $script:Sess.StandardInput.WriteLine($Line)
    for ($i = 0; $i -lt 50; $i++) {
        $answer = $script:Sess.StandardOutput.ReadLine()
        if ($null -eq $answer) { return 'EOF' }
        if ($answer -notlike 'EVENT *') { return $answer }
    }
    return 'NOANSWER'
}
function Ev-Lines { @(Get-Content $Events -ErrorAction SilentlyContinue) }
function Ev-Count([string]$Line) { @(Ev-Lines | Where-Object { $_ -eq $Line }).Count }
function Start-TestKin {
    $KinArgs = @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-port', "$Port",
        '--dev-received-dir', ('"{0}"' -f (Join-Path $Data 'received')), '--dev-print-pair-url', '--dev-no-windows', '--dev-toast-quiet', '--dev-media-aumid', $script:TestAumid)
    $script:KinProc = Start-DevKin $KinArgs
}
function Set-PcSession([string[]]$Lines) { foreach ($l in $Lines) { $r = Send-Session $l; if ($r -ne 'OK') { Say "test session: '$l' -> $r" } } }

# ================================================================================================================================== run
try {
    Assert-NoGameRunning
    $Kin = (Resolve-Path $Kin -ErrorAction Stop).Path
    foreach ($f in $Apk, $Session, $Fake) { if (-not (Test-Path $f)) { throw "missing: $f" } }
    New-Item -ItemType Directory -Force $Data | Out-Null
    $installedBefore = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
    $volBefore = (& $Session volume-read) -join ' '
    Say "S6 the real master volume, read only: $volBefore"
    $KinCopy = Join-Path $Work 'Kin.exe'
    Copy-Item $Kin $KinCopy
    Set-KinDev -Exe $KinCopy -DataDir $Data
    Assert-NoDevKinOnDataDir

    # --- the PC's own silent test session (hidden window, Paused, no audio)
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $Session
    $psi.Arguments = "serve --log `"$Events`""
    $psi.UseShellExecute = $false; $psi.CreateNoWindow = $true; $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true
    $script:Sess = [Diagnostics.Process]::Start($psi)
    $ready = $script:Sess.StandardOutput.ReadLine()
    Check 'setup: the PC test session is up (its own AUMID, a nonce title)' ($ready -match '^READY aumid=(app\.kinrelay\.dev\.mediatest-\d+) title=(mediatest-[0-9a-f]{12})$') $ready
    $script:TestAumid = $Matches[1]; $script:Nonce = $Matches[2]
    $PcSid = Get-Sid $script:TestAumid
    $TestStrings += $script:Nonce
    Set-PcSession @('title ZQPcSong', 'artist ZQPcArtist', 'album ZQPcAlbum', 'duration 240000', 'pos 30000', 'state paused', 'actions play,pause,next,prev,seek')

    '{ "mediaShare": true, "mediaShow": true, "hotkey": "", "autoClipboardSync": false, "mirrorNotifications": false }' | Set-Content (Join-Path $Data 'settings.json') -Encoding ASCII

    Say "starting $Avd"
    $started.Emulator = $true
    Invoke-EmuScript 'start.ps1' @('-Avd', $Avd) | Out-Null
    $isBooted = { ((A shell getprop sys.boot_completed 2>$null | Out-String).Trim() -eq '1') }
    if (-not (& $isBooted)) { & $Adb kill-server 2>$null | Out-Null; Start-Sleep -Seconds 2; & $Adb start-server 2>$null | Out-Null; Wait-Until $isBooted 90 | Out-Null }
    if (-not (& $isBooted)) { throw "emulator $Avd did not start" }
    A shell svc wifi enable | Out-Null
    $script:Sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    A uninstall $Pkg 2>&1 | Out-Null
    $out = (& $Adb -s $Serial install -r (Resolve-Path $Apk).Path 2>&1 | Out-String)
    Check 'setup: install Kin (debug build)' ($out -match 'Success')
    if ($script:Sdk -ge 37) { A shell pm grant $Pkg android.permission.ACCESS_LOCAL_NETWORK | Out-Null }
    if ($script:Sdk -ge 33) { A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null }
    A shell am start -n "$Pkg/.MainActivity" | Out-Null

    $started.Kin = $true
    Start-TestKin
    Check 'setup: the test Kin started' (Wait-Until { (Get-Status).state -eq 'not_paired' } 30)
    Wait-Until { (Kin-Log) -match "sharing the PC's media with the phone" } 30 | Out-Null
    $log0 = Kin-Log
    if ($log0 -match '(app\.kinrelay\.desktop\.dev-[0-9a-f]+)') { $script:KinAumid = $Matches[1] }
    Check 'setup: the test Kin is a TEST run (never the installed Kin''s media identity)' ($log0 -match 'sharing the PC''s media with the phone \(test run\)' -and $script:KinAumid -like 'app.kinrelay.desktop.dev-*' -and $script:KinAumid -ne 'app.kinrelay.desktop')
    $kv = Kv
    Check 'setup: status.json says mode test, reading, flyout' ($kv.mode -eq 'test' -and $kv.reading -eq $true -and $kv.flyout -eq $true) ($kv | ConvertTo-Json -Compress)
    Invoke-Kin --dev-start-pairing | Out-Null
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    $paired = $false
    for ($try = 1; $try -le 3 -and -not $paired; $try++) {
        A shell am start -a android.intent.action.VIEW -p $Pkg -d "'$url'" --ez app.kinrelay.android.AUTO_CONFIRM true 2>$null | Out-Null
        $paired = Wait-Until { (Get-Status).state -eq 'phone_connected' } 25
    }
    Check 'setup: paired and connected to the real test Kin' $paired
    if (-not $paired) { throw 'not paired' }
    A logcat -G 16M | Out-Null
    # the first-run reliability setup comes up after pairing: finish it, and say "Not now" to the mirroring offer
    for ($i = 0; $i -lt 4; $i++) { if (Tap-Text @('Done', 'Finish', 'Skip', 'Continue') 1) { break }; A shell input swipe 540 1900 540 700 300 | Out-Null; Start-Sleep -Milliseconds 600 }
    Start-Sleep -Seconds 2
    if ((Ui-Dump) -match 'Show notifications on your PC') { Tap-Text @('Not now') 3 | Out-Null; Start-Sleep -Seconds 1 }
    A shell input keyevent KEYCODE_HOME | Out-Null

    # --- with the phone's switches off (the default), nothing flows
    Start-Sleep -Seconds 4
    $st = Media-State
    Check 'setup: the phone''s switches are off by default and the PC sent no media' ((Mval $st 'share') -eq 'False' -and (Mval $st 'show') -eq 'False' -and (Kc 'sent.states') -eq 0) "$st | sent.states=$(Kc 'sent.states')"
    Check 'setup: the PC said what it shows and shares (media.status reached the phone)' ((Mval $st 'peerShow') -eq 'true' -and (Mval $st 'peerShare') -eq 'true') $st

    # --- the user's switches (the shell is the user) and the access the way the shell can grant it
    Phone-Cmd 'MEDIA_SET' '--ez share true --ez show true' | Out-Null
    A shell cmd notification allow_listener $Service | Out-Null
    $statusOk = Wait-Until { $s = Media-State; (Mval $s 'access') -eq 'true' -and (Mval $s 'listener') -eq 'true' } $(if ($script:Sdk -lt 30) { 70 } else { 20 }) 1500
    Check 'setup: access granted and the phone''s reader runs (the PC shows media, so the phone reads)' $statusOk (Media-State)
    $pcLine = @((Kv).phones)[0].status
    Say "the PC's line for the phone: $pcLine"

    # ============================================================================================================================ S1 phone -> PC
    if (Want 'S1') {
        Assert-NoGameRunning   # a game may have started since the last scenario: stop at the boundary
        Say '--- S1 phone -> PC'
        $r0 = Kc 'received.states'
        Phone-Cmd 'MEDIA_TEST_START' '--es title ZQPhoneSong --es artist ZQPhoneArtist --ei durationMs 200000' | Out-Null
        Phone-Cmd 'MEDIA_TEST_STATE' '--es state paused --ei positionMs 5000 --es actions play,pause,toggle,next,prev,seek' | Out-Null
        Check 'S1 the phone''s session reaches the PC and shows in the flyout (title)' (Wait-Until { Pc-Flyout-Title 'ZQPhoneSong' } 25 1000) (Pc-Read)
        $read = Pc-Read
        Check 'S1 flyout: artist, duration 200000 ms, a thumbnail (the phone''s art, downscaled), position 5000+' ($read -match 'artist=ZQPhoneArtist' -and $read -match 'durationMs=200000' -and $read -match 'thumbnailBytes=[1-9]' -and $read -match 'positionMs=(\d+)' -and [int]$Matches[1] -ge 5000) $read
        Check 'S1 flyout: status 5 = Paused (the dev guard: a test run never shows Playing)' ($read -match 'status=5 ') $read
        Check 'S1 flyout: the buttons are the phone''s (play, pause, next, prev, position)' ($read -match 'controls=\S*play' -and $read -match 'controls=\S*next' -and $read -match 'controls=\S*prev' -and $read -match 'controls=\S*position') $read
        Check 'S1 the PC counted a state and a picture' ((Kc 'received.states') -gt $r0 -and (Kc 'received.images') -ge 1) "states $r0 -> $(Kc 'received.states'), images $(Kc 'received.images')"
        Check 'S1 the PC says phoneMedia paused' ((Kv).phoneMedia -eq 'paused') ((Kv) | ConvertTo-Json -Compress)
        Shot '01-s1-phone-session-on-pc-status'
        # a state change: playing on the phone, still Paused in the flyout, the PC knows it plays
        Phone-Cmd 'MEDIA_TEST_STATE' '--es state playing --ei positionMs 20000' | Out-Null
        Check 'S1 the phone plays: the PC''s view says playing (the position moves)' (Wait-Until { (Kv).phoneMedia -eq 'playing' } 10 500) ((Kv).phoneMedia)
        $read = Pc-Read
        Check 'S1 and the flyout is STILL Paused (test run) with a position of 20000 or more' ($read -match 'status=5 ' -and $read -match 'positionMs=(\d+)' -and [int]$Matches[1] -ge 20000) $read
        Phone-Cmd 'MEDIA_TEST_TRACK' '--es title ZQPhoneTwo --es artist ZQPhoneArtist2' | Out-Null
        Check 'S1 a track change reaches the flyout' (Wait-Until { Pc-Flyout-Title 'ZQPhoneTwo' } 15 750) (Pc-Read)

        # --- the flyout's buttons and seek (media_test_session control: GSMTC TryXAsync on the dev Kin's OWN session only) -> media.cmd -> the phone's test session
        $c = Media-State
        $r1 = (& $Session control $script:KinAumid toggle) -join ' '; Start-Sleep -Milliseconds 700
        $r2 = (& $Session control $script:KinAumid next) -join ' '; Start-Sleep -Milliseconds 700
        $r3 = (& $Session control $script:KinAumid prev) -join ' '; Start-Sleep -Milliseconds 700
        $r4 = (& $Session control $script:KinAumid seek 120000) -join ' '; Start-Sleep -Milliseconds 1500
        $st = Media-State
        $in = MBrace $st 'cmdsReceived'
        Check 'S1 the flyout''s toggle, next, prev and seek were accepted by Windows (the dev Kin''s own session)' (($r1 -eq 'RESULT true') -and ($r2 -eq 'RESULT true') -and ($r3 -eq 'RESULT true') -and ($r4 -eq 'RESULT true')) "$r1 / $r2 / $r3 / $r4"
        Check 'S1 the phone''s test session got next, prev, seek 120000 and a play or pause (media.cmd, the effect is the next state)' ((MKey $in 'next') -ge 1 -and (MKey $in 'prev') -ge 1 -and (MKey $in 'seek') -ge 1 -and ((MKey $in 'play') + (MKey $in 'pause')) -ge 1 -and (Mval $st 'lastCmd') -match 'seek') "$in lastCmd=$(Mval $st 'lastCmd')"
        Check 'S1 PC counted the commands out' ((Kc 'cmds.out') -ge 4) "out=$(Kc 'cmds.out')"
        $mv0 = Mval (Media-State) 'musicVolume'
        # a target that is not the emulator's current volume (it keeps its volume between runs): 20 % or 80 %
        $mvNow = if ($mv0 -match '^(\d+)/(\d+)$') { [int]$Matches[1] / [int]$Matches[2] } else { 0.4 }
        $mvTarget = if ($mvNow -gt 0.5) { 20 } else { 80 }
        Invoke-Kin --dev-media volume $mvTarget | Out-Null; Start-Sleep -Milliseconds 1500
        $mv1 = Mval (Media-State) 'musicVolume'
        Check 'S1 PC -> phone volume command (Kin.exe --dev-media volume 40, the user''s own path) changes the EMULATOR''s music volume (not the PC''s)' ($mv0 -ne $mv1) "$mv0 -> $mv1"

        # --- share off -> one clear
        $cl0 = Mint (Media-State) 'clears'
        Phone-Cmd 'MEDIA_SET' '--ez share false' 1500 | Out-Null
        Check 'S1 share off: the flyout session is cleared on the PC' (Wait-Until { (Pc-Read) -notmatch 'title=ZQPhone' } 10 500) (Pc-Read)
        $st = Media-State
        Check 'S1 share off: exactly one media.clear was sent, and the reader stopped' ((Mint $st 'clears') -eq ($cl0 + 1) -and (Mval $st 'listener') -eq 'false') "clears $cl0 -> $(Mint $st 'clears') listener=$(Mval $st 'listener')"
        $s0 = Kc 'received.states'
        Phone-Cmd 'MEDIA_TEST_TRACK' '--es title ZQGateSong --es artist ZQPhoneArtist' 1500 | Out-Null
        Check 'S1 share off: a track change sends nothing' ((Kc 'received.states') -eq $s0) "received.states $s0 -> $(Kc 'received.states')"
        Phone-Cmd 'MEDIA_SET' '--ez share true' 1500 | Out-Null
        Check 'S1 share on again: the session is back (new title)' (Wait-Until { Pc-Flyout-Title 'ZQGateSong' } 20 1000) (Pc-Read)

        # --- access revoked while sharing -> the PC's card is cleared (review fix 2)
        A shell cmd notification disallow_listener $Service | Out-Null
        Check 'S1 access revoked while sharing: the stale card is cleared on the PC' (Wait-Until { (Pc-Read) -notmatch 'title=ZQ' } 15 750) (Pc-Read)
        $st = Media-State
        Check 'S1 access revoked: the phone says access false, reader stopped' ((Mval $st 'access') -eq 'false' -and (Mval $st 'listener') -eq 'false') $st
        Say ("the PC's line after the revoke: " + (@((Kv).phones)[0].status))
        A shell cmd notification allow_listener $Service | Out-Null
        Check 'S1 access granted again: the session comes back' (Wait-Until { Pc-Flyout-Title 'ZQGateSong' } $(if ($script:Sdk -lt 30) { 70 } else { 30 }) 1500) (Pc-Read)
    }

    # ============================================================================================================================ S2 PC -> phone
    if (Want 'S2') {
        Assert-NoGameRunning
        Say '--- S2 PC -> phone'
        Phone-Cmd 'MEDIA_TEST_STOP' '' 800 | Out-Null     # nothing of the phone's own now
        Set-PcSession @('title ZQPcSong', 'artist ZQPcArtist', 'state paused', 'pos 30000')
        Check 'S2 the phone shows the PC session (kin-pc-media is an active MediaSession)' (Wait-Until { Pc-Session-On-Phone } 20 1000) ''
        $card = Pc-Card-Text
        $cardState = (($card -split "`n") | Where-Object { $_ -match 'PlaybackState|\bstate=' } | Select-Object -First 3) -join ' | '
        Check 'S2 the session''s title is the PC test session''s, state paused' ($card -match 'ZQPcSong' -and $cardState -match '(?i)state=(2\b|PAUSED)') ($cardState -replace '\s+', ' ')
        $nd = Notif-Dump
        Check 'S2 a MediaStyle notification on channel kin_pc_media, no foreground media service' ($nd -match 'kin_pc_media' -and $nd -match 'MediaStyle' -and (A shell dumpsys activity services $Pkg | Out-String) -notmatch 'foregroundServiceType=0x2[0-9a-f]*\b.*mediaPlayback') ''
        $mst = Media-State
        Check 'S2 the phone''s own state: pcSession true, pcState paused' ((Mval $mst 'pcSession') -eq 'True' -and (Mval $mst 'pcState') -eq 'paused') $mst
        if ($script:Sdk -ge 33) {
            A shell cmd statusbar expand-settings | Out-Null; Start-Sleep -Seconds 2
            Shot '02-s2-carousel-pc-media-paused'
            A shell cmd statusbar collapse | Out-Null; Start-Sleep -Milliseconds 700
        } else {
            A shell cmd statusbar expand-notifications | Out-Null; Start-Sleep -Seconds 2
            Shot '02-s2-shade-pc-media-api29'
            A shell cmd statusbar collapse | Out-Null; Start-Sleep -Milliseconds 700
        }
        # --- the phone's buttons (what the carousel does: the session's own controller) -> media.cmd -> the PC test session's own handler
        $e0 = @(Ev-Lines).Count
        foreach ($c in @(@('play', ''), @('pause', ''), @('next', ''), @('prev', ''), @('seek', '--ei positionMs 90000'))) {
            Phone-Cmd 'MEDIA_PC_CMD' ("--es cmd {0} {1}" -f $c[0], $c[1]) 1300 | Out-Null
        }
        Start-Sleep -Seconds 1
        $ev = @(Ev-Lines | Select-Object -Skip $e0)
        # The test session is always Paused, and Windows lists no `pause` control for a Paused session (read: controls=play,toggle,...), so the phone's pause button
        # sends `toggle`, which Windows turns into a Play press: two `button play` (play, and pause as a toggle), never a `button pause`.
        $plays = @($ev | Where-Object { $_ -eq 'button play' }).Count
        Check 'S2 the phone''s play, pause (a toggle: the session is Paused), next, prev, seek 90000 reached ONLY the PC test session (its own press log)' (($plays -eq 2) -and ($ev -notcontains 'button pause') -and ($ev -contains 'button next') -and ($ev -contains 'button prev') -and ($ev -contains 'seek 90000') -and $ev.Count -eq 5) ($ev -join ' | ')

        # --- the volume: the FAKE endpoint (a test run), 51 steps of 2 %, starts at 50
        $v0 = Pc-Volume
        Check 'S2 the PC volume is the FAKE endpoint, starting at 50' ($v0 -eq 50 -and (Kv).mode -eq 'test') "level=$v0"
        Phone-Cmd 'MEDIA_SET' '--ez volkeys true' 1500 | Out-Null
        Phone-Cmd 'MEDIA_PC_CMD' '--es cmd volume --ei delta 0 --ei level 30' 1500 | Out-Null   # the debug command's delta defaults to 5 and wins over a level unless it is 0
        Check 'S2 media.cmd volume level 30 (the phone''s volume provider) sets the fake to 30' (Wait-Until { (Pc-Volume) -eq 30 } 8 1000) "level=$(Pc-Volume)"
        Phone-Cmd 'MEDIA_PC_CMD' '--es cmd volume --ei delta 5' 1500 | Out-Null
        $vd = Pc-Volume
        Check 'S2 a +5 delta moves the fake by at least one 2 % step (30 -> 32..36)' ($vd -ge 32 -and $vd -le 36) "level=$vd"
        Check 'S2 the PC sent media.volume back (count)' ((Kc 'sent.volumes') -ge 2) "volumes=$(Kc 'sent.volumes')"
        Phone-Cmd 'MEDIA_SET' '--ez volkeys false' 1000 | Out-Null
        Check 'S2 the REAL master volume is still what it was (read only)' ($volBefore -eq ((& $Session volume-read) -join ' ')) ''
        # --- the slider on the Home card: the final value must go even when the gate swallowed the middle (review fix 5)
        Go-Home
        $sl = Find-Slider
        if ($sl) {
            $x1 = $sl[0] + [int](($sl[2] - $sl[0]) * 0.12); $x2 = $sl[0] + [int](($sl[2] - $sl[0]) * 0.85); $y = [int](($sl[1] + $sl[3]) / 2)
            Shot '03-s2-home-pc-volume-slider'
            A shell input swipe $x1 $y $x2 $y 600 | Out-Null
            $ok = Wait-Until { $l = Pc-Volume; $l -ge 70 -and $l -le 92 } 8 800
            Check 'S2 the slider''s final value (dragged to about 85 %) reached the PC''s fake endpoint' $ok "level=$(Pc-Volume)"
        } else { Check 'S2 the Home card has the PC volume slider' $false (Ui-Dump).Length }
        # --- no PC session: the volume is still shown (review fix 4) and still settable
        Set-PcSession @('state stopped')
        Check 'S2 the PC session stops: the card is removed from the phone' (Wait-Until { -not (Pc-Session-On-Phone) } 12 800) ''
        Go-Home
        $sl = Find-Slider
        Check 'S2 with no PC session the Home card still shows the PC volume slider (fix 4)' ($null -ne $sl) ''
        if ($sl) {
            $x1 = $sl[0] + [int](($sl[2] - $sl[0]) * 0.80); $x2 = $sl[0] + [int](($sl[2] - $sl[0]) * 0.30); $y = [int](($sl[1] + $sl[3]) / 2)
            Shot '04-s2-home-volume-no-pc-session'
            A shell input swipe $x1 $y $x2 $y 600 | Out-Null
            $ok = Wait-Until { $l = Pc-Volume; $l -ge 20 -and $l -le 42 } 8 800
            Check 'S2 and it still works with no session (dragged to about 30 %)' $ok "level=$(Pc-Volume)"
        }
        A shell input keyevent KEYCODE_HOME | Out-Null
        # --- fast reconnect clears the old card (review fix 8): the card is on screen, the PC session ends while the phone is cut off, the link comes back
        Set-PcSession @('state paused', 'title ZQPcTwo')
        Check 'S2 (before the reconnect) the card is back with the new title' (Wait-Until { (Pc-Card-Text) -match 'ZQPcTwo' } 15 1000) ''
        A shell svc wifi disable | Out-Null
        Start-Sleep -Milliseconds 1200
        Set-PcSession @('state stopped')
        A shell svc wifi enable | Out-Null
        Check 'S2 Wi-Fi flap: the phone reconnects' (Wait-Until { (Get-Status).state -eq 'phone_connected' -and ((Media-State) -match 'online=true') } 60 1500) ''
        Start-Sleep -Seconds 3
        Check 'S2 fast reconnect: the old card is gone (the PC has no session now)' (-not (Pc-Session-On-Phone)) (Pc-Card-Text)
        Set-PcSession @('state paused', 'title ZQPcSong')
        Check 'S2 and a new PC session shows after the reconnect' (Wait-Until { (Pc-Card-Text) -match 'ZQPcSong' } 20 1000) ''
        # the PC restarted behind the phone's back (the connection object is replaced): the old card must not survive it either
        Stop-DevKin 15
        Check 'S2 the test Kin stopped: the phone''s card is removed when the connection closes' (Wait-Until { -not (Pc-Session-On-Phone) } 25 1000) ''
        Set-PcSession @('state stopped')
        Start-TestKin
        Check 'S2 the test Kin is back and the phone reconnected' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 90 1500) ''
        Start-Sleep -Seconds 4
        Check 'S2 after the PC came back with no session there is no stale card' (-not (Pc-Session-On-Phone)) (Pc-Card-Text)
        Check 'S2 the reconnected PC sent the phone its status again, and still only the test session is read' ((Kv).reading -eq $true -and (Kv).mode -eq 'test') ''
        Set-PcSession @('state paused', 'title ZQPcSong')
        Check 'S2 the PC session shows again' (Wait-Until { (Pc-Card-Text) -match 'ZQPcSong' } 20 1000) ''
    }

    # ============================================================================================================================ S3 gates and rates
    if (Want 'S3') {
        Assert-NoGameRunning
        Say '--- S3 gates and rates'
        Set-PcSession @('state paused', 'title ZQPcSong')
        if (-not (Pc-Flyout-Title 'ZQGateSong') -and -not (Pc-Flyout-Title 'ZQPhoneTwo')) {
            Phone-Cmd 'MEDIA_TEST_START' '--es title ZQGateSong --es artist ZQPhoneArtist --ei durationMs 200000' | Out-Null
            Phone-Cmd 'MEDIA_TEST_STATE' '--es state paused --ei positionMs 5000 --es actions play,pause,toggle,next,prev,seek' | Out-Null
            Wait-Until { Pc-Flyout-Title 'ZQGateSong' } 25 1000 | Out-Null
        }
        Wait-Until { (Pc-Card-Text) -match 'ZQPcSong' } 20 1000 | Out-Null
        # --- the PC's "show" off: the phone stops reading and sending, the flyout session goes
        Invoke-Kin --dev-set mediaShow false | Out-Null
        Check 'S3 PC show off: the flyout session is removed at once' (Wait-Until { (Pc-Read) -notmatch 'title=ZQ' } 10 500) (Pc-Read)
        Check 'S3 PC show off: the phone sees peerShow false and stops its reader (nothing is read while nobody shows it)' (Wait-Until { $s = Media-State; (Mval $s 'peerShow') -eq 'false' -and (Mval $s 'listener') -eq 'false' } 20 1500) (Media-State)
        $st0 = Mint (Media-State) 'sentStates'; $r0 = Kc 'received.states'
        Phone-Cmd 'MEDIA_TEST_TRACK' '--es title ZQGateSong2 --es artist ZQPhoneArtist' 2000 | Out-Null
        Check 'S3 PC show off: a track change sends nothing (phone sentStates and PC received.states unchanged)' ((Mint (Media-State) 'sentStates') -eq $st0 -and (Kc 'received.states') -eq $r0) "sent $st0, received $r0"
        Invoke-Kin --dev-set mediaShow true | Out-Null
        Check 'S3 PC show on again: the session returns' (Wait-Until { Pc-Flyout-Title 'ZQGateSong2' } 25 1000) (Pc-Read)
        # --- the PC's "share" off: the phone's PC card goes (one clear), nothing is sent
        Invoke-Kin --dev-set mediaShare false | Out-Null
        Check 'S3 PC share off: the phone''s PC-media session is removed' (Wait-Until { -not (Pc-Session-On-Phone) } 12 800) ''
        $ss0 = Kc 'sent.states'
        Set-PcSession @('title ZQPcTwo', 'state paused')
        Start-Sleep -Seconds 3
        Check 'S3 PC share off: a change of the PC session sends nothing' ((Kc 'sent.states') -eq $ss0 -and -not (Pc-Session-On-Phone)) "sent.states $ss0 -> $(Kc 'sent.states')"
        Invoke-Kin --dev-set mediaShare true | Out-Null
        Check 'S3 PC share on again: the phone shows the PC session' (Wait-Until { Pc-Session-On-Phone } 25 1000) ''
        # --- the phone's "show" off: the PC sends nothing more, the phone's card goes
        Phone-Cmd 'MEDIA_SET' '--ez show false' 1500 | Out-Null
        Check 'S3 phone show off: the phone''s PC-media session is removed' (Wait-Until { -not (Pc-Session-On-Phone) } 12 800) ''
        Check 'S3 phone show off: the PC sees show false' (Wait-Until { @((Kv).phones)[0].status -match "isn't showing your media" } 15 1000) (@((Kv).phones)[0].status)
        $ss0 = Kc 'sent.states'
        Set-PcSession @('title ZQPcSong', 'state paused')
        Start-Sleep -Seconds 3
        Check 'S3 phone show off: the PC sends no state' ((Kc 'sent.states') -eq $ss0) "sent.states $ss0 -> $(Kc 'sent.states')"
        Phone-Cmd 'MEDIA_SET' '--ez show true' 1500 | Out-Null
        Check 'S3 phone show on again: the PC session shows on the phone' (Wait-Until { Pc-Session-On-Phone } 25 1000) ''
        # --- the phone's "share" off: the PC sends no command to it (no media.cmd out), receives nothing
        Invoke-Kin --dev-media next | Out-Null; Start-Sleep -Milliseconds 1200
        $co0 = Kc 'cmds.out'; $ci0 = MKey (MBrace (Media-State) 'cmdsReceived') 'next'
        Phone-Cmd 'MEDIA_SET' '--ez share false' 1500 | Out-Null
        Wait-Until { -not (Pc-Read).StartsWith('SESSION') } 8 500 | Out-Null
        Invoke-Kin --dev-media next | Out-Null; Invoke-Kin --dev-media play | Out-Null; Start-Sleep -Milliseconds 1500
        Check 'S3 phone share off: the PC sends no media.cmd (nothing shown, command out unchanged)' ((Kc 'cmds.out') -eq $co0 -and (MKey (MBrace (Media-State) 'cmdsReceived') 'next') -eq $ci0) "cmds.out $co0 -> $(Kc 'cmds.out')"
        Phone-Cmd 'MEDIA_SET' '--ez share true' 1500 | Out-Null
        Wait-Until { Pc-Flyout-Title 'ZQGateSong2' } 25 1000 | Out-Null

        # --- the rate limits
        $r0 = Kc 'received.states'; $d0 = Kc 'dropped.rate'
        Phone-Cmd 'MEDIA_TEST_BURST' '--ei count 200' 4500 | Out-Null
        Start-Sleep -Seconds 2
        $dr = (Kc 'received.states') - $r0
        Check 'S3 rate: 200 phone state changes in ~3 s reach the PC as a handful (3..40), never 200' ($dr -ge 3 -and $dr -le 40) "received.states +$dr"
        $s0 = Kc 'sent.states'
        $sw = [Diagnostics.Stopwatch]::StartNew()
        1..120 | ForEach-Object { Send-Session "title ZQPcBurst$_" | Out-Null }
        $took = $sw.Elapsed.TotalSeconds
        Start-Sleep -Seconds 2
        $ds = (Kc 'sent.states') - $s0
        $cap = [int]([Math]::Ceiling($took / 0.25) + 14)
        Check "S3 rate: 120 changes of the PC session in $([int]$took) s reach the phone as a handful (1..$cap; the reader's debounce can fold a fast burst into one state), never 120" ($ds -ge 1 -and $ds -le $cap) "sent.states +$ds"
        Set-PcSession @('title ZQPcSong', 'state paused', 'pos 30000')
        Wait-Until { (Pc-Card-Text) -match 'ZQPcSong' } 15 1000 | Out-Null
        # a flood of commands: 40 'next' in about 2 s -> at most 20 reach the PC test session (the phone and the PC each limit it)
        $e0 = Ev-Count 'button next'
        Flush-PhoneLog
        $script = '/data/local/tmp/kin-flood.sh'
        $lines = @('i=0', 'while [ $i -lt 40 ]; do', "  am broadcast -a $Pkg.debug.MEDIA_PC_CMD -n $Pkg/.debug.DebugCommandReceiver --es cmd next >/dev/null 2>&1", '  i=$((i+1))', 'done')
        [IO.File]::WriteAllText((Join-Path $Work 'flood.sh'), (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding($false)))
        & $Adb -s $Serial push (Join-Path $Work 'flood.sh') $script | Out-Null
        A shell sh $script | Out-Null
        Start-Sleep -Seconds 3
        $got = (Ev-Count 'button next') - $e0
        $stF = Media-State
        Check 'S3 rate: 40 phone commands in a burst: at most 20 reach the PC session in 10 s, and the limit is counted' ($got -ge 3 -and $got -le 22) "button next +$got; dropped={$(MBrace $stF 'dropped')} cmdErrors={$(MBrace $stF 'cmdErrors')}"
        A shell rm $script | Out-Null

        # --- a peer without `media`, and a peer that never sent its status: the PC sends them nothing (fake_phone pairs as a second phone; both stay silent)
        $variants = @(
            @{ Name = 'a 0.1.x phone (hello without media)'; Args = '--old-phone' },
            @{ Name = 'a phone that lists media but never sent media.status'; Args = '--media' },
            @{ Name = 'a phone whose status says show:false'; Args = '--media --media-status false,true,granted' }
        )
        $vi = 0
        foreach ($v in $variants) {
            $vi++
            Invoke-Kin --dev-start-pairing | Out-Null
            Start-Sleep -Milliseconds 1200
            $u = (Get-Content $urlFile -Raw).Trim()
            $out = Join-Path $Work ("fake-$vi.txt")
            $s0 = Kc 'sent.states'
            $argLine = "`"$u`" --host 127.0.0.1 --name `"Fake $vi`" --seconds 14 $($v.Args)"
            $p = Start-Process $Fake -ArgumentList $argLine -PassThru -RedirectStandardOutput $out -RedirectStandardError (Join-Path $Work "fake-$vi.err") -WindowStyle Hidden
            $script:FakeProcs += $p
            Start-Sleep -Seconds 4
            Set-PcSession @("title ZQPcTwo$vi")
            if (-not $p.WaitForExit(30000)) { Stop-OwnProcess $p $Fake }
            $txt = Get-Content $out -Raw -ErrorAction SilentlyContinue
            Check "S3 $($v.Name): the PC sent it no media.state, image, clear, volume or cmd (media.status, the PC's own switches, is allowed to a peer that lists media)" (($txt -match 'hello|paired|connected|received') -and ($txt -notmatch 'type=media\.(state|image|clear|volume|cmd)')) (($txt -split "`n" | Select-Object -First 3) -join ' | ')
        }
        Set-PcSession @('title ZQPcSong', 'state paused')
    }

    # ============================================================================================================================ S4 privacy (while everything is up)
    Say '--- S4 privacy'
    Flush-PhoneLog
    $kinLog = Kin-Log
    $leakLog = @($TestStrings | Where-Object { $kinLog.Contains($_) })
    Check 'S4 the test Kin''s log has no title, artist, album, app name or AUMID of what played' ($leakLog.Count -eq 0 -and $kinLog.Length -gt 0) ($leakLog -join ',')
    $phoneKinLog = if (Test-Path $PhoneLog) { Get-Content $PhoneLog -Raw } else { '' }
    $leakPhone = @($TestStrings | Where-Object { $phoneKinLog.Contains($_) })
    Check 'S4 the phone''s Kin log (whole run, every Kin tag line) has none of them' ($leakPhone.Count -eq 0 -and $phoneKinLog.Length -gt 0) ("$($leakPhone -join ',') ($($phoneKinLog.Length) chars)")
    Check 'S4 the phone''s log has the one-line counts of the media feature' ($phoneKinLog -match 'media: sent \d+ states') ''
    $sys = if (Test-Path $SystemHits) { @(Get-Content $SystemHits).Count } else { 0 }
    if ($sys -gt 0) { Note "the phone's system log (other tags than Kin's) holds $sys line(s) with a test string (Android's own services: see $SystemHits)" } else { Say "the phone's system log had none either" }
    $tar = Join-Path $Work 'phone-data.tar'
    cmd /c "`"$Adb`" -s $Serial exec-out run-as $Pkg tar cf - . > `"$tar`" 2>nul"
    $tarText = ''
    if (Test-Path $tar) { $tarText = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($tar)) }
    $leakDb = @($TestStrings | Where-Object { $tarText.Contains($_) })
    Check 'S4 the phone''s app data (databases, DataStore, files, caches: one tar of all of it) has none of them' ($tarText.Length -gt 100000 -and $leakDb.Count -eq 0) ("tar $($tarText.Length) bytes; leaks: $($leakDb -join ',')")
    Remove-Item $tar -Force -ErrorAction SilentlyContinue
    $dumpView = View-Media
    $viewText = if ($dumpView) { $dumpView | ConvertTo-Json -Depth 8 -Compress } else { '' }
    Check 'S4 --dev-dump-view media is redacted (no title, artist, app)' ($viewText.Length -gt 0 -and (@($TestStrings | Where-Object { $viewText.Contains($_) }).Count -eq 0)) ($viewText.Substring(0, [Math]::Min(200, $viewText.Length)))
    $filesWithText = @(Get-ChildItem $Data -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $c = Read-Shared $_.FullName; ($TestStrings | Where-Object { $c.Contains($_) }).Count -gt 0 })
    Check 'S4 no file of the test Kin''s data directory (log, status.json, settings, database, ...) has any of them' ($filesWithText.Count -eq 0) (($filesWithText | ForEach-Object { $_.Name }) -join ',')
    $files = @(Get-ChildItem $Data -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch '\\logs\\' } | ForEach-Object { $_.FullName.Substring($Data.Length + 1) })
    Say ('the test Kin''s data directory holds: ' + ($files -join ', '))
    $mediaFiles = @($files | Where-Object { $_ -match 'media|art|thumb|\.(png|jpg|jpeg)$' -and $_ -notmatch 'view-media\.json' })
    Check 'S4 no picture or media file is on the PC''s disk (art is in memory only)' ($mediaFiles.Count -eq 0) ($mediaFiles -join ',')
    Say ('status.json media: ' + ((Get-Status).media | ConvertTo-Json -Compress -Depth 6))
}
catch {
    Say "ERROR: $_ ($($_.ScriptStackTrace))"
    $script:Failures.Add("exception: $_")
}
finally {
    foreach ($p in $script:FakeProcs) { Stop-OwnProcess $p $Fake }
    if ($started.Emulator -and ((A get-state | Out-String) -match 'device')) {
        try { A shell svc wifi enable | Out-Null } catch {}
        try { Phone-Cmd 'MEDIA_TEST_STOP' '' 300 | Out-Null } catch {}
        try { Phone-Cmd 'MEDIA_SET' '--ez share false --ez show false --ez volkeys false' 300 | Out-Null } catch {}
        try { A shell cmd notification disallow_listener $Service | Out-Null } catch {}
    }
    if ($started.Kin) { Stop-DevKin 15 }
    if ($script:Sess) {
        try { $script:Sess.StandardInput.WriteLine('exit') } catch { }
        if (-not $script:Sess.WaitForExit(5000)) { Stop-OwnProcess $script:Sess $Session }
    }
}
# ---------------------------------------------------------------------------------------------------------------------------------- S6 guard
try {
    $installedAfter = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
    Check 'S6 the installed Kin was not started, stopped or restarted (same PIDs)' (($installedBefore -join ',') -eq ($installedAfter -join ',')) "$($installedBefore -join ',') -> $($installedAfter -join ',')"
    $volAfter = (& $Session volume-read) -join ' '
    Check 'S6 the REAL master volume is what it was before the run (level and mute, read only)' ($volBefore -eq $volAfter) "$volBefore -> $volAfter"
    Check 'S6 no Kin process of this run is left' (@(Get-DevKinProcesses).Count -eq 0)
    Check 'S6 the test session process (and every fake phone this run started) is gone' ((-not ($script:Sess -and -not $script:Sess.HasExited)) -and @($script:FakeProcs | Where-Object { $_ -and -not $_.HasExited }).Count -eq 0)
} catch { Say "ERROR in the guard checks: $_"; $script:Failures.Add("exception: $_") }
if (-not $KeepRunning -and $started.Emulator) { Invoke-EmuScript 'stop.ps1' @('-All') 120 | Out-Null }
$env:COMPUTERNAME = $script:OrigComputerName
Remove-DevDataDir $Data
Exit-KinLock
Write-Host ''
$script:Results.GetEnumerator() | ForEach-Object { Write-Host ("{0,-5} {1}" -f $_.Value, $_.Key) }
if ($script:Notes.Count -gt 0) { Write-Host "`nNotes:"; $script:Notes | ForEach-Object { Write-Host " - $_" } }
Say "work folder (logs of the phone): $Work"
if ($script:Failures.Count -gt 0) { Write-Host "`nFAILED: $($script:Failures -join '; ')"; exit 1 }
Write-Host "`nOK: all $($script:Results.Count) checks passed on $Avd."
exit 0
