# End-to-end check of milestone 1.1 on the emulator: the real Windows app (Kin.exe, developer mode, loopback only)
# and the real debug APK. Pair by deep link, ping both ways, check the state on both sides, tear everything down.
# Exits 0 if every check passed, 1 otherwise. Later milestones extend the section marked "EXTEND HERE".
# -Milestone 1.2 adds the text, link and clipboard scenarios of tools\e2e-1-2-scenarios.ps1 (use tools\e2e-1-2.ps1, which sets it).
# -Milestone 1.3 adds the file scenarios of tools\e2e-1-3-scenarios.ps1 (use tools\e2e-1-3.ps1, which sets it).
# -Milestone 1.4 adds the queue, expiry, 404 and notification scenarios of tools\e2e-1-4-scenarios.ps1 (use tools\e2e-1-4.ps1, which sets it).
#
#   tools\e2e-1-1.ps1                      # kin_api37 (Android 17)
#   tools\e2e-1-1.ps1 -Avd kin_api29       # Android 10
#   tools\e2e-1-1.ps1 -Build               # build Kin.exe and the debug APK first (slow)
#   tools\e2e-1-1.ps1 -KeepRunning         # leave emulator and Kin running, for debugging
#   tools\e2e-1-1.ps1 -Milestone 1.2      # the same plus milestone 1.2; it saves and restores the real PC clipboard
#
# The desktop app only ever runs with --dev-bind 127.0.0.1 (never all interfaces, so no firewall prompt) and a throwaway
# data directory under %TEMP%, never %APPDATA%\Kin. Needs ANDROID_HOME and JAVA_HOME (user environment; read from the
# registry if the session predates them) and, with -Build, Rust (desktop\_env.ps1) and bun.
param(
    [string]$Avd = 'kin_api37',
    [switch]$Build,
    [switch]$KeepRunning,
    [ValidateSet('1.1', '1.2', '1.3', '1.4')][string]$Milestone = '1.1',
    [switch]$DiscardClipboard,   # 1.2: go on although the PC clipboard holds more than plain text (it can't be put back, and is lost)
    [string[]]$Only = @(),       # 1.3, 1.4: run only these scenarios (S1, S2, ...), for debugging
    [int]$BigMb = 40,            # 1.3: size of the "big file" with one drop (the emulator does about 2 MB/s; 200 to 500 is for a manual run)
    [int]$BurstTexts = 20,       # 1.4: texts in the notification burst (the full check is 60)
    [int]$BurstFiles = 6,        # 1.4: transfers in the notification storm (the full check is 15)
    [string]$ScreenshotDir = '',   # default: an ignored folder in %LOCALAPPDATA%\Temp\kin-e2e-screens\phase-1-<n>; with -UpdateScreens: docs\results\phase-1-<n>-integration-screens
    [switch]$UpdateScreens,        # write the pictures to the tracked docs\results folder of the milestone (a run never overwrites a committed picture without this)
    [string]$Kin = (Join-Path $PSScriptRoot '..\desktop\src-tauri\target\release\Kin.exe'),
    [string]$Apk = (Join-Path $PSScriptRoot '..\android\app\build\outputs\apk\debug\app-debug.apk')
)
# Kin takes the PC's name from COMPUTERNAME and shows it on the phone, so every picture of a notification or a pairing screen would carry the real
# name of this computer. A neutral name for the run (put back at the end).
$script:OrigComputerName = $env:COMPUTERNAME
$script:PcName = 'DESK-PC'
$env:COMPUTERNAME = $script:PcName
$ErrorActionPreference = 'Continue'   # Windows PowerShell 5.1 turns native stderr text into errors under 'Stop'
# Where the pictures go. The tracked folders of docs\results hold the pictures of the reports: a run writes there only when asked (-UpdateScreens),
# otherwise to a folder in the temp area that no repository tracks. (An explicit -ScreenshotDir that points into docs\results is treated the same.)
$TrackedScreens = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\docs\results'))
$TempScreens = Join-Path $env:LOCALAPPDATA ('Temp\kin-e2e-screens\phase-1-{0}' -f $Milestone.Split('.')[1])
if (-not $ScreenshotDir) {
    $ScreenshotDir = if ($UpdateScreens) { Join-Path $PSScriptRoot ('..\docs\results\phase-1-{0}-integration-screens' -f $Milestone.Split('.')[1]) } else { $TempScreens }
}
$ScreenshotDir = [IO.Path]::GetFullPath($ScreenshotDir)
if (-not $UpdateScreens -and $ScreenshotDir.StartsWith($TrackedScreens, [StringComparison]::OrdinalIgnoreCase)) {
    Write-Host "NOTE: $ScreenshotDir is a tracked folder; the pictures go to $TempScreens instead (pass -UpdateScreens to write there)"
    $ScreenshotDir = $TempScreens
}

