# 2B.2: the phone (debug build on an emulator) against the REAL desktop build, a test Kin beside the user's installed one (own --data-dir, loopback only, no window,
# --dev-toast-quiet: every phone toast goes to the Action Center with no popup and no sound). ASCII only.
#
#   android\tools\verify-2b2-desktop.ps1 [-Avd kin_api37] [-Kin desktop\src-tauri\target\release\Kin.exe]
#
# phone -> PC: notifications posted by `cmd notification post` and by the debug "Mirror test" reach the test Kin (status.json: shown, icons; the phone's line);
# PC -> phone: the commands the COM class and the toast events call (`--dev-toast reply|button|dismiss <nid>`) reach the phone (the debug receiver's echo: a hash and
# a length, never the text; the dismissal removes the notification and is not echoed back); a removal on the phone removes the toast; no test string in the Kin log.
# It never presses Copy (that is the one thing that touches the PC clipboard; the desktop's own check does it with the clipboard saved and restored), never touches the
# installed Kin, its data, the Run key, SendTo, Downloads or the firewall, and refuses to run while a game or an anti-cheat is running.
param(
    [string]$Avd = 'kin_api37',
    [string]$Kin = (Join-Path $PSScriptRoot '..\..\desktop\src-tauri\target\release\Kin.exe'),
    [string]$Apk = (Join-Path $PSScriptRoot '..\app\build\outputs\apk\debug\app-debug.apk'),
    [int]$Port = 49341,
    [switch]$KeepRunning,
    [string]$Work = ''
)
. (Join-Path $PSScriptRoot '..\..\tools\kin-lock.ps1'); Enter-KinLock 'verify-2b2-desktop'
. (Join-Path $PSScriptRoot '..\..\tools\kin-dev.ps1')
if (-not $Work) { $Work = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-verify-2b2-desktop-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
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
$Service = "$Pkg/$Pkg.notifmirror.MirrorListenerService"
$Serial = 'emulator-5554'
$Data = Join-Path $Work 'data'
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Results = [ordered]@{}
$started = @{ Emulator = $false; Kin = $false }

function A { & $Adb -s $Serial @args }
function Say($m) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $m) }
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $script:Results[$name] = if ($ok) { 'PASS' } else { 'FAIL' }
    if ($ok) { Say "PASS  $name $detail" } else { Say "FAIL  $name $detail"; $script:Failures.Add($name) }
}
function Wait-Until([scriptblock]$Cond, [double]$Seconds, [int]$PollMs = 250) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds $PollMs }
    return $false
}
function Invoke-EmuScript([string]$Script, [string[]]$ScriptArgs = @(), [int]$TimeoutSec = 400) {
    $out = Join-Path $env:TEMP "kin-verify-2b2d-$Script.out"
    $p = Start-Process powershell -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $Emu $Script))) + $ScriptArgs)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; return $false }
    return $true
}
function Get-Status { try { Get-Content (Join-Path $Data 'status.json') -Raw -ErrorAction Stop | ConvertFrom-Json } catch { $null } }
function Phone-Cmd([string]$Command, [string]$Extras = '', [int]$Wait = 1200) {
    A shell "am broadcast -a app.kinrelay.android.debug.$Command -n $Pkg/.debug.DebugCommandReceiver $Extras" | Out-Null
    Start-Sleep -Milliseconds $Wait
    @(A logcat -d -s KinRelay | Where-Object { $_ -match 'CMD' } | ForEach-Object { ($_ -replace '^.*CMD ', 'CMD ') })
}
function Phone-State { (Phone-Cmd 'MIRROR_STATE' '' 800 | Where-Object { $_ -match 'CMD MIRROR_STATE' } | Select-Object -Last 1) }
function Nid-Of([string]$Key) {
    $sha = [Security.Cryptography.SHA256]::Create()
    $hex = ($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Key)) | ForEach-Object { $_.ToString('x2') }) -join ''
    $hex.Substring(0, 32)
}
function Active-Keys {
    $d = (A shell dumpsys notification --noredact | Out-String)
    $i = $d.IndexOf('Notification List:'); if ($i -lt 0) { return @() }
    $m = [regex]::Match($d.Substring($i), '(?m)^\s*(Archive|mArchive|Snoozed|Archived)')
    $part = if ($m.Success) { $d.Substring($i, $m.Index) } else { $d.Substring($i) }
    @([regex]::Matches($part, 'key=(0\|[^\s:]+)') | ForEach-Object { $_.Groups[1].Value } | Select-Object -Unique)
}
function Counts { (Get-Status).mirror.counts }

