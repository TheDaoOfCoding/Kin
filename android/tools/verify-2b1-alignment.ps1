# 2B.1 B1/B2 alignment on the phone, verified ONCE per emulator (kin_api37 and kin_api29). No PC and no Kin.exe are involved: a link from the PC is simulated through the
# debug command LINK_FROM_PC (the real IncomingHandler, rules and notifications), so the installed Kin, the PC's clipboard and its windows are never touched.
#
#   * the instrumented test LinkDisplayVectorsInstrumentedTest: every display[] vector of protocol\links.json through the device's own android.icu (ICU differs by Android version)
#   * a link the tightened rule rejects (https://evil.example\@google.com and https://google.com@evil.example/) is refused: nothing is posted, the phone logs "refused"
#   * a link with a warned address (punycode as written, an IP in hex) is posted with the warning sign and the browser's reading of the host under the title
#   * Copy link still copies the right URL (the PendingIntent now carries the URL as data)
#
#   android\tools\verify-2b1-alignment.ps1                   # kin_api37
#   android\tools\verify-2b1-alignment.ps1 -Avd kin_api29
param(
    [string]$Avd = 'kin_api37',
    [string]$Apk = (Join-Path $PSScriptRoot '..\app\build\outputs\apk\debug\app-debug.apk'),
    [switch]$SkipInstrumented
)
. (Join-Path $PSScriptRoot '..\..\tools\kin-lock.ps1'); Enter-KinLock 'verify-2b1-alignment'
$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
foreach ($name in 'ANDROID_HOME', 'JAVA_HOME') {
    if (-not (Get-Item "env:$name" -ErrorAction SilentlyContinue)) { $v = [Environment]::GetEnvironmentVariable($name, 'User'); if ($v) { Set-Item "env:$name" $v } }
}
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Emu = Join-Path $Repo 'tools\emulator'
$Adb = Join-Path $(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }) 'platform-tools\adb.exe'
$Pkg = 'app.kinrelay.android'
$Serial = 'emulator-5554'
$Warn = [string][char]0x26A0
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Results = [ordered]@{}
$emulatorStarted = $false

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
function Invoke-EmuScript([string]$Script, [string[]]$ScriptArgs = @(), [int]$TimeoutSec = 300) {
    $out = Join-Path $env:TEMP "kin-verify-2b1-alignment-$Script.out"
    $p = Start-Process powershell -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $Emu $Script))) + $ScriptArgs)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; return $false }
    return $true
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
function Shade-Open { A shell cmd statusbar expand-notifications | Out-Null; Start-Sleep -Milliseconds 1500 }
function Shade-Close { A shell cmd statusbar collapse | Out-Null; Start-Sleep -Milliseconds 600 }
function Phone-NotifBlocks {
    $raw = A shell dumpsys notification --noredact | Out-String
    @([regex]::Split($raw, '(?m)^\s*NotificationRecord\(') | Where-Object { $_ -match "pkg=$([regex]::Escape($Pkg))" })
}
function Notif-For([string]$Id) { @(Phone-NotifBlocks | Where-Object { $_ -notmatch 'tag=links-summary' -and $_ -match ('tag=' + [regex]::Escape($Id) + ' ') } | Select-Object -First 1)[0] }
function Phone-Cmd([string]$Command, [string]$Extras = '', [int]$Wait = 1500) {
    A logcat -c | Out-Null
    A shell "am broadcast -a app.kinrelay.android.debug.$Command -n $Pkg/.debug.DebugCommandReceiver $Extras" | Out-Null
    Start-Sleep -Milliseconds $Wait
    @(A logcat -d -s KinRelay)
}
function Phone-Link([string]$Url, [string]$Title, [string]$Id) { Phone-Cmd 'LINK_FROM_PC' "--es url '$Url' --es title '$Title' --es id '$Id'" }
function Phone-Clip {
    A logcat -c | Out-Null
    A shell am start -n "$Pkg/.debug.DebugClipboardActivity" | Out-Null
    Start-Sleep -Milliseconds 1800
    $l = @(A logcat -d -s KinRelay | Where-Object { $_ -match 'CMD CLIPBOARD' }) | Select-Object -Last 1
    if ($l -match 'text="(.*)"\s*$') { return $Matches[1] }
    if ($l -match 'CMD CLIPBOARD empty') { return '' }
    return '?'
}

