# Smoke test of the SIGNED RELEASE APK (milestone 1.5) on an emulator against the real Windows app (Kin.exe, developer mode, loopback only).
# The release build has no debug hooks, so everything goes through what a user (or `adb`) can do: a pairing link opened with `am start`,
# a tap on Confirm found with uiautomator, the share intent, and the notifications. Checks: the camera permission flow of "Scan code",
# pairing, ping (PC -> phone), text both ways, a file both ways, and that logcat shows no crash of the app (R8 problems show up here).
#
#   android\tools\release-smoke.ps1                    # kin_api37
#   android\tools\release-smoke.ps1 -Avd kin_api29
#   android\tools\release-smoke.ps1 -KeepRunning       # leave the emulator and Kin up
#
# Exits 0 when every check passed. Needs the release APK (gradlew assembleRelease, with signing set up), and
# desktop\src-tauri\target\release\Kin.exe. A copy of Kin.exe is run from %TEMP%, so a desktop rebuild in the meantime does not matter.
# Kin runs only with --dev-bind 127.0.0.1 and a throwaway data directory; the PC clipboard is saved and put back.
param(
    [string]$Avd = 'kin_api37',
    [switch]$KeepRunning,
    [string]$Apk = (Join-Path $PSScriptRoot '..\app\build\outputs\apk\release\app-release.apk'),
    [string]$Kin = (Join-Path $PSScriptRoot '..\..\desktop\src-tauri\target\release\Kin.exe'),
    [string]$ScreenshotDir = ''
)
$script:OrigComputerName = $env:COMPUTERNAME
$env:COMPUTERNAME = 'DESK-PC'   # Kin shows the PC's name on the phone; a neutral one for the run
$ErrorActionPreference = 'Continue'
foreach ($name in 'ANDROID_HOME', 'JAVA_HOME') {
    if (-not (Get-Item "env:$name" -ErrorAction SilentlyContinue)) { $v = [Environment]::GetEnvironmentVariable($name, 'User'); if ($v) { Set-Item "env:$name" $v } }
}
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Emu = Join-Path $Repo 'tools\emulator'
$Adb = Join-Path $(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }) 'platform-tools\adb.exe'
$Pkg = 'app.kinrelay.android'
$Serial = 'emulator-5554'
$Run = Get-Date -Format 'HHmmss'
$Data = Join-Path $env:TEMP ('kin-smoke-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
$Work = Join-Path $env:TEMP "kin-smoke-work-$Run"
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Results = [ordered]@{}

function A { & $Adb -s $Serial @args }
function Say($m) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $m) }
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $script:Results[$name] = if ($ok) { 'PASS' } else { 'FAIL' }
    if ($ok) { Say "PASS  $name $detail" } else { Say "FAIL  $name $detail"; $script:Failures.Add($name) }
}
function Wait-Until([scriptblock]$Cond, [double]$Seconds) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds 250 }
    return $false
}
# A script of tools\emulator in a child process (the adb daemon that start.ps1 launches would otherwise keep our output pipe open).
function Invoke-EmuScript([string]$Script, [string[]]$ScriptArgs = @(), [int]$TimeoutSec = 300) {
    $out = Join-Path $env:TEMP "kin-smoke-$Script.out"
    $p = Start-Process powershell -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $Emu $Script))) + $ScriptArgs)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; return $false }
    return $true
}
function Get-Status { try { Get-Content (Join-Path $Data 'status.json') -Raw -ErrorAction Stop | ConvertFrom-Json } catch { $null } }
function Ui-Dump { A shell uiautomator dump /sdcard/kin-smoke-ui.xml 2>$null | Out-Null; A shell cat /sdcard/kin-smoke-ui.xml | Out-String }
function Ui-Has([string]$Text) { (Ui-Dump) -match ('text="{0}"' -f [regex]::Escape($Text)) }
# Taps the first node whose text matches one of these regular expressions (the adb output is not UTF-8 safe, so a curly apostrophe is matched with .). $false when none is on screen.
function Tap-UiText([string[]]$Texts, [int]$Tries = 6) {
    for ($try = 0; $try -lt $Tries; $try++) {
        $x = Ui-Dump
        foreach ($t in $Texts) {
            if ($x -match ('text="{0}"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"' -f $t)) {
                A shell input tap ([int](([int]$Matches[1] + [int]$Matches[3]) / 2)) ([int](([int]$Matches[2] + [int]$Matches[4]) / 2)) | Out-Null
                return $true
            }
        }
        Start-Sleep -Milliseconds 700
    }
    return $false
}
function Shot([string]$Name) {
    if (-not $ScreenshotDir) { return }
    New-Item -ItemType Directory -Force $ScreenshotDir | Out-Null
    Invoke-EmuScript 'screenshot.ps1' @('-Out', ('"{0}"' -f (Join-Path $ScreenshotDir "$Name.png"))) 60 | Out-Null
}
function Phone-Notifs { A shell dumpsys notification --noredact | Out-String }
function Sha-Pc([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLower() }

if (-not (Test-Path (Join-Path $Repo 'desktop\clipboard-backup.ps1'))) { throw 'desktop\clipboard-backup.ps1 is missing' }
. (Join-Path $Repo 'desktop\clipboard-backup.ps1')   # Save-Clipboard, Restore-Clipboard
Add-Type -AssemblyName System.Windows.Forms
$started = @{ Emulator = $false; Kin = $false }
$clip = $null
$KinCopy = $null
try {
    $Apk = (Resolve-Path $Apk -ErrorAction Stop).Path
    & $Adb start-server 2>&1 | Out-Null
    if (@(& $Adb devices | Select-String '^emulator-\d+\s').Count -gt 0) { throw 'An emulator is already running (somebody else may be using it). Not touching it.' }
    New-Item -ItemType Directory -Force $Work | Out-Null
    $KinCopy = Join-Path $Work 'Kin.exe'
    Copy-Item (Resolve-Path $Kin -ErrorAction Stop).Path $KinCopy

    Say "starting $Avd"
    $started.Emulator = $true
    if (-not (Invoke-EmuScript 'start.ps1' @('-Avd', $Avd)) -or ((A shell getprop sys.boot_completed | Out-String).Trim() -ne '1')) { throw "emulator $Avd did not start" }
    # Right after the boot flag the device can still show as offline for a moment (seen when an emulator was started straight after another stopped).
    if (-not (Wait-Until { (A get-state | Out-String).Trim() -eq 'device' -and (A shell echo ok | Out-String).Trim() -eq 'ok' } 60)) { throw 'the emulator is not online' }
    A shell svc wifi enable | Out-Null
    $sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    Say "API $sdk"

    # ---- install the release APK over a clean slate (the debug build has another signature, so it has to go first)
    A uninstall $Pkg 2>&1 | Out-Null
    A shell rm -rf /sdcard/Download/Kin | Out-Null
    $out = (& $Adb -s $Serial install -r $Apk 2>&1 | Out-String)
    Check 'install the release APK' ($out -match 'Success') ($out.Trim())
    $pkgDump = A shell dumpsys package $Pkg | Out-String
    Check 'the app is not debuggable' ($pkgDump -notmatch 'DEBUGGABLE' -and $pkgDump -match 'versionName=0\.1\.0' -and $pkgDump -match 'versionCode=1 ') ("versionName=" + ([regex]::Match($pkgDump, 'versionName=\S+').Value))
    Check 'no CAMERA permission granted before "Scan code"' ($pkgDump -notmatch 'android.permission.CAMERA: granted=true')
    A logcat -c | Out-Null

    # ---- "Scan code": the camera permission is asked for only on the tap, a denial is handled, and the scan screen opens after "Allow"
    if ($sdk -ge 37) { A shell pm grant $Pkg android.permission.ACCESS_LOCAL_NETWORK | Out-Null }
    if ($sdk -ge 33) { A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null }
    A shell am start -n "$Pkg/.MainActivity" | Out-Null
    Check 'the pairing screen shows "Scan code"' (Wait-Until { Ui-Has 'Scan code' } 20)
    Check 'the camera permission was not asked at start-up' (-not ((Ui-Dump) -match 'take pictures'))
    Shot '01-pairing-screen'
    Check 'tap Scan code' (Tap-UiText @('Scan code'))
    $prompt = Wait-Until { (Ui-Dump) -match 'take pictures|permission_deny|Don.t allow|Deny' } 15
    Check 'the system asks for the camera' $prompt
    Shot '02-camera-prompt'
    Check 'tap Deny' (Tap-UiText @('Don.{1,9}t allow', 'Deny'))
    $denied = Wait-Until { $x = Ui-Dump; $x -match 'paste the pairing link instead' } 10
    Check 'denial: a clear message that keeps the pasted link' $denied
    Check 'denial: the paste field and the Pair button are still there' ((Ui-Has 'Pair') -and ((Ui-Dump) -match 'kinrelay://pair'))
    Shot '03-camera-denied'
    Check 'tap Scan code again' (Tap-UiText @('Scan code'))
    Check 'the system asks again after the first denial' (Wait-Until { (Ui-Dump) -match 'take pictures' } 15)
    $okTap = Tap-UiText @('While using the app', 'Allow')
    Check 'tap Allow' $okTap
    $scanOpen = Wait-Until { Ui-Has 'Point the camera at the code on your PC' } 15
    Check 'the scan screen opens after Allow' $scanOpen
    Start-Sleep -Seconds 4   # the camera preview runs; a crash would show now
    Shot '04-scan-screen'
    $pidNow = (A shell pidof $Pkg | Out-String).Trim()
    Check 'the app is still running with the camera open' ($pidNow -ne '')
    Check 'Cancel closes the scan screen' ((Tap-UiText @('Cancel')) -and (Wait-Until { Ui-Has 'Scan code' } 10))

    # ---- the desktop app: developer mode, loopback only, throwaway data dir
    New-Item -ItemType Directory $Data | Out-Null
    $clip = Save-Clipboard | Select-Object -Last 1
    $KinArgs = @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-received-dir', ('"{0}"' -f (Join-Path $Data 'received')), '--dev-print-pair-url', '--dev-no-windows')
    # A Kin that somebody else started in the meantime would hold the single-instance slot: wait for it to go, never kill it.
    if (-not (Wait-Until { -not (Get-Process Kin -ErrorAction SilentlyContinue) } 300)) { throw 'Another Kin.exe is running (not ours); gave up waiting for it.' }
    $started.Kin = $true
    Start-Process $KinCopy -ArgumentList $KinArgs | Out-Null
    Check 'desktop started' (Wait-Until { (Get-Status).state -eq 'not_paired' } 30)
    $pc = Get-Status
    Check 'desktop listens on loopback only' ($pc.bind -eq '127.0.0.1') "port $($pc.port)"
    & $KinCopy --dev-start-pairing
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    Check 'pairing URL has the emulator address' ($url -match "a=10\.0\.2\.2:$($pc.port)&")

    # ---- pair: the link arrives from outside the app, so the confirmation screen appears and a tap on Confirm pairs
    $paired = $false
    for ($try = 1; $try -le 3 -and -not $paired; $try++) {
        if ($try -gt 1) { Say "pairing attempt $try" }
        A shell am start -a android.intent.action.VIEW -d "'$url'" 2>$null | Out-Null
        $shown = Wait-Until { Ui-Has 'Pair with this PC?' } 15
        if ($try -eq 1) { Check 'the link opens the confirmation screen' $shown; Shot '05-confirm-pairing' }
        if ($shown) { Tap-UiText @('Confirm') | Out-Null }
        $paired = Wait-Until { (Get-Status).state -eq 'phone_connected' } 25
    }
    Check 'paired and connected after Confirm' $paired
    $pc = Get-Status
    Check "the PC lists the phone" (@($pc.devices).Count -eq 1 -and $pc.devices[0].connected) "$($pc.devices[0].name)"
    Start-Sleep -Seconds 2
    Shot '06-home'

    # ---- ping PC -> phone
    $before = if ($pc.lastPing) { $pc.lastPing.at } else { 0 }
    & $KinCopy --ping
    Check 'ping PC -> phone' (Wait-Until { $lp = (Get-Status).lastPing; $lp -and $lp.at -gt $before -and $null -ne $lp.rttMs } 15) ("{0} ms" -f (Get-Status).lastPing.rttMs)

    # ---- text phone -> PC (the share intent) and PC -> phone (--send-clipboard)
    $t1 = "release smoke text $Run"
    $beforeId = (Get-Status).lastReceived.text.id
    A shell "am start -n $Pkg/.share.ShareActivity -a android.intent.action.SEND -t text/plain --es android.intent.extra.TEXT '$t1'" | Out-Null
    Check 'text phone -> PC arrived' (Wait-Until { $r = (Get-Status).lastReceived.text; $r -and $r.id -ne $beforeId } 15)
    Check 'the PC clipboard holds that text' (Wait-Until { (Get-Clipboard -Raw) -ceq $t1 } 5) ''
    A shell input keyevent KEYCODE_HOME | Out-Null
    $t2 = "release smoke reply $Run"
    Set-Clipboard -Value $t2
    Start-Sleep -Milliseconds 800
    $beforeSent = (Get-Status).lastSent.text.id
    & $KinCopy --send-clipboard
    Check 'text PC -> phone acked' (Wait-Until { $s = (Get-Status).lastSent.text; $s -and $s.id -ne $beforeSent -and $s.result } 10) "$((Get-Status).lastSent.text.result)"
    Check 'the phone shows a notification for it' (Wait-Until { (Phone-Notifs) -match 'Text from DESK-PC' } 10)

    # ---- file PC -> phone
    $f1 = Join-Path $Work "kin-smoke-down-$Run.bin"
    $bytes = New-Object byte[] 700000; (New-Object Random 42).NextBytes($bytes); [IO.File]::WriteAllBytes($f1, $bytes)
    $h1 = Sha-Pc $f1
    & $KinCopy --send $f1
    $name1 = Split-Path $f1 -Leaf
    $got = Wait-Until { ((A shell "ls /sdcard/Download/Kin/ 2>/dev/null" | Out-String) -match [regex]::Escape($name1)) } 40
    Check 'file PC -> phone: it is in Download/Kin' $got
    $devHash = ((A shell "sha256sum /sdcard/Download/Kin/$name1" | Out-String) -split '\s+')[0]
    Check 'file PC -> phone: the SHA-256 matches' ($devHash -eq $h1) $devHash

    # ---- file phone -> PC: a file the shell puts in MediaStore, shared through the share intent
    $f2 = Join-Path $Work "kin-smoke-up-$Run.bin"
    $bytes2 = New-Object byte[] 500000; (New-Object Random 7).NextBytes($bytes2); [IO.File]::WriteAllBytes($f2, $bytes2)
    $h2 = Sha-Pc $f2
    $name2 = Split-Path $f2 -Leaf
    A push $f2 "/sdcard/Download/$name2" | Out-Null
    A shell am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE -d "file:///sdcard/Download/$name2" | Out-Null
    $mediaId = $null
    Wait-Until {
        $q = "content query --uri content://media/external/file --projection _id --where `"_display_name='$name2'`"" | & $Adb -s $Serial shell | Out-String   # through stdin: the quotes survive
        if ($q -match '_id=(\d+)') { $script:mid = $Matches[1]; $true } else { $false }
    } 20 | Out-Null
    $mediaId = $script:mid
    Check 'the test file is in MediaStore' ($null -ne $mediaId) "id $mediaId"
    if ($mediaId) {
        $uri = "content://media/external/file/$mediaId"
        $before = @(@((Get-Status).transfers.active) + @((Get-Status).transfers.recent) | Where-Object { $_ } | ForEach-Object { $_.id })
        A shell "am start --grant-read-uri-permission -n $Pkg/.share.ShareActivity -a android.intent.action.SEND -d $uri -t application/octet-stream --eu android.intent.extra.STREAM $uri" | Out-Null
        $done = Wait-Until { $x = @(@((Get-Status).transfers.active) + @((Get-Status).transfers.recent)) | Where-Object { $_ -and $_.direction -eq 'in' -and $before -notcontains $_.id } | Select-Object -First 1; if ($x -and $x.status -eq 'done') { $true } else { $false } } 60
        Check 'file phone -> PC: the PC finished the transfer' $done
        $recv = Join-Path (Join-Path $Data 'received') $name2
        Check 'file phone -> PC: the SHA-256 matches' ((Test-Path -LiteralPath $recv) -and (Sha-Pc $recv) -eq $h2)
    }

    # ---- the app did not crash (R8 problems show up as a FATAL EXCEPTION, ClassNotFound, NoSuchMethod, ...)
    $log = A logcat -d | Out-String
    $mine = ($log -split "`n") | Where-Object { $_ -match 'FATAL EXCEPTION|AndroidRuntime' -and $_ -match $Pkg }
    Check 'no crash of the app in logcat' ((@($mine).Count -eq 0) -and ($log -notmatch 'ClassNotFoundException|NoSuchMethodError|NoSuchFieldError|MissingKotlin|Could not find class')) (@($mine) -join ' ')
    Check 'the app process is still the same and running' ((A shell pidof $Pkg | Out-String).Trim() -ne '')
    Shot '07-end'
}
catch {
    Say "ERROR: $_"
    $script:Failures.Add("exception: $_")
}
finally {
    if ($started.Emulator -and ((A get-state | Out-String) -match 'device')) {
        A shell rm -rf /sdcard/Download/Kin "/sdcard/Download/kin-smoke-*" | Out-Null
        "content delete --uri content://media/external/file --where `"_display_name LIKE 'kin-smoke-%'`"" | & $Adb -s $Serial shell 2>$null | Out-Null
    }
    if (-not $KeepRunning) {
        Say 'tearing down'
        if ($started.Kin) {
            if ($KinCopy) { & $KinCopy --quit 2>$null }
            if (-not (Wait-Until { -not (Get-Process Kin -ErrorAction SilentlyContinue) } 10)) { Stop-Process -Name Kin -Force -ErrorAction SilentlyContinue }
        }
        if ($started.Emulator) { Invoke-EmuScript 'stop.ps1' @('-All') 120 | Out-Null }
        & $Adb kill-server 2>$null | Out-Null
    }
    $env:COMPUTERNAME = $script:OrigComputerName
    if ($clip) { try { Restore-Clipboard $clip | Out-Null } catch { Say "clipboard restore failed: $_" } }
    if (Test-Path $Work) { try { [IO.Directory]::Delete($Work, $true) } catch { Say "could not remove $Work" } }
    if ($script:Failures.Count -gt 0 -or $KeepRunning) { if (Test-Path $Data) { Say "data and logs kept in $Data" } }
    elseif (Test-Path $Data) { try { [IO.Directory]::Delete($Data, $true) } catch { Say "could not remove $Data" } }
}

Write-Host ''
$script:Results.GetEnumerator() | ForEach-Object { Write-Host ("{0,-5} {1}" -f $_.Value, $_.Key) }
if ($script:Failures.Count -gt 0) { Write-Host "`nFAILED: $($script:Failures -join '; ')"; exit 1 }
Write-Host "`nOK: all $($script:Results.Count) checks passed on $Avd."
exit 0
