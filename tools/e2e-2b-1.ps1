# End-to-end check of milestone 2B.1 (links and the browser extension): the REAL Kin.exe (a dev instance beside the user's installed Kin: own --data-dir, loopback only,
# no windows, --dev-no-open so no browser ever opens) and the REAL debug APK on the emulator, plus the real dev native host and (S5) headless Chromium.
# Exits 0 only if every check passed.
#
#   tools\e2e-2b-1.ps1                                  # all scenarios on kin_api37
#   tools\e2e-2b-1.ps1 -Only S2,S3                      # some of them (S1 setup/pairing always runs)
#   tools\e2e-2b-1.ps1 -Avd kin_api29 -Only S2api29     # the quick API 29 pass of the link-open path
#   tools\e2e-2b-1.ps1 -Build                           # build Kin.exe (release), the debug APKs, the dev host and the extension test build first (slow)
#   tools\e2e-2b-1.ps1 -KeepRunning                     # leave the emulator and the test Kin running (debugging)
#
# Scenarios (docs/results/phase-2b-1-integration.md):
#   S1 pair; both hellos carry capabilities ["link"]     S2 PC -> phone links (notification, tap, Copy link, group, lock screen, YouTube t=)
#   S3 phone -> PC links (open automatically on/off)     S4 fallbacks: old phone (fake_phone --old-phone), old PC (Kin v0.1.1 build), queued link
#   S5 the extension end to end, headless Chromium       S6 URL edge cases on both sides (protocol/links.json + the backslash and IDN cases)
#   S7 v0.1.1 transfers still work (one file each way, 10-file batches, byte for byte)
#
# Safety, by construction (see tools/README.md, kin-dev.ps1): the installed Kin is never started, stopped or asked (the test Kin has its own data directory, port, lock and pipe
# app.kinrelay.dev.<SID>.<suffix>); nothing is killed by name; the PC clipboard is saved in every format first and put back at the end; no browser window is ever opened (headless
# Chromium, --dev-no-open); the only registry key written is HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev (S5, removed in a finally, refused if it exists);
# KIN_HOST_DEV_KIN_EXE is never set; no input is injected into the PC; the emulator is silent.
param(
    [string]$Avd = 'kin_api37',
    [switch]$Build,
    [switch]$KeepRunning,
    [string[]]$Only = @(),
    [string]$ScreenshotDir = '',
    [switch]$UpdateScreens,
    [string]$Kin = (Join-Path $PSScriptRoot '..\desktop\src-tauri\target\release\Kin.exe'),
    [string]$Apk = (Join-Path $PSScriptRoot '..\android\app\build\outputs\apk\debug\app-debug.apk'),
    [string]$LinkTargetApk = (Join-Path $PSScriptRoot '..\android\link-target\build\outputs\apk\debug\link-target-debug.apk'),
    [string]$HostExe = (Join-Path $PSScriptRoot '..\desktop\native-host\target\dev\debug\kin-native-host.exe'),
    [string]$FakePhone = (Join-Path $PSScriptRoot '..\desktop\src-tauri\target\debug\examples\fake_phone.exe'),
    [string]$OldKinDir = (Join-Path $env:LOCALAPPDATA 'Temp\kin-e2e2b1-oldpc'),   # a Kin built from the v0.1.1 tag (git archive + cargo build, see the integration results); S4 skips the old-PC part without it
    [int]$Port = 49325,
    [string]$Work = ''
)
. (Join-Path $PSScriptRoot 'kin-lock.ps1'); Enter-KinLock 'e2e-2b-1'
. (Join-Path $PSScriptRoot 'kin-dev.ps1')
. (Join-Path $PSScriptRoot '..\desktop\clipboard-backup.ps1')
$script:OrigComputerName = $env:COMPUTERNAME
$env:COMPUTERNAME = 'DESK-PC'    # so no screenshot carries the real name of this computer
$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
foreach ($name in 'ANDROID_HOME', 'JAVA_HOME') {
    if (-not (Get-Item "env:$name" -ErrorAction SilentlyContinue)) { $v = [Environment]::GetEnvironmentVariable($name, 'User'); if ($v) { Set-Item "env:$name" $v } }
}
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$Emu = Join-Path $PSScriptRoot 'emulator'
$Adb = Join-Path $(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }) 'platform-tools\adb.exe'
$Pkg = 'app.kinrelay.android'
$Target = 'app.kinrelay.linktarget'
$Serial = 'emulator-5554'
if (-not $Work) { $Work = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-e2e-2b-1-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
$Data = Join-Path $Work 'data'
$Dumps = Join-Path $Work 'dumps'
$RecvDir = Join-Path $Data 'received'
$Suffix = 'e2e2b1'
$Sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$DevPipe = "\\.\pipe\app.kinrelay.dev.$Sid.$Suffix"
$ExtId = 'cbioefpddjpaionhljhffgbedncfchmn'
$Owned = 'https://kin-link-test.example/watch?v=dQw4w9WgXcQ&t=83s&list=PLabc123#frag'   # a host the throwaway link-target app owns, like YouTube owns youtube.com
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Results = [ordered]@{}
$script:Notes = New-Object System.Collections.Generic.List[string]
$script:Edge = New-Object System.Collections.Generic.List[object]
$started = @{ Emulator = $false; Kin = $false }
$TempScreens = Join-Path $env:LOCALAPPDATA 'Temp\kin-e2e-screens\phase-2b-1'
$TrackedScreens = [IO.Path]::GetFullPath((Join-Path $Repo 'docs\results\phase-2b-1-integration-screens'))
if (-not $ScreenshotDir) { $ScreenshotDir = if ($UpdateScreens) { $TrackedScreens } else { $TempScreens } }
$ScreenshotDir = [IO.Path]::GetFullPath($ScreenshotDir)
if (-not $UpdateScreens -and $ScreenshotDir.StartsWith($TrackedScreens, [StringComparison]::OrdinalIgnoreCase)) { $ScreenshotDir = $TempScreens }   # a run never overwrites tracked pictures without -UpdateScreens

function A { & $Adb -s $Serial @args }
function Say($m) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $m) }
function Note([string]$m) { $script:Notes.Add($m); Say "NOTE: $m" }
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
    $out = Join-Path $env:TEMP "kin-e2e-2b-1-$Script.out"
    $p = Start-Process powershell -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $Emu $Script))) + $ScriptArgs)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; return $false }
    return $true
}
function Get-Status { try { Get-Content (Join-Path $Data 'status.json') -Raw -ErrorAction Stop | ConvertFrom-Json } catch { $null } }
function Pc-Log { (Get-ChildItem (Join-Path $Data 'logs') -Filter 'kin*.log' -ErrorAction SilentlyContinue | Sort-Object Name | ForEach-Object { Get-Content $_.FullName -Raw }) -join "`n" }
function Pc-LogCount([string]$Pattern) { ([regex]::Matches((Pc-Log), $Pattern)).Count }
function Phone-Log { (A shell run-as $Pkg cat files/logs/kin.log | Out-String) }
function Shot([string]$Name) {
    if (-not $ScreenshotDir) { return }
    New-Item -ItemType Directory -Force $ScreenshotDir | Out-Null
    Invoke-EmuScript 'screenshot.ps1' @('-Out', ('"{0}"' -f (Join-Path $ScreenshotDir "$Name.png"))) 60 | Out-Null
}