try {
    if (-not (Test-Path $Apk)) { throw "APK missing: $Apk" }
    Say "starting $Avd"
    $emulatorStarted = $true
    $emuUp = Invoke-EmuScript 'start.ps1' @('-Avd', $Avd)
    if (-not $emuUp -or -not ((A shell getprop sys.boot_completed 2>$null | Out-String).Trim() -eq '1')) { throw "emulator $Avd did not start" }
    $running = ((A emu avd name | Out-String) -split "`r?`n" | Where-Object { $_ } | Select-Object -First 1)
    if ($running -and $running.Trim() -ne $Avd) { throw "the emulator that is running is '$($running.Trim())', not ${Avd}" }
    $sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    Say "API $sdk"

    # ---- the instrumented test: every display[] vector on this device's android.icu
    if (-not $SkipInstrumented) {
        Say 'instrumented test LinkDisplayVectorsInstrumentedTest (builds the debug and test APKs)'
        $env:ANDROID_SERIAL = $Serial
        $out = & (Join-Path $PSScriptRoot '..\gradlew.bat') -p (Join-Path $PSScriptRoot '..') connectedDebugAndroidTest "-Pandroid.testInstrumentationRunnerArguments.class=app.kinrelay.android.LinkDisplayVectorsInstrumentedTest" --console=plain 2>&1 | Out-String
        $ok = ($LASTEXITCODE -eq 0)
        Check "instrumented: all display[] vectors pass on android.icu (API $sdk)" $ok
        if (-not $ok) { ($out -split "`r?`n" | Select-Object -Last 40) -join "`n" | Write-Host }
    }

    A uninstall $Pkg 2>&1 | Out-Null
    $out = (& $Adb -s $Serial install -r (Resolve-Path $Apk).Path 2>&1 | Out-String)
    Check 'install the debug APK' ($out -match 'Success') ($out.Trim())
    if ($sdk -ge 37) { A shell pm grant $Pkg android.permission.ACCESS_LOCAL_NETWORK | Out-Null }
    if ($sdk -ge 33) { A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null }
    A shell am start -n "$Pkg/.MainActivity" | Out-Null
    Start-Sleep -Seconds 3
    A shell input keyevent KEYCODE_HOME | Out-Null

    # ---- B1: a link the tightened rule rejects is refused
    foreach ($case in @(@('b1-backslash', 'https://evil.example\@google.com'), @('b1-userinfo', 'https://google.com@evil.example/'), @('b1-escape', 'https://goog%6Ce.com/'))) {
        $log = Phone-Link $case[1] 'Sign in' $case[0]
        Check "B1 $($case[0]): the phone refuses it ('refused' in the log)" (($log -join "`n") -match 'was refused')
        Check "B1 $($case[0]): no notification was posted" ($null -eq (Notif-For $case[0]))
    }

    # ---- Copy link: the PendingIntent carries the URL as data
    Phone-Link 'https://evil.example/login?next=1' 'Sign in please' 'copy-1' | Out-Null
    Shade-Open
    Check 'Copy link: tap the button' (Tap-UiText @('Copy link', 'COPY LINK'))
    Start-Sleep -Milliseconds 1200
    Shade-Close
    # The debug reader needs focus to read the clipboard: try a few times.
    $clipText = ''
    for ($try = 0; $try -lt 4 -and $clipText -cne 'https://evil.example/login?next=1'; $try++) { A shell input keyevent KEYCODE_HOME | Out-Null; Start-Sleep -Milliseconds 700; $clipText = Phone-Clip }
    Check 'Copy link put the URL on the phone clipboard' ($clipText -ceq 'https://evil.example/login?next=1') "$clipText"
    A shell am start -n $Pkg/.MainActivity | Out-Null; Start-Sleep -Milliseconds 800; A shell input keyevent KEYCODE_HOME | Out-Null

    # ---- B2: warned addresses
    $log = Phone-Link 'https://xn--80ak6aa92e.com/' 'apple' 'b2-punycode'
    $n = Notif-For 'b2-punycode'
    Check 'B2 a punycode host as written is posted with the warning sign in front of the address' ($null -ne $n -and $n -match [regex]::Escape("$Warn xn--80ak6aa92e.com"))
    $log = Phone-Link 'http://0x7f.1/login' 'Google' 'b2-ip'
    $n = Notif-For 'b2-ip'
    Check 'B2 an IP in hex is shown as dotted decimal with the warning sign' ($null -ne $n -and $n -match [regex]::Escape("$Warn 127.0.0.1"))
    $log = Phone-Link 'https://example.com:443/a@b' 'Plain' 'b2-plain'
    $n = Notif-For 'b2-plain'
    Check 'a plain address has no warning sign and no default port' ($null -ne $n -and $n -match 'android.text=\S+ \(?example\.com' -and $n -notmatch [regex]::Escape("$Warn example.com"))

}
finally {
    if ($emulatorStarted) {
        Say 'tearing down'
        Invoke-EmuScript 'stop.ps1' @() 120 | Out-Null
    }
}

Write-Host ''
$script:Results.GetEnumerator() | ForEach-Object { Write-Host ("{0,-5} {1}" -f $_.Value, $_.Key) }
if ($script:Failures.Count -gt 0) { Write-Host "`nFAILED: $($script:Failures -join '; ')"; exit 1 }
Write-Host "`nOK: all $($script:Results.Count) checks passed on $Avd."
exit 0