foreach ($name in 'ANDROID_HOME', 'JAVA_HOME') {
    if (-not (Get-Item "env:$name" -ErrorAction SilentlyContinue)) {
        $v = [Environment]::GetEnvironmentVariable($name, 'User')
        if ($v) { Set-Item "env:$name" $v }
    }
}
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$Emu = Join-Path $PSScriptRoot 'emulator'
$Adb = Join-Path $(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }) 'platform-tools\adb.exe'
$Pkg = 'app.kinrelay.android'
$Serial = 'emulator-5554'
$Data = Join-Path $env:TEMP ('kin-e2e-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
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
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) {
        if (& $Cond) { return $true }
        Start-Sleep -Milliseconds 250
    }
    return $false
}
# Runs a script of tools\emulator in a child process and waits only for that process. Called directly, the adb daemon that
# start.ps1 launches inherits the caller's output pipe, and a caller that reads the pipe (a CI step, an agent) then never returns.
function Invoke-EmuScript([string]$Script, [string[]]$ScriptArgs = @(), [int]$TimeoutSec = 300) {
    $out = Join-Path $env:TEMP "kin-e2e-$Script.out"
    $p = Start-Process powershell -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $Emu $Script))) + $ScriptArgs)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; return $false }
    # The exit code of a -File script is the last native command's, so it is not a reliable verdict: callers check the result.
    return $true
}
function Get-Status { try { Get-Content (Join-Path $Data 'status.json') -Raw -ErrorAction Stop | ConvertFrom-Json } catch { $null } }
function Phone-Cmd([string]$Command, [string]$Wait = '') {
    A logcat -c | Out-Null
    A shell am broadcast -a "app.kinrelay.android.debug.$Command" -n "$Pkg/.debug.DebugCommandReceiver" | Out-Null
    $line = $null
    Wait-Until { $script:l = (A logcat -d -s KinRelay) | Where-Object { $_ -match "CMD $Command\b" } | Select-Object -Last 1; [bool]$script:l } 15 | Out-Null
    return $script:l
}
function Parse-Rtt($line) { if ($line -match 'Rtt\(millis=([0-9.]+)\)') { [double]$Matches[1] } else { $null } }
if ($Milestone -in '1.2', '1.3', '1.4') { . (Join-Path $PSScriptRoot 'e2e-1-2-scenarios.ps1') }   # 1.3 and 1.4 reuse its helpers
if ($Milestone -in '1.3', '1.4') { . (Join-Path $PSScriptRoot 'e2e-1-3-scenarios.ps1') }          # ... and those of 1.3
if ($Milestone -eq '1.4') { . (Join-Path $PSScriptRoot 'e2e-1-4-scenarios.ps1') }
function Phone-State { $l = Phone-Cmd 'STATE'; $h = @{}; if ($l) { foreach ($m in [regex]::Matches($l, '(\w+)=(\S+)')) { $h[$m.Groups[1].Value] = $m.Groups[2].Value } }; $h }

