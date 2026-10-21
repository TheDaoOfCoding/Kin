# 2B.1 on the phone (links), verified ONCE on an emulator: the real debug APK, a test Kin beside the user's installed Kin (own --data-dir, loopback only, no PC window),
# and a throwaway test app (android\link-target) that owns https://kin-link-test.example/... like YouTube owns youtube.com.
#
#   * a link from the PC (debug command LINK_FROM_PC: the real IncomingHandler and notifications; the desktop's own link send is not needed): the title with the real
#     address under it, "Open in Link target" for a link an app owns and "Open in browser" for the rest, "Copy link" as the ONLY button, a grouped stack,
#     a tap that opens the app that owns the link (with the same URL, timestamp included), Copy link that puts the URL on the phone's clipboard
#   * the lock screen: previews hidden by default, shown after the setting; a secure lock is set on the emulator for the screenshot and cleared again
#   * text from the PC (the real desktop: --send-clipboard): copied automatically by default and a Copy button; with the setting off the clipboard is left alone and Copy copies
#   * a URL shared to Kin (the share intent) goes to the PC: as a `link` if the desktop lists the capability, else as a `text` (the fallback), queued while Wi-Fi is off
#
#   android\tools\verify-2b1.ps1 -ScreenshotDir <folder>
#   android\tools\verify-2b1.ps1 -Avd kin_api29 -Only Api29      # the Android 10 path (resolve check, no REQUIRE_NON_BROWSER)
#
# Never touches the user's installed Kin and its data, the Run key, SendTo, Downloads, the firewall. The PC clipboard is saved in every format first and put back (it
# is written only through Set-QuietClipboardText). No PC window opens: the test Kin runs with --dev-no-windows and "Open links from phone automatically" OFF in its own
# throwaway settings.json, so a link sent to it never opens a browser on this PC; its automatic clipboard sync is off too (it would put the PC's text on the phone clipboard by itself and hide what the setting does). The emulator is headless and silent.
param(
    [string]$Avd = 'kin_api37',
    [string]$Kin = (Join-Path $PSScriptRoot '..\..\desktop\src-tauri\target\release\Kin.exe'),
    [string]$Apk = (Join-Path $PSScriptRoot '..\app\build\outputs\apk\debug\app-debug.apk'),
    [string]$LinkTargetApk = (Join-Path $PSScriptRoot '..\link-target\build\outputs\apk\debug\link-target-debug.apk'),
    [string[]]$Only = @(),
    [int]$Port = 49323,
    [string]$ScreenshotDir = '',
    [switch]$KeepRunning,
    [string]$Work = ''
)
. (Join-Path $PSScriptRoot '..\..\tools\kin-lock.ps1'); Enter-KinLock 'verify-2b1'
. (Join-Path $PSScriptRoot '..\..\tools\kin-dev.ps1')
. (Join-Path $PSScriptRoot '..\..\desktop\clipboard-backup.ps1')
if (-not $Work) { $Work = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-verify-2b1-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
$script:OrigComputerName = $env:COMPUTERNAME
$env:COMPUTERNAME = 'DESK-PC'
$ErrorActionPreference = 'Continue'
foreach ($name in 'ANDROID_HOME', 'JAVA_HOME') {
    if (-not (Get-Item "env:$name" -ErrorAction SilentlyContinue)) { $v = [Environment]::GetEnvironmentVariable($name, 'User'); if ($v) { Set-Item "env:$name" $v } }
}
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Emu = Join-Path $Repo 'tools\emulator'
$Adb = Join-Path $(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }) 'platform-tools\adb.exe'
$Pkg = 'app.kinrelay.android'
$Target = 'app.kinrelay.linktarget'
$Serial = 'emulator-5554'
$Data = Join-Path $Work 'data'
$Dumps = Join-Path $Work 'dumps'
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Results = [ordered]@{}
$started = @{ Emulator = $false; Kin = $false; Pin = $false }
$Owned = 'https://kin-link-test.example/watch?v=dQw4w9WgXcQ&t=83s&list=PLabc123#frag'

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
function Invoke-EmuScript([string]$Script, [string[]]$ScriptArgs = @(), [int]$TimeoutSec = 300) {
    $out = Join-Path $env:TEMP "kin-verify-2b1-$Script.out"
    $p = Start-Process powershell -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $Emu $Script))) + $ScriptArgs)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; return $false }
    return $true
}
function Get-Status { try { Get-Content (Join-Path $Data 'status.json') -Raw -ErrorAction Stop | ConvertFrom-Json } catch { $null } }
function Shot([string]$Name) {
    if (-not $ScreenshotDir) { return }
    New-Item -ItemType Directory -Force $ScreenshotDir | Out-Null
    Invoke-EmuScript 'screenshot.ps1' @('-Out', ('"{0}"' -f (Join-Path $ScreenshotDir "$Name.png"))) 60 | Out-Null
}
function Ui-Dump { A shell uiautomator dump /sdcard/kin-ui.xml 2>$null | Out-Null; A shell cat /sdcard/kin-ui.xml | Out-String }
function Tap-UiText([string[]]$Texts, [int]$Tries = 8) {
    for ($try = 0; $try -lt $Tries; $try++) {
        $x = Ui-Dump
        foreach ($t in $Texts) {
            if ($x -match ('text="{0}"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"' -f $t)) {
                A shell input tap ([int](([int]$Matches[1] + [int]$Matches[3]) / 2)) ([int](([int]$Matches[2] + [int]$Matches[4]) / 2)) | Out-Null
                return $true
            }
        }
        Start-Sleep -Milliseconds 1000
    }
    return $false
}
# Opens the notification shade (the notifications are shown) and closes it again.
function Shade-Open { A shell cmd statusbar expand-notifications | Out-Null; Start-Sleep -Milliseconds 1500 }
function Shade-Close { A shell cmd statusbar collapse | Out-Null; Start-Sleep -Milliseconds 600 }
# The records of this app in `dumpsys notification --noredact`, one string per notification.
function Phone-NotifBlocks {
    $raw = A shell dumpsys notification --noredact | Out-String
    $blocks = [regex]::Split($raw, '(?m)^\s*NotificationRecord\(') | Where-Object { $_ -match "pkg=$([regex]::Escape($Pkg))" }
    @($blocks)
}
# By the notification's tag (the message ID the test gave it), or by a part of its title (never the group summary: its lines hold the addresses).
function Notif-For([string]$Part) { @(Phone-NotifBlocks | Where-Object { $_ -notmatch 'tag=links-summary' -and ($_ -match ('tag=' + [regex]::Escape($Part) + ' ') -or $_ -match ('android.title=String \(' + [regex]::Escape($Part))) } | Select-Object -First 1)[0] }
function Notif-Actions([string]$Block) { @([regex]::Matches($Block, '(?m)^\s+\[\d+\]\s+"([^"]+)"') | ForEach-Object { $_.Groups[1].Value }) }
function Phone-Cmd([string]$Command, [string]$Extras = '', [int]$Wait = 1500) {
    A logcat -c | Out-Null
    A shell "am broadcast -a app.kinrelay.android.debug.$Command -n $Pkg/.debug.DebugCommandReceiver $Extras" | Out-Null
    Start-Sleep -Milliseconds $Wait
    @(A logcat -d -s KinRelay)
}
function Phone-Link([string]$Url, [string]$Title, [string]$Id = '') {
    $extras = "--es url '$Url' --es title '$Title'"
    if ($Id) { $extras += " --es id '$Id'" }
    Phone-Cmd 'LINK_FROM_PC' $extras | Out-Null
}
# The phone's clipboard through the debug build's DebugClipboardActivity (only a focused app may read it): "CMD CLIPBOARD ... text=<json string>".
function Phone-Clip {
    A logcat -c | Out-Null
    A shell am start -n "$Pkg/.debug.DebugClipboardActivity" | Out-Null
    Start-Sleep -Milliseconds 1800
    $l = @(A logcat -d -s KinRelay | Where-Object { $_ -match 'CMD CLIPBOARD' }) | Select-Object -Last 1
    if ($l -match 'text="(.*)"\s*$') { return $Matches[1] }
    if ($l -match 'CMD CLIPBOARD empty') { return '' }
    return '?'
}
function Phone-Resumed { (A shell dumpsys activity activities | Select-String 'ResumedActivity' | Select-Object -First 1 | Out-String).Trim() }
function Phone-Texts { @(Phone-Cmd 'TEXTS' '' 1500 | Where-Object { $_ -match 'CMD TEXT ' }) }

