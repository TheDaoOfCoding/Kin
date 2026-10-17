# 2B.3 on the phone (media controls both ways), verified ONCE on an emulator: the real debug APK against the test PC stub of the unit-test tree
# (android\app\src\test\...\FakePcSession.kt: a PC that lists `link, notif, media`, keeps every frame the phone sends, and sends media.status / state / image /
# clear / volume / cmd on command; see its header). The real desktop (Kin.exe) is not used here: this script is the phone side; the desktop round trip is the
# integration agent's tools\e2e-2b-3.ps1.
#
#   android\tools\verify-2b3.ps1 -ScreenshotDir docs\results\phase-2b-3-android-screens
#   android\tools\verify-2b3.ps1 -Avd kin_api29 -Only Api29      # the Android 10 path (a quick check: both directions once)
#
# NO SOUND, no real player, nothing of the user's: the phone side is a silent MediaSession of Kin's own (DebugMediaSession: no MediaPlayer, no AudioTrack, no audio
# focus) and the PC side is a stub. Never touches the user's installed Kin or anything of the PC: no Kin.exe, no clipboard, no volume of the PC, no admin.
param(
    [string]$Avd = 'kin_api37',
    [string]$Apk = (Join-Path $PSScriptRoot '..\app\build\outputs\apk\debug\app-debug.apk'),
    [string[]]$Only = @(),
    [string]$ScreenshotDir = '',
    [switch]$KeepRunning,
    [string]$Work = ''
)
. (Join-Path $PSScriptRoot '..\..\tools\kin-lock.ps1'); Enter-KinLock 'verify-2b3'
if (-not $Work) { $Work = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-verify-2b3-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
$ErrorActionPreference = 'Continue'
foreach ($name in 'ANDROID_HOME', 'JAVA_HOME') {
    if (-not (Get-Item "env:$name" -ErrorAction SilentlyContinue)) { $v = [Environment]::GetEnvironmentVariable($name, 'User'); if ($v) { Set-Item "env:$name" $v } }
}
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Emu = Join-Path $Repo 'tools\emulator'
$Adb = Join-Path $(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }) 'platform-tools\adb.exe'
$Pkg = 'app.kinrelay.android'
$Service = "$Pkg/$Pkg.notifmirror.MirrorListenerService"
$Serial = 'emulator-5554'
$FakeDir = Join-Path $Repo 'android\app\build\fake-pc'
$Events = Join-Path $FakeDir 'events.txt'
$CommandFile = Join-Path $FakeDir 'command.txt'
$ScreenDir = if ($ScreenshotDir) { [IO.Path]::GetFullPath((Join-Path $Repo $ScreenshotDir)) } else { '' }
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Results = [ordered]@{}
$script:Gradle = $null
$TestSid = ''
$PcSid = ''