try {
    Assert-NoGameRunning
    $Kin = (Resolve-Path $Kin -ErrorAction Stop).Path
    if (-not (Test-Path $Apk)) { throw "APK missing: $Apk" }
    New-Item -ItemType Directory -Force $Data | Out-Null
    $KinCopy = Join-Path $Work 'Kin.exe'
    Copy-Item $Kin $KinCopy
    Set-KinDev -Exe $KinCopy -DataDir $Data
    Assert-NoDevKinOnDataDir
    # The switch is on in this throwaway data directory; no hotkey and no automatic clipboard sync (the clipboard is never touched).
    Set-Content -Path (Join-Path $Data 'settings.json') -Value '{"mirrorNotifications":true,"hotkey":"","autoClipboardSync":false,"openLinksAutomatically":false}' -Encoding ASCII

    Say "starting $Avd"
    $started.Emulator = $true
    $emuUp = Invoke-EmuScript 'start.ps1' @('-Avd', $Avd)
    $isBooted = { ((A shell getprop sys.boot_completed 2>$null | Out-String).Trim() -eq '1') }
    if (-not (& $isBooted)) { & $Adb kill-server 2>$null | Out-Null; Start-Sleep -Seconds 2; & $Adb start-server 2>$null | Out-Null; Wait-Until $isBooted 90 | Out-Null }
    if (-not (& $isBooted)) { throw "emulator $Avd did not start" }
    A shell svc wifi enable | Out-Null
    $sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    A uninstall $Pkg 2>&1 | Out-Null
    $out = (& $Adb -s $Serial install -r (Resolve-Path $Apk).Path 2>&1 | Out-String)
    Check 'install Kin (debug build)' ($out -match 'Success')
    if ($sdk -ge 37) { A shell pm grant $Pkg android.permission.ACCESS_LOCAL_NETWORK | Out-Null }
    if ($sdk -ge 33) { A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null }
    A shell am start -n "$Pkg/.MainActivity" | Out-Null

    $started.Kin = $true
    $KinArgs = @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-port', "$Port",
        '--dev-received-dir', ('"{0}"' -f (Join-Path $Data 'received')), '--dev-print-pair-url', '--dev-no-windows', '--dev-toast-quiet')
    $script:KinProc = Start-DevKin $KinArgs
    Check 'the test Kin started' (Wait-Until { (Get-Status).state -eq 'not_paired' } 30)
    Check 'the phone notifications layer is on in the test Kin' (Wait-Until { (Get-Status).mirror.view } 10)
    Invoke-Kin --dev-start-pairing
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    $paired = $false
    for ($try = 1; $try -le 3 -and -not $paired; $try++) {
        A shell am start -a android.intent.action.VIEW -p $Pkg -d "'$url'" --ez app.kinrelay.android.AUTO_CONFIRM true 2>$null | Out-Null
        $paired = Wait-Until { (Get-Status).state -eq 'phone_connected' } 25
    }
    Check 'paired and connected to the real test Kin' $paired
    if (-not $paired) { throw 'not paired' }
    A shell input keyevent KEYCODE_HOME | Out-Null
    Start-Sleep -Seconds 2
    Say ('mirror view of the test Kin: ' + ((Get-Status).mirror.view | ConvertTo-Json -Compress -Depth 6))

    A shell cmd notification allow_listener $Service | Out-Null
    Phone-Cmd 'MIRROR_ALLOW' "--es pkg 'com.android.shell,$Pkg'" | Out-Null
    Phone-Cmd 'MIRROR_ENABLE' | Out-Null
    Wait-Until { (Phone-State) -match 'listener=true' } 20 500 | Out-Null
    Start-Sleep -Seconds 3
    $state = Phone-State
    Check 'the phone sees the PC list notif (the setting is on)' ($state -match 'pcAcceptsNotif=true' -and $state -match 'online=true') $state
    $view = (Get-Status).mirror.view | ConvertTo-Json -Compress -Depth 6
    Check 'the PC shows the phone''s status line (apps allowed, access granted)' ($view -match '2' -and $view -match 'granted|mirroring') $view
    Phone-Cmd 'MIRROR_CLEAR_ALL' | Out-Null

    # ------------------------------------------------------------------ phone -> PC
    $c0 = Counts
    Phone-Cmd 'MIRROR_POST' '--es kind a' | Out-Null
    Phone-Cmd 'MIRROR_POST' '--es kind b' | Out-Null
    A shell "cmd notification post -t 'Real Alice' shellreal 'Hello from the real phone side'" | Out-Null
    Check 'D1 the real Kin shows the three notifications (shown counter)' (Wait-Until { $c = Counts; ([int]$c.shown + [int]$c.replaced) -ge ([int]$c0.shown + [int]$c0.replaced + 3) } 15) ((Counts | ConvertTo-Json -Compress))
    $c1 = Counts
    Check 'D1 an icon reached the PC and was cached' ([int]$c1.icons.stored -ge 1) "icons stored=$($c1.icons.stored) dropped=$($c1.icons.dropped)"
    $keys = Active-Keys
    $keyA = $keys | Where-Object { $_ -match '^0\|app\.kinrelay\.android\|1\|t\|' } | Select-Object -First 1
    $keyB = $keys | Where-Object { $_ -match '^0\|app\.kinrelay\.android\|2\|t\|' } | Select-Object -First 1
    Check 'D1 the notification keys were found on the phone' ($keyA -and $keyB) "$keyA | $keyB"
    $nidA = Nid-Of $keyA
    $nidB = Nid-Of $keyB

    # ------------------------------------------------------------------ PC -> phone: the same functions the COM class and the toast events call
    Invoke-Kin --dev-toast reply $nidA 'hello from the PC'
    $ok = Wait-Until { (Phone-State) -match 'replies=1' } 10 500
    $s = Phone-State
    Check 'D2 a reply typed on the PC reached the app behind the phone notification' ($ok -and $s -match 'lastReplySha=[0-9a-f]{8}') ($s | ForEach-Object { if ($_ -match 'replies=\d+ lastReplySha=\S+') { $Matches[0] } })
    Check 'D2 and the phone answered ack (lastAction ack)' ($s -match 'lastAction=ack') ($s -replace '.*lastAction=(\S+).*', '$1')
    Invoke-Kin --dev-toast button $nidA 1
    Check 'D3 a button pressed on the PC ran the app''s Mark as read' (Wait-Until { (Phone-State) -match 'marksRead=1' } 10 500)
    $d0 = if ((Phone-State) -match 'dismissed=(\d+)') { [int]$Matches[1] } else { -1 }
    Invoke-Kin --dev-toast dismiss $nidB
    Check 'D4 the X on the PC dismissed the notification on the phone' (Wait-Until { (Phone-State) -match ('dismissed=' + ($d0 + 1)) } 10 500)
    Start-Sleep -Seconds 2
    Check 'D4 and it is gone from the phone''s active list' (@(Active-Keys | Where-Object { $_ -eq $keyB }).Count -eq 0)
    $c2 = Counts
    # ------------------------------------------------------------------ the phone removes one: the toast goes
    Phone-Cmd 'MIRROR_CANCEL' '--es tag t --ei id 1' | Out-Null
    Check 'D5 a notification removed on the phone is removed from the PC (removed counter)' (Wait-Until { [int](Counts).removed -gt [int]$c2.removed } 10) ((Counts | ConvertTo-Json -Compress))

    # ------------------------------------------------------------------ privacy: nothing of the test strings in the Kin log
    $kinLog = Join-Path $Data 'logs\kin.log'
    $text = ''
    try { $fs = [IO.File]::Open($kinLog, 'Open', 'Read', 'ReadWrite'); $sr = New-Object IO.StreamReader($fs); $text = $sr.ReadToEnd(); $sr.Dispose() } catch { }
    $bad = @('Real Alice', 'Hello from the real', 'hello from the PC', '482913', 'Are you coming tonight', 'Example Service', $nidA, $nidB)
    $leak = @($bad | Where-Object { $text -cmatch [regex]::Escape($_) })
    Check 'D6 no notification text, reply, code or nid in the PC log' ($leak.Count -eq 0 -and $text.Length -gt 0) ($leak -join ', ')
    (Get-Status).mirror | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $Work 'mirror-status.json')
}
catch {
    Say "ERROR: $_"
    $script:Failures.Add("exception: $_")
}
finally {
    if ($started.Emulator -and ((A get-state | Out-String) -match 'device')) {
        try { Phone-Cmd 'MIRROR_CLEAR_ALL' '' 500 | Out-Null } catch {}
        try { A shell cmd notification disallow_listener $Service | Out-Null } catch {}
    }
    if ($started.Kin -and $script:KinProc) { Stop-DevKin 10 }
    if (-not $KeepRunning -and $started.Emulator) { Invoke-EmuScript 'stop.ps1' @('-All') 120 | Out-Null }
    $env:COMPUTERNAME = $script:OrigComputerName
    Exit-KinLock
}
Write-Host ''
$script:Results.GetEnumerator() | ForEach-Object { Write-Host ("{0,-5} {1}" -f $_.Value, $_.Key) }
if ($script:Failures.Count -gt 0) { Write-Host "`nFAILED: $($script:Failures -join '; ')"; exit 1 }
Write-Host "`nOK: all $($script:Results.Count) checks passed on $Avd."
exit 0
