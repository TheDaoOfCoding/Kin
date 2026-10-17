# 2B.2 on the phone (notification mirroring, phone -> PC), verified ONCE on an emulator: the real debug APK against the test PC stub of the unit-test tree
# (android\app\src\test\...\FakePcSession.kt: a PC that lists `notif`, keeps every frame the phone sends, and sends notif.dismiss / notif.action on command).
# The real desktop (Kin.exe) is not used here: this script is the phone side; the desktop round trip is the integration agent's tools\e2e-2b-2.ps1.
#
#   android\tools\verify-2b2.ps1 -ScreenshotDir docs\results\phase-2b-2-android-screens
#   android\tools\verify-2b2.ps1 -Avd kin_api29 -Only Api29      # the Android 10 path (a quick check: post, reply, button, dismiss)
#
# What it checks: the setup screens (screenshots), notification access granted with `cmd notification allow_listener`, the status and sync frames after the
# hello, notifications posted by `cmd notification post` and by the debug "Mirror test" posts (MessagingStyle with reply and buttons, a code, ongoing, group
# summary, hidden on the lock screen, low importance, a redacted look-alike) and what reaches the PC stub, update and remove, reply / button / stale /
# activity action / dismiss from the PC (no echo), offline and sync after a capability change, the rate limit (100 posts in 3 s), what the log holds, and
# what an Android 15+ image does to an obvious one-time code. Never touches the user's installed Kin or anything of the PC: no Kin.exe, no clipboard, no admin.
param(
    [string]$Avd = 'kin_api37',
    [string]$Apk = (Join-Path $PSScriptRoot '..\app\build\outputs\apk\debug\app-debug.apk'),
    [string[]]$Only = @(),
    [string]$ScreenshotDir = '',
    [switch]$KeepRunning,
    [string]$Work = ''
)
. (Join-Path $PSScriptRoot '..\..\tools\kin-lock.ps1'); Enter-KinLock 'verify-2b2'
if (-not $Work) { $Work = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-verify-2b2-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
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
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Results = [ordered]@{}
$script:Gradle = $null

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
    $out = Join-Path $env:TEMP "kin-verify-2b2-$Script.out"
    $p = Start-Process powershell -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $Emu $Script))) + $ScriptArgs)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; return $false }
    return $true
}
function Shot([string]$Name) {
    if (-not $ScreenshotDir) { return }
    New-Item -ItemType Directory -Force $ScreenshotDir | Out-Null
    Invoke-EmuScript 'screenshot.ps1' @('-Out', ('"{0}"' -f (Join-Path $ScreenshotDir "$Name.png"))) 60 | Out-Null
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
function Tap-Regex([string]$Pattern, [int]$Tries = 4) {
    for ($try = 0; $try -lt $Tries; $try++) {
        $x = Ui-Dump
        if ($x -match ('text="({0})"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"' -f $Pattern)) {
            A shell input tap ([int](([int]$Matches[2] + [int]$Matches[4]) / 2)) ([int](([int]$Matches[3] + [int]$Matches[5]) / 2)) | Out-Null
            return $true
        }
        Start-Sleep -Milliseconds 700
    }
    return $false
}
function Scroll-Down { A shell input swipe 540 1900 540 700 300 | Out-Null; Start-Sleep -Milliseconds 600 }
function Scroll-To([string]$Pattern, [int]$Max = 8) {
    for ($i = 0; $i -lt $Max; $i++) {
        if ((Ui-Dump) -match $Pattern) { return $true }
        A shell input swipe 540 1700 540 1000 300 | Out-Null; Start-Sleep -Milliseconds 500
    }
    return $false
}
function Scroll-Up { A shell input swipe 540 700 540 1900 300 | Out-Null; Start-Sleep -Milliseconds 600 }
function Phone-Cmd([string]$Command, [string]$Extras = '', [int]$Wait = 1200) {
    A logcat -c | Out-Null
    A shell "am broadcast -a app.kinrelay.android.debug.$Command -n $Pkg/.debug.DebugCommandReceiver $Extras" | Out-Null
    Start-Sleep -Milliseconds $Wait
    @(A logcat -d -s KinRelay | Where-Object { $_ -match 'CMD' } | ForEach-Object { ($_ -replace '^.*CMD ', 'CMD ') })
}
function Phone-State { (Phone-Cmd 'MIRROR_STATE' '' 800 | Where-Object { $_ -match 'CMD MIRROR_STATE' } | Select-Object -Last 1) }
function Pc-Command([string]$Line) { Set-Content -Path $CommandFile -Value $Line -Encoding ASCII; Start-Sleep -Milliseconds 1500 }
# The frames the PC stub got from the phone after line $From of its events file, as objects (the png is cut: it is long).
function Pc-Frames([int]$From = 0) {
    @(Get-Content $Events -ErrorAction SilentlyContinue | Select-Object -Skip $From | Where-Object { $_ -match 'frame from phone: ' } |
        ForEach-Object { try { ($_ -replace '^\d+ frame from phone: ', '') | ConvertFrom-Json } catch { $null } } | Where-Object { $_ })
}
function Pc-Lines { @(Get-Content $Events -ErrorAction SilentlyContinue).Count }
# The active notifications part of `dumpsys notification` (the dump also lists the archive of dismissed ones, and `cmd notification list` is not on Android 10).
function Active-Notifs {
    $d = (A shell dumpsys notification --noredact | Out-String)
    $i = $d.IndexOf('Notification List:')
    if ($i -lt 0) { return $d }
    $m = [regex]::Match($d.Substring($i), '(?m)^\s*(Archive|mArchive|Snoozed|Archived)')
    if ($m.Success) { return $d.Substring($i, $m.Index) } else { return $d.Substring($i) }
}
function Post-Shell([string]$Tag, [string]$Text, [string]$Title = 'Example', [string]$Style = '') { A shell "cmd notification post -t '$Title' $Style $Tag '$Text'" | Out-Null }
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

    # The test PC stub (a unit-test class run on demand): it lists `notif` and keeps every frame the phone sends.
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
    $hello = Pc-Frames | Where-Object { $_.type -eq 'hello' } | Select-Object -First 1
    Check 'the hello lists link and notif' (($hello.capabilities -join ',') -eq 'link,notif') ($hello.capabilities -join ',')
    $first = Pc-Frames | Where-Object { $_.type -like 'notif.*' }
    Check 'after the hello: notif.status (access missing, off) then notif.sync (empty)' ($first.Count -ge 2 -and $first[0].type -eq 'notif.status' -and $first[0].access -eq 'missing' -and $first[0].enabled -eq $false -and $first[1].type -eq 'notif.sync' -and @($first[1].nids).Count -eq 0)
    # The first-run reliability setup comes up after pairing: finish it.
    for ($i = 0; $i -lt 4; $i++) { if (Tap-Text @('Done', 'Finish', 'Skip', 'Continue') 1) { break }; Scroll-Down }
    Start-Sleep -Seconds 2
    # Notification mirroring is offered once, right after the reliability setup: this run says "Not now" and opens it from Settings later.
    $offered = (Ui-Dump) -match 'Show notifications on your PC'
    Check 'the mirroring setup is offered once, right after the first setup' $offered
    if ($offered) { Shot '00-offered-after-setup'; Tap-Text @('Not now') 3 | Out-Null; Start-Sleep -Seconds 1 }

    # ------------------------------------------------------------------ U the screens (no access yet)
    if (Want 'Ui') {
        Say 'U: Settings card, setup steps, picker, restricted settings help'
        Tap-Text @('Settings') 4 | Out-Null; Start-Sleep -Seconds 1
        Scroll-To 'Show notifications on your PC' | Out-Null
        Shot '01-settings-card-off'
        Check 'U1 Settings has "Show notifications on your PC"' ((Ui-Dump) -match 'Show notifications on your PC')
        Check 'U1 tap "Set up notification access"' (Tap-Text @('Set up notification access') 6)
        Start-Sleep -Seconds 1; Shot '02-setup-1-what-it-does'
        Check 'U2 step 1 says what it does and that nothing is stored' ((Ui-Dump) -match 'Kin keeps none of it')
        Check 'U2 tap "Choose apps"' (Tap-Text @('Choose apps'))
        Start-Sleep -Seconds 2; Shot '03-setup-2-pick-apps'
        $ui = Ui-Dump
        Check 'U3 the picker lists the apps with a search box and the sections' ($ui -match 'Search apps' -and $ui -match 'Other apps|Messaging apps')
        Check 'U3 tap Next' (Tap-Regex 'Next \(\d+ apps?\)')
        Start-Sleep -Seconds 1; Shot '04-setup-3-give-access'
        Check 'U4 step 3 has the "Open notification access" button' ((Ui-Dump) -match 'Open notification access')
        Check 'U4 tap it (the system opens the Notification access screen)' (Tap-Text @('Open notification access'))
        Start-Sleep -Seconds 2; Shot '05-system-notification-access'
        A shell input keyevent KEYCODE_BACK | Out-Null; Start-Sleep -Seconds 2
        $ui = Ui-Dump
        Check 'U5 back with no access: the Restricted setting steps are shown' ($ui -match 'Notification access isn.t on yet' -or $ui -match 'Restricted setting')
        Shot '06-setup-4-restricted-settings-help'
        Scroll-Down; Shot '06b-setup-4-restricted-settings-help-2'
        # Grant it the way the shell can (the user would use the system screen), come back, finish.
        A shell cmd notification allow_listener $Service | Out-Null
        Start-Sleep -Seconds 2
        A shell am start -n "$Pkg/.MainActivity" | Out-Null; Start-Sleep -Seconds 2
        Check 'U6 granted: "Turn it on" is offered' (Wait-Until { (Ui-Dump) -match 'Turn it on' } 8)
        Shot '07-setup-4-done'
        Check 'U6 tap "Turn it on"' (Tap-Text @('Turn it on'))
        Start-Sleep -Seconds 2
        $state = Phone-State
        Check 'U6 mirroring is on, access granted, the listener is connected' ($state -match 'enabled=true' -and $state -match 'access=true' -and $state -match 'listener=true') $state
        Tap-Text @('Settings') 4 | Out-Null; Start-Sleep -Seconds 1; Scroll-To 'Mirror notifications' | Out-Null
        Shot '08-settings-card-on'
        Check 'U7 Settings says it is on and for how many apps' ((Ui-Dump) -match 'On for \d+ apps?')
        Scroll-To 'Apps \(\d+ allowed\)' | Out-Null
        Check 'U7 tap Apps' (Tap-Regex 'Apps \(\d+ allowed\)')
        Start-Sleep -Seconds 2; Shot '09-apps-list'
        A shell input keyevent KEYCODE_BACK | Out-Null; Start-Sleep -Seconds 1
        Tap-Text @('Kin') 3 | Out-Null; Start-Sleep -Seconds 1; Shot '10-home-mirroring-row'
        # The shell is the other source of notifications in the tests.
        Phone-Cmd 'MIRROR_ALLOW' "--es pkg 'com.android.shell,$Pkg'" | Out-Null
        Phone-Cmd 'MIRROR_ENABLE' | Out-Null
    } else {
        A shell cmd notification allow_listener $Service | Out-Null
        Phone-Cmd 'MIRROR_ALLOW' "--es pkg 'com.android.shell,$Pkg'" | Out-Null
        Phone-Cmd 'MIRROR_ENABLE' | Out-Null
    }
    A shell input keyevent KEYCODE_HOME | Out-Null
    # A notification posted before the listener is connected is not delivered to it (and is not mirrored: a stale toast is worse than none): wait for the binding.
    Wait-Until { (Phone-State) -match 'listener=true' } 20 500 | Out-Null
    Start-Sleep -Seconds 2
    $state = Phone-State
    Phone-Cmd 'MIRROR_CLEAR_ALL' | Out-Null   # an earlier run may have left the shell at its 50-notification limit
    Check 'access, listener and the PC capability are all in place' ($state -match 'access=true' -and $state -match 'listener=true' -and $state -match 'pcAcceptsNotif=true' -and $state -match 'online=true') $state

    # ------------------------------------------------------------------ P posts: the shell, update, remove, MessagingStyle, code, skips
    if ((Want 'Posts') -or (Want 'Api29')) {
        Say 'P: posts from the shell and from the debug Mirror test'
        $n = Pc-Lines
        Post-Shell 'shell1' 'Hello world' 'Alice'
        Check 'P1 a notification from the shell reaches the PC' (Wait-Until { @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' -and $_.title -eq 'Alice' }).Count -ge 1 } 10)
        $post = @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' })[0]
        Check 'P1 it has the app name, the nid (32 hex, not the key) and the text' ($post.app -and $post.nid -match '^[0-9a-f]{32}$' -and $post.text -eq 'Hello world') ("app=$($post.app)")
        Check 'P1 the frame holds no raw key and no package of the shell' ((($post | ConvertTo-Json -Compress) -notmatch 'com\.android\.shell') -and (($post | ConvertTo-Json -Compress) -notmatch '\|'))
        $icons = @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.icon' })
        Check 'P1 an icon went first, once, and the post uses its hash' ($icons.Count -le 1 -and ($post.iconHash -eq $null -or $icons.Count -eq 1 -and $icons[0].hash -eq $post.iconHash))
        Start-Sleep -Milliseconds 1200
        $n = Pc-Lines
        Post-Shell 'shell1' 'Hello again' 'Alice'
        Check 'P2 the same tag with new text is an update with the same nid' (Wait-Until { @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' -and $_.text -eq 'Hello again' -and $_.nid -eq $post.nid }).Count -eq 1 } 10)
        $n = Pc-Lines
        Phone-Cmd 'MIRROR_POST' '--es kind a' | Out-Null
        $a = Wait-Until { @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' -and $_.messages }).Count -ge 1 } 10
        Check 'P3 a MessagingStyle notification: title, last messages oldest first, You has no sender' $a
        $am = @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' -and $_.messages })[0]
        Check 'P3 three messages, the user''s own without a sender' ($am.messages.Count -eq 3 -and $am.messages[0].sender -eq 'Alice' -and $am.messages[1].sender -eq $null -and $am.messages[1].text -eq 'Yes, at eight')
        Check 'P3 a reply is offered with its hint and "Mark as read" is a button' ($am.reply.i -ne $null -and $am.reply.hint -eq 'Reply' -and (@($am.actions | Where-Object { $_.label -eq 'Mark as read' }).Count -eq 1))
        Check 'P3 the activity action "Open" is NOT offered' (-not (@($am.actions | Where-Object { $_.label -eq 'Open' }).Count) -or $sdk -lt 31) "sdk=$sdk actions=$(($am.actions | ForEach-Object { $_.label }) -join ',')"
        $nidA = $am.nid
        $n = Pc-Lines
        Phone-Cmd 'MIRROR_POST' '--es kind b' | Out-Null
        Check 'P4 a verification code is found and sent as code' (Wait-Until { @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' -and $_.code -eq '482913' }).Count -ge 1 } 10)
        $nidB = @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' -and $_.code -eq '482913' })[0].nid
        $n = Pc-Lines
        foreach ($k in 'c', 'd', 'e', 'f', 'g') { Phone-Cmd 'MIRROR_POST' "--es kind $k" 600 | Out-Null }
        Start-Sleep -Seconds 3
        $posts = @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' })
        Check 'P5 ongoing, the group summary and the hidden-on-lock-screen one are skipped (not sent)' (-not ($posts | Where-Object { $_.title -in 'Ongoing task', '2 messages', 'Secret' }))
        Check 'P5 the two group children are sent' (@($posts | Where-Object { $_.title -like 'Group child*' }).Count -eq 2)
        Check 'P5 a low-importance one is sent as silent with importance 2' (@($posts | Where-Object { $_.title -eq 'Low importance' -and $_.silent -eq $true -and $_.importance -eq 2 }).Count -eq 1)
        if ($sdk -ge 35) {
            Check 'P5 a look-alike of the redacted placeholder is sent as redacted, with no text' (@($posts | Where-Object { $_.title -eq 'Looks redacted' -and $_.redacted -eq $true -and $_.text -eq $null }).Count -eq 1)
        } else {
            Check 'P5 before Android 15 nothing is redacted: the same text is sent as an ordinary text' (@($posts | Where-Object { $_.title -eq 'Looks redacted' -and -not $_.redacted -and $_.text -eq 'Sensitive notification content hidden' }).Count -eq 1)
        }
        $state = Phone-State
        Check 'P5 the counters say why' ($state -match 'ongoing:[1-9]' -and $state -match 'summary:[1-9]' -and $state -match 'secret:[1-9]') $state
        Post-Shell 'nonallowed' 'x' 'x'
    }

    # ------------------------------------------------------------------ R reply, buttons, stale, activity action, dismiss
    if ((Want 'Actions') -or (Want 'Api29')) {
        Say 'R: reply, buttons and dismiss from the PC'
        if (-not $nidA) {
            Phone-Cmd 'MIRROR_POST' '--es kind a' | Out-Null; Phone-Cmd 'MIRROR_POST' '--es kind b' | Out-Null; Start-Sleep -Seconds 2
            $nidA = @(Pc-Frames | Where-Object { $_.type -eq 'notif.post' -and $_.messages })[-1].nid
            $nidB = @(Pc-Frames | Where-Object { $_.type -eq 'notif.post' -and $_.code })[-1].nid
        }
        $n = Pc-Lines
        Pc-Command "reply $nidA 0 Reply|On my way"
        $ack = @(Pc-Frames $n | Where-Object { $_.type -eq 'ack' -or $_.type -eq 'error' })[0]
        Check 'R1 a reply from the PC is acked' ($ack.type -eq 'ack') ($ack | ConvertTo-Json -Compress)
        $log = Phone-Cmd 'MIRROR_STATE' '' 800
        Check 'R1 the app received the reply text (its hash and length, not the text)' (($log -join ' ') -match 'lastReplySha=[0-9a-f]{8}' -and ($log -join ' ') -match 'replies=1') ($log -join ' ' | ForEach-Object { if ($_ -match 'replies=\d+ lastReplySha=\S+') { $Matches[0] } })
        $n = Pc-Lines
        Pc-Command "action $nidA 1 Mark as read"
        $ack = @(Pc-Frames $n | Where-Object { $_.type -eq 'ack' -or $_.type -eq 'error' })[0]
        Check 'R2 a button from the PC is acked and the app''s receiver ran' ($ack.type -eq 'ack' -and ((Phone-State) -match 'marksRead=1')) ($ack | ConvertTo-Json -Compress)
        $n = Pc-Lines
        Pc-Command "action $nidA 1 Delete"
        $err = @(Pc-Frames $n | Where-Object { $_.type -eq 'error' })[0]
        Check 'R3 a label that is not the button''s is stale (never a different button)' ($err.code -eq 'stale') ($err | ConvertTo-Json -Compress)
        if ($sdk -ge 31) {
            $n = Pc-Lines
            Pc-Command "action $nidA 2 Open"
            $err = @(Pc-Frames $n | Where-Object { $_.type -eq 'error' })[0]
            Check 'R4 an activity action is refused, not fired' ($err.code -eq 'action_failed') ($err | ConvertTo-Json -Compress)
        }
        $n = Pc-Lines
        Pc-Command "action 00000000000000000000000000000000 0 x"
        $err = @(Pc-Frames $n | Where-Object { $_.type -eq 'error' })[0]
        Check 'R5 an unknown nid is not_found' ($err.code -eq 'not_found') ($err | ConvertTo-Json -Compress)
        $d0 = if ((Phone-State) -match 'dismissed=(\d+)') { [int]$Matches[1] } else { -1 }
        $n = Pc-Lines
        Pc-Command "dismiss $nidB"
        Start-Sleep -Milliseconds 1500
        $after = (Active-Notifs)
        $d1 = if ((Phone-State) -match 'dismissed=(\d+)') { [int]$Matches[1] } else { -1 }
        Check 'R6 a dismiss from the PC removes the notification on the phone' (($d1 -eq $d0 + 1) -and ($after -notmatch 'app\.kinrelay\.android\|2\|t\|')) "dismissed $d0 -> $d1"
        Check 'R6 and no notif.remove is echoed back' (@(Pc-Frames $n | Where-Object { $_.type -eq 'notif.remove' }).Count -eq 0)
        Phone-Cmd 'MIRROR_DISMISS_PREF' '--ez on false' | Out-Null
        Phone-Cmd 'MIRROR_POST' '--es kind b' | Out-Null; Start-Sleep -Seconds 1
        Pc-Command "dismiss $nidB"
        Check 'R7 with "Dismiss on the phone too" off the dismiss is ignored' ((Active-Notifs) -match 'app\.kinrelay\.android\|2\|t\|')
        Phone-Cmd 'MIRROR_DISMISS_PREF' '--ez on true' | Out-Null
        $n = Pc-Lines
        Phone-Cmd 'MIRROR_CANCEL' '--es tag t --ei id 2' | Out-Null
        Check 'R8 a notification cancelled on the phone sends notif.remove' (Wait-Until { @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.remove' -and $_.nid -eq $nidB }).Count -eq 1 } 8)
    }

    if ((Want 'Actions') -and -not (@($Only) -contains 'Api29')) {
        Say 'R: taking an app off the list, and the master switch, remove its notifications on the PC'
        Start-Sleep -Seconds 12   # the package cap of the shell (20 per minute) is not for this step
        $n = Pc-Lines
        Post-Shell 'offlist' 'about to go' 'Gone soon'
        $gone = Wait-Until { @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' -and $_.title -eq 'Gone soon' }).Count -eq 1 } 10
        $goneNid = @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' -and $_.title -eq 'Gone soon' })[0].nid
        $n = Pc-Lines
        Phone-Cmd 'MIRROR_ALLOW' "--es pkg com.android.shell --ez on false" | Out-Null
        Check 'R9 an app taken off the list: a notif.remove for its notification' ($gone -and (Wait-Until { @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.remove' -and $_.nid -eq $goneNid }).Count -eq 1 } 8))
        Check 'R9 the phone sent a new status with one app fewer' (@(Pc-Frames $n | Where-Object { $_.type -eq 'notif.status' }).Count -ge 1)
        Phone-Cmd 'MIRROR_ALLOW' "--es pkg com.android.shell" | Out-Null
        Start-Sleep -Seconds 6
        $n = Pc-Lines
        Phone-Cmd 'MIRROR_ENABLE' '--ez on false' | Out-Null
        Start-Sleep -Seconds 2
        $removed = @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.remove' })
        Check 'R10 the master switch off: a notif.remove for every announced notification' ($removed.Count -ge 1) "removes=$($removed.Count)"
        $st = @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.status' } | Select-Object -Last 1)[0]
        Check 'R10 and the status says disabled' ($st.enabled -eq $false -and $st.access -eq 'granted') ($st | ConvertTo-Json -Compress)
        $n = Pc-Lines
        Post-Shell 'while-off' 'nothing should go' 'Off'
        Start-Sleep -Seconds 3
        Check 'R10 nothing is sent while it is off' (@(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' }).Count -eq 0)
        Phone-Cmd 'MIRROR_ENABLE' | Out-Null
        Start-Sleep -Seconds 2
    }
    if (@($Only) -contains 'Api29') { throw [System.OperationCanceledException]::new('api29 subset done') }

    # ------------------------------------------------------------------ O offline and sync
    if (Want 'Offline') {
        Say 'O: the PC turns the setting off, then on again'
        $n = Pc-Lines
        Pc-Command 'notifoff'
        Phone-Cmd 'MIRROR_POST' '--es kind b --es tag off1' | Out-Null
        Phone-Cmd 'MIRROR_CANCEL' '--es tag t --ei id 1' | Out-Null
        Start-Sleep -Seconds 1
        Check 'O1 with the PC not listing notif nothing is sent (dropped, not queued)' (@(Pc-Frames $n | Where-Object { $_.type -like 'notif.*' }).Count -eq 0)
        $state = Phone-State
        Check 'O1 the phone knows (pcAcceptsNotif false) and counted the drops' ($state -match 'online=false' -and $state -match 'offline:[1-9]') $state
        $n = Pc-Lines
        Pc-Command 'notifon'
        Start-Sleep -Seconds 2
        $f = @(Pc-Frames $n | Where-Object { $_.type -like 'notif.*' })
        Check 'O2 on again: notif.status then notif.sync, and no post (nothing that was missed is shown)' ($f.Count -ge 2 -and $f[0].type -eq 'notif.status' -and $f[1].type -eq 'notif.sync' -and -not ($f | Where-Object { $_.type -eq 'notif.post' }))
        Check 'O2 the sync lists what is active now and not what was cancelled' (@($f[1].nids).Count -ge 1 -and $f[1].nids -notcontains $nidA)
    }

    # ------------------------------------------------------------------ X Android 15+: does this image redact an obvious one-time code?
    if ((Want 'Redaction') -and $sdk -ge 35) {
        Say 'X: Android 15+ redaction on this image'
        $n = Pc-Lines
        Post-Shell 'otp-x' 'Your verification code is 731946. Do not share it.' 'Example'
        Start-Sleep -Seconds 4
        $p = @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' })[0]
        $dump = (A shell dumpsys notification --noredact | Out-String)
        $redacted = ($p.redacted -eq $true)
        Say ("redaction result: redacted=$redacted code=$($p.code) text=$($p.text)")
        Check ('X1 recorded: this image does ' + $(if ($redacted) { 'redact' } else { 'NOT redact' }) + ' an obvious code') $true "redacted=$redacted code=$($p.code)"
        Set-Content (Join-Path $Work 'redaction.txt') ("sdk=$sdk redacted=$redacted code=$($p.code) sensitiveContentFlags=" + (([regex]::Matches($dump, 'mSensitiveContent=\w+') | ForEach-Object { $_.Value } | Select-Object -Unique) -join ','))
    }
    # ------------------------------------------------------------------ L rate limit and the log
    if (Want 'Rate') {
        Say 'L: 100 posts in about 3 s'
        Start-Sleep -Seconds 12
        $n = Pc-Lines
        $sw = [Diagnostics.Stopwatch]::StartNew()
        # Android accepts 50 notifications per package: two bursts of 50, the shade cleared in between (100 posts in all).
        $script = "for i in `$(seq 1 50); do cmd notification post -t Burst b`$i x >/dev/null 2>&1; done"
        A shell $script | Out-Null
        Phone-Cmd 'MIRROR_CLEAR_ALL' '' 300 | Out-Null
        A shell $script | Out-Null
        Phone-Cmd 'MIRROR_CLEAR_ALL' '' 300 | Out-Null
        Say ("posted 100 in {0:N1} s" -f $sw.Elapsed.TotalSeconds)
        Start-Sleep -Seconds 4
        $burst = @(Pc-Frames $n | Where-Object { $_.type -eq 'notif.post' -and $_.title -eq 'Burst' })
        Check 'L1 at most the budget reached the PC; the rest were counted' ($burst.Count -ge 1 -and $burst.Count -le 40) "reached=$($burst.Count)"
        $state = Phone-State
        Check 'L1 the counters hold the drops (rate or package cap)' ($state -match 'rate:[1-9]') $state
    }
    if (Want 'Logs') {
        Say 'G: what the logs hold'
        # The app's own file log (what Export logs would hold), every line: the debug build's `debug: CMD` echo lines are test hooks (and are dropped by the export).
        $fileLog = ((A shell "run-as $Pkg cat files/logs/kin.log.1 files/logs/kin.log" | Out-String) -split "`n")
        $fileLog | Set-Content (Join-Path $Work 'kin.log')
        $product = @($fileLog | Where-Object { $_ -notmatch ' I debug: CMD ' -and $_ -notmatch ' debug: CMD ' })
        $bad = @('Your verification code', '482913', '731946', 'Alice', 'Hello world', 'On my way', 'Looks redacted', 'com.android.shell', 'Secret', 'Group child', 'Example Service')
        $leak = @($bad | Where-Object { ($product -join "`n") -cmatch [regex]::Escape($_) })
        $mirrorLines = @($product | Where-Object { $_ -match ' mirror: ' })
        Check 'G1 no test string, label, key or package is in the log (every line but the debug CMD echo)' ($leak.Count -eq 0) ($leak -join ', ')
        Check 'G1 the log has the one-line counts' (($mirrorLines -join "`n") -match 'mirror: sent \d+, removed \d+, dropped')
        Check 'G1 the nid of a posted notification is not in the log' (-not (($product -join "`n") -match '[0-9a-f]{32}'))
    }
}
catch [System.OperationCanceledException] { Say $_.Exception.Message }
catch {
    Say ("ERROR: " + $_.Exception.Message)
    $script:Failures.Add("error: " + $_.Exception.Message)
}
finally {
    Say 'cleaning up'
    try { Phone-Cmd 'MIRROR_CLEAR_ALL' '' 500 | Out-Null } catch {}
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