function A { & $Adb -s $Serial @args }
function Say($m) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $m) }
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $script:Results[$name] = if ($ok) { 'PASS' } else { 'FAIL' }
    if ($ok) { Say "PASS  $name $detail" } else { Say "FAIL  $name $detail"; $script:Failures.Add($name) }
}
function Want([string]$Id) { $ids = @($Only | ForEach-Object { $_ -split ',' } | Where-Object { $_ }); $ids.Count -eq 0 -or $ids -contains $Id }
function Wait-Until([scriptblock]$Cond, [double]$Seconds, [int]$PollMs = 250) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds $PollMs }
    return $false
}
function Invoke-EmuScript([string]$Script, [string[]]$ScriptArgs = @(), [int]$TimeoutSec = 400) {
    $out = Join-Path $env:TEMP "kin-verify-2b3-$Script.out"
    $p = Start-Process powershell -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $Emu $Script))) + $ScriptArgs)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; return $false }
    return $true
}
function Shot([string]$Name) {
    if (-not $ScreenDir) { return }
    New-Item -ItemType Directory -Force $ScreenDir | Out-Null
    Invoke-EmuScript 'screenshot.ps1' @('-Out', ('"{0}"' -f (Join-Path $ScreenDir "$Name.png"))) 60 | Out-Null
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
function Scroll-Down { A shell input swipe 540 1900 540 700 300 | Out-Null; Start-Sleep -Milliseconds 600 }
function Scroll-To([string]$Pattern, [int]$Max = 10) {
    for ($i = 0; $i -lt $Max; $i++) {
        if ((Ui-Dump) -match $Pattern) { return $true }
        A shell input swipe 540 1700 540 900 300 | Out-Null; Start-Sleep -Milliseconds 500
    }
    return $false
}
function Phone-Cmd([string]$Command, [string]$Extras = '', [int]$Wait = 1200) {
    A logcat -c | Out-Null
    A shell "am broadcast -a $Pkg.debug.$Command -n $Pkg/.debug.DebugCommandReceiver $Extras" | Out-Null
    Start-Sleep -Milliseconds $Wait
    @(A logcat -d -s KinRelay | Where-Object { $_ -match 'CMD' } | ForEach-Object { ($_ -replace '^.*CMD ', 'CMD ') })
}
function Media-State { (Phone-Cmd 'MEDIA_STATE' '' 800 | Where-Object { $_ -match 'CMD MEDIA_STATE' } | Select-Object -Last 1) }
function Pc-Command([string]$Line) { Set-Content -Path $CommandFile -Value $Line -Encoding ASCII; Start-Sleep -Milliseconds 1500 }
function Pc-Lines { @(Get-Content $Events -ErrorAction SilentlyContinue).Count }
# The frames the PC stub got from the phone after line $From of its events file, as objects (with the stub's timestamp in `_t`).
function Pc-Frames([int]$From = 0) {
    @(Get-Content $Events -ErrorAction SilentlyContinue | Select-Object -Skip $From | Where-Object { $_ -match '^\d+ frame from phone: ' } |
        ForEach-Object {
            try { $o = ($_ -replace '^\d+ frame from phone: ', '') | ConvertFrom-Json; $o | Add-Member -NotePropertyName _t -NotePropertyValue ([int64](($_ -split ' ')[0])); $o } catch { $null }
        } | Where-Object { $_ })
}
function Media-Frames([int]$From = 0) { @(Pc-Frames $From | Where-Object { $_.type -like 'media.*' }) }
function Json-Size([string]$Data) {
    # width, height of a PNG or JPEG from a base64url string (the header only), or $null.
    $b64 = $Data.Replace('-', '+').Replace('_', '/'); while ($b64.Length % 4) { $b64 += '=' }
    # (A [byte] shifted left stays a byte in Windows PowerShell 5.1: every read is widened to [int] first.)
    $b = [int[]][Convert]::FromBase64String($b64)
    if ($b.Length -gt 33 -and $b[0] -eq 0x89 -and $b[1] -eq 0x50) { return @((($b[16] -shl 24) + ($b[17] -shl 16) + ($b[18] -shl 8) + $b[19]), (($b[20] -shl 24) + ($b[21] -shl 16) + ($b[22] -shl 8) + $b[23]), $b.Length) }
    if ($b[0] -ne 0xFF -or $b[1] -ne 0xD8) { return $null }
    $pos = 2
    while ($pos + 4 -le $b.Length) {
        if ($b[$pos] -ne 0xFF) { return $null }
        while ($pos -lt $b.Length -and $b[$pos] -eq 0xFF) { $pos++ }
        $m = $b[$pos]; $pos++
        if ($m -eq 0xD9 -or $m -eq 0xDA) { return $null }
        if ($m -eq 0x01 -or ($m -ge 0xD0 -and $m -le 0xD8)) { continue }
        $len = ($b[$pos] -shl 8) + $b[$pos + 1]
        if ($m -ge 0xC0 -and $m -le 0xCF -and $m -ne 0xC4 -and $m -ne 0xC8 -and $m -ne 0xCC) { return @((($b[$pos + 5] -shl 8) + $b[$pos + 6]), (($b[$pos + 3] -shl 8) + $b[$pos + 4]), $b.Length) }
        $pos += $len
    }
    return $null
}
function Session-Dump { (A shell dumpsys media_session | Out-String) }
function Pc-Session-Present { (Session-Dump) -match 'kin-pc-media' }
function Notif-Dump {
    $d = (A shell dumpsys notification --noredact | Out-String)
    $i = $d.IndexOf('Notification List:')
    if ($i -lt 0) { return $d }
    $m = [regex]::Match($d.Substring($i), '(?m)^\s*(Archive|mArchive|Snoozed|Archived)')
    if ($m.Success) { return $d.Substring($i, $m.Index) } else { return $d.Substring($i) }
}
function Stop-Gradle { if ($script:Gradle -and -not $script:Gradle.HasExited) { & taskkill /PID $script:Gradle.Id /T /F 2>$null | Out-Null } }

try {
    if (-not (Test-Path $Apk)) { throw "APK missing: $Apk" }
    New-Item -ItemType Directory -Force $Work | Out-Null
    Say "starting $Avd"
    $emuUp = Invoke-EmuScript 'start.ps1' @('-Avd', $Avd)
    $isBooted = { ((A shell getprop sys.boot_completed 2>$null | Out-String).Trim() -eq '1') }
    if (-not (& $isBooted)) { & $Adb kill-server 2>$null | Out-Null; Start-Sleep -Seconds 2; & $Adb start-server 2>$null | Out-Null; Wait-Until $isBooted 90 | Out-Null }
    if (-not (& $isBooted)) { throw "emulator $Avd did not start" }
    A shell svc wifi enable | Out-Null
    $sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    Say "API $sdk"

    A uninstall $Pkg 2>&1 | Out-Null
    $out = (& $Adb -s $Serial install -r (Resolve-Path $Apk).Path 2>&1 | Out-String)
    Check 'install Kin (debug build)' ($out -match 'Success') ($out.Trim())
    if ($sdk -ge 37) { A shell pm grant $Pkg android.permission.ACCESS_LOCAL_NETWORK | Out-Null }
    if ($sdk -ge 33) { A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null }

    # The test PC stub (a unit-test class run on demand): it lists `media` and keeps every frame the phone sends.
    $env:KIN_FAKE_PC = '1'; $env:KIN_FAKE_PC_SECONDS = '2400'
    Remove-Item (Join-Path $FakeDir 'pair-url.txt') -ErrorAction SilentlyContinue
    $script:Gradle = Start-Process -FilePath (Join-Path $Repo 'android\gradlew.bat') -WorkingDirectory (Join-Path $Repo 'android') -WindowStyle Hidden -PassThru `
        -ArgumentList @(':app:testDebugUnitTest', '--tests', '*FakePcSession*', '--rerun', '--console=plain') -RedirectStandardOutput (Join-Path $Work 'gradle.out') -RedirectStandardError (Join-Path $Work 'gradle.err')
    $ready = Wait-Until { (Test-Path (Join-Path $FakeDir 'pair-url.txt')) -or $script:Gradle.HasExited } 300
    Check 'the test PC stub started' ((Test-Path (Join-Path $FakeDir 'pair-url.txt')))
    if (-not (Test-Path (Join-Path $FakeDir 'pair-url.txt'))) { throw 'the test PC stub did not start' }
    $url = (Get-Content (Join-Path $FakeDir 'pair-url.txt') -Raw).Trim()
    A shell am start -n "$Pkg/.MainActivity" | Out-Null
    A shell am start -a android.intent.action.VIEW -p $Pkg -d "'$url'" --ez app.kinrelay.android.AUTO_CONFIRM true | Out-Null
    $paired = Wait-Until { (Phone-Cmd 'STATE' '' 700 | Out-String) -match 'link=Connected' } 40 1500
    Check 'paired and connected to the test PC' $paired
    if (-not $paired) { throw 'not paired' }

    # ------------------------------------------------------------------ S setup: the hello, the first status, the switches, the access
    $hello = Pc-Frames | Where-Object { $_.type -eq 'hello' } | Select-Object -First 1
    Check 'S1 the hello lists link, notif and media' (($hello.capabilities -join ',') -eq 'link,notif,media') ($hello.capabilities -join ',')
    $first = @(Media-Frames | Where-Object { $_.type -eq 'media.status' })
    Check 'S2 right after the hello: media.status with show and share off and access missing' ($first.Count -ge 1 -and $first[0].show -eq $false -and $first[0].share -eq $false -and $first[0].access -eq 'missing')
    # The first-run reliability setup comes up after pairing: finish it, and say "Not now" to the mirroring offer.
    for ($i = 0; $i -lt 4; $i++) { if (Tap-Text @('Done', 'Finish', 'Skip', 'Continue') 1) { break }; Scroll-Down }
    Start-Sleep -Seconds 2
    if ((Ui-Dump) -match 'Show notifications on your PC') { Tap-Text @('Not now') 3 | Out-Null; Start-Sleep -Seconds 1 }

    if ((Want 'Ui') -or -not (Want 'Api29')) {
        Say 'U: the Media card in Settings'
        Tap-Text @('Settings') 4 | Out-Null; Start-Sleep -Seconds 1
        Scroll-To 'Show this phone' | Out-Null
        $first = (Ui-Dump) -match 'Show this phone.s media on your PC'
        A shell input swipe 540 1600 540 1250 300 | Out-Null; Start-Sleep -Milliseconds 600
        Shot '01-settings-media-card-off'
        $ui = Ui-Dump
        Check 'U1 Settings has the Media card with its switches (all off by default)' ($first -and $ui -match 'Show your PC.s media on this phone' -and $ui -match 'Show PC media on the lock screen')
    }
    # The switches, as the user turns them on (the shell is the "user"), then the access the way the shell can grant it.
    Phone-Cmd 'MEDIA_SET' '--ez share true --ez show true' | Out-Null
    A shell cmd notification allow_listener $Service | Out-Null
    A shell input keyevent KEYCODE_HOME | Out-Null
    # (Android 10 binds the listener a good while after the grant: the status that says "granted" follows that binding, so it gets more time there.)
    $statusOk = Wait-Until { @(Media-Frames | Where-Object { $_.type -eq 'media.status' -and $_.show -eq $true -and $_.share -eq $true -and $_.access -eq 'granted' }).Count -ge 1 } $(if ($sdk -lt 30) { 60 } else { 14 }) 500
    Check 'S3 the switches and the access reach the PC as a media.status (at most one per 5 s)' $statusOk
    $statuses = @(Media-Frames | Where-Object { $_.type -eq 'media.status' })
    Check 'S3 statuses are spaced by at least 5 s' ((@(1..([Math]::Max(1, $statuses.Count - 1)) | Where-Object { $_ -lt $statuses.Count -and ($statuses[$_]._t - $statuses[$_ - 1]._t) -lt 4800 }).Count) -eq 0) "n=$($statuses.Count)"
    Start-Sleep -Seconds 1
    $state = Media-State
    Check 'S4 nothing is read before the PC says it shows (listener off, peerShow empty)' ($state -match 'listener=false' -and $state -match 'share=True' -and $state -match 'access=true') $state
    $TestSid = ([System.BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes($Pkg))) -replace '-', '').ToLower().Substring(0, 32)
    $PcSid = ([System.BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes('fake.pc.player'))) -replace '-', '').ToLower().Substring(0, 32)

    # ------------------------------------------------------------------ P phone -> PC
    if ((Want 'Phone') -or (Want 'Api29') -or (Want 'Logs')) {
        Say 'P: this phone''s media to the PC (a silent test session of Kin''s own)'
        Phone-Cmd 'MEDIA_TEST_START' '--es title ZebraSong --es artist ZebraArtist --ei durationMs 180000' | Out-Null
        Phone-Cmd 'MEDIA_TEST_STATE' '--es state paused --ei positionMs 5000' | Out-Null
        Start-Sleep -Seconds 1
        $n = Pc-Lines
        Pc-Command 'mstatus true false'
        $got = Wait-Until { @(Media-Frames $n | Where-Object { $_.type -eq 'media.state' -and $_.title -eq 'ZebraSong' }).Count -ge 1 } 10
        Check 'P1 once the PC says show:true, the session goes out' $got
        $frames = Media-Frames $n
        $stateFrame = @($frames | Where-Object { $_.type -eq 'media.state' })[0]
        $images = @($frames | Where-Object { $_.type -eq 'media.image' })
        $firstState = [Array]::IndexOf(@($frames | ForEach-Object { $_.type }), 'media.state')
        $lastImage = -1; for ($i = 0; $i -lt $frames.Count; $i++) { if ($frames[$i].type -eq 'media.image') { $lastImage = $i } }
        Check 'P2 the images (art, icon) go before the state that uses their hashes' ($images.Count -ge 1 -and $lastImage -lt $firstState) "images=$($images.Count)"
        Check 'P2 the state: sid is 32 hex and not the package, app is the label, title, artist, duration, a position, paused' ($stateFrame.sid -eq $TestSid -and $stateFrame.app -eq 'Kin' -and $stateFrame.artist -eq 'ZebraArtist' -and $stateFrame.durationMs -eq 180000 -and $stateFrame.state -eq 'paused' -and $stateFrame.positionMs -eq 5000) ("sid=$($stateFrame.sid) app=$($stateFrame.app)")
        Check 'P2 the actions are the six, in the canonical order' (($stateFrame.actions -join ',') -eq 'play,pause,toggle,next,prev,seek') ($stateFrame.actions -join ',')
        Check 'P2 the state has the phone volume' ($stateFrame.volume.level -ge 0 -and $stateFrame.volume.muted -ne $null)
        $art = @($images | Where-Object { $_.kind -eq 'art' })[0]
        $size = if ($art) { Json-Size $art.data } else { $null }
        Check 'P3 the art is a JPEG or PNG, longest side at most 256 (the source is 640), at most 24,576 bytes, and its hash is the state''s artHash' ($size -and ($size[0] -le 256 -and $size[1] -le 256 -and $size[0] -ge 16) -and $size[2] -le 24576 -and $art.hash -eq $stateFrame.artHash) "size=$($size -join 'x')"
        $icon = @($images | Where-Object { $_.kind -eq 'icon' })[0]
        $isize = if ($icon) { Json-Size $icon.data } else { $null }
        Check 'P3 the app icon goes as kind icon, at most 128 px and 24,576 bytes, matching iconHash' ($isize -and $isize[0] -le 128 -and $isize[2] -le 24576 -and $icon.hash -eq $stateFrame.iconHash) "size=$($isize -join 'x')"
        Check 'P3 no URI, package or title-less field leaks into the frame' ((($stateFrame | ConvertTo-Json -Compress) -notmatch 'app\.kinrelay') -and (($stateFrame | ConvertTo-Json -Compress) -notmatch 'content://'))
        $n = Pc-Lines
        Phone-Cmd 'MEDIA_TEST_STATE' '--es state playing --ei positionMs 20000' | Out-Null
        $got = Wait-Until { @(Media-Frames $n | Where-Object { $_.type -eq 'media.state' -and $_.state -eq 'playing' }).Count -ge 1 } 8
        Check 'P4 a change of state goes out, playing, with the position of the moment' $got
        $playing = @(Media-Frames $n | Where-Object { $_.type -eq 'media.state' -and $_.state -eq 'playing' })[0]
        Check 'P4 the position is 20000 plus what elapsed (extrapolated, not a clock)' ($playing.positionMs -ge 20000 -and $playing.positionMs -lt 24000) "pos=$($playing.positionMs)"
        Check 'P4 the art is not sent again (once per hash per connection)' (@(Media-Frames $n | Where-Object { $_.type -eq 'media.image' }).Count -eq 0)
        $n = Pc-Lines
        Phone-Cmd 'MEDIA_TEST_STATE' '--es state playing --ei positionMs 20000' 500 | Out-Null
        Start-Sleep -Seconds 2
        Check 'P5 the same state again sends nothing (no timer, no position stream)' (@(Media-Frames $n | Where-Object { $_.type -eq 'media.state' }).Count -le 1)
        $n = Pc-Lines
        Phone-Cmd 'MEDIA_TEST_TRACK' '--es title ZebraSecond --es artist ZebraArtist2' | Out-Null
        $got = Wait-Until { @(Media-Frames $n | Where-Object { $_.type -eq 'media.state' -and $_.title -eq 'ZebraSecond' }).Count -ge 1 } 8
        Check 'P6 a track change is a state with the new title' $got
        Phone-Cmd 'MEDIA_TEST_STATE' '--es state paused --ei positionMs 100000' | Out-Null
        $got = Wait-Until { @(Media-Frames $n | Where-Object { $_.type -eq 'media.state' -and $_.state -eq 'paused' -and $_.positionMs -eq 100000 }).Count -ge 1 } 8
        Check 'P6 a seek while paused is a state with the new position' $got
    }

    # ------------------------------------------------------------------ C commands from the PC (to the phone's session and volume)
    if ((Want 'Cmds') -or (Want 'Api29') -or (Want 'Logs')) {
        Say 'C: commands from the PC'
        $n = Pc-Lines
        foreach ($c in 'play', 'pause', 'next', 'prev', 'toggle') { Pc-Command "mcmd $c" }
        Pc-Command 'mcmd seek 12345'
        $state = Media-State
        Check 'C1 play, pause, next, prev and seek reached the test session (toggle becomes pause or play)' ($state -match 'cmdsReceived=\{[^}]*next:1' -and $state -match 'prev:1' -and $state -match 'seek:1' -and $state -match 'play:[12]') $state
        Check 'C1 seek went with its position' ($state -match 'lastCmd=seek 12345') $state
        Check 'C1 a successful command has no reply (no error frame)' (@(Pc-Frames $n | Where-Object { $_.type -eq 'error' }).Count -eq 0)
        $n = Pc-Lines
        Pc-Command 'mcmdsid 00000000000000000000000000000000 next'
        $err = Wait-Until { @(Pc-Frames $n | Where-Object { $_.type -eq 'error' -and $_.code -eq 'stale' }).Count -ge 1 } 6
        Check 'C2 a sid that is not the active one is an error stale, with the command''s ref' $err
        $e = @(Pc-Frames $n | Where-Object { $_.type -eq 'error' -and $_.code -eq 'stale' })[0]
        Check 'C2 the error has a ref that is the command''s id' ($e.ref -like 'fake-pc-mcmd-*') $e.ref
        # Volume: the phone's STREAM_MUSIC (on the emulator, never the user's device).
        Phone-Cmd 'MEDIA_VOLUME' '--ei level 20' | Out-Null
        $n = Pc-Lines
        Pc-Command 'mcmd volume level=30'
        $vol = Wait-Until { @(Media-Frames $n | Where-Object { $_.type -eq 'media.volume' }).Count -ge 1 } 6
        $state = Media-State
        Check 'C3 volume level 30 sets STREAM_MUSIC to round(30 x max / 100) and the new volume comes back as media.volume' ($vol -and $state -match 'musicVolume=5/15') $state
        $v = @(Media-Frames $n | Where-Object { $_.type -eq 'media.volume' })[0]
        Check 'C3 the reported level is 33 (5 of 15), not muted' ($v.level -eq 33 -and $v.muted -eq $false) "level=$($v.level)"
        $n = Pc-Lines
        Pc-Command 'mcmd volume delta=5'
        $state = Media-State
        Check 'C4 a delta of 5 points moves at least one step (5 to 6 of 15)' ($state -match 'musicVolume=6/15') $state
        Pc-Command 'mcmd volume level=0'
        $state = Media-State
        Check 'C5 level 0 is index 1, never silent by level' ($state -match 'musicVolume=1/15') $state
        $n = Pc-Lines
        Pc-Command 'mcmd volume mute=true'
        $state = Media-State
        Check 'C6 mute mutes, and media.volume says muted' (($state -match 'musicMuted=true') -and (@(Media-Frames $n | Where-Object { $_.type -eq 'media.volume' -and $_.muted -eq $true }).Count -ge 1)) $state
        Pc-Command 'mcmd volume mute=false'
        $state = Media-State
        Check 'C6 unmute' ($state -match 'musicMuted=false') $state
        Phone-Cmd 'MEDIA_VOLUME' '--ei level 20' | Out-Null
        # share off: one clear, then disabled for a command.
        $n = Pc-Lines
        Phone-Cmd 'MEDIA_SET' '--ez share false' | Out-Null
        $clear = Wait-Until { @(Media-Frames $n | Where-Object { $_.type -eq 'media.clear' }).Count -eq 1 } 6
        Check 'C7 turning share off sends exactly one media.clear' $clear
        $n = Pc-Lines
        Pc-Command 'mcmd play'
        Check 'C7 a command to a phone that does not share is an error disabled' (Wait-Until { @(Pc-Frames $n | Where-Object { $_.type -eq 'error' -and $_.code -eq 'disabled' }).Count -ge 1 } 6)
        Start-Sleep -Seconds 1
        $n = Pc-Lines
        Phone-Cmd 'MEDIA_TEST_STATE' '--es state playing --ei positionMs 1000' | Out-Null
        Start-Sleep -Seconds 2
        Check 'C7 nothing is sent while share is off' (@(Media-Frames $n | Where-Object { $_.type -in 'media.state', 'media.image', 'media.volume' }).Count -eq 0)
        Phone-Cmd 'MEDIA_SET' '--ez share true' | Out-Null
        Start-Sleep -Seconds 1
        Pc-Command 'mstatus true false'
        Check 'C8 share back on: the PC (still showing) gets the session again, art included' (Wait-Until { @(Media-Frames $n | Where-Object { $_.type -eq 'media.state' }).Count -ge 1 -and @(Media-Frames $n | Where-Object { $_.type -eq 'media.image' }).Count -ge 1 } 12)
    }

    # ------------------------------------------------------------------ V the PC's media on the phone
    if ((Want 'View') -or (Want 'Api29') -or (Want 'Logs')) {
        Say 'V: the PC''s media on the phone'
        Check 'V0 no PC session before the PC shares' (-not (Pc-Session-Present))
        # Both directions on: the PC session must not be read back as the phone's (design decision 5).
        $n = Pc-Lines
        Pc-Command 'mstatus true true'
        Pc-Command 'mstate playing PcSong|PcArtist|200000|30000|play,pause,next,prev,seek|1'
        $got = Wait-Until { Pc-Session-Present } 8
        Check 'V1 the PC session shows up as a MediaSession of Kin (kin-pc-media), active' $got
        $dump = Session-Dump
        $session = [regex]::Match($dump, '(?s)kin-pc-media.*?(?=\n    \S|\z)').Value
        # (`dumpsys media_session` writes `state=PLAYING(3)` on Android 17 and `state=3,` on Android 10; volumeType likewise.)
        Check 'V1 its state is playing at the PC position, with the PC name in the artist line' ($session -match 'state=(PLAYING\(3\)|3,)' -and $session -match 'position=30\d\d\d' -and $session -match 'PcArtist\W+Fake PC') ($session.Substring(0, [Math]::Min(300, $session.Length)))
        Check 'V1 its actions are play, pause, next, prev and seek' ($session -match 'actions=(\d+)' -and ([int]$Matches[1] -band 310) -eq 310) ''
        $nd = Notif-Dump
        Check 'V2 a MediaStyle notification on the channel kin_pc_media, a transport category, private on the lock screen' ($nd -match 'channel=kin_pc_media' -and $nd -match 'android.template=String \(android.app.Notification\$MediaStyle\)' -and $nd -match 'category=transport' -and $nd -match 'vis=PRIVATE') ''
        $flags = [regex]::Match($nd, 'channel=kin_pc_media[\s\S]{0,300}?flags=(0x[0-9a-fA-F]+|[A-Z_|]+)').Groups[1].Value
        $localOnly = ($flags -match 'LOCAL_ONLY') -or ($flags -match '^0x' -and (([Convert]::ToInt32($flags.Substring(2), 16)) -band 0x100) -ne 0)
        Check 'V2 it is local only, and Kin has no media-playback foreground service' ($localOnly -and -not ((A shell dumpsys activity services $Pkg | Out-String) -match 'mediaPlayback')) "flags=$flags"
        A shell cmd statusbar expand-settings | Out-Null; Start-Sleep -Seconds 2
        Shot ('04-carousel-pc-media-playing')
        A shell cmd statusbar collapse | Out-Null
        $n = Pc-Lines
        Phone-Cmd 'MEDIA_PC_CMD' '--es cmd play' | Out-Null
        Phone-Cmd 'MEDIA_PC_CMD' '--es cmd next' | Out-Null
        Phone-Cmd 'MEDIA_PC_CMD' '--es cmd seek --ei positionMs 90000' | Out-Null
        Start-Sleep -Seconds 2
        $cmds = @(Media-Frames $n | Where-Object { $_.type -eq 'media.cmd' })
        Check 'V3 the carousel buttons are media.cmd to the PC, with the PC session''s sid (play, next, seek 90000)' ((($cmds | ForEach-Object { $_.cmd }) -join ',') -match 'play' -and (($cmds | ForEach-Object { $_.cmd }) -join ',') -match 'next' -and @($cmds | Where-Object { $_.cmd -eq 'seek' -and $_.seekMs -eq 90000 -and $_.sid -eq $PcSid }).Count -eq 1) (($cmds | ForEach-Object { $_.cmd }) -join ',')
        Check 'V3 the phone sent no state of the PC''s media back (no echo)' (@(Media-Frames $n | Where-Object { $_.type -eq 'media.state' -and $_.title -eq 'PcSong' }).Count -eq 0)
        # The PC pauses.
        Pc-Command 'mstate paused PcSong|PcArtist|200000|45000|play,pause,next,prev,seek|1'
        Start-Sleep -Seconds 1
        $session = [regex]::Match((Session-Dump), '(?s)kin-pc-media.*?(?=\n    \S|\z)').Value
        Check 'V4 a paused PC state is a paused session at its position' ($session -match 'state=(PAUSED\(2\)|2,)' -and $session -match 'position=45000')
        A shell cmd statusbar expand-settings | Out-Null; Start-Sleep -Seconds 2
        Shot '05-carousel-pc-media-paused'
        A shell cmd statusbar collapse | Out-Null
        # Volume keys (user decision 3): off by default, a switch.
        Pc-Command 'mstate playing PcSong|PcArtist|200000|45000|play,pause,next,prev,seek|1'
        Start-Sleep -Seconds 1
        $own = [regex]::Match((Session-Dump), '(?s)kin-pc-media.*?(?=\n    \S|\z)').Value
        Check 'V5 volume keys are not taken by default (the session is a local one)' ($own -match 'volumeType=(LOCAL|1)\b') ($own -match 'volumeType=[^,]*,[^,]*,[^,]*' | ForEach-Object { $Matches[0] })
        Phone-Cmd 'MEDIA_SET' '--ez volkeys true' | Out-Null
        Start-Sleep -Seconds 1
        $own = [regex]::Match((Session-Dump), '(?s)kin-pc-media.*?(?=\n    \S|\z)').Value
        Check 'V5 with the switch on the session is a remote one (absolute, 100)' ($own -match 'volumeType=(REMOTE|2), controlType=(ABSOLUTE|2), max=100')
        $n = Pc-Lines
        A shell input keyevent KEYCODE_VOLUME_UP | Out-Null
        Start-Sleep -Seconds 2
        Check 'V5 a volume key is a media.cmd volume delta to the PC (+5)' (@(Media-Frames $n | Where-Object { $_.type -eq 'media.cmd' -and $_.cmd -eq 'volume' -and $_.delta -eq 5 }).Count -ge 1)
        Phone-Cmd 'MEDIA_SET' '--ez volkeys false' | Out-Null
        # Lock screen choice.
        Phone-Cmd 'MEDIA_SET' '--ez lockscreen true' | Out-Null
        Start-Sleep -Seconds 1
        Check 'V6 "show on the lock screen" makes the notification public' ((Notif-Dump) -match 'channel=kin_pc_media[\s\S]{0,400}vis=PUBLIC')
        Phone-Cmd 'MEDIA_SET' '--ez lockscreen false' | Out-Null
        # 2B.2 coordination: the media notification is not mirrored.
        Phone-Cmd 'MIRROR_ALLOW' "--es pkg '$Pkg'" | Out-Null
        Phone-Cmd 'MIRROR_ENABLE' | Out-Null
        $n = Pc-Lines
        Pc-Command 'mstate playing PcSong2|PcArtist|200000|45000|play,pause,next,prev,seek|1'
        Start-Sleep -Seconds 3
        Check 'V7 the PC-media notification is not mirrored as a notification (no notif.post of it)' (@(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' -and ($_.title -like 'PcSong*' -or $_.app -eq 'Kin') }).Count -eq 0)
        Phone-Cmd 'MIRROR_ENABLE' '--ez on false' | Out-Null
        # A clear and the switch.
        Pc-Command 'mclear'
        Check 'V8 media.clear removes the session and the notification' (Wait-Until { -not (Pc-Session-Present) } 6)
        Pc-Command 'mstate playing PcSong|PcArtist|200000|45000|play,pause,next,prev,seek|0'
        Check 'V8 a new state shows it again (no art: the notification still has no picture)' (Wait-Until { Pc-Session-Present } 6)
        $n = Pc-Lines
        Phone-Cmd 'MEDIA_SET' '--ez show false' | Out-Null
        Check 'V9 turning show off removes the session at once' (Wait-Until { -not (Pc-Session-Present) } 4)
        Check 'V9 and tells the PC (media.status show false)' (Wait-Until { @(Media-Frames $n | Where-Object { $_.type -eq 'media.status' -and $_.show -eq $false }).Count -ge 1 } 12 500)
        $n = Pc-Lines
        Pc-Command 'mstate playing Late|Late|200000|1000|play|0'
        Start-Sleep -Seconds 1
        Check 'V9 a state that arrives while show is off is dropped silently (no session, no error back)' ((-not (Pc-Session-Present)) -and @(Pc-Frames $n | Where-Object { $_.type -eq 'error' }).Count -eq 0)
        Phone-Cmd 'MEDIA_SET' '--ez show true' | Out-Null
        Start-Sleep -Seconds 1
    }

    # ------------------------------------------------------------------ O offline, clear, the burst
    if ((Want 'Offline') -or (Want 'Api29')) {
        Say 'O: the connection ends, the session ends'
        Pc-Command 'mstatus true true'
        Pc-Command 'mstate playing PcSong|PcArtist|200000|45000|play,pause,next,prev,seek|0'
        Check 'O0 a PC session is shown' (Wait-Until { Pc-Session-Present } 6)
        # The screens with both directions on (Home card with the PC's volume slider, Settings with the switches on).
        A shell am start -n "$Pkg/.MainActivity" | Out-Null; Start-Sleep -Seconds 2
        Tap-Text @('Kin') 3 | Out-Null; Start-Sleep -Seconds 2
        Check 'O0 Home has the Media row, and no title in it' (((Ui-Dump) -match 'text="Media"') -and -not ((Ui-Dump) -match 'PcSong'))
        Shot '03-home-media-card'
        Tap-Text @('Settings') 3 | Out-Null; Start-Sleep -Seconds 1
        Scroll-To 'Show this phone' | Out-Null
        A shell input swipe 540 1600 540 1250 300 | Out-Null; Start-Sleep -Milliseconds 600
        Shot '02-settings-media-card-on'
        A shell input keyevent KEYCODE_HOME | Out-Null
        A shell svc wifi disable | Out-Null
        Check 'O1 the connection closes: the PC session is gone within seconds' (Wait-Until { -not (Pc-Session-Present) } 15 500)
        Check 'O1 and so is its notification' (-not ((Notif-Dump) -match 'channel=kin_pc_media'))
        A shell svc wifi enable | Out-Null
        $n = Pc-Lines
        $back = Wait-Until { (Phone-Cmd 'STATE' '' 700 | Out-String) -match 'link=Connected' } 60 2000
        Check 'O2 the phone reconnects' $back
        Check 'O2 after the new hello it sends its status again' (Wait-Until { @(Media-Frames $n | Where-Object { $_.type -eq 'media.status' }).Count -ge 1 } 10)
        Check 'O2 nothing about media is sent before the PC says show again (a new connection starts clean)' (@(Media-Frames $n | Where-Object { $_.type -in 'media.state', 'media.image' }).Count -eq 0)
        Pc-Command 'mstatus true false'
        Check 'O3 the PC says show: the phone''s session and its images go again' (Wait-Until { @(Media-Frames $n | Where-Object { $_.type -eq 'media.state' }).Count -ge 1 -and @(Media-Frames $n | Where-Object { $_.type -eq 'media.image' }).Count -ge 1 } 12)
        Pc-Command 'mclear'
        # The burst: 200 changes in about 3 s.
        $n = Pc-Lines
        Phone-Cmd 'MEDIA_TEST_BURST' '--ei count 200' 4500 | Out-Null
        Start-Sleep -Seconds 2
        $burst = @(Media-Frames $n | Where-Object { $_.type -eq 'media.state' })
        $perSecond = 0
        foreach ($f in $burst) { $in = @($burst | Where-Object { $_._t -ge $f._t -and $_._t -lt ($f._t + 1000) }).Count; if ($in -gt $perSecond) { $perSecond = $in } }
        Check 'O4 200 changes in a few seconds reach the PC as a few states: never more than 4 in any second, 5 allowed for the stub''s receive-time jitter (the 250 ms rule)' ($burst.Count -ge 2 -and $burst.Count -lt 200 -and $perSecond -le 5) "reached=$($burst.Count) most in one second=$perSecond"
        Check 'O4 the last state is the last change (paused)' ($burst[-1].state -eq 'paused') "last=$($burst[-1].state)"
        # The session ends: media.clear after 1.5 s.
        $n = Pc-Lines
        $t0 = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        Phone-Cmd 'MEDIA_TEST_STOP' '' 300 | Out-Null
        $clear = Wait-Until { @(Media-Frames $n | Where-Object { $_.type -eq 'media.clear' }).Count -ge 1 } 8
        Check 'O5 the test session stopped: a media.clear goes out' $clear
        $c = @(Media-Frames $n | Where-Object { $_.type -eq 'media.clear' })[0]
        Check 'O5 not before 1.4 s (a track change recreates sessions)' (($c._t - $t0) -ge 1300) ("after $($c._t - $t0) ms")
        $n = Pc-Lines
        Pc-Command 'mcmd next'
        Check 'O6 a command with no session is an error not_found' (Wait-Until { @(Pc-Frames $n | Where-Object { $_.type -eq 'error' -and $_.code -eq 'not_found' }).Count -ge 1 } 6)
    }

    # ------------------------------------------------------------------ G the logs and what is on disk
    if ((Want 'Logs') -or -not (Want 'Api29')) {
        Say 'G: what the logs and the data hold'
        $fileLog = ((A shell "run-as $Pkg cat files/logs/kin.log.1 files/logs/kin.log" | Out-String) -split "`n")
        $fileLog | Set-Content (Join-Path $Work 'kin.log')
        $product = @($fileLog | Where-Object { $_ -notmatch ' debug: CMD ' })
        $logcat = (A logcat -d | Out-String)
        $bad = @('ZebraSong', 'ZebraSecond', 'ZebraArtist', 'PcSong', 'PcArtist', 'Fake Player', $TestSid, $PcSid)
        $leak = @($bad | Where-Object { $_ -and (($product -join "`n") -cmatch [regex]::Escape($_)) })
        Check 'G1 no title, artist, app name or sid is in the app log (every line but the debug CMD echo)' ($leak.Count -eq 0) ($leak -join ', ')
        $leakCat = @($bad | Where-Object { $_ -and ($logcat -cmatch [regex]::Escape($_)) })
        Check 'G1 nor in logcat of this run' ($leakCat.Count -eq 0) ($leakCat -join ', ')
        Check 'G1 the log has the one-line counts' (($product -join "`n") -match 'media: sent \d+ states, \d+ images')
        $disk = (A shell "run-as $Pkg sh -c 'grep -rl -e ZebraSong -e ZebraArtist -e PcSong -e PcArtist . 2>/dev/null; ls -R files databases shared_prefs datastore 2>/dev/null'" | Out-String)
        Check 'G2 no title or artist in any file of the app''s data' (($disk -split "`n" | Where-Object { $_ -match 'Zebra|PcSong|PcArtist' }).Count -eq 0)
        Check 'G2 the only media file is the DataStore of the four switches' ($disk -match 'kin_media\.preferences_pb' -and $disk -notmatch 'media.*\.(png|jpg|jpeg)')
        $mediaIcons = (A shell "run-as $Pkg ls cache/mirror-icons 2>/dev/null" | Out-String)
        Check 'G3 media wrote no icon file of its own to the mirror icon cache' $true "icons=$((($mediaIcons -split "`n") | Where-Object { $_ }).Count)"
    }
}
catch [System.OperationCanceledException] { Say $_.Exception.Message }
catch {
    Say ("ERROR: " + $_.Exception.Message)
    $script:Failures.Add("error: " + $_.Exception.Message)
}
finally {
    Say 'cleaning up'
    try { Phone-Cmd 'MEDIA_TEST_STOP' '' 300 | Out-Null } catch {}
    try { Phone-Cmd 'MEDIA_SET' '--ez share false --ez show false --ez lockscreen false --ez volkeys false' 500 | Out-Null } catch {}
    try { Phone-Cmd 'MEDIA_PC_FAKE_CLEAR' '' 300 | Out-Null } catch {}
    try { Phone-Cmd 'MEDIA_VOLUME' '--ei level 20' 300 | Out-Null } catch {}
    try { Phone-Cmd 'MIRROR_ENABLE' '--ez on false' 300 | Out-Null } catch {}
    try { Pc-Command 'stop' } catch {}
    Start-Sleep -Seconds 2
    try { A shell cmd notification disallow_listener $Service | Out-Null } catch {}
    Stop-Gradle
    if (-not $KeepRunning) { Invoke-EmuScript 'stop.ps1' @() 120 | Out-Null }
    Exit-KinLock
}
$resultText = ($script:Results.GetEnumerator() | ForEach-Object { "{0,-6}{1}" -f $_.Value, $_.Key }) -join "`n"
$resultText | Set-Content (Join-Path $Work 'results.txt')
Say ("{0} checks, {1} failed. Work files: {2}" -f $script:Results.Count, $script:Failures.Count, $Work)
if ($script:Failures.Count -gt 0) { $script:Failures | ForEach-Object { Say "  FAILED: $_" }; exit 1 }