if (-not (Test-Path (Join-Path $Repo 'desktop\clipboard-backup.ps1'))) { throw 'desktop\clipboard-backup.ps1 is missing' }
Add-Type -AssemblyName System.Windows.Forms
$clip = $null
try {
    $Kin = (Resolve-Path $Kin -ErrorAction Stop).Path
    if (-not (Test-Path $Apk)) { throw "APK missing: $Apk" }
    if (-not (Test-Path $LinkTargetApk)) {
        Say 'building link-target (the test app that owns kin-link-test.example)'
        & (Join-Path $PSScriptRoot '..\gradlew.bat') -p (Join-Path $PSScriptRoot '..\link-target') assembleDebug --console=plain 2>&1 | Out-Null
    }
    New-Item -ItemType Directory -Force $Data, $Dumps | Out-Null
    $KinCopy = Join-Path $Work 'Kin.exe'
    Copy-Item $Kin $KinCopy      # a copy: a desktop rebuild in the meantime does not matter
    Set-KinDev -Exe $KinCopy -DataDir $Data
    Assert-NoDevKinOnDataDir
    # "Open links from phone automatically" OFF in this throwaway data directory: a link sent to the test Kin never opens a browser on this PC.
    Set-Content -Path (Join-Path $Data 'settings.json') -Value '{"openLinksAutomatically":false,"autoClipboardSync":false}' -Encoding ASCII
    $clip = Save-Clipboard | Select-Object -Last 1

    Say "starting $Avd"
    $started.Emulator = $true
    $emuUp = Invoke-EmuScript 'start.ps1' @('-Avd', $Avd)
    $isBooted = { ((A shell getprop sys.boot_completed 2>$null | Out-String).Trim() -eq '1') }
    if ($emuUp -and -not (& $isBooted)) { & $Adb kill-server 2>$null | Out-Null; Start-Sleep -Seconds 2; & $Adb start-server 2>$null | Out-Null; Wait-Until $isBooted 60 | Out-Null }
    if (-not $emuUp -or -not (& $isBooted)) { throw "emulator $Avd did not start" }
    A shell svc wifi enable | Out-Null
    $running = ((A emu avd name | Out-String) -split "`r?`n" | Where-Object { $_ } | Select-Object -First 1)
    if ($running -and $running.Trim() -ne $Avd) { throw "the emulator that is running is '$($running.Trim())', not ${Avd}" }
    $sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    Say "API $sdk"

    A uninstall $Pkg 2>&1 | Out-Null
    A uninstall $Target 2>&1 | Out-Null
    $out = (& $Adb -s $Serial install -r (Resolve-Path $Apk).Path 2>&1 | Out-String)
    Check 'install Kin (debug build)' ($out -match 'Success') ($out.Trim())
    $out = (& $Adb -s $Serial install -r (Resolve-Path $LinkTargetApk).Path 2>&1 | Out-String)
    Check 'install the link target test app' ($out -match 'Success') ($out.Trim())
    if ($sdk -ge 37) { A shell pm grant $Pkg android.permission.ACCESS_LOCAL_NETWORK | Out-Null }
    if ($sdk -ge 33) { A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null }
    A shell am start -n "$Pkg/.MainActivity" | Out-Null

    $started.Kin = $true
    $KinArgs = @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-port', "$Port",
        '--dev-received-dir', ('"{0}"' -f (Join-Path $Data 'received')), '--dev-print-pair-url', '--dev-no-windows')
    $script:KinProc = Start-DevKin $KinArgs
    Check 'the test Kin started' (Wait-Until { (Get-Status).state -eq 'not_paired' } 30)
    Invoke-Kin --dev-start-pairing
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    $paired = $false
    for ($try = 1; $try -le 3 -and -not $paired; $try++) {
        A shell am start -a android.intent.action.VIEW -p $Pkg -d "'$url'" --ez app.kinrelay.android.AUTO_CONFIRM true 2>$null | Out-Null
        $paired = Wait-Until { (Get-Status).state -eq 'phone_connected' } 25
    }
    Check 'paired and connected' $paired
    if (-not $paired) { throw 'not paired' }
    A shell input keyevent KEYCODE_HOME | Out-Null
    # Android 12+ sends a web link to an app only if that app is verified for the host or the user approved it (Settings > Open by default); the test app is
    # not verified, so it is approved here the way the user would (YouTube is verified for youtube.com).
    if ($sdk -ge 31) {
        A shell pm set-app-links --package $Target 1 kin-link-test.example | Out-Null
        A shell pm set-app-links-user-selection --user 0 --package $Target true kin-link-test.example | Out-Null
    }

    # ------------------------------------------------------------------ L1 a link the PC sent, owned by an app
    if (Want 'Links') {
        Say 'L1: a link from the PC that an app owns'
        Phone-Link $Owned 'Cat video: part 2' 'link-owned-1'
        $b = Wait-Until { Notif-For 'link-owned-1' } 10
        $n = Notif-For 'link-owned-1'
        $n | Set-Content (Join-Path $Dumps 'notification-owned.txt')
        Check 'L1 a notification was posted with the page title' ($null -ne $n -and $n -match 'Cat video: part 2')
        Check 'L1 the real address is under the title' ($n -match 'android.text=\S+ \(?kin-link-test\.example|kin-link-test\.example')
        Check 'L1 it says what a tap does: Open in Link target' ($n -match 'android.subText=String \(Open in Link target\)')
        $actions = @(Notif-Actions $n)
        Check 'L1 the only button is "Copy link"' ($actions.Count -eq 1 -and $actions[0] -eq 'Copy link') ("actions: " + ($actions -join ', '))
        Check 'L1 the lock screen hides the preview by default (private)' ($n -match 'vis=PRIVATE')
        Shade-Open; Shot '01-link-owned-shade'
        Check 'L1 tap the notification' (Tap-UiText @('Cat video: part 2'))
        Check 'L1 the app that owns the link opened, not the browser' (Wait-Until { (Phone-Resumed) -match $Target } 10) (Phone-Resumed)
        $ui = Ui-Dump
        Check 'L1 it got the same URL (timestamp, list and fragment untouched)' ($ui -match 'LINK TARGET OPENED' -and $ui -match [regex]::Escape($Owned.Replace('&', '&amp;'))) ''
        Shot '02-link-target-opened'
        A shell input keyevent KEYCODE_HOME | Out-Null
        Start-Sleep -Milliseconds 800
        Check 'L1 the tapped notification is gone' (-not (Notif-For 'link-owned-1'))
    }

    # ------------------------------------------------------------------ L2 a spoofing title, a link nobody owns, Copy link, grouping
    if (Want 'Links') {
        Say 'L2: a title that pretends to be another address; Copy link; the group'
        Phone-Link 'https://evil.example/login?next=1' 'https://accounts.google.com - Sign in' 'link-evil-1'
        $e = Notif-For 'link-evil-1'
        $e | Set-Content (Join-Path $Dumps 'notification-spoof.txt')
        Check 'L2 the address line is the real host, not the one in the title' ($e -match 'evil\.example' -and $e -match 'android.title=\S+ \(?https://accounts.google.com - Sign in')
        Check 'L2 a link nobody owns says Open link (it never claims a browser)' ($e -match 'android.subText=String \(Open link\)')
        Shade-Open; Shot '03a-spoof-title-real-address'
        Check 'L2 tap "Copy link"' (Tap-UiText @('Copy link'))
        Start-Sleep -Milliseconds 1200
        Shade-Close
        $clipText = Phone-Clip
        Check 'L2 Copy link put the URL on the phone clipboard' ($clipText -ceq 'https://evil.example/login?next=1') "$clipText"
        Phone-Link 'https://example.org/a' 'Second' 'link-2'
        Phone-Link 'https://example.net/b' 'Third' 'link-3'
        Start-Sleep -Seconds 2
        $summary = @(Phone-NotifBlocks | Where-Object { $_ -match 'tag=links-summary' })
        $summary | Set-Content (Join-Path $Dumps 'notification-summary.txt')
        Check 'L2 three links in a row are one group with a summary' ($summary.Count -ge 1 -and ($summary -join "`n") -match '[3-9] links from DESK-PC') (($summary -join ' ') -replace '\s+', ' ' | ForEach-Object { $_.Substring(0, [Math]::Min(120, $_.Length)) })
        Shade-Open; Shot '03-links-grouped'
        $ui = Ui-Dump
        Check 'L2 the shade shows the summary of the group' ($ui -match 'links from DESK-PC' -or $ui -match 'evil.example')
        Shade-Close
    }

    # ------------------------------------------------------------------ L3 the lock screen
    if ((Want 'Lock') -and $sdk -ge 33) {
        Say 'L3: the lock screen (a secure lock is set on the emulator for this and cleared after)'
        Phone-Cmd 'SET_LINK_PREFS' '--ez lockscreen false' 800 | Out-Null
        # Android shows a private notification in full unless the phone's own lock-screen setting hides sensitive content (the emulator's default shows all):
        # this turns that setting on for the run and puts it back.
        $oldPrivate = (A shell settings get secure lock_screen_allow_private_notifications | Out-String).Trim()
        A shell settings put secure lock_screen_allow_private_notifications 0 | Out-Null
        A shell locksettings set-pin 1234 | Out-Null
        $started.Pin = $true
        Phone-Link 'https://example.com/private-article' 'My private article title' 'link-lock-1'
        $l = Notif-For 'link-lock-1'
        Check 'L3 default: the notification is private with a public version without the title' ($l -match 'vis=PRIVATE')
        A shell input keyevent KEYCODE_SLEEP | Out-Null; Start-Sleep -Seconds 2
        A shell input keyevent KEYCODE_WAKEUP | Out-Null; Start-Sleep -Seconds 3
        Shot '04-lockscreen-previews-off'
        $lockUi = Ui-Dump
        Check 'L3 default: the locked phone shows only a count of links from the PC, no title and no address' (($lockUi -match 'links? from DESK-PC') -and ($lockUi -notmatch 'My private article title') -and ($lockUi -notmatch 'example\.com')) ''
        # unlock, turn the setting on, post a new link, lock again
        A shell input keyevent 82 | Out-Null; Start-Sleep -Milliseconds 800
        A shell input text 1234 | Out-Null; A shell input keyevent 66 | Out-Null; Start-Sleep -Seconds 2
        Phone-Cmd 'SET_LINK_PREFS' '--ez lockscreen true' 800 | Out-Null
        Phone-Link 'https://example.com/shown-article' 'My shown article title' 'link-lock-2'
        $l2 = Notif-For 'link-lock-2'
        Check 'L3 setting on: the notification is public' ($l2 -match 'vis=PUBLIC')
        A shell input keyevent KEYCODE_SLEEP | Out-Null; Start-Sleep -Seconds 2
        A shell input keyevent KEYCODE_WAKEUP | Out-Null; Start-Sleep -Seconds 3
        Shot '05-lockscreen-previews-on'
        $lockUi2 = Ui-Dump
        Check 'L3 setting on: the locked phone shows the title and the address' (($lockUi2 -match 'My shown article title') -and ($lockUi2 -match 'example.com')) ''
        A shell input keyevent 82 | Out-Null; Start-Sleep -Milliseconds 800
        A shell input text 1234 | Out-Null; A shell input keyevent 66 | Out-Null; Start-Sleep -Seconds 2
        A shell locksettings clear --old 1234 | Out-Null
        if ($oldPrivate -and $oldPrivate -ne 'null') { A shell settings put secure lock_screen_allow_private_notifications $oldPrivate | Out-Null } else { A shell settings delete secure lock_screen_allow_private_notifications | Out-Null }
        $started.Pin = $false
        Phone-Cmd 'SET_LINK_PREFS' '--ez lockscreen false' 800 | Out-Null
        A shell input keyevent KEYCODE_HOME | Out-Null
    }

    # ------------------------------------------------------------------ T text from the PC (the real desktop): the setting and the Copy button
    if ((Want 'Text') -and $sdk -ge 29) {
        Say 'T: text from the PC, copied automatically by default, and the setting off'
        A shell cmd notification cancel_all 2>$null | Out-Null
        Set-QuietClipboardText 'text from the PC one'
        Start-Sleep -Milliseconds 800
        Invoke-Kin --send-clipboard
        Check 'T1 the phone shows a notification for the text' (Wait-Until { Notif-For 'Text from DESK-PC' } 15)
        $t = Notif-For 'Text from DESK-PC'
        $t | Set-Content (Join-Path $Dumps 'notification-text.txt')
        $ta = @(Notif-Actions $t)
        Check 'T1 the text notification has a Copy button' ($ta -contains 'Copy') ("actions: " + ($ta -join ', '))
        $c = Phone-Clip; Check 'T1 default: the text is on the phone clipboard already' ($c -ceq 'text from the PC one') "$c"
        Shade-Open; Shot '06-text-copied-automatically'; Shade-Close
        Phone-Cmd 'SET_CLIPBOARD' "--es text 'something else on the phone'" 800 | Out-Null
        Phone-Cmd 'SET_LINK_PREFS' '--ez copytext false' 800 | Out-Null
        A shell cmd notification cancel_all 2>$null | Out-Null
        Set-QuietClipboardText 'text from the PC two'
        Start-Sleep -Milliseconds 800
        Invoke-Kin --send-clipboard
        Check 'T2 setting off: a notification with a Copy button' (Wait-Until { $x = Notif-For 'Text from DESK-PC'; $x -and (@(Notif-Actions $x) -contains 'Copy') } 15)
        $c = Phone-Clip; Check 'T2 setting off: the phone clipboard was left alone' ($c -ceq 'something else on the phone') "$c"
        Shade-Open; Shot '07-text-copy-button'
        Check 'T2 tap Copy (Copy latest when texts are grouped)' (Tap-UiText @('Copy latest', 'Copy') 4)
        Start-Sleep -Milliseconds 1200
        Shade-Close
        $c = Phone-Clip; Check 'T2 Copy puts the text on the clipboard' ($c -ceq 'text from the PC two') "$c"
        Shade-Close
        Phone-Cmd 'SET_LINK_PREFS' '--ez copytext true' 800 | Out-Null
    }

    # ------------------------------------------------------------------ S a URL shared to Kin goes to the PC
    if (Want 'Share') {
        Say 'S: a URL shared to Kin'
        $before = (Get-Status).lastReceived.text.id
        $shareUrl = 'https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=83s'
        A shell "am start -n $Pkg/.share.ShareActivity -a android.intent.action.SEND -t text/plain --es android.intent.extra.TEXT '$shareUrl' --es android.intent.extra.SUBJECT 'A shared video'" | Out-Null
        Check 'S1 the PC received it' (Wait-Until { $r = (Get-Status).lastReceived.text; $r -and $r.id -ne $before } 20)
        $log1 = @(A logcat -d -s KinRelay | Where-Object { $_ -match 'on the wire as|link sent|text sent' }) | Select-Object -Last 3
        $log1 | Set-Content (Join-Path $Dumps 'share-log.txt')
        Say ("phone log: " + ($log1 -join ' | '))
        $rows = Phone-Texts
        Check 'S1 the phone keeps it as a link in its history, done' (@($rows | Where-Object { $_ -match 'kind=link' -and $_ -match ' done ' }).Count -ge 1) (($rows | Select-Object -First 1) -as [string])
        # offline: queued, goes out when Wi-Fi is back
        A shell svc wifi disable | Out-Null
        Wait-Until { (Phone-Cmd 'STATE' '' 800 | Out-String) -match 'NotOnWifi' } 15 | Out-Null
        $before2 = (Get-Status).lastReceived.text.id
        A shell "am start -n $Pkg/.share.ShareActivity -a android.intent.action.SEND -t text/plain --es android.intent.extra.TEXT 'https://example.com/queued-while-offline' --es android.intent.extra.SUBJECT 'Queued page'" | Out-Null
        Start-Sleep -Seconds 6
        $rows2 = Phone-Texts
        Check 'S2 offline: the link is queued, not lost' (@($rows2 | Where-Object { $_ -match 'kind=link' -and $_ -match ' queued ' }).Count -ge 1)
        A shell svc wifi enable | Out-Null
        Check 'S2 when the PC is reachable again it arrives' (Wait-Until { $r = (Get-Status).lastReceived.text; $r -and $r.id -ne $before2 } 60)
        $rows3 = Phone-Texts
        Check 'S2 and the phone shows it done' (@($rows3 | Where-Object { $_ -match 'kind=link' -and $_ -match ' done ' }).Count -ge 2)
    }

    # ------------------------------------------------------------------ Api29 the Android 10 path
    if (Want 'Api29') {
        Say 'Api29: the resolve-check path'
        Phone-Link $Owned 'Cat video: part 2' 'link-owned-1'
        Wait-Until { Notif-For 'link-owned-1' } 10 | Out-Null
        $n = Notif-For 'link-owned-1'
        $n | Set-Content (Join-Path $Dumps 'notification-owned-api29.txt')
        Check 'A29 Open in Link target (found by the resolve check)' ($n -match 'Open in Link target')
        $actions = @(Notif-Actions $n)
        Check 'A29 the only button is "Copy link"' ($actions.Count -eq 1 -and $actions[0] -eq 'Copy link') ("actions: " + ($actions -join ', '))
        Shade-Open; Shot 'api29-01-link-notification'
        Check 'A29 tap the notification' (Tap-UiText @('Cat video: part 2'))
        Check 'A29 the owning app opened' (Wait-Until { (Phone-Resumed) -match $Target } 10) (Phone-Resumed)
        A shell input keyevent KEYCODE_HOME | Out-Null
        Phone-Link 'https://example.org/browse-me' 'A plain page' 'link-a29-2'
        Wait-Until { Notif-For 'link-a29-2' } 10 | Out-Null
        $b = Notif-For 'link-a29-2'
        Check 'A29 a link nobody owns says Open link' ($b -match 'android.subText=String \(Open link\)')
        Shade-Open
        Check 'A29 tap the plain link' (Tap-UiText @('A plain page'))
        Start-Sleep -Seconds 3
        $resumed = Phone-Resumed
        Check 'A29 the browser took it (not the link target, no crash)' ($resumed -notmatch $Target -and $resumed -match 'ResumedActivity') $resumed
        Shot 'api29-02-browser'
    }

    $log = A logcat -d | Out-String
    $mine = ($log -split "`n") | Where-Object { $_ -match 'FATAL EXCEPTION|AndroidRuntime' -and $_ -match $Pkg }
    Check 'no crash of the app in logcat' (@($mine).Count -eq 0) (@($mine) -join ' ')
    Shot '99-end'
}
catch {
    Say "ERROR: $_"
    $script:Failures.Add("exception: $_")
}
finally {
    if ($started.Emulator -and ((A get-state | Out-String) -match 'device')) {
        if ($started.Pin) { A shell locksettings clear --old 1234 | Out-Null }
        A shell cmd notification cancel_all 2>$null | Out-Null
    }
    if (-not $KeepRunning) {
        Say 'tearing down'
        if ($started.Kin -and $script:KinProc) { Stop-DevKin 10 }
        if ($started.Emulator) { Invoke-EmuScript 'stop.ps1' @('-All') 120 | Out-Null }
        & $Adb kill-server 2>$null | Out-Null
    }
    $env:COMPUTERNAME = $script:OrigComputerName
    if ($clip) { try { Restore-Clipboard $clip | Out-Null } catch { Say "clipboard restore failed: $_" } }
    if (Test-Path $Dumps) { Say "notification dumps kept in $Dumps" }
}

Write-Host ''
$script:Results.GetEnumerator() | ForEach-Object { Write-Host ("{0,-5} {1}" -f $_.Value, $_.Key) }
if ($script:Failures.Count -gt 0) { Write-Host "`nFAILED: $($script:Failures -join '; ')"; exit 1 }
Write-Host "`nOK: all $($script:Results.Count) checks passed on $Avd."
exit 0