$started = @{ Emulator = $false; Kin = $false }
try {
    if ($Milestone -eq '1.2') { Save-PcClipboard $DiscardClipboard.IsPresent }   # before anything can touch it
    if ($Milestone -in '1.3', '1.4') { Save-AllClipboard }                        # every format: 1.3 puts an image on it
    if ($Build) {
        Say 'building Kin.exe'
        Push-Location (Join-Path $Repo 'desktop'); . .\_env.ps1
        powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 | Out-Null; $ok = $LASTEXITCODE -eq 0; Pop-Location
        Check 'build desktop' $ok
        Say 'building the debug APK'
        # `Push-Location` moves PowerShell's own location, which a native command started with `&` uses as its working directory.
        # (`cmd /c "gradlew.bat ..."` didn't find the wrapper: it ran in the process's directory.) `-p` says where the project is as well.
        $androidDir = Join-Path $Repo 'android'
        Push-Location $androidDir; & .\gradlew.bat -p $androidDir assembleDebug | Out-Null; $ok = $LASTEXITCODE -eq 0
        & .\gradlew.bat -p $androidDir --stop | Out-Null; Pop-Location
        Check 'build android' $ok
        if (-not $ok) { throw 'build failed' }
    }
    $Kin = (Resolve-Path $Kin -ErrorAction Stop).Path
    $Apk = (Resolve-Path $Apk -ErrorAction Stop).Path
    # `cargo test --release` (and `cargo build --release`) overwrites target\release\Kin.exe with a build that loads the dev server instead of its
    # own pages. `tauri build` writes the exe and then the installer a moment later, so an exe that is newer than the installer is not from it.
    $installer = Get-ChildItem (Join-Path (Split-Path $Kin) 'bundle\nsis') -Filter 'Kin_*_x64-setup.exe' -ErrorAction SilentlyContinue | Sort-Object LastWriteTime | Select-Object -Last 1
    if (-not $installer -or (Get-Item $Kin).LastWriteTime -gt $installer.LastWriteTime.AddSeconds(30)) {
        throw 'Kin.exe is not from `tauri build` (the installer in bundle\nsis is missing or older: `cargo test --release` replaces the exe with a dev-server build). Run with -Build, or desktop\build.ps1.'
    }
    if (Get-Process Kin -ErrorAction SilentlyContinue) { throw 'Kin.exe is already running; quit it first (a dev instance and a normal one share the single-instance slot).' }

    # ---- emulator, install, permission
    Say "starting $Avd"
    $started.Emulator = $true
    if (-not (Invoke-EmuScript 'start.ps1' @('-Avd', $Avd)) -or ((A shell getprop sys.boot_completed | Out-String).Trim() -ne '1')) { throw "emulator $Avd did not start (see tools\emulator\emulator-$Avd.log)" }
    # The emulator boots from the snapshot the last run left, and a run that died with Wi-Fi switched off (scenarios turn it off on purpose) left it off.
    A shell svc wifi enable | Out-Null
    $sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    Say "API $sdk"
    # Files of an earlier install stay in Pictures/Kin, Movies/Kin and Download/Kin but the new install can't see (or replace) them, and a same-named
    # file then clashes with the new one: delete what the old install made while it can still do it.
    if ($Milestone -in '1.3', '1.4' -and ((A shell pm path $Pkg | Out-String) -match 'package:')) {
        A shell am broadcast -a app.kinrelay.android.debug.DELETE_RECEIVED -n "$Pkg/.debug.DebugCommandReceiver" | Out-Null
        Start-Sleep -Seconds 3
    }
    A uninstall $Pkg 2>&1 | Out-Null
    if ($Milestone -in '1.3', '1.4') {
        # ... and whatever an earlier run that died left behind (the shell may delete what no app can see)
        @'
rm -rf /sdcard/Download/Kin /sdcard/Pictures/Kin /sdcard/Movies/Kin
content delete --uri content://media/external/file --where "_data LIKE '/storage/emulated/0/%/Kin/%'"
'@ | & $Adb -s $Serial shell | Out-Null
    }
    $out = (& $Adb -s $Serial install -r $Apk 2>&1 | Out-String)
    Check 'install' ($out -match 'Success')
    if ($sdk -ge 37) {
        A shell pm grant $Pkg android.permission.ACCESS_LOCAL_NETWORK | Out-Null   # without it a connect to 10.0.2.2 just times out
        Check 'ACCESS_LOCAL_NETWORK granted' ((A shell dumpsys package $Pkg | Out-String) -match 'ACCESS_LOCAL_NETWORK: granted=true')
    }
    if ($sdk -ge 33) { A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null }
    A shell am start -n "$Pkg/.MainActivity" | Out-Null

    # ---- the desktop app: developer mode, loopback only, throwaway data dir
    New-Item -ItemType Directory $Data | Out-Null
    $started.Kin = $true
    # --dev-received-dir: received files never go to the real Downloads\Kin
    $KinArgs = @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-received-dir', ('"{0}"' -f (Join-Path $Data 'received')), '--dev-print-pair-url', '--dev-no-windows')
    Start-Process $Kin -ArgumentList $KinArgs | Out-Null
    Check 'desktop started' (Wait-Until { (Get-Status).state -eq 'not_paired' } 30)
    $pc = Get-Status
    Check 'desktop listens on loopback' ($pc.bind -eq '127.0.0.1') "port $($pc.port)"

    # ---- pair
    & $Kin --dev-start-pairing
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    $token = ([regex]::Match($url, 'tk=([^&]+)')).Groups[1].Value
    Check 'pairing URL has the emulator address' ($url -match "a=10\.0\.2\.2:$($pc.port)&")
    if ($Milestone -eq '1.2') { Test-ConfirmationScreen $url $pc }   # a link without AUTO_CONFIRM must not pair by itself
    $sw = [Diagnostics.Stopwatch]::StartNew()
    # AUTO_CONFIRM is the debug build's way past the confirmation screen that every link from outside the app gets
    # Right after the emulator booted the first connection to 10.0.2.2 now and then fails ("Couldn't reach the PC") although the PC never saw it:
    # the code is still valid, so the same link is opened again (up to three times).
    $paired = $false
    for ($try = 1; $try -le 3 -and -not $paired; $try++) {
        if ($try -gt 1) { Say "pairing attempt $try (the phone could not reach the PC the first time)" }
        A shell am start -a android.intent.action.VIEW -d "'$url'" --ez app.kinrelay.android.AUTO_CONFIRM true 2>$null | Out-Null
        $paired = Wait-Until { (Get-Status).state -eq 'phone_connected' } $(if ($try -lt 3) { 20 } else { 40 })
    }
    Check 'paired and connected' $paired ("{0:N1} s" -f $sw.Elapsed.TotalSeconds)
    $pc = Get-Status
    $ph = Phone-State
    Check 'phone state is Connected' ($ph['link'] -like 'Connected*') $ph['link']
    Check "phone shows the PC's device ID" ($ph['pcId'] -eq $pc.deviceId)
    Check "PC lists the phone's device ID" (@($pc.devices).Count -eq 1 -and $pc.devices[0].deviceId -eq $ph['phoneId'] -and $pc.devices[0].connected)

    # ---- ping both ways
    $rtt = Parse-Rtt (Phone-Cmd 'PING')
    Check 'ping phone -> PC' ($null -ne $rtt) ("{0:N1} ms" -f $rtt)
    $before = if ($pc.lastPing) { $pc.lastPing.at } else { 0 }
    & $Kin --ping
    Check 'ping PC -> phone' (Wait-Until { $lp = (Get-Status).lastPing; $lp -and $lp.at -gt $before -and $null -ne $lp.rttMs } 15) ("{0:N1} ms" -f (Get-Status).lastPing.rttMs)
    $pcLog = Get-Content (Join-Path $Data 'logs\kin.log') -Raw
    Check 'PC shows "Ping from" the phone' ($pcLog -match 'toast: Ping from ')
    Check 'phone shows "Ping from" the PC' (Wait-Until { (A shell dumpsys notification --noredact | Out-String) -match 'Ping from ' } 10)

    # ---- no secrets in either log
    $phoneLog = (A shell run-as $Pkg cat files/logs/kin.log | Out-String) + (A logcat -d -s KinRelay | Out-String)
    $pcLog = Get-Content (Join-Path $Data 'logs\kin.log') -Raw
    Check 'no pairing token in the logs' ($token.Length -eq 22 -and -not $phoneLog.Contains($token) -and -not $pcLog.Contains($token))
    Check 'no private key in the logs' ($phoneLog -notmatch 'PRIVATE KEY' -and $pcLog -notmatch 'PRIVATE KEY')

    # ---- EXTEND HERE: later milestones add their scenarios (text, clipboard, files, ...) before the unpair.
    if ($Milestone -eq '1.2') { Run-Milestone12 }
    if ($Milestone -eq '1.3') { Run-Milestone13 }
    if ($Milestone -eq '1.4') { Run-Milestone14 }

    # ---- unpair from the phone: the PC cleans up
    Phone-Cmd 'UNPAIR' | Out-Null
    Check 'unpair from the phone removes the device on the PC' (Wait-Until { $s = Get-Status; $s.state -eq 'not_paired' -and @($s.devices).Count -eq 0 } 15)
    if ($Milestone -eq '1.2') { Test-ConfirmByTap }
}
catch {
    Say "ERROR: $_"
    $script:Failures.Add("exception: $_")
}
finally {
    # (not when the emulator never came up: adb would wait for a device for ever)
    if ($Milestone -in '1.3', '1.4' -and $started.Emulator -and ((A get-state | Out-String) -match 'device')) { try { Clear-Phone13 } catch { Say "could not clean the phone: $_" } }   # the test files, before the emulator goes
    if (-not $KeepRunning) {
        Say 'tearing down'
        if ($started.Kin) {
            & $Kin --quit 2>$null
            $p = Get-Process Kin -ErrorAction SilentlyContinue
            if ($p -and -not (Wait-Until { -not (Get-Process Kin -ErrorAction SilentlyContinue) } 10)) { Stop-Process -Name Kin -Force -ErrorAction SilentlyContinue }
        }
        if ($started.Emulator) { Invoke-EmuScript 'stop.ps1' @('-All') 120 | Out-Null }
        & $Adb kill-server 2>$null | Out-Null
    }
    $env:COMPUTERNAME = $script:OrigComputerName
    # After Kin has quit, so that the restored text is not synced to the phone. Also with -KeepRunning: it is the user's clipboard.
    if ($Milestone -eq '1.2') { Restore-PcClipboard }
    if ($Milestone -in '1.3', '1.4') { Restore-AllClipboard }
    if ($Milestone -in '1.3', '1.4') { Remove-Work13 }
    if ($Milestone -eq '1.4') { Remove-Work14 }
    $shouldKeep = $script:Failures.Count -gt 0 -or $KeepRunning
    if ($shouldKeep -and (Test-Path $Data)) { Say "data and logs kept in $Data" }
    elseif (Test-Path $Data) { try { [IO.Directory]::Delete($Data, $true) } catch { Say "could not remove $Data" } }
}

Write-Host ''
if ($script:Timings -and $script:Timings.Count -gt 0) {
    Write-Host 'Arrival times:'
    $script:Timings.GetEnumerator() | ForEach-Object { Write-Host ("  {0,-95} {1}" -f $_.Key, $_.Value) }
    Write-Host ''
}
$script:Results.GetEnumerator() | ForEach-Object { Write-Host ("{0,-5} {1}" -f $_.Value, $_.Key) }
if ($script:Failures.Count -gt 0) { Write-Host "`nFAILED: $($script:Failures -join '; ')"; exit 1 }
Write-Host "`nOK: all $($script:Results.Count) checks passed on $Avd."
exit 0

