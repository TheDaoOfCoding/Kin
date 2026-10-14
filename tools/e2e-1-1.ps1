# End-to-end check of milestone 1.1 on the emulator: the real Windows app (Kin.exe, developer mode, loopback only)
# and the real debug APK. Pair by deep link, ping both ways, check the state on both sides, tear everything down.
# Exits 0 if every check passed, 1 otherwise. Later milestones extend the section marked "EXTEND HERE".
#
#   tools\e2e-1-1.ps1                      # kin_api37 (Android 17)
#   tools\e2e-1-1.ps1 -Avd kin_api29       # Android 10
#   tools\e2e-1-1.ps1 -Build               # build Kin.exe and the debug APK first (slow)
#   tools\e2e-1-1.ps1 -KeepRunning         # leave emulator and Kin running, for debugging
#
# The desktop app only ever runs with --dev-bind 127.0.0.1 (never all interfaces, so no firewall prompt) and a throwaway
# data directory under %TEMP%, never %APPDATA%\Kin. Needs ANDROID_HOME and JAVA_HOME (user environment; read from the
# registry if the session predates them) and, with -Build, Rust (desktop\_env.ps1) and bun.
param(
    [string]$Avd = 'kin_api37',
    [switch]$Build,
    [switch]$KeepRunning,
    [string]$Kin = (Join-Path $PSScriptRoot '..\desktop\src-tauri\target\release\Kin.exe'),
    [string]$Apk = (Join-Path $PSScriptRoot '..\android\app\build\outputs\apk\debug\app-debug.apk')
)
$ErrorActionPreference = 'Continue'   # Windows PowerShell 5.1 turns native stderr text into errors under 'Stop'

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
function Phone-State { $l = Phone-Cmd 'STATE'; $h = @{}; if ($l) { foreach ($m in [regex]::Matches($l, '(\w+)=(\S+)')) { $h[$m.Groups[1].Value] = $m.Groups[2].Value } }; $h }

$started = @{ Emulator = $false; Kin = $false }
try {
    if ($Build) {
        Say 'building Kin.exe'
        Push-Location (Join-Path $Repo 'desktop'); . .\_env.ps1
        powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 | Out-Null; $ok = $LASTEXITCODE -eq 0; Pop-Location
        Check 'build desktop' $ok
        Say 'building the debug APK'
        Push-Location (Join-Path $Repo 'android'); cmd /c "gradlew.bat assembleDebug" | Out-Null; $ok = $LASTEXITCODE -eq 0
        cmd /c "gradlew.bat --stop" | Out-Null; Pop-Location
        Check 'build android' $ok
        if (-not $ok) { throw 'build failed' }
    }
    $Kin = (Resolve-Path $Kin -ErrorAction Stop).Path
    $Apk = (Resolve-Path $Apk -ErrorAction Stop).Path
    if (Get-Process Kin -ErrorAction SilentlyContinue) { throw 'Kin.exe is already running; quit it first (a dev instance and a normal one share the single-instance slot).' }

    # ---- emulator, install, permission
    Say "starting $Avd"
    $started.Emulator = $true
    if (-not (Invoke-EmuScript 'start.ps1' @('-Avd', $Avd)) -or ((A shell getprop sys.boot_completed | Out-String).Trim() -ne '1')) { throw "emulator $Avd did not start (see tools\emulator\emulator-$Avd.log)" }
    $sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    Say "API $sdk"
    A uninstall $Pkg 2>&1 | Out-Null
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
    Start-Process $Kin -ArgumentList '--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-print-pair-url', '--dev-no-windows' | Out-Null
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
    $sw = [Diagnostics.Stopwatch]::StartNew()
    A shell am start -a android.intent.action.VIEW -d "'$url'" 2>$null | Out-Null
    $paired = Wait-Until { (Get-Status).state -eq 'phone_connected' } 40
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

    # ---- unpair from the phone: the PC cleans up
    Phone-Cmd 'UNPAIR' | Out-Null
    Check 'unpair from the phone removes the device on the PC' (Wait-Until { $s = Get-Status; $s.state -eq 'not_paired' -and @($s.devices).Count -eq 0 } 15)
}
catch {
    Say "ERROR: $_"
    $script:Failures.Add("exception: $_")
}
finally {
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
    $shouldKeep = $script:Failures.Count -gt 0 -or $KeepRunning
    if ($shouldKeep -and (Test-Path $Data)) { Say "data and logs kept in $Data" }
    elseif (Test-Path $Data) { try { [IO.Directory]::Delete($Data, $true) } catch { Say "could not remove $Data" } }
}

Write-Host ''
$script:Results.GetEnumerator() | ForEach-Object { Write-Host ("{0,-5} {1}" -f $_.Value, $_.Key) }
if ($script:Failures.Count -gt 0) { Write-Host "`nFAILED: $($script:Failures -join '; ')"; exit 1 }
Write-Host "`nOK: all $($script:Results.Count) checks passed on $Avd."
exit 0