# ---------------------------------------------------------------- the phone (UI, notifications, debug commands)
function Ui-Dump { A shell uiautomator dump /sdcard/kin-ui.xml 2>$null | Out-Null; A shell cat /sdcard/kin-ui.xml | Out-String }
function Tap-UiText([string[]]$Texts, [int]$Tries = 8) {
    for ($try = 0; $try -lt $Tries; $try++) {
        $x = Ui-Dump
        foreach ($t in $Texts) {
            if ($x -match ('text="{0}"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"' -f [regex]::Escape($t))) {
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
# There is no `cmd notification cancel_all`: the shade's own "Clear all" button removes everything that can be cleared (the service notification stays).
function Clear-Notifs {
    Shade-Open
    Tap-UiText @('Clear all') 3 | Out-Null
    Start-Sleep -Milliseconds 800
    Shade-Close
}
function Phone-NotifBlocks {
    $raw = A shell dumpsys notification --noredact | Out-String
    @([regex]::Split($raw, '(?m)^\s*NotificationRecord\(') | Where-Object { $_ -match "pkg=$([regex]::Escape($Pkg))" })
}
# One notification of the app by a part of its title (never the group summary, whose lines hold the addresses).
function Notif-For([string]$Part) { @(Phone-NotifBlocks | Where-Object { $_ -notmatch 'tag=links-summary' -and $_ -match ('android.title=String \(' + [regex]::Escape($Part)) } | Select-Object -First 1)[0] }
function Notif-Actions([string]$Block) { @([regex]::Matches($Block, '(?m)^\s+\[\d+\]\s+"([^"]+)"') | ForEach-Object { $_.Groups[1].Value }) }
function Notif-Field([string]$Block, [string]$Name) { if ($Block -match ('android\.' + $Name + '=\w+ \(([^\r\n]*)\)\r?\n')) { $Matches[1] } else { $null } }
function Phone-Cmd([string]$Command, [string]$Extras = '', [int]$Wait = 1500) {
    A logcat -c | Out-Null
    A shell "am broadcast -a app.kinrelay.android.debug.$Command -n $Pkg/.debug.DebugCommandReceiver $Extras" | Out-Null
    Start-Sleep -Milliseconds $Wait
    @(A logcat -d -s KinRelay)
}
function Phone-State { $l = (Phone-Cmd 'STATE' '' 800 | Where-Object { $_ -match 'CMD STATE' } | Select-Object -Last 1); $h = @{}; if ($l) { foreach ($m in [regex]::Matches($l, '(\w+)=(\S+)')) { $h[$m.Groups[1].Value] = $m.Groups[2].Value } }; $h }
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
function Phone-Link([string]$Url, [string]$Title, [string]$Id = '') {   # the debug shortcut (the phone's own IncomingHandler); S2 uses the real PC instead
    $extras = "--es url '$Url' --es title '$Title'"; if ($Id) { $extras += " --es id '$Id'" }
    Phone-Cmd 'LINK_FROM_PC' $extras | Out-Null
}
# The URL (and the title) go to the phone as FILES read by a script on the phone, so that no quote, space, control character or non-ASCII letter depends on how Windows PowerShell 5.1 passes
# arguments to adb.exe (it drops embedded double quotes, which split a URL with a space at the space).
$script:ShareScriptPushed = $false
function Share-Url([string]$Text, [string]$Subject = '') {
    $enc = New-Object Text.UTF8Encoding($false)
    if (-not $script:ShareScriptPushed) {
        $sh = "am start -n $Pkg/.share.ShareActivity -a android.intent.action.SEND -t text/plain --es android.intent.extra.TEXT ""`$(cat /data/local/tmp/kin-e2e-url.txt)"" --es android.intent.extra.SUBJECT ""`$(cat /data/local/tmp/kin-e2e-subject.txt)""`n"
        [IO.File]::WriteAllText((Join-Path $Work 'share.sh'), $sh, $enc)
        A push (Join-Path $Work 'share.sh') /data/local/tmp/kin-e2e-share.sh | Out-Null
        $script:ShareScriptPushed = $true
    }
    [IO.File]::WriteAllBytes((Join-Path $Work 'url.txt'), $enc.GetBytes($Text))
    [IO.File]::WriteAllBytes((Join-Path $Work 'subject.txt'), $enc.GetBytes($Subject))
    A push (Join-Path $Work 'url.txt') /data/local/tmp/kin-e2e-url.txt | Out-Null
    A push (Join-Path $Work 'subject.txt') /data/local/tmp/kin-e2e-subject.txt | Out-Null
    A shell sh /data/local/tmp/kin-e2e-share.sh | Out-Null
}

# ---------------------------------------------------------------- the PC: the real native host, driven over stdin/stdout (what a browser does), against the test Kin's pipe
function Read-Exactly($Stream, [int]$Count, [int]$TimeoutMs) {
    $buf = New-Object byte[] $Count; $got = 0
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($got -lt $Count) {
        $left = $TimeoutMs - [int]$sw.ElapsedMilliseconds
        if ($left -le 0) { return $null }
        $t = $Stream.ReadAsync($buf, $got, $Count - $got)
        if (-not $t.Wait($left)) { return $null }
        if ($t.Result -le 0) { return $null }
        $got += $t.Result
    }
    , $buf
}
function Invoke-NativeHost {
    param([Parameter(Mandatory = $true)]$Request, [string]$Pipe = $DevPipe, [int]$TimeoutSec = 60, [switch]$Raw)
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $HostExe
    $psi.Arguments = "chrome-extension://$ExtId/"
    $psi.UseShellExecute = $false; $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true; $psi.CreateNoWindow = $true
    $psi.EnvironmentVariables['KIN_HOST_DEV_PIPE'] = $Pipe
    $p = [Diagnostics.Process]::Start($psi)
    try {
        $json = if ($Request -is [string]) { $Request } else { ConvertTo-Json $Request -Compress -Depth 8 }
        $body = (New-Object Text.UTF8Encoding($false)).GetBytes($json)
        $in = $p.StandardInput.BaseStream
        $in.Write([BitConverter]::GetBytes([uint32]$body.Length), 0, 4); $in.Write($body, 0, $body.Length); $in.Flush()
        $out = $p.StandardOutput.BaseStream
        $hdr = Read-Exactly $out 4 ($TimeoutSec * 1000)
        if (-not $hdr) { return [pscustomobject]@{ ok = $false; code = 'NO_ANSWER'; message = 'the host did not answer' } }
        $len = [BitConverter]::ToUInt32($hdr, 0)
        $resp = Read-Exactly $out ([int]$len) ($TimeoutSec * 1000)
        if (-not $resp) { return [pscustomobject]@{ ok = $false; code = 'NO_ANSWER'; message = 'the host answer was cut' } }
        $text = [Text.Encoding]::UTF8.GetString($resp)
        if ($Raw) { return $text }
        return ($text | ConvertFrom-Json)
    } finally {
        try { $p.StandardInput.Close() } catch { }
        if (-not $p.WaitForExit(3000)) { Stop-OwnProcess $p $HostExe }
        $p.Dispose()
    }
}
function Send-ViaHost([string]$Kind, [hashtable]$Fields, [string]$Pipe = $DevPipe) {
    $req = @{ v = 1; id = [guid]::NewGuid().ToString(); type = 'send'; kind = $Kind }
    foreach ($k in $Fields.Keys) { $req[$k] = $Fields[$k] }
    Invoke-NativeHost -Request $req -Pipe $Pipe
}

# ---------------------------------------------------------------- PC side helpers copied in spirit from e2e-1-3 (files)
function Long([string]$Path) { if ($Path.StartsWith('\\?\')) { $Path } else { '\\?\' + $Path } }
function Pc-Sha([string]$Path) {
    $fs = [IO.File]::OpenRead((Long $Path))
    try { ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($fs)) -replace '-', '').ToLower() } finally { $fs.Dispose() }
}
function Run-As([string]$Script) { ($Script | & $Adb -s $Serial shell "run-as $Pkg sh" | Out-String) }
function Phone-SourceHashes {
    $h = @{}
    foreach ($l in ((Run-As 'cd files/debug-share 2>/dev/null && sha256sum *') -split "`r?`n")) { if ($l -match '^([0-9a-f]{64})  (.+)$') { $h[$Matches[2]] = $Matches[1] } }
    $h
}
function Make-PhoneFiles([string]$Spec) {
    A logcat -c | Out-Null
    A shell "am broadcast -a app.kinrelay.android.debug.MAKE_FILES -n $Pkg/.debug.DebugCommandReceiver --es spec '$Spec'" | Out-Null
    $n = @($Spec -split ',').Count
    Wait-Until { @((A logcat -d -s KinRelay) | Where-Object { $_ -match 'CMD MAKE_FILES ' }).Count -ge $n } 90 | Out-Null
}
function Share-Files([string]$Names) { A shell "am start -n $Pkg/.debug.DebugShareActivity --es files '$Names'" | Out-Null }
function Pc-Ids { $s = Get-Status; @(@($s.transfers.active) + @($s.transfers.recent)) | Where-Object { $_ } | ForEach-Object { $_.id } }
function Wait-NewPcTransfer([string[]]$Before, [string]$Dir, [int]$Sec = 90) {
    $script:npt = $null
    Wait-Until { $s = Get-Status; $script:npt = @($s.transfers.recent) | Where-Object { $_ -and $_.id -notin $Before -and ($Dir -eq '' -or $_.direction -eq $Dir) } | Select-Object -First 1; [bool]$script:npt } $Sec | Out-Null
    $script:npt
}
function Phone-Received {
    $lines = @(Phone-Cmd 'RECEIVED' '' 1500)
    for ($i = 0; $i -lt 6; $i++) {
        $more = @(A logcat -d -s KinRelay)
        if ($more.Count -eq $lines.Count -and ($more | Where-Object { $_ -match 'CMD RECEIVED' })) { break }
        $lines = $more; Start-Sleep -Milliseconds 700
    }
    foreach ($l in $lines) {
        if ($l -match "CMD RECEIVED (images|video|downloads) id=(\d+) name='(.*)' size=(\d+) mime=(\S*) path=(\S+) pending=(\d)") {
            [pscustomobject]@{ Kind = $Matches[1]; Name = $Matches[3]; Size = [long]$Matches[4]; Mime = $Matches[5]; Path = $Matches[6]; Pending = [int]$Matches[7] }
        }
    }
}
function Phone-RecvSha($Rec) { Phone-FileSha ('/sdcard/' + $Rec.Path.Trim('/') + '/' + $Rec.Name) }   # RECEIVED gives the folder (Download/Kin/) and the name
function Phone-FileSha([string]$Path) { $o = (A shell "sha256sum '$Path'" | Out-String); if ($o -match '^([0-9a-f]{64})') { $Matches[1] } else { $null } }


# ================================================================ setup helpers
function Start-Emulator {
    Say "starting $Avd"
    $script:started.Emulator = $true
    $emuUp = Invoke-EmuScript 'start.ps1' @('-Avd', $Avd)
    $isBooted = { ((A shell getprop sys.boot_completed 2>$null | Out-String).Trim() -eq '1') }
    if ($emuUp -and -not (& $isBooted)) { & $Adb kill-server 2>$null | Out-Null; Start-Sleep -Seconds 2; & $Adb start-server 2>$null | Out-Null; Wait-Until $isBooted 60 | Out-Null }
    if (-not $emuUp -or -not (& $isBooted)) { throw "emulator $Avd did not start (see tools\emulator\emulator-$Avd.log)" }
    A shell svc wifi enable | Out-Null
    $running = ((A emu avd name | Out-String) -split "`r?`n" | Where-Object { $_ } | Select-Object -First 1)
    if ($running -and $running.Trim() -ne $Avd) { throw "the emulator that is running is '$($running.Trim())', not ${Avd}: stop it first (tools\emulator\stop.ps1)" }
    $script:Sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    Say "API $($script:Sdk)"
}
function Install-Apps {
    A uninstall $Pkg 2>&1 | Out-Null
    A uninstall $Target 2>&1 | Out-Null
    # files an earlier install left in Pictures/Kin, Download/Kin: the new install can't see them
    @'
rm -rf /sdcard/Download/Kin /sdcard/Pictures/Kin /sdcard/Movies/Kin
content delete --uri content://media/external/file --where "_data LIKE '/storage/emulated/0/%/Kin/%'"
'@ | & $Adb -s $Serial shell | Out-Null
    $out = (& $Adb -s $Serial install -r $Apk 2>&1 | Out-String)
    Check 'install Kin (debug build)' ($out -match 'Success') ($out.Trim())
    $out = (& $Adb -s $Serial install -r $LinkTargetApk 2>&1 | Out-String)
    Check 'install the link target test app' ($out -match 'Success') ($out.Trim())
    if ($script:Sdk -ge 37) { A shell pm grant $Pkg android.permission.ACCESS_LOCAL_NETWORK | Out-Null }
    if ($script:Sdk -ge 33) { A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null }
    A shell am start -n "$Pkg/.MainActivity" | Out-Null
    # Android 12+ sends a web link to a non-browser app only if it is verified for the host or the user approved it (Settings > Open by default)
    if ($script:Sdk -ge 31) {
        A shell pm set-app-links --package $Target 1 kin-link-test.example | Out-Null
        A shell pm set-app-links-user-selection --user 0 --package $Target true kin-link-test.example | Out-Null
    }
}
function Start-TestKin([string[]]$Extra = @()) {
    $env:KIN_DEV_PIPE_SUFFIX = $Suffix
    $script:KinArgs = @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-port', "$Port",
        '--dev-received-dir', ('"{0}"' -f $RecvDir), '--dev-print-pair-url', '--dev-no-windows', '--dev-no-open')
    $script:started.Kin = $true
    Start-DevKin ($script:KinArgs + $Extra) | Out-Null
}
function Pair-Phone {
    $urlFile = Join-Path $Data 'pair-url.txt'
    Remove-Item $urlFile -ErrorAction SilentlyContinue
    Invoke-Kin --dev-start-pairing
    Wait-Until { Test-Path $urlFile } 10 | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    $paired = $false
    for ($try = 1; $try -le 3 -and -not $paired; $try++) {
        if ($try -gt 1) { Say "pairing attempt $try" }
        A shell am start -a android.intent.action.VIEW -p $Pkg -d "'$url'" --ez app.kinrelay.android.AUTO_CONFIRM true 2>$null | Out-Null
        $paired = Wait-Until { (Get-Status).state -eq 'phone_connected' } $(if ($try -lt 3) { 25 } else { 40 })
    }
    $paired
}
function Link-Rows { @(Phone-Texts | Where-Object { $_ -match 'kind=link' }) }   # `down` = from the PC, `up` = to the PC

# ================================================================ more helpers
function Hex-ToString([string]$Hex) {
    if (-not $Hex) { return '' }
    $b = New-Object byte[] ($Hex.Length / 2)
    for ($i = 0; $i -lt $b.Length; $i++) { $b[$i] = [Convert]::ToByte($Hex.Substring(2 * $i, 2), 16) }
    [Text.Encoding]::UTF8.GetString($b)
}
# The phone's text and link table with the exact bytes of title and body (hex, so that no control character or quote depends on adb's output). Needs the debug build (run-as) and the system image's sqlite3.
function Phone-DbTexts {
    $out = (A shell "run-as $Pkg sqlite3 databases/kin.db 'select id,direction,status,kind,hex(title),hex(body),source from texts order by createdAtMillis'" | Out-String)
    foreach ($l in ($out -split "`r?`n")) {
        $f = $l.Split('|')
        if ($f.Count -ge 7) { [pscustomobject]@{ Id = $f[0]; Dir = $f[1]; Status = $f[2]; Kind = $f[3]; Title = (Hex-ToString $f[4]); Body = (Hex-ToString $f[5]); Source = $f[6] } }
    }
}
function Get-PcView([string]$Name) {
    $f = Join-Path $Data "view-$Name.json"
    Remove-Item $f -ErrorAction SilentlyContinue
    Invoke-Kin --dev-dump-view $Name
    if (-not (Wait-Until { Test-Path $f } 10)) { return $null }
    Start-Sleep -Milliseconds 300
    Get-Content $f -Raw -Encoding UTF8 | ConvertFrom-Json
}
function Pc-Opened { Pc-LogCount 'would have been opened in the browser' }
function Pc-Setting([string]$Key, [string]$Value) {
    $f = Join-Path $Data 'dev-set-result.json'
    Remove-Item $f -ErrorAction SilentlyContinue
    Invoke-Kin --dev-set $Key $Value
    Wait-Until { (Test-Path $f) -and (Get-Content $f -Raw) -match '"ok":true' } 10
}
function Wait-Notif([string]$Part, [int]$Sec = 15) { $script:wn = $null; Wait-Until { $script:wn = Notif-For $Part; [bool]$script:wn } $Sec | Out-Null; $script:wn }
function Phone-ForceStopAndBack {
    A shell am force-stop $Pkg | Out-Null
}
function Phone-Reconnect {
    A shell am start -n "$Pkg/.MainActivity" | Out-Null
    Start-Sleep -Seconds 3
    Phone-Cmd 'CONNECT' '' 500 | Out-Null
}

# ================================================================ S2 PC -> phone links
function Run-S2 {
    Say 'S2: PC -> phone links (real Kin, real host, the wire, the phone)'
    Clear-Notifs
    $r = Send-ViaHost 'link' @{ url = $Owned; title = 'Cat video: part 2'; timestampSec = 83 }
    Check 'S2 the host and Kin accepted the link (delivered, not queued)' ($r.ok -eq $true -and $r.queued -eq $false) ($r | ConvertTo-Json -Compress)
    Check 'S2 a notification with the page title was posted' ([bool](Wait-Notif 'Cat video: part 2'))
    $n = Notif-For 'Cat video: part 2'
    $n | Set-Content (Join-Path $Dumps 's2-owned.txt')
    Check 'S2 the real domain is the line under the title' ((Notif-Field $n 'text') -eq 'kin-link-test.example') (Notif-Field $n 'text')
    Check 'S2 it says what a tap does: Open in Link target' ((Notif-Field $n 'subText') -eq 'Open in Link target') (Notif-Field $n 'subText')
    $actions = @(Notif-Actions $n)
    Check 'S2 the only button is Copy link' ($actions.Count -eq 1 -and $actions[0] -eq 'Copy link') ($actions -join ', ')
    Check 'S2 lock screen: private by default (the public version is "Link from DESK-PC", without the title)' ($n -match 'vis=PRIVATE' -and $n -match 'android\.title=String \(Link from DESK-PC\)')
    $row = @(Phone-DbTexts | Where-Object { $_.Dir -eq 'down' -and $_.Kind -eq 'link' -and $_.Title -eq 'Cat video: part 2' })[0]
    Check 'S2 the phone stored the URL byte for byte (t=83s, list and fragment unchanged)' ($row -and $row.Body -ceq $Owned) $row.Body
    Shade-Open; Shot 's2-01-link-notification'
    Check 'S2 tap the notification' (Tap-UiText @('Cat video: part 2'))
    Check 'S2 the app that owns the link opened, not the browser' (Wait-Until { (Phone-Resumed) -match $Target } 10) (Phone-Resumed)
    $ui = Ui-Dump
    Check 'S2 it was handed the same URL' ($ui -match 'LINK TARGET OPENED' -and $ui -match [regex]::Escape($Owned.Replace('&', '&amp;')))
    Shot 's2-02-link-target-opened'
    A shell input keyevent KEYCODE_HOME | Out-Null; Start-Sleep -Milliseconds 800
    Check 'S2 the tapped notification is gone' (-not (Notif-For 'Cat video: part 2'))

    # a real YouTube URL with a timestamp, sent as a page (a separate kind): arrives unchanged
    $yt = 'https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=83s&list=PLabc123#frag'
    $r = Send-ViaHost 'page' @{ url = $yt; title = 'YouTube video' }
    Check 'S2 a YouTube page with t= is accepted' ($r.ok -eq $true) ($r.code)
    Check 'S2 its notification shows www.youtube.com' ([bool](Wait-Notif 'YouTube video') -and (Notif-Field (Notif-For 'YouTube video') 'text') -eq 'www.youtube.com') (Notif-Field (Notif-For 'YouTube video') 'text')
    $row = @(Phone-DbTexts | Where-Object { $_.Dir -eq 'down' -and $_.Title -eq 'YouTube video' })[0]
    Check 'S2 the YouTube URL (t=83s, list, fragment) is stored byte for byte' ($row -and $row.Body -ceq $yt) $row.Body
    $yb = Notif-For 'YouTube video'
    $lab = Notif-Field $yb 'subText'
    Check "S2 the label names the app that owns the link (YouTube is on this image) or says Open link, never 'browser'" ($lab -eq 'Open in YouTube' -or $lab -eq 'Open link') $lab

    # a title that pretends to be another address; Copy link
    $r = Send-ViaHost 'link' @{ url = 'https://evil.example/login?next=1'; title = 'https://accounts.google.com - Sign in' }
    Check 'S2 the spoofing title link is accepted' ($r.ok -eq $true)
    $e = Wait-Notif 'https://accounts.google.com - Sign in'
    $e | Set-Content (Join-Path $Dumps 's2-spoof.txt')
    Check 'S2 the address line is the real host, whatever the title says' ($e -and (Notif-Field $e 'text') -eq 'evil.example') (Notif-Field $e 'text')
    Shade-Open; Shot 's2-03-spoof-title-real-address'
    Check 'S2 tap Copy link' (Tap-UiText @('Copy link'))
    Start-Sleep -Milliseconds 1200; Shade-Close
    $c = Phone-Clip
    Check 'S2 Copy link put the URL on the phone clipboard' ($c -ceq 'https://evil.example/login?next=1') $c

    # a group
    Clear-Notifs; Start-Sleep -Milliseconds 800
    1..3 | ForEach-Object { Send-ViaHost 'link' @{ url = "https://example.org/g$_"; title = "Group link $_" } | Out-Null; Start-Sleep -Milliseconds 500 }
    Start-Sleep -Seconds 2
    $summary = @(Phone-NotifBlocks | Where-Object { $_ -match 'tag=links-summary' })
    $summary | Set-Content (Join-Path $Dumps 's2-summary.txt')
    Check 'S2 three links in a row are one group with a summary' ($summary.Count -ge 1 -and ($summary -join "`n") -match '[3-9] links from DESK-PC')
    Shade-Open; Shot 's2-04-links-grouped'; Shade-Close

    # lock-screen setting: default hides the title; "Show link previews on the lock screen" shows it
    Phone-Cmd 'SET_LINK_PREFS' '--ez lockscreen true' 800 | Out-Null
    Send-ViaHost 'link' @{ url = 'https://example.com/shown'; title = 'Shown on lock screen' } | Out-Null
    $s = Wait-Notif 'Shown on lock screen'
    Check 'S2 lock-screen setting on: the notification is public' ($s -and $s -match 'vis=PUBLIC')
    Phone-Cmd 'SET_LINK_PREFS' '--ez lockscreen false' 800 | Out-Null
    Send-ViaHost 'link' @{ url = 'https://example.com/hidden'; title = 'Hidden on lock screen' } | Out-Null
    $s = Wait-Notif 'Hidden on lock screen'
    Check 'S2 lock-screen setting off (default): the notification is private again' ($s -and $s -match 'vis=PRIVATE')
    Clear-Notifs
}
function Run-S2api29 {
    Say 'S2api29: the Android 10 path (resolve check, no REQUIRE_NON_BROWSER), the link from the real PC'
    Clear-Notifs
    $r = Send-ViaHost 'link' @{ url = $Owned; title = 'Cat video: part 2' }
    Check 'A29 the host and Kin accepted the link' ($r.ok -eq $true)
    $n = Wait-Notif 'Cat video: part 2'
    $n | Set-Content (Join-Path $Dumps 's2-api29-owned.txt')
    Check 'A29 Open in Link target (found by the resolve check)' ($n -and (Notif-Field $n 'subText') -eq 'Open in Link target') (Notif-Field $n 'subText')
    $actions = @(Notif-Actions $n)
    Check 'A29 the only button is Copy link' ($actions.Count -eq 1 -and $actions[0] -eq 'Copy link') ($actions -join ', ')
    Shade-Open; Shot 'api29-01-link-notification'
    Check 'A29 tap the notification' (Tap-UiText @('Cat video: part 2'))
    Check 'A29 the owning app opened with the same URL' ((Wait-Until { (Phone-Resumed) -match $Target } 10) -and (Ui-Dump) -match [regex]::Escape($Owned.Replace('&', '&amp;'))) (Phone-Resumed)
    A shell input keyevent KEYCODE_HOME | Out-Null
    Send-ViaHost 'link' @{ url = 'https://example.org/browse-me'; title = 'A plain page' } | Out-Null
    $b = Wait-Notif 'A plain page'
    Check 'A29 a link nobody owns says Open link' ($b -and (Notif-Field $b 'subText') -eq 'Open link')
    Shade-Open
    Check 'A29 tap the plain link' (Tap-UiText @('A plain page'))
    Start-Sleep -Seconds 3
    $resumed = Phone-Resumed
    Check 'A29 the browser took it (not the link target, no crash)' ($resumed -notmatch $Target -and $resumed -match 'ResumedActivity') $resumed
    Shot 'api29-02-browser'
    A shell input keyevent KEYCODE_HOME | Out-Null
}

# ================================================================ S3 phone -> PC links
function Run-S3 {
    Say 'S3: phone -> PC links (share sheet)'
    $sv = Get-PcView 'settings'
    $dflt = if ($sv.PSObject.Properties.Name -contains 'openLinksAutomatically') { $sv.openLinksAutomatically } else { $null }
    Check 'S3 "Open links from phone automatically" is on by default' ($dflt -eq $true) "$dflt"
    $o0 = Pc-Opened; $l0 = Pc-LogCount 'link \S+ from device \S+: \d+ characters, opened in the default browser'
    $u1 = 'https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=83s'
    Share-Url $u1 'A shared video'
    Check 'S3 (on) the PC asked its opener to open it (the mocked opener: no browser started)' (Wait-Until { (Pc-Opened) -gt $o0 } 20)
    Check 'S3 (on) the log says opened in the default browser' ((Pc-LogCount 'link \S+ from device \S+: \d+ characters, opened in the default browser') -gt $l0)
    Check 'S3 (on) the toast is "Opened link from phone"' (Wait-Until { (Pc-LogCount 'toast: Opened link from phone') -ge 1 } 5)
    $h = Get-PcView 'history'
    $it = @($h.items | Where-Object { $_.direction -eq 'in' -and $_.isLink -and $_.title -eq 'A shared video' })[0]
    Check 'S3 (on) the history has the link, with its title and the exact URL' ($it -and $it.text -ceq $u1) $it.text
    $row = @(Phone-DbTexts | Where-Object { $_.Dir -eq 'up' -and $_.Kind -eq 'link' -and $_.Title -eq 'A shared video' })[0]
    Check 'S3 (on) the phone keeps it as a link, done' ($row -and $row.Status -eq 'done' -and $row.Body -ceq $u1) "$($row.Status)"
    $pcClip = try { [Windows.Forms.Clipboard]::GetText() } catch { '' }
    Check 'S3 (on) the link was not put on the PC clipboard by Kin' ($pcClip -ne $u1)

    Check 'S3 turn the setting off (--dev-set)' (Pc-Setting 'openLinksAutomatically' 'false')
    $o1 = Pc-Opened; $f1 = Pc-LogCount 'not opened \(the setting is off'
    $u2 = 'https://example.com/off-page?q=1#x'
    Share-Url $u2 'Off page'
    Check 'S3 (off) the PC did NOT open it and offered it in a toast' (Wait-Until { (Pc-LogCount 'not opened \(the setting is off') -gt $f1 } 20)
    Check 'S3 (off) the opener was not asked' ((Pc-Opened) -eq $o1)
    Check 'S3 (off) the toast is "Link from phone" (Open and Copy buttons)' (Wait-Until { (Pc-LogCount 'toast: Link from phone') -ge 1 } 5)
    $h = Get-PcView 'history'
    $it2 = @($h.items | Where-Object { $_.direction -eq 'in' -and $_.isLink -and $_.title -eq 'Off page' })[0]
    Check 'S3 (off) the history has it' ($it2 -and $it2.text -ceq $u2)
    $res = Join-Path $Data 'dev-history-result.json'
    Remove-Item $res -ErrorAction SilentlyContinue
    Invoke-Kin --dev-history open $it2.id
    Check 'S3 (off) the history item action on a link is "Copy" (the history has no Open button yet), and it does not open anything' ((Wait-Until { (Test-Path $res) -and (Get-Content $res -Raw) -match 'Copied to the clipboard' } 10) -and (Pc-Opened) -eq $o1)
    Note 'S3: the toast buttons (Open, Copy) cannot be clicked without OS input injection and the test Kin shows no toast (--dev-no-windows logs them): only the toast title and the decision (opened or offered) are verified from the log. The Open button of the "Link from phone" toast is NOT verified end to end (unit-tested only); the history window offers Copy only'
    Check 'S3 the URL and the title are not in the PC log' (-not ((Pc-Log) -match 'youtube\.com|off-page|A shared video|Off page'))
    Pc-Setting 'openLinksAutomatically' 'true' | Out-Null
}

# ================================================================ S4 fallbacks
$script:FakeProc = $null
function Start-FakePhone([string]$LogFile, [string[]]$Extra, [string]$PairUrl, [string]$KeyFile, [string]$SaveDir) {
    New-Item -ItemType Directory -Force $SaveDir | Out-Null
    $args2 = @($PairUrl, '--host', '127.0.0.1', '--seconds', '600', '--save-dir', ('"{0}"' -f $SaveDir), '--identity-file', ('"{0}"' -f $KeyFile)) + $Extra
    $script:FakeProc = Start-Process $FakePhone -ArgumentList $args2 -PassThru -WindowStyle Hidden -RedirectStandardOutput $LogFile -RedirectStandardError "$LogFile.err"
}
function Stop-FakePhone { if ($script:FakeProc) { Stop-OwnProcess $script:FakeProc ((Resolve-Path $FakePhone).Path); $script:FakeProc = $null; Start-Sleep -Milliseconds 500 } }
function Run-S4a {
    Say 'S4a: PC with the capability -> a 0.1.x phone (fake_phone --old-phone): the link arrives as a text, queued items too'
    if (-not (Test-Path $FakePhone)) { Check 'S4a fake_phone.exe exists' $false $FakePhone; return }
    $dataB = Join-Path $Work 'kinB'
    $portB = $Port + 1
    $pipeB = "\\.\pipe\app.kinrelay.dev.$Sid.${Suffix}b"
    $logB = { try { Get-Content (Join-Path $dataB 'logs\kin.log') -Raw } catch { '' } }
    $stB = { try { Get-Content (Join-Path $dataB 'status.json') -Raw | ConvertFrom-Json } catch { $null } }
    New-Item -ItemType Directory -Force $dataB | Out-Null
    Set-KinDev -Exe $KinCopy -DataDir $dataB
    $env:KIN_DEV_PIPE_SUFFIX = "${Suffix}b"
    try {
        Start-DevKin @('--data-dir', ('"{0}"' -f $dataB), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-port', "$portB", '--dev-received-dir', ('"{0}"' -f (Join-Path $dataB 'received')), '--dev-print-pair-url', '--dev-no-windows', '--dev-no-open') | Out-Null
        Check 'S4a the second test Kin (own data dir, port and pipe) started' (Wait-Until { (& $stB).state -eq 'not_paired' } 30)
        Invoke-Kin --dev-start-pairing
        Wait-Until { Test-Path (Join-Path $dataB 'pair-url.txt') } 10 | Out-Null
        $pairUrl = (Get-Content (Join-Path $dataB 'pair-url.txt') -Raw).Trim()
        $key = Join-Path $Work 'fake-phone.key'
        $save = Join-Path $Work 'fake-phone-files'
        $f1 = Join-Path $Work 'fake1.log'
        Start-FakePhone $f1 @('--old-phone') $pairUrl $key $save
        Check 'S4a the old fake phone paired and is connected' (Wait-Until { (& $stB).state -eq 'phone_connected' } 30)
        $u = 'https://example.com/for-an-old-phone?t=5#x'
        $r = Send-ViaHost 'link' @{ url = $u; title = 'dropped for an old phone' } $pipeB
        Check 'S4a the link is accepted and delivered' ($r.ok -eq $true -and $r.queued -eq $false)
        $got = Wait-Until { (Get-Content $f1 -Raw) -match 'received text .*isUrl=true' } 15
        $fl = Get-Content $f1 -Raw
        Check 'S4a the old phone got a text with isUrl=true and the same URL, no title' ($got -and $fl.Contains("text=$u") -and $fl -notmatch 'received link ')
        # queued while the old phone is away: still a text when it comes back, with the same item
        Stop-FakePhone
        Check 'S4a the PC sees the phone gone' (Wait-Until { (& $stB).state -eq 'phone_offline' } 30)
        $u2 = 'https://example.com/queued-for-the-old-phone'
        $r = Send-ViaHost 'link' @{ url = $u2; title = 'queued' } $pipeB
        Check 'S4a offline: queued (DEVICE_OFFLINE_QUEUED), not an error' ($r.ok -eq $true -and $r.queued -eq $true -and $r.code -eq 'DEVICE_OFFLINE_QUEUED') ($r | ConvertTo-Json -Compress)
        $f2 = Join-Path $Work 'fake2.log'
        Start-FakePhone $f2 @('--old-phone', '--skip-pairing') $pairUrl $key $save
        $got = Wait-Until { (Get-Content $f2 -Raw) -match 'received text .*isUrl=true' } 25
        $fl = Get-Content $f2 -Raw
        Check 'S4a the queued link reaches the old phone as a text after the reconnect' ($got -and $fl.Contains("text=$u2") -and $fl -notmatch 'received link ')
        # the same again, but the phone that comes back is a capable one: the form is chosen when the message goes on the wire
        Stop-FakePhone
        Wait-Until { (& $stB).state -eq 'phone_offline' } 30 | Out-Null
        $u3 = 'https://example.com/queued-for-the-new-phone'
        $r = Send-ViaHost 'link' @{ url = $u3; title = 'queued for the new' } $pipeB
        Check 'S4a offline again: queued' ($r.queued -eq $true)
        $f3 = Join-Path $Work 'fake3.log'
        Start-FakePhone $f3 @('--skip-pairing') $pairUrl $key $save
        $got = Wait-Until { (Get-Content $f3 -Raw) -match 'received link ' } 25
        $fl = Get-Content $f3 -Raw
        Check 'S4a the same kind of item, now to a capable phone, goes as a link with its title (form chosen at send time)' ($got -and $fl.Contains("url=$u3 title=queued for the new") )
        Check 'S4a the PC log has no URL' (-not ((& $logB) -match 'for-an-old-phone|queued-for'))
    } finally {
        Stop-FakePhone
        Stop-DevKin 10
        $env:KIN_DEV_PIPE_SUFFIX = $Suffix
        Set-KinDev -Exe $KinCopy -DataDir $Data
    }
}
function Use-Instance([string]$Dir) { $script:Data = $Dir; $script:RecvDir = Join-Path $Dir 'received' }
function Run-S4b {
    Say 'S4b: the phone (2B.1) -> a 0.1.1 PC that does not list link: the link arrives as a text; queued link too'
    $oldExe = Join-Path $OldKinDir 'target\debug\Kin.exe'
    if (-not (Test-Path $oldExe)) { Note "S4b skipped: no Kin built from v0.1.1 at $oldExe (git archive v0.1.1 desktop; cargo build --bin kin; see the integration results)"; return }
    $mainData = $Data
    $oldData = Join-Path $Work 'oldpc'
    New-Item -ItemType Directory -Force $oldData, (Join-Path $Work 'oldpc-bin') | Out-Null
    $oldCopy = Join-Path $Work 'oldpc-bin\Kin.exe'        # it has to be called Kin.exe: kin-dev.ps1 finds the test Kin's processes by that name
    Copy-Item $oldExe $oldCopy
    # the test Kin of this run goes away first; the phone is unpaired locally and pairs with the old PC
    if (@(Get-DevKinProcesses).Count -gt 0) { Stop-DevKin 10 }
    Phone-Cmd 'UNPAIR' '' 1500 | Out-Null
    Set-KinDev -Exe $oldCopy -DataDir $oldData
    Use-Instance $oldData
    $oldArgs = @('--data-dir', ('"{0}"' -f $oldData), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-port', "$($Port + 2)", '--dev-received-dir', ('"{0}"' -f (Join-Path $oldData 'received')), '--dev-print-pair-url', '--dev-no-windows')
    try {
        Start-DevKin $oldArgs | Out-Null
        Check 'S4b the Kin built from the v0.1.1 tag started (no extension pipe, no link capability)' (Wait-Until { (Get-Status).state -eq 'not_paired' } 30)
        Check 'S4b the phone paired with the old PC' (Pair-Phone)
        $u = 'https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=83s'
        Share-Url $u 'Title lost on the way'
        Check 'S4b the old PC received it (as a text)' (Wait-Until { $h = Get-PcView 'history'; @($h.items | Where-Object { $_.direction -eq 'in' -and $_.text -ceq $u }).Count -ge 1 } 30)
        $h = Get-PcView 'history'
        $it = @($h.items | Where-Object { $_.direction -eq 'in' -and $_.text -ceq $u })[0]
        Check 'S4b it is a normal text on the old PC (isUrl true, no title; it shows its usual text toast with Open)' ($it -and $it.isUrl -eq $true -and -not $it.title) ($it | ConvertTo-Json -Compress)
        $row = @(Phone-DbTexts | Where-Object { $_.Dir -eq 'up' -and $_.Body -ceq $u })[0]
        Check 'S4b the phone keeps it as a link in its own history, done (the old PC acked the text)' ($row -and $row.Kind -eq 'link' -and $row.Status -eq 'done') "$($row.Kind) $($row.Status)"
        # queued while the old PC is away
        Stop-DevKin 10
        Check 'S4b the old PC has quit' (@(Get-DevKinProcesses).Count -eq 0)
        Check 'S4b the phone notices (its link state is no longer Connected)' (Wait-Until { (Phone-State)['link'] -notlike 'Connected*' } 40) ((Phone-State)['link'])
        $u2 = 'https://example.com/queued-for-the-old-pc?x=1#y'
        Share-Url $u2 'queued link'
        $seen = ''
        Check 'S4b PC away: the share is queued on the phone as a link (kind link, not delivered while the PC is away)' (Wait-Until { $x = @(Phone-DbTexts | Where-Object { $_.Body -ceq $u2 })[0]; $script:seen = "$($x.Kind)/$($x.Status)"; $x -and $x.Kind -eq 'link' -and $x.Status -ne 'done' } 25) $script:seen
        Start-DevKin ($oldArgs) | Out-Null
        Wait-Until { (Get-Status).state -in 'phone_offline', 'phone_connected' } 30 | Out-Null
        Phone-Cmd 'CONNECT' '' 500 | Out-Null
        Check 'S4b the queued link reaches the old PC as a text after the reconnect' (Wait-Until { $h = Get-PcView 'history'; @($h.items | Where-Object { $_.direction -eq 'in' -and $_.text -ceq $u2 -and -not $_.title }).Count -ge 1 } 60)
        Check 'S4b and the phone shows it done' (Wait-Until { $x = @(Phone-DbTexts | Where-Object { $_.Body -ceq $u2 })[0]; $x -and $x.Status -eq 'done' } 20)
    } finally {
        Stop-DevKin 10
        Phone-Cmd 'UNPAIR' '' 1000 | Out-Null
        Set-KinDev -Exe $KinCopy -DataDir $mainData
        Use-Instance $mainData
    }
}

# ================================================================ S6 URL edge cases on both sides
function U([int[]]$cps) { -join ($cps | ForEach-Object { [char]$_ }) }
# What the PC's toast shows for each URL, and whether the address has a warning, read from the real code (`links::link_address`, the 2B.1 review's rule: the browser's
# reading of the host, marked): desktop\src-tauri\examples\link_address.rs, built and run with cargo. (The old `display_host` of phonelinks.rs is gone.) Returns a
# hashtable keyed by the hex of the URL's UTF-8 bytes: @{ Shown = '...'; Warn = $true/$false }. Toasts are only logged by a test Kin, so this is how "what the PC shows" is read.
function Get-PcLinkAddresses([string[]]$Urls) {
    $dir = Join-Path $Work 'linkaddress'; New-Item -ItemType Directory -Force $dir | Out-Null
    $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $cargoBin })) { $env:Path = "$cargoBin;$env:Path" }
    $hexOf = { param([string]$t) -join ([Text.Encoding]::UTF8.GetBytes($t) | ForEach-Object { $_.ToString('x2') }) }
    $lines = @($Urls | ForEach-Object { & $hexOf $_ })
    [IO.File]::WriteAllLines((Join-Path $dir 'in.txt'), $lines)
    Push-Location (Join-Path $Repo 'desktop\src-tauri')
    try { $out = & cargo run --quiet --example link_address -- (Join-Path $dir 'in.txt') 2>$null } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0 -or -not $out) { return $null }
    $h = @{}
    foreach ($l in $out) {
        $p = "$l".Split('|')
        if ($p.Count -ne 3 -or $p[2] -eq 'error') { continue }
        $shown = if ($p[1]) { [Text.Encoding]::UTF8.GetString([byte[]]($p[1] -split '(?<=\G..)(?!$)' | ForEach-Object { [Convert]::ToByte($_, 16) })) } else { '' }
        $h[$p[0]] = @{ Shown = $shown; Warn = ($p[2] -eq 'true') }
    }
    $h
}
# What a browser (WHATWG URL, bun's) navigates to for each URL.
function Get-BrowserReading([string[]]$Urls) {
    $dir = Join-Path $Work 'browser'; New-Item -ItemType Directory -Force $dir | Out-Null
    [IO.File]::WriteAllText((Join-Path $dir 'in.json'), (ConvertTo-Json @($Urls) -Compress), (New-Object Text.UTF8Encoding($false)))
    [IO.File]::WriteAllText((Join-Path $dir 'read.ts'), @'
const urls: string[] = JSON.parse(await Bun.file(process.argv[2]).text());
const out = urls.map((u) => { try { const x = new URL(u); return { host: x.hostname, port: x.port, user: x.username, href: x.href }; } catch (e) { return { host: null, error: String(e) }; } });
await Bun.write(process.argv[3], JSON.stringify(out));
'@, (New-Object Text.UTF8Encoding($false)))
    & bun (Join-Path $dir 'read.ts') (Join-Path $dir 'in.json') (Join-Path $dir 'out.json') | Out-Null
    $r = Get-Content (Join-Path $dir 'out.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    @($r)
}
$script:SpecWhite = @(9, 10, 11, 12, 13, 32, 0x85, 0xA0, 0x1680, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000) + (0x2000..0x200A)   # protocol.md: the Unicode White_Space set, listed
# The PC's history view shows a PREVIEW of a text (whitespace and control characters become one space, long texts are cut): compared like that.
function Norm-Text([string]$x) {
    $sb = New-Object Text.StringBuilder
    foreach ($ch in $x.ToCharArray()) { $c = [int]$ch; if ($c -le 0x20 -or ($c -ge 0x7f -and $c -le 0xa0) -or $script:SpecWhite -contains $c) { [void]$sb.Append(' ') } else { [void]$sb.Append($ch) } }
    $y = ([regex]::Replace($sb.ToString(), ' +', ' ')).Trim()
    if ($y.Length -gt 150) { $y.Substring(0, 150) } else { $y }
}
function Trim-Spec([string]$s) {
    $ws = $script:SpecWhite
    $a = 0; $b = $s.Length
    while ($a -lt $b -and $ws -contains [int]$s[$a]) { $a++ }
    while ($b -gt $a -and $ws -contains [int]$s[$b - 1]) { $b-- }
    $s.Substring($a, $b - $a)
}
function Run-S6 {
    Say 'S6: URL edge cases on both sides (protocol/links.json + backslash, userinfo, IDN and homograph hosts)'
    $j = Get-Content (Join-Path $Repo 'protocol\links.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $entries = New-Object System.Collections.Generic.List[object]
    foreach ($u in $j.urls) {
        $in = if ($u.synthesize) { $s = $u.synthesize; $s.prefix + ([string]$s.fill * [int]$s.fillCount) + $s.suffix } else { [string]$u.input }
        $entries.Add([pscustomobject]@{ Name = $u.name; Input = $in; Valid = [bool]$u.valid; Unchanged = [bool]$u.unchanged; Extra = $false })
    }
    # the display cases: since changelog 55 only the IDN, homograph, case and IP ones pass the openable-URL rule; the backslash, userinfo and %-escape ones are texts on both sides (the table says so)
    $cy = U 0x430, 0x440, 0x440, 0x4CF, 0x435      # Cyrillic a, r, r, palochka, e: "apple" to the eye
    $extras = @(
        @('x-backslash-authority', 'https://evil.example\@google.com'),
        @('x-backslash-authority-slash', 'https://evil.example\@google.com/'),
        @('x-userinfo-lookalike', 'https://google.com@evil.example/'),
        @('x-userinfo-port-lookalike', 'https://google.com:443@evil.example/'),
        @('x-escape-in-host', 'https://goog%6Ce.com/'),
        @('x-idn-umlaut', ('https://b' + (U 0xFC) + 'cher.example/')),
        @('x-punycode', 'https://xn--bcher-kva.example/'),
        @('x-homograph-cyrillic', ("https://$cy.com/")),
        @('x-homograph-mixed-script', ('https://paypa' + (U 0x4CF) + '.com/')),
        @('x-fullwidth-host', ('https://' + (U 0xFF47, 0xFF4F, 0xFF4F, 0xFF47, 0xFF4C, 0xFF45) + '.com/')),
        @('x-upper-case-host', 'https://EXAMPLE.COM/Path'),
        @('x-ip-octal', 'https://0x7f.1/'),
        @('x-ipv6-mapped', 'https://[::ffff:127.0.0.1]/'),
        @('x-colon-userinfo-in-host', 'https://example.com:8443@evil.example/')
    )
    foreach ($e in $extras) { $entries.Add([pscustomobject]@{ Name = $e[0]; Input = $e[1]; Valid = $true; Unchanged = $true; Extra = $true }) }
    $ordinary = @($entries | Where-Object { -not $_.Extra })
    $displayCases = @($entries | Where-Object { $_.Extra })
    Check "S6 the vectors file has the expected size ($($ordinary.Count) URL cases)" ($ordinary.Count -ge 40) "$($ordinary.Count)"

    # --- PC -> phone: the host/Kin rule on each, then what the phone stored (nothing from a rejected one)
    Clear-Notifs
    $pcRes = @{}
    foreach ($e in $ordinary) {
        $r = Send-ViaHost 'link' @{ url = $e.Input; title = $e.Name }
        $pcRes[$e.Name] = $r
        Start-Sleep -Milliseconds 450      # the pipe takes at most 30 sends per 10 seconds
    }
    Start-Sleep -Seconds 5
    $rows = @(Phone-DbTexts | Where-Object { $_.Dir -eq 'down' -and $_.Kind -eq 'link' })
    $mismatchRule = @(); $mismatchExact = @()
    foreach ($e in $ordinary) {
        $r = $pcRes[$e.Name]
        if (([bool]$r.ok) -ne $e.Valid) { $mismatchRule += "$($e.Name) (host/Kin: ok=$($r.ok) $($r.code), rule: valid=$($e.Valid))" }
        $row = @($rows | Where-Object { $_.Title -eq $e.Name })[0]
        if ($e.Valid) { if (-not $row -or $row.Body -cne $e.Input) { $mismatchExact += $e.Name } }
        elseif ($row) { $mismatchExact += "$($e.Name) (a rejected URL reached the phone)" }
        $e | Add-Member -NotePropertyName PcToPhoneOk -NotePropertyValue ([bool]$r.ok) -Force
        $e | Add-Member -NotePropertyName PcToPhoneCode -NotePropertyValue "$($r.code)" -Force
        $e | Add-Member -NotePropertyName PhoneStored -NotePropertyValue ([bool]($row -and $row.Body -ceq $e.Input)) -Force
    }
    Check "S6 PC -> phone: the host and Kin accept exactly the URLs of the rule ($(@($ordinary | Where-Object Valid).Count) valid, $(@($ordinary | Where-Object { -not $_.Valid }).Count) invalid)" ($mismatchRule.Count -eq 0) ($mismatchRule -join '; ')
    Check 'S6 PC -> phone: every accepted URL is on the phone byte for byte (YouTube t=, escapes, userinfo, IDN, 8,192 bytes); no rejected one reached it' ($mismatchExact.Count -eq 0) ($mismatchExact -join '; ')

    # --- the display cases, one at a time (the phone shows at most 10 link notifications): what the phone's notification says
    $browser = Get-BrowserReading @($displayCases | ForEach-Object { $_.Input })
    $pcShown = Get-PcLinkAddresses @($displayCases | ForEach-Object { $_.Input })
    Check 'S6 the PC toast address function (links::link_address) was built from the source and run' ($null -ne $pcShown)
    $i = 0
    foreach ($e in $displayCases) {
        Clear-Notifs
        $r = Send-ViaHost 'link' @{ url = $e.Input; title = $e.Name }
        $e | Add-Member -NotePropertyName PcToPhoneOk -NotePropertyValue ([bool]$r.ok) -Force
        $n = if ($r.ok) { Wait-Notif $e.Name 12 } else { $null }
        $e | Add-Member -NotePropertyName PhoneAddress -NotePropertyValue $(if ($n) { Notif-Field $n 'text' } else { '(none)' }) -Force
        $e | Add-Member -NotePropertyName PhoneSub -NotePropertyValue $(if ($n) { Notif-Field $n 'subText' } else { '' }) -Force
        $e | Add-Member -NotePropertyName PhoneBig -NotePropertyValue $(if ($n -and $n -match '(?s)android\.bigText=String \((.*?)\)\r?\n\s+android\.infoText') { ($Matches[1] -replace "`r?`n", ' / ') } else { '' }) -Force
        $hexIn = -join ([Text.Encoding]::UTF8.GetBytes($e.Input) | ForEach-Object { $_.ToString('x2') })
        $e | Add-Member -NotePropertyName Browser -NotePropertyValue $browser[$i].host -Force
        $e | Add-Member -NotePropertyName PcShows -NotePropertyValue $(if ($pcShown -and $pcShown[$hexIn]) { $pcShown[$hexIn].Shown } else { '?' }) -Force
        $i++
    }

    # --- phone -> PC: the share sheet with each URL (the PC opens automatically, mocked); a text that isn't exactly one web link stays a text
    Pc-Setting 'openLinksAutomatically' 'true' | Out-Null
    $o0 = Pc-Opened
    $all = $entries.ToArray()
    # What the PC must NOT open by itself (the 2B.1 review's rule): a link whose address has a warning is offered in a toast, never opened. The warning comes from the real code.
    $expectTrimmed = @($all | ForEach-Object { Trim-Spec $_.Input } | Where-Object { Test-OpenableSpec $_ } | Select-Object -Unique)
    $addresses = Get-PcLinkAddresses $expectTrimmed
    Check 'S6 the PC address function answered for every link of the table' ($null -ne $addresses -and $addresses.Count -ge ($expectTrimmed.Count - 1)) "$($addresses.Count) of $($expectTrimmed.Count)"
    foreach ($e in $all) { Share-Url $e.Input $e.Name; Start-Sleep -Milliseconds 1100 }
    Start-Sleep -Seconds 8
    $h = Get-PcView 'history'
    $histIn = @($h.items | Where-Object { $_.direction -eq 'in' })
    $expectLinks = 0; $expectWarned = 0; $bad = @()
    foreach ($e in $all) {
        $t = Trim-Spec $e.Input
        $expectLink = ((Test-OpenableSpec $t))
        $link = @($histIn | Where-Object { $_.isLink -and $_.title -eq $e.Name })[0]
        $text = @($histIn | Where-Object { -not $_.isLink -and $_.text -and (Norm-Text $_.text) -ceq (Norm-Text $e.Input) })[0]
        $where = if ($link) { 'link' } elseif ($text) { 'text' } else { 'none' }
        $e | Add-Member -NotePropertyName PhoneToPc -NotePropertyValue $where -Force
        $e | Add-Member -NotePropertyName PhoneToPcExact -NotePropertyValue ([bool]($link -and $link.text -ceq $t)) -Force
        # (the history view of the PC shows at most a preview of a long text, so the two 8,192-byte URLs are checked by the length the PC logged when it opened them)
        $isLong = [Text.Encoding]::UTF8.GetByteCount($t) -gt 1000
        if ($expectLink) {
            $expectLinks++
            $hexT = -join ([Text.Encoding]::UTF8.GetBytes($t) | ForEach-Object { $_.ToString('x2') })
            $warned = [bool]($addresses -and $addresses[$hexT] -and $addresses[$hexT].Warn)
            if ($warned) { $expectWarned++ }
            $e | Add-Member -NotePropertyName PcWarned -NotePropertyValue $warned -Force
            if ($where -ne 'link' -or (-not $isLong -and $link.text -cne $t)) { $bad += "$($e.Name): expected a link '$($t.Substring(0, [Math]::Min(40, $t.Length)))', got $where" }
        }
        else { if ($where -eq 'link') { $bad += "$($e.Name): became a link on the PC but is not a web link" } }
    }
    $upRows = @(Phone-DbTexts | Where-Object { $_.Dir -eq 'up' })
    $notExact = @($all | Where-Object { $_.PhoneToPc -ne 'link' -and $_.Input -ne '' } | Where-Object { $e2 = $_; -not ($upRows | Where-Object { $_.Kind -eq 'text' -and $_.Body -ceq $e2.Input }) } | ForEach-Object { $_.Name })
    Check 'S6 phone -> PC: a share that is not exactly one web link was sent as a text with the exact bytes (NBSP, NEL, tab, control characters, 8,193 bytes: checked in the phone database)' ($notExact.Count -eq 0) ($notExact -join ', ')
    $opened = (Pc-Opened) - $o0
    Check "S6 phone -> PC: exactly the URLs that are web links after trimming ($expectLinks) became links on the PC, byte for byte; everything else stayed a text or was refused" ($bad.Count -eq 0) ($bad -join '; ')
    Check 'S6 phone -> PC: the two 8,192-byte links were opened with all 8,192 characters' ((Pc-LogCount 'link \S+ from device \S+: 8192 characters, opened in the default browser') -ge 2)
    $expectOpened = $expectLinks - $expectWarned
    Check "S6 phone -> PC: the PC asked its opener once per link WITHOUT a warning and for nothing else ($opened of $expectOpened; the $expectWarned links with a warning were offered in a toast and not opened)" ($opened -eq $expectOpened)
    Check 'S6 phone -> PC: a link with a warning was logged as not opened (the address has a warning)' ($expectWarned -eq 0 -or (Pc-LogCount 'not opened \(the address has a warning') -ge $expectWarned)
    $script:Edge = $all
    # --- the table
    $md = New-Object System.Collections.Generic.List[string]
    $md.Add('| case | valid (rule) | PC->phone | phone stored exact | phone->PC | PC opened |'); $md.Add('|---|---|---|---|---|---|')
    foreach ($e in $ordinary) { $md.Add(("| {0} | {1} | {2} | {3} | {4} | {5} |" -f $e.Name, $e.Valid, $(if ($e.PcToPhoneOk) { 'accepted' } else { 'rejected ' + $e.PcToPhoneCode }), $e.PhoneStored, $e.PhoneToPc, $(if ($e.PhoneToPc -eq 'link') { 'yes' } else { 'no' }))) }
    $md.Add(''); $md.Add('| display case | URL | a browser goes to | the PC toast shows | the phone shows (line under the title) | phone label |'); $md.Add('|---|---|---|---|---|---|')
    foreach ($e in $displayCases) { $md.Add(("| {0} | ``{1}`` | {2} | {3} | {4} | {5} |" -f $e.Name, $e.Input, $e.Browser, $e.PcShows, $e.PhoneAddress, $e.PhoneSub)) }
    $md | Set-Content (Join-Path $Dumps 'edge-table.md') -Encoding UTF8
    # kept outside the work folder (which a passing run deletes): the table and what the PC history held for each share, for the results document
    $keepDir = Join-Path $env:TEMP 'kin-e2e-2b-1-edge'; New-Item -ItemType Directory -Force $keepDir | Out-Null
    $md | Set-Content (Join-Path $keepDir 'edge-table.md') -Encoding UTF8
    ($all | ForEach-Object { [pscustomobject]@{ name = $_.Name; inputHex = (-join ([Text.Encoding]::UTF8.GetBytes($_.Input) | Select-Object -First 80 | ForEach-Object { $_.ToString('x2') })); valid = $_.Valid; phoneToPc = $_.PhoneToPc } } | ConvertTo-Json) | Set-Content (Join-Path $keepDir 'edge-details.json') -Encoding UTF8
    @(Phone-DbTexts | Where-Object { $_.Dir -eq 'up' } | ForEach-Object { [pscustomobject]@{ kind = $_.Kind; status = $_.Status; source = $_.Source; title = $_.Title; bodyHex = (-join ([Text.Encoding]::UTF8.GetBytes([string]$_.Body) | Select-Object -First 80 | ForEach-Object { $_.ToString('x2') })) } }) | ConvertTo-Json | Set-Content (Join-Path $keepDir 'phone-up-rows.json') -Encoding UTF8
    @($histIn | ForEach-Object { [pscustomobject]@{ title = $_.title; isLink = $_.isLink; textHex = (-join ([Text.Encoding]::UTF8.GetBytes([string]$_.text) | Select-Object -First 80 | ForEach-Object { $_.ToString('x2') })) } }) | ConvertTo-Json | Set-Content (Join-Path $keepDir 'pc-history-in.json') -Encoding UTF8
    Say 'edge-case table (also in dumps\edge-table.md):'; $md | ForEach-Object { Say $_ }
    # the interop question, answered as data
    foreach ($e in $displayCases) {
        $same = ($e.PcShows -eq $e.PhoneAddress)
        $agreesBrowser = ($e.PcShows -eq $e.Browser)
        if (-not $agreesBrowser) { Note ("S6 {0}: a browser goes to '{1}', the PC toast shows '{2}', the phone shows '{3}'" -f $e.Name, $e.Browser, $e.PcShows, $e.PhoneAddress) }
    }
    Clear-Notifs
}
# the openable-URL rule of protocol.md applied to an already trimmed text (written from the spec, not from either app)
function Test-OpenableSpec([string]$t) {
    if ([Text.Encoding]::UTF8.GetByteCount($t) -gt 8192) { return $false }
    if ($t -notmatch '^(?i:https?)://') { return $false }
    foreach ($ch in $t.ToCharArray()) { $c = [int]$ch; if ($c -le 0x1F -or ($c -ge 0x7F -and $c -le 0x9F)) { return $false } }
    foreach ($ch in $t.ToCharArray()) { if ($script:SpecWhite -contains [int]$ch) { return $false } }
    $rest = $t.Substring($t.IndexOf('://') + 3)
    $auth = ($rest -split '[/?#]', 2)[0]
    if ($auth -match '[@\\%]') { return $false }   # changelog 55, step 4: no user information, no backslash and no percent-encoding in the authority: not openable, as a text or as a link
    if ($auth.StartsWith('[')) {
        $close = $auth.IndexOf(']'); if ($close -lt 2) { return $false }
        if ($auth.Substring(1, $close - 1) -match '[\[\]]') { return $false }
        $after = $auth.Substring($close + 1)
        if ($after -eq '') { return $true }
        if (-not $after.StartsWith(':')) { return $false }
        $port = $after.Substring(1); $hostOk = $true
    } else {
        $colon = $auth.IndexOf(':')
        if ($colon -ge 0) { $hostPart = $auth.Substring(0, $colon); $port = $auth.Substring($colon + 1) } else { $hostPart = $auth; $port = $null }
        if ($hostPart -eq '' -or $hostPart -match '^\.+$') { return $false }
        if ($hostPart -match '[\[\]<>^|]') { return $false }   # step 5: a host that is not a bracketed IPv6 literal has none of [ ] < > ^ |
    }
    if ($null -ne $port) {
        if ($port -notmatch '^[0-9]+$') { return $false }
        $pv = [decimal]::Parse($port); if ($pv -lt 1 -or $pv -gt 65535) { return $false }
    }
    $true
}

# ================================================================ S7 v0.1.1 transfers still work
function Make-PcFile([string]$Path, [long]$Bytes, [int]$Seed = 7) {
    $rnd = New-Object Random $Seed
    $buf = New-Object byte[] (64KB)
    $fs = [IO.File]::Create((Long $Path))
    try { $left = $Bytes; while ($left -gt 0) { $rnd.NextBytes($buf); $n = [int][Math]::Min($left, $buf.Length); $fs.Write($buf, 0, $n); $left -= $n } } finally { $fs.Dispose() }
}
function Run-S7 {
    Say 'S7: transfers of v0.1.1 still work (one file each way, 10-file batches, byte for byte)'
    $w = Join-Path $Work 'files'; New-Item -ItemType Directory -Force $w | Out-Null
    # PC -> phone, one small file
    $small = Join-Path $w 'e2e-2b1-small.bin'
    Make-PcFile $small 120000 3
    $sha = Pc-Sha $small
    Invoke-Kin --send $small
    $got = Wait-Until { @(Phone-Received | Where-Object { $_.Name -eq 'e2e-2b1-small.bin' -and $_.Pending -eq 0 }).Count -ge 1 } 60
    $rec = @(Phone-Received | Where-Object { $_.Name -eq 'e2e-2b1-small.bin' })[0]
    Check 'S7 PC -> phone: the small file arrived' ($got -and $rec) ($rec | ConvertTo-Json -Compress)
    Check 'S7 PC -> phone: byte for byte (SHA-256)' ($rec -and (Phone-RecvSha $rec) -eq $sha)
    # PC -> phone, 10 files
    $names = 1..10 | ForEach-Object { 'e2e-2b1-b{0:D2}.bin' -f $_ }
    $paths = @(); $shas = @{}
    $i = 0; foreach ($n in $names) { $p = Join-Path $w $n; Make-PcFile $p (40000 + 9000 * $i) (20 + $i); $paths += $p; $shas[$n] = Pc-Sha $p; $i++ }
    Invoke-Kin --send @paths
    $ok = Wait-Until { $r = @(Phone-Received | Where-Object { $_.Name -like 'e2e-2b1-b*' -and $_.Pending -eq 0 }); $r.Count -ge 10 } 120
    $recs = @(Phone-Received | Where-Object { $_.Name -like 'e2e-2b1-b*' })
    Check 'S7 PC -> phone: all 10 files of the batch arrived' ($ok -and $recs.Count -eq 10) "$($recs.Count)"
    $bad = @($recs | Where-Object { (Phone-RecvSha $_) -ne $shas[$_.Name] })
    Check 'S7 PC -> phone: the 10 files are byte for byte (SHA-256)' ($recs.Count -eq 10 -and $bad.Count -eq 0) (($bad | ForEach-Object Name) -join ',')
    # phone -> PC, one small file and 10 files
    Make-PhoneFiles 'e2e-2b1-up.bin:150000'
    $src = Phone-SourceHashes
    $before = Pc-Ids
    Share-Files 'e2e-2b1-up.bin'
    $t = Wait-NewPcTransfer $before 'in' 90
    $f = Join-Path $RecvDir 'e2e-2b1-up.bin'
    Check 'S7 phone -> PC: the small file arrived and the transfer is done' ($t -and $t.status -eq 'done' -and (Test-Path -LiteralPath $f)) "$($t.status) $($t.error)"
    Check 'S7 phone -> PC: byte for byte (SHA-256)' ((Test-Path -LiteralPath $f) -and (Pc-Sha $f) -eq $src['e2e-2b1-up.bin'])
    $unames = 1..10 | ForEach-Object { 'e2e-2b1-u{0:D2}.jpg' -f $_ }
    Make-PhoneFiles (($unames | ForEach-Object { "$_@640x480" }) -join ',')
    $src = Phone-SourceHashes
    $before = Pc-Ids
    Share-Files ($unames -join ',')
    $t = Wait-NewPcTransfer $before 'in' 120
    Check 'S7 phone -> PC: the 10-file transfer is done' ($t -and $t.status -eq 'done' -and $t.files -eq 10) "$($t.status) files=$($t.files) $($t.error)"
    $bad = @($unames | Where-Object { $p = Join-Path $RecvDir $_; -not ((Test-Path -LiteralPath $p) -and (Pc-Sha $p) -eq $src[$_]) })
    Check 'S7 phone -> PC: the 10 files are byte for byte (SHA-256)' ($bad.Count -eq 0) ($bad -join ',')
    Check 'S7 no .part file left on the PC' (@(Get-ChildItem -LiteralPath $RecvDir -Filter '*.part' -ErrorAction SilentlyContinue).Count -eq 0)
    Remove-Item -Recurse -Force $w -ErrorAction SilentlyContinue
    Phone-Cmd 'DELETE_RECEIVED' '' 1500 | Out-Null
    Run-As 'rm -rf files/debug-share' | Out-Null
}

# ================================================================ S5 the extension, end to end (headless Chromium; the work is in e2e-2b-1-extension.ts)
function Run-S5 {
    Say 'S5: the browser extension -> real host -> real Kin -> the emulator (headless Chromium, throwaway profile, HKCU dev host key only)'
    $keyPath = 'HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev'
    & reg query $keyPath /ve 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) { Check 'S5 the dev host registry key does not exist yet (refusing to touch one that does)' $false $keyPath; return }
    $ext = Join-Path $Repo 'extension\dist\chromium-test\manifest.json'
    if (-not (Test-Path $ext)) { Check 'S5 the extension test build exists (bun scripts/build.ts --test in extension\; -Build does it)' $false; return }
    $bun = (Get-Command bun -ErrorAction Stop).Source
    $log = Join-Path $Work 's5-extension.log'
    $env:E2E_DATA = $Data; $env:E2E_SUFFIX = $Suffix; $env:E2E_KIN = $KinCopy; $env:E2E_HOST_EXE = (Resolve-Path $HostExe).Path
    $env:E2E_ADB = $Adb; $env:E2E_SERIAL = $Serial; $env:E2E_PKG = $Pkg; $env:E2E_OUT = (Join-Path $Work 's5-results.json'); $env:E2E_WORK = $Work
    $env:E2E_SHOTS = $ScreenshotDir
    $p = Start-Process $bun -ArgumentList @((Join-Path $PSScriptRoot 'e2e-2b-1-extension.ts')) -PassThru -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError "$log.err" -WorkingDirectory $Repo
    if (-not $p.WaitForExit(900000)) { Stop-OwnProcess $p $bun; Check 'S5 the extension run finished in 15 minutes' $false }
    foreach ($k in 'E2E_DATA', 'E2E_SUFFIX', 'E2E_KIN', 'E2E_HOST_EXE', 'E2E_ADB', 'E2E_SERIAL', 'E2E_PKG', 'E2E_OUT', 'E2E_WORK', 'E2E_SHOTS') { Remove-Item "Env:\$k" -ErrorAction SilentlyContinue }
    $lines = @(Get-Content $log -ErrorAction SilentlyContinue)
    $lines | Where-Object { $_ -match '^\s+(ok|FAIL)\s' -or $_ -match 'steps passed|WARNING' } | ForEach-Object { Say "  [ext] $($_.Trim())" }
    foreach ($l in $lines) {
        if ($l -match '^\s+ok\s+(.*)$') { Check "S5 $($Matches[1])" $true }
        elseif ($l -match '^\s+FAIL\s+(.*)$') { Check "S5 $($Matches[1])" $false }
    }
    if (-not ($lines -match 'steps passed')) { Check 'S5 the extension run printed its summary' $false (($lines | Select-Object -Last 5) -join ' | ') }
    & reg query $keyPath /ve 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) {
        Check 'S5 the dev host registry key is gone after the run' $false
        & reg delete $keyPath /f | Out-Null     # only the dev host name, only after our own run
    } else { Check 'S5 the dev host registry key is gone after the run' $true }
}

# ================================================================ -Build
function Build-Everything {
    Say 'building Kin.exe (release, tauri build) and the installer'
    Push-Location (Join-Path $Repo 'desktop'); . .\_env.ps1
    powershell -NoProfile -ExecutionPolicy Bypass -File build.ps1 | Out-Null; $ok = $LASTEXITCODE -eq 0; Pop-Location
    Check 'build desktop (Kin.exe)' $ok
    Say 'building the debug dev host, fake_phone and Kin (debug)'
    Push-Location (Join-Path $Repo 'desktop
ative-host'); cargo build --features dev --bin kin-native-host --target-dir target/dev | Out-Null; $ok = $LASTEXITCODE -eq 0; Pop-Location
    Check 'build the dev native host' $ok
    Push-Location (Join-Path $Repo 'desktop\src-tauri'); cargo build --example fake_phone | Out-Null; $ok = $LASTEXITCODE -eq 0; Pop-Location
    Check 'build fake_phone' $ok
    Say 'building the debug APKs'
    $androidDir = Join-Path $Repo 'android'
    & (Join-Path $androidDir 'gradlew.bat') -p $androidDir assembleDebug --console=plain | Out-Null; $ok = $LASTEXITCODE -eq 0
    Check 'build the debug APK' $ok
    & (Join-Path $androidDir 'gradlew.bat') -p (Join-Path $androidDir 'link-target') assembleDebug --console=plain | Out-Null; $ok = $LASTEXITCODE -eq 0
    Check 'build the link-target test app' $ok
    & (Join-Path $androidDir 'gradlew.bat') -p $androidDir --stop | Out-Null
    Say 'building the extension test build'
    Push-Location (Join-Path $Repo 'extension'); bun scripts/build.ts --test | Out-Null; $ok = $LASTEXITCODE -eq 0; Pop-Location
    Check 'build the extension (test flavor)' $ok
    if (-not (Test-Path (Join-Path $OldKinDir 'target\debug\Kin.exe'))) {
        Say 'building a Kin from the v0.1.1 tag (the "old PC" of S4b)'
        New-Item -ItemType Directory -Force $OldKinDir | Out-Null
        Push-Location $Repo; git archive v0.1.1 desktop -o (Join-Path $OldKinDir 'a.tar'); Pop-Location
        tar -x -f (Join-Path $OldKinDir 'a.tar') -C $OldKinDir; Remove-Item (Join-Path $OldKinDir 'a.tar')
        Copy-Item (Join-Path $Repo 'desktop\dist') (Join-Path $OldKinDir 'desktop\dist') -Recurse -Force   # only so that the build finds a frontend; the old Kin runs with --dev-no-windows
        Push-Location (Join-Path $OldKinDir 'desktop\src-tauri'); $env:CARGO_TARGET_DIR = Join-Path $OldKinDir 'target'; cargo build --bin kin | Out-Null; $ok = $LASTEXITCODE -eq 0; Remove-Item Env:\CARGO_TARGET_DIR; Pop-Location
        Check 'build Kin from the v0.1.1 tag' $ok
    }
}

# ================================================================ main
Add-Type -AssemblyName System.Windows.Forms
$clip = $null
$phoneName = ''
try {
    if ($Build) { Build-Everything }
    foreach ($f in @($Kin, $Apk, $LinkTargetApk, $HostExe)) { if (-not (Test-Path $f)) { throw "missing: $f (run with -Build)" } }
    $Kin = (Resolve-Path $Kin).Path
    $installer = Get-ChildItem (Join-Path (Split-Path $Kin) 'bundle\nsis') -Filter 'Kin_*_x64-setup.exe' -ErrorAction SilentlyContinue | Sort-Object LastWriteTime | Select-Object -Last 1
    if (-not $installer -or (Get-Item $Kin).LastWriteTime -gt $installer.LastWriteTime.AddSeconds(30)) { throw 'Kin.exe is not from `tauri build` (desktop\build.ps1): run with -Build' }
    Assert-NoGameRunning
    New-Item -ItemType Directory -Force $Work, $Data, $Dumps | Out-Null
    $KinCopy = Join-Path $Work 'Kin.exe'
    Copy-Item $Kin $KinCopy        # a copy: a desktop rebuild in the meantime does not matter
    Set-KinDev -Exe $KinCopy -DataDir $Data
    Assert-NoDevKinOnDataDir
    # no automatic clipboard sync in the test Kin: it would put this PC's text on the phone by itself and hide what the scenarios check
    Set-Content -Path (Join-Path $Data 'settings.json') -Value '{"autoClipboardSync":false}' -Encoding ASCII
    $clip = Save-Clipboard | Select-Object -Last 1     # every format; put back in the finally (a file that arrives on the PC goes on the clipboard)

    Start-Emulator
    Install-Apps
    Start-TestKin
    Check 'the test Kin started' (Wait-Until { (Get-Status).state -eq 'not_paired' } 30)
    Check 'the extension pipe is listening (dev pipe, with the suffix)' (Wait-Until { (Pc-Log) -match ('extension pipe: listening as app\.kinrelay\.dev\.\S+\.' + $Suffix) } 15)

    # ---------------------------------------------------------------- S1 pair; capabilities
    $paired = Pair-Phone
    Check 'S1 paired and connected' $paired
    if (-not $paired) { throw 'not paired' }
    A shell input keyevent KEYCODE_HOME | Out-Null
    $pc = Get-Status
    $phoneName = $pc.devices[0].name
    $ph = Phone-State
    Check 'S1 phone state is Connected' ($ph['link'] -like 'Connected*') $ph['link']
    Check "S1 both sides know each other's device ID" ($ph['pcId'] -eq $pc.deviceId -and $pc.devices[0].deviceId -eq $ph['phoneId'])
    # The hellos are not logged by either side; what each side does with the other's `capabilities` shows them: a link goes out as a `link` only to a peer whose hello lists it.
    $r = Send-ViaHost 'link' @{ url = 'https://example.com/s1-pc-to-phone'; title = 'S1 pc to phone' }
    Check 'S1 PC -> phone: the host and Kin accept a link, delivered, not queued' ($r.ok -eq $true -and $r.queued -eq $false) ($r | ConvertTo-Json -Compress)
    Check "S1 the PC saw the phone's capability: the phone stored it as a LINK, not as a text" (Wait-Until { @(Link-Rows | Where-Object { $_ -match ' down ' }).Count -ge 1 } 15)
    Share-Url 'https://example.com/s1-phone-to-pc' 'S1 phone to pc'
    Check "S1 the phone saw the PC's capability: it arrived on the PC as a link" (Wait-Until { (Pc-Log) -match 'link \S+ from device \S+: \d+ characters, ' } 20)

    if ($script:Sdk -lt 30) {
        # Android 10: only the link-open path (the resolve check), unless more is asked for
        if (Want 'S2api29') { Run-S2api29 }
    } else {
        if (Want 'S2') { Run-S2 }
    }
    if ($Only.Count -gt 0 -or $script:Sdk -ge 30) {
        if (Want 'S3') { Run-S3 }
        if (Want 'S4a') { Run-S4a }
        if (Want 'S6') { Run-S6 }
        if (Want 'S7') { Run-S7 }
        if (Want 'S5') { Run-S5 }          # quits the test Kin at its end
        if (Want 'S4b') { Run-S4b }        # last: it unpairs the phone from the test Kin and pairs it with the 0.1.1 build
    }
}
catch {
    Say "ERROR: $_ (line $($_.InvocationInfo.ScriptLineNumber): $($_.InvocationInfo.Line.Trim()))"
    $script:Failures.Add("exception: $_")
}
finally {
    if ($started.Emulator -and ((A get-state | Out-String) -match 'device')) { Clear-Notifs }
    if (-not $KeepRunning) {
        Say 'tearing down'
        if ($started.Kin) { Stop-DevKin 10 }
        if ($started.Emulator) { Invoke-EmuScript 'stop.ps1' @('-All') 120 | Out-Null }
        & $Adb kill-server 2>$null | Out-Null
    }
    Remove-Item Env:\KIN_DEV_PIPE_SUFFIX -ErrorAction SilentlyContinue
    # nothing of this run may be left running (only processes whose program file is in this run's work folder, by ID)
    if (-not $KeepRunning) {
        $left = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($Work, [StringComparison]::OrdinalIgnoreCase) })
        foreach ($l in $left) { Say "left running: $($l.ProcessId) $($l.ExecutablePath): stopping that process"; Stop-Process -Id $l.ProcessId -Force -ErrorAction SilentlyContinue }
        Check 'no process of this run is left running' ($left.Count -eq 0)
    }
    $env:COMPUTERNAME = $script:OrigComputerName
    # After Kin has quit, so that the restored text is not synced to the phone
    if ($clip) { try { $ok = [bool](Restore-Clipboard $clip | Select-Object -Last 1); Check 'PC clipboard restored (every format)' $ok } catch { Say "clipboard restore failed: $_" } }
    $keep = $script:Failures.Count -gt 0 -or $KeepRunning
    if ($keep) { Say "data, logs and dumps kept in $Work" }
    elseif (Test-Path $Work) { Remove-DevDataDir $Work }
}

Write-Host ''
$script:Results.GetEnumerator() | ForEach-Object { Write-Host ("{0,-5} {1}" -f $_.Value, $_.Key) }
foreach ($n in $script:Notes) { Write-Host "NOTE: $n" }
if ($script:Failures.Count -gt 0) { Write-Host "`nFAILED: $($script:Failures -join '; ')"; exit 1 }
Write-Host "`nOK: all $($script:Results.Count) checks passed on $Avd."
exit 0
