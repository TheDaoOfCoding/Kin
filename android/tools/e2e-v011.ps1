# End-to-end check of v0.1.1 (up to 4 files of a transfer at once) on the emulator: the real debug APK and the real Kin.exe of `tauri build`
# (desktop\build.ps1), started as a test Kin BESIDE the user's installed Kin (own --data-dir, so its own single-instance lock, port and toast
# identity; tools\kin-dev.ps1), loopback only, a throwaway data dir, no PC window. (Before the "dev Kin beside the installed Kin" work this needed a
# private build with another app identifier, build-dev-pc.ps1: retired.)
#
#   S1 phone -> PC, 50 files through the share path (DebugShareActivity): all there, byte for byte
#   S2 PC -> phone, 50 files: all in Pictures/Kin, byte for byte, no pending entry, no "(1)" copies
#   S3 PC -> phone, 8 files with the same name from 8 folders: 8 different names, 8 different contents (names are made one at a time)
#   S4 PC -> phone, 12 files, the user cancels on the phone while several are in flight: nothing partial is left, what is left is complete
#   S5 phone -> PC, 16 files, the user cancels on the phone while several are in flight: no .part, what is left is complete
#   S6 phone -> PC, 12 files, the PC cancels: the phone ends canceled (not failed)
#   S7 PC -> phone and phone -> PC, 10 x 3 MB files, Wi-Fi off for a few seconds in the middle: everything resumes and arrives, byte for byte
#   S10 phone -> PC 16 x 2 MB: the user cancels on the phone in the middle, then Retry (a new transfer with only the missing files; all 16 right at
#      the end); PC -> phone 12 x 1 MB: the PC cancels in the middle (phone canceled, not failed; nothing pending)
#   S11 the queue with a batch: 12 files shared on the phone while the PC is stopped (the PC then starts: they arrive, byte for byte); 12 files sent
#      by the PC while the phone has no Wi-Fi (they arrive when it is back)
#   S8 (not in a default run: -Only S8) the stall hunt: -S8Runs Wi-Fi cuts per direction, of random length at random moments of a 10 x 3 MB batch,
#      with the phone's logcat and log file and the PC's log kept for every run, and a verdict per run (recovery time; "stalled" if the batch is not
#      done 45 s after the Wi-Fi came back). Kept in -StallDir (default <Work>\stall). See docs\results\v0-1-1-integration.md.
#   S12 (not in a default run: -Only S12) a silent peer: the PC process is frozen (NtSuspendProcess: TCP stays up, nothing is read or answered) with a
#      10 x 3 MB upload running or the link idle; how long until the phone's WebSocket ends (ping/pong timeout), what the transfer does, how it recovers
#      (-S12Cases '15:up','15:idle','0:up'; 0 = the default 240 s interval, which takes up to 8 minutes)
#   S13 (not in a default run: -Only S13) the phone's reconnect backoff: a 10 x 3 MB upload, Wi-Fi off 3 s, and the PC is frozen for -S13Freezes seconds
#      from the moment the Wi-Fi is back (so the phone's first reconnects get no TLS answer); how long after the thaw until the phone is connected and
#      the batch is done (the backoff is 5 s, 30 s, 2 min, 10 min)
#   S14 (not in a default run: -Only S14) a silent drop: the emulator is rooted (`adb root`, a google_apis image) and `iptables` drops everything between the phone
#      and the PC for -S14Seconds while the Wi-Fi stays up (no "Wi-Fi lost", no connection abort, as when a router is unplugged); a 10 x 3 MB batch in each
#      of -S14Dirs. What the phone does (its WebSocket, the HTTP calls, the retries), and how long until the batch is done after the packets flow again
#   S9 (not in a default run: -Only S9) timings: 50 x 3.4 MB in both directions, -S9Runs times each (indicative only: the emulator's own network
#      path is the limit, not the phone or the PC)
#
#   tools\e2e-v011.ps1                               # the milestone's entry: kin_api37, S1 to S7 (it runs this script)
#   android\tools\e2e-v011.ps1 -Avd kin_api29 -Only S1,S2
#
# What it never touches: the user's installed Kin and its data (the test Kin has its own --data-dir, so its own single-instance lock, port and toast
# identity; every command carries --data-dir; processes are recognised by program file AND data directory and stopped by ID, never by name), the
# Run key, SendTo, Downloads, the firewall. The PC clipboard is saved in every format first and put back at the end (the files are random bytes:
# nothing should change it). No input is injected on the PC and no PC window opens. The emulator is headless and silent.
param(
    [string]$Avd = 'kin_api37',
    [string]$Kin = (Join-Path $PSScriptRoot '..\..\desktop\src-tauri\target\release\Kin.exe'),
    [string]$Apk = (Join-Path $PSScriptRoot '..\app\build\outputs\apk\debug\app-debug.apk'),
    [string[]]$Only = @(),
    [int]$Port = 49322,
    [int]$S7Runs = 1,            # S7 more than once in one session (an intermittent failure needs many tries)
    [int]$S8Runs = 30,           # S8: Wi-Fi cuts per direction
    [int]$S9Runs = 3,
    [int]$S14WaitSec = 720,      # S14: how long to wait for the batch after the packets flow again
    [string]$S14Seconds = '6,30,60,100',   # S14: seconds of a real silent drop (iptables DROP of everything to and from the PC on the phone; the Wi-Fi stays "up")
    [string[]]$S14Dirs = @('in', 'out'),
    [string]$S13Freezes = '10,40,100',   # S13: seconds the PC process is frozen right after the Wi-Fi comes back (a PC that cannot answer the phone's reconnect)
    [string[]]$S12Cases = @('15:up', '15:idle', '0:up'),   # S12: <phone WebSocket ping interval in s (0 = the app's 240 s)>:<up = a 10 x 3 MB upload is running | idle>
    [int]$S8CutMinMs = 1500,     # S8: the Wi-Fi is off for a random time in this range (every 10th run 20 s, unless -S8CutMaxMs is under 5000)
    [int]$S8CutMaxMs = 10000,
    [string[]]$S8Dirs = @('out', 'in'),
    [int]$S8Seed = 8011,         # S8: the random cut lengths and moments
    [int]$PingSeconds = 0,       # S8: set the phone's WebSocket ping interval (debug command SET_PING_INTERVAL; 0 = the app's default, 240 s)
    [string]$StallDir = '',
    [string]$Work = ''
)
# Only one Kin script at a time (the emulator, the real clipboard and the Kin.exe slot are shared): tools\kin-lock.ps1.
. (Join-Path $PSScriptRoot '..\..\tools\kin-lock.ps1'); Enter-KinLock 'e2e-v011'
. (Join-Path $PSScriptRoot '..\..\tools\kin-dev.ps1')
. (Join-Path $PSScriptRoot '..\..\desktop\clipboard-backup.ps1')
# the long form of the temp folder ($env:TEMP is 8.3 here; [IO.Path]::GetFullPath expands it only for a path that exists, which would make the data
# directory of Set-KinDev and the one on Kin's command line differ)
if (-not $Work) { $Work = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-e2e-v011-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
if (-not $StallDir) { $StallDir = Join-Path $Work 'stall' }
$script:OrigComputerName = $env:COMPUTERNAME
$env:COMPUTERNAME = 'DESK-PC'    # Kin shows the PC's name on the phone: not the real one
$ErrorActionPreference = 'Continue'
foreach ($name in 'ANDROID_HOME', 'JAVA_HOME') {
    if (-not (Get-Item "env:$name" -ErrorAction SilentlyContinue)) { $v = [Environment]::GetEnvironmentVariable($name, 'User'); if ($v) { Set-Item "env:$name" $v } }
}
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Emu = Join-Path $Repo 'tools\emulator'
$Adb = Join-Path $(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }) 'platform-tools\adb.exe'
$Pkg = 'app.kinrelay.android'
$Serial = 'emulator-5554'
$Data = Join-Path $Work 'data'
$Recv = Join-Path $Data 'received'
$PcFiles = Join-Path $Work 'pc-files'
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Results = [ordered]@{}
$started = @{ Emulator = $false; Kin = $false }

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
    $out = Join-Path $env:TEMP "kin-e2e-v011-$Script.out"
    $p = Start-Process powershell -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $Emu $Script))) + $ScriptArgs)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; return $false }
    return $true
}
function Get-Status { try { Get-Content (Join-Path $Data 'status.json') -Raw -ErrorAction Stop | ConvertFrom-Json } catch { $null } }
function Pc-Ids { $s = Get-Status; @(@($s.transfers.active) + @($s.transfers.recent)) | Where-Object { $_ } | ForEach-Object { $_.id } }
function Wait-NewPc([string[]]$Before, [string]$Dir, [int]$Sec) {
    $script:nd = $null
    Wait-Until { $s = Get-Status; $script:nd = @($s.transfers.recent) | Where-Object { $_ -and $_.id -notin $Before -and $_.direction -eq $Dir -and $_.status -in 'done', 'failed', 'canceled' } | Select-Object -First 1; [bool]$script:nd } $Sec 100 | Out-Null
    $script:nd
}
function Wait-PcActive([string]$Dir, [long]$MinBytes, [int]$Sec = 60) {
    $script:pac = $null
    Wait-Until { $s = Get-Status; $script:pac = @($s.transfers.active) | Where-Object { $_ -and $_.direction -eq $Dir -and $_.bytesDone -ge $MinBytes } | Select-Object -First 1; [bool]$script:pac } $Sec 50 | Out-Null
    $script:pac
}
function Pc-Send([string[]]$Paths) { Invoke-Kin --send @Paths }
function Pc-Recv { @(Get-ChildItem -LiteralPath $Recv -Force -Recurse -File -ErrorAction SilentlyContinue) }
function Pc-Sha([string]$Path) {
    $fs = [IO.File]::OpenRead($Path)
    try { ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($fs)) -replace '-', '').ToLower() } finally { $fs.Dispose() }
}
function Make-PcFile([string]$Path, [long]$Bytes, [int]$Seed) {
    $rnd = New-Object Random $Seed
    $buf = New-Object byte[] (1MB)
    New-Item -ItemType Directory -Force (Split-Path $Path) | Out-Null
    $fs = [IO.File]::Create($Path)
    try { $left = $Bytes; while ($left -gt 0) { $rnd.NextBytes($buf); $n = [int][Math]::Min($left, $buf.Length); $fs.Write($buf, 0, $n); $left -= $n } } finally { $fs.Dispose() }
}

function Phone-Bcast([string]$Command, [string]$Extras = '', [int]$Wait = 1200) {
    A logcat -c | Out-Null
    A shell "am broadcast -a app.kinrelay.android.debug.$Command -n $Pkg/.debug.DebugCommandReceiver $Extras" | Out-Null
    Start-Sleep -Milliseconds $Wait
    @(A logcat -d -s KinRelay)
}
function Run-As([string]$Script) { ($Script | & $Adb -s $Serial shell "run-as $Pkg sh" | Out-String) }
function Phone-DirHashes([string]$Dir) {
    # `adb shell` through PowerShell now and then returns nothing at all under load (seen twice in 15 runs: the files were fine), so an empty answer
    # for a folder that has files is asked again.
    for ($try = 0; $try -lt 3; $try++) {
        $h = @{}
        $out = (A shell "cd '$Dir' 2>/dev/null && sha256sum *" | Out-String)
        foreach ($l in ($out -split "`r?`n")) { if ($l -match '^([0-9a-f]{64})  (.+)$') { $h[$Matches[2]] = $Matches[1] } }
        if ($h.Count -gt 0) { return $h }
        Start-Sleep -Seconds 2
    }
    $h
}
function Phone-SourceHashes {
    $h = @{}
    $out = Run-As 'cd files/debug-share 2>/dev/null && sha256sum *'
    foreach ($l in ($out -split "`r?`n")) { if ($l -match '^([0-9a-f]{64})  (.+)$') { $h[$Matches[2]] = $Matches[1] } }
    $h
}
function Make-PhoneFiles([string]$Spec) {
    A logcat -c | Out-Null
    A shell "am broadcast -a app.kinrelay.android.debug.MAKE_FILES -n $Pkg/.debug.DebugCommandReceiver --es spec '$Spec'" | Out-Null
    $n = @($Spec -split ',').Count
    Wait-Until { @((A logcat -d -s KinRelay) | Where-Object { $_ -match 'CMD MAKE_FILES ' }).Count -ge $n } 90 | Out-Null
}
function Share-Files([string]$Names) { A shell "am start -n $Pkg/.debug.DebugShareActivity --es files '$Names'" | Out-Null }
function Phone-Transfers {
    $lines = @(Phone-Bcast 'TRANSFERS' '' 1000)
    for ($i = 0; $i -lt 8; $i++) {
        $want = if (($lines -join "`n") -match 'CMD TRANSFERS count=(\d+)') { [int]$Matches[1] } else { -1 }
        if ($want -ge 0 -and @($lines | Where-Object { $_ -match 'CMD TRANSFER \S+ (up|down) ' }).Count -ge $want) { break }
        Start-Sleep -Milliseconds 700
        $lines = @(A logcat -d -s KinRelay)
    }
    foreach ($l in $lines) {
        if ($l -match 'CMD TRANSFER (\S+) (up|down) (\w+) (\d+)/(\d+) files=(\d+) names=(.*) err=(\S*)(?: retriedAs=(\S*))?(?: .*)?$') {
            [pscustomobject]@{ Id = $Matches[1]; Dir = $Matches[2]; Status = $Matches[3]; Done = [long]$Matches[4]; Total = [long]$Matches[5]; Files = [int]$Matches[6]; Err = $Matches[8] }
        }
    }
}
function Phone-NewTransfer([string[]]$Before, [string]$Dir, [int]$Sec = 90) {
    $script:pnt = $null
    Wait-Until { $script:pnt = @(Phone-Transfers) | Where-Object { $_.Id -notin $Before -and $_.Dir -eq $Dir -and $_.Status -in 'done', 'failed', 'canceled' } | Select-Object -First 1; [bool]$script:pnt } $Sec | Out-Null
    $script:pnt
}
function Phone-Ids { @(Phone-Transfers | ForEach-Object { $_.Id }) }
function Phone-Received {
    $lines = @(Phone-Bcast 'RECEIVED' '' 1500)
    for ($i = 0; $i -lt 6; $i++) {
        $more = @(A logcat -d -s KinRelay)
        if ($more.Count -eq $lines.Count -and ($more | Where-Object { $_ -match 'CMD RECEIVED' })) { break }
        $lines = $more; Start-Sleep -Milliseconds 700
    }
    foreach ($l in $lines) {
        if ($l -match "CMD RECEIVED (images|video|downloads) id=(\d+) name='(.*)' size=(\d+) mime=(\S*) path=(\S+) pending=(\d)") {
            [pscustomobject]@{ Kind = $Matches[1]; Name = $Matches[3]; Size = [long]$Matches[4]; Path = $Matches[6]; Pending = [int]$Matches[7] }
        }
    }
}
function Clear-Phone {
    A shell svc wifi enable | Out-Null
    Phone-Bcast 'DELETE_RECEIVED' '' 3000 | Out-Null
    Run-As 'rm -rf files/debug-share cache/queue' | Out-Null
    @'
rm -rf /sdcard/Download/Kin /sdcard/Pictures/Kin /sdcard/Movies/Kin
content delete --uri content://media/external/file --where "_data LIKE '/storage/emulated/0/%/Kin/%'"
'@ | & $Adb -s $Serial shell | Out-Null
}
function Clear-Pc {
    Get-ChildItem $Recv -Force -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    Get-ChildItem $PcFiles -Force -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
}
function Wifi([bool]$On) { A shell svc wifi $(if ($On) { 'enable' } else { 'disable' }) | Out-Null }
function Clear-PcRecv { Get-ChildItem $Recv -Force -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue }
function Clear-PhoneReceived {
    Phone-Bcast 'DELETE_RECEIVED' '' 2500 | Out-Null
    @'
rm -rf /sdcard/Download/Kin /sdcard/Pictures/Kin /sdcard/Movies/Kin
content delete --uri content://media/external/file --where "_data LIKE '/storage/emulated/0/%/Kin/%'"
'@ | & $Adb -s $Serial shell | Out-Null
}
# n files of one size made on the phone with head -c from /dev/urandom (MAKE_FILES keeps a whole file in memory; 50 x 3.4 MB is fine for it too, but this is faster)
function Make-PhoneBulk([string]$Prefix, [int]$Count, [long]$Bytes, [string]$Ext = 'bin') {
    $sh = "mkdir -p files/debug-share; i=1; while [ `$i -le $Count ]; do n=`$(printf '%02d' `$i); head -c $Bytes /dev/urandom > files/debug-share/$Prefix-`$n.$Ext; i=`$((i+1)); done; ls files/debug-share | wc -l"
    Run-As $sh | Out-Null
}
function Pc-Link { $s = Get-Status; if ($s) { $s.state } else { $null } }
# Both sides say connected. (The PC's own state alone can be stale: it keeps a phone it has not noticed is gone as connected until a send gets no ack.)
function Wait-Link([int]$Sec = 60) {
    Wait-Until { (Pc-Link) -eq 'phone_connected' -and (((Phone-Bcast 'STATE' '' 700) | Out-String) -match 'link=Connected') } $Sec 300
}
# the link timeline of one run, from the phone's own log lines (logcat -v threadtime): when the phone saw Wi-Fi go and come, and when it was connected again
function Get-PhoneTimeline([string[]]$Lines) {
    $ev = @()
    foreach ($l in $Lines) {
        if ($l -match '(\d{4}-\d\d-\d\dT[\d:.]+)Z? [IWE] (link|service): (.*)$') {
            $t = [DateTime]::Parse($Matches[1], [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::AssumeUniversal -bor [Globalization.DateTimeStyles]::AdjustToUniversal)
            $ev += [pscustomobject]@{ T = $t; Msg = $Matches[3] }
        }
    }
    $ev
}

try {
    $Kin = (Resolve-Path $Kin -ErrorAction Stop).Path
    if (-not (Test-Path $Apk)) { throw "APK missing: $Apk" }
    # `cargo test --release` / `cargo build --release` replace target\release\Kin.exe with a dev-server build: only the exe of `tauri build` is used
    $installer = Get-ChildItem (Join-Path (Split-Path $Kin) 'bundle\nsis') -Filter 'Kin_*_x64-setup.exe' -ErrorAction SilentlyContinue | Sort-Object LastWriteTime | Select-Object -Last 1
    if (-not $installer -or (Get-Item $Kin).LastWriteTime -gt $installer.LastWriteTime.AddSeconds(30)) { throw 'Kin.exe is not from `tauri build` (run desktop\build.ps1)' }
    Set-KinDev -Exe $Kin -DataDir $Data
    New-Item -ItemType Directory -Force $Data, $PcFiles | Out-Null
    Assert-NoDevKinOnDataDir
    $script:ClipBackup = Save-Clipboard | Select-Object -Last 1     # every format; put back in `finally`

    Say "starting $Avd"
    $started.Emulator = $true
    # The adb server that start.ps1 leaves behind sometimes shows the new emulator as "offline" (seen on 2026-10-17): restart the server once, then look again.
    $emuUp = Invoke-EmuScript 'start.ps1' @('-Avd', $Avd)
    $isBooted = { ((A shell getprop sys.boot_completed 2>$null | Out-String).Trim() -eq '1') }
    if ($emuUp -and -not (& $isBooted)) { & $Adb kill-server 2>$null | Out-Null; Start-Sleep -Seconds 2; & $Adb start-server 2>$null | Out-Null; Wait-Until $isBooted 60 | Out-Null }
    if (-not $emuUp -or -not (& $isBooted)) { throw "emulator $Avd did not start" }
    A shell svc wifi enable | Out-Null
    # start.ps1 reuses whatever emulator is running (it refuses a second AVD but does not say so to a caller that only looks at the boot flag): the AVD asked for must be the one running
    $running = ((A emu avd name | Out-String) -split "`r?`n" | Where-Object { $_ } | Select-Object -First 1)
    if ($running -and $running.Trim() -ne $Avd) { throw "the emulator that is running is '$($running.Trim())', not ${Avd}: stop it first (tools\emulator\stop.ps1)" }
    $sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    Say "API $sdk"
    A shell am broadcast -a app.kinrelay.android.debug.DELETE_RECEIVED -n "$Pkg/.debug.DebugCommandReceiver" 2>$null | Out-Null
    Start-Sleep -Seconds 2
    A uninstall $Pkg 2>&1 | Out-Null
    @'
rm -rf /sdcard/Download/Kin /sdcard/Pictures/Kin /sdcard/Movies/Kin
content delete --uri content://media/external/file --where "_data LIKE '/storage/emulated/0/%/Kin/%'"
'@ | & $Adb -s $Serial shell | Out-Null
    $out = (& $Adb -s $Serial install -r (Resolve-Path $Apk).Path 2>&1 | Out-String)
    Check 'install' ($out -match 'Success')
    if ($sdk -ge 37) { A shell pm grant $Pkg android.permission.ACCESS_LOCAL_NETWORK | Out-Null }
    if ($sdk -ge 33) { A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null }
    A shell am start -n "$Pkg/.MainActivity" | Out-Null

    $started.Kin = $true
    $KinArgs = @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-port', "$Port",
        '--dev-received-dir', ('"{0}"' -f $Recv), '--dev-print-pair-url', '--dev-no-windows')
    Start-DevKin $KinArgs | Out-Null
    Check 'desktop started' (Wait-Until { (Get-Status).state -eq 'not_paired' } 30)
    # Kin falls back to a random port when -Port is taken (a Kin left over from a script that was killed): the phone could not find it again after a restart
    if ((Get-Status).port -ne $Port) { throw "the test Kin listens on port $((Get-Status).port), not $Port (taken by another process: a Kin left over from an earlier run?); free it or pass -Port" }
    Invoke-Kin --dev-start-pairing
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    $paired = $false
    for ($try = 1; $try -le 3 -and -not $paired; $try++) {
        A shell am start -a android.intent.action.VIEW -d "'$url'" --ez app.kinrelay.android.AUTO_CONFIRM true 2>$null | Out-Null
        $paired = Wait-Until { (Get-Status).state -eq 'phone_connected' } 25
    }
    Check 'paired and connected' $paired
    if (-not $paired) { throw 'not paired' }

    # ---------------------------------------------------------------- S1 phone -> PC, 50 files, through the share path
    if (Want 'S1') {
        Say 'S1: phone -> PC, 50 files through the share activity'
        Clear-Pc
        $spec = (1..50 | ForEach-Object { 'e2e-s1-{0:D2}.jpg:{1}' -f $_, (20000 + $_ * 1000) }) -join ','
        Make-PhoneFiles $spec
        $src = Phone-SourceHashes
        $before = Pc-Ids
        $sw = [Diagnostics.Stopwatch]::StartNew()
        Share-Files ((1..50 | ForEach-Object { 'e2e-s1-{0:D2}.jpg' -f $_ }) -join ',')
        $t = Wait-NewPc $before 'in' 120
        $sw.Stop()
        Check 'S1 the PC finished the transfer with 50 files' ($t -and $t.status -eq 'done' -and $t.files -eq 50) ("$($t.status) $($t.files) files, {0:N1} s from the tap" -f $sw.Elapsed.TotalSeconds)
        $got = @(Pc-Recv)
        Check 'S1 50 files and no .part in the received folder' ($got.Count -eq 50 -and @($got | Where-Object { $_.Name -like '*.part' }).Count -eq 0) "$($got.Count) files"
        $bad = @($got | Where-Object { (Pc-Sha $_.FullName) -ne $src[$_.Name] })
        Check 'S1 every file is byte for byte the phone''s' ($bad.Count -eq 0 -and $src.Count -ge 50) "$($bad.Count) differ"
        $pt = Phone-NewTransfer @() 'up' 20
        Check 'S1 the phone says done' ($pt -and $pt.Status -eq 'done') "$($pt.Status) $($pt.Err)"
    }

    # ---------------------------------------------------------------- S2 PC -> phone, 50 files
    if (Want 'S2') {
        Say 'S2: PC -> phone, 50 files'
        Clear-Pc; Clear-Phone
        $paths = @(); $want = @{}
        for ($i = 1; $i -le 50; $i++) {
            $p = Join-Path $PcFiles ('e2e-s2-{0:D2}.jpg' -f $i); Make-PcFile $p (20000 + $i * 1000) (100 + $i)
            $paths += $p; $want[(Split-Path $p -Leaf)] = Pc-Sha $p
        }
        $before = Pc-Ids
        $sw = [Diagnostics.Stopwatch]::StartNew()
        Pc-Send $paths
        $t = Wait-NewPc $before 'out' 180
        $sw.Stop()
        Check 'S2 the PC finished the transfer (the phone sent complete)' ($t -and $t.status -eq 'done' -and $t.files -eq 50) ("$($t.status) $($t.files) files, {0:N1} s" -f $sw.Elapsed.TotalSeconds)
        $have = Phone-DirHashes '/sdcard/Pictures/Kin'
        $missing = @($want.Keys | Where-Object { $have[$_] -ne $want[$_] })
        Check 'S2 all 50 are in Pictures/Kin, byte for byte' ($missing.Count -eq 0 -and $have.Count -eq 50) "$($have.Count) files, $($missing.Count) missing or different"
        Check 'S2 no "(1)" copies' (@($have.Keys | Where-Object { $_ -match ' \(\d+\)\.' }).Count -eq 0)
        $rec = @(Phone-Received)
        Check 'S2 no entry is left pending' (@($rec | Where-Object { $_.Pending -eq 1 }).Count -eq 0) "$($rec.Count) entries"
    }

    # ---------------------------------------------------------------- S3 the same name from 8 folders
    if (Want 'S3') {
        Say 'S3: PC -> phone, 8 files called IMG.jpg'
        Clear-Pc; Clear-Phone
        $paths = @(); $hashes = @()
        for ($i = 1; $i -le 8; $i++) {
            $p = Join-Path (Join-Path $PcFiles "dir$i") 'IMG.jpg'; Make-PcFile $p 30000 (200 + $i); $paths += $p; $hashes += (Pc-Sha $p)
        }
        $before = Pc-Ids
        Pc-Send $paths
        $t = Wait-NewPc $before 'out' 90
        Check 'S3 the transfer is done' ($t -and $t.status -eq 'done' -and $t.files -eq 8) "$($t.status) $($t.error)"
        $have = Phone-DirHashes '/sdcard/Pictures/Kin'
        $names = @($have.Keys | Sort-Object)
        Check 'S3 eight different names' ($names.Count -eq 8 -and ($names -contains 'IMG.jpg') -and ($names -contains 'IMG (7).jpg')) ($names -join ', ')
        Check 'S3 eight different contents, each is one of the PC''s' ((@($have.Values | Sort-Object -Unique)).Count -eq 8 -and @($have.Values | Where-Object { $_ -notin $hashes }).Count -eq 0)
    }

    # ---------------------------------------------------------------- S4 PC -> phone, the user cancels on the phone
    if (Want 'S4') {
        Say 'S4: PC -> phone, 12 x 5 MB, canceled on the phone with several in flight'
        Clear-Pc; Clear-Phone
        $paths = @(); $want = @{}
        for ($i = 1; $i -le 12; $i++) { $p = Join-Path $PcFiles ('e2e-s4-{0:D2}.bin' -f $i); Make-PcFile $p 5MB (300 + $i); $paths += $p; $want[(Split-Path $p -Leaf)] = Pc-Sha $p }
        $before = Pc-Ids; $pbefore = Phone-Ids
        Pc-Send $paths
        $a = Wait-PcActive 'out' 2MB 90
        Check 'S4 the transfer is running' ($null -ne $a)
        $ptx = @(Phone-Transfers) | Where-Object { $_.Id -notin $pbefore -and $_.Dir -eq 'down' -and $_.Status -eq 'active' } | Select-Object -First 1
        if ($ptx) { Phone-Bcast 'CANCEL_TRANSFER' "--es id $($ptx.Id)" 600 | Out-Null }
        $pt = Phone-NewTransfer $pbefore 'down' 30
        Check 'S4 phone: canceled' ($pt -and $pt.Status -eq 'canceled') "$($pt.Status) $($pt.Err)"
        $t = Wait-NewPc $before 'out' 30
        Check 'S4 PC: canceled' ($t -and $t.status -eq 'canceled') "$($t.status) $($t.error)"
        Start-Sleep -Seconds 2
        $rec = @(Phone-Received)
        Check 'S4 no pending entry is left' (@($rec | Where-Object { $_.Pending -eq 1 }).Count -eq 0) "$($rec.Count) entries left"
        $have = Phone-DirHashes '/sdcard/Download/Kin'
        $wrong = @($have.Keys | Where-Object { $have[$_] -ne $want[$_] })
        Check 'S4 what is left is complete and right (files saved before the cancel stay)' ($wrong.Count -eq 0 -and $have.Count -lt 12) "$($have.Count) of 12 left, $($wrong.Count) wrong"
    }

    # ---------------------------------------------------------------- S5 phone -> PC, the user cancels on the phone
    if (Want 'S5') {
        Say 'S5: phone -> PC, 16 x 2 MB, canceled on the phone with several in flight'
        Clear-Pc; Clear-Phone
        Make-PhoneFiles ((1..16 | ForEach-Object { 'e2e-s5-{0:D2}.bin:2000000' -f $_ }) -join ',')
        $src = Phone-SourceHashes
        $before = Pc-Ids; $pbefore = Phone-Ids
        Share-Files ((1..16 | ForEach-Object { 'e2e-s5-{0:D2}.bin' -f $_ }) -join ',')
        $a = Wait-PcActive 'in' 1MB 90
        Check 'S5 the upload is running' ($null -ne $a)
        # the PC's record of an upload has the phone's transfer ID: cancel at once, no look-up that takes a second or two
        if ($a) { Phone-Bcast 'CANCEL_TRANSFER' "--es id $($a.id)" 300 | Out-Null }
        $pt = Phone-NewTransfer $pbefore 'up' 30
        Check 'S5 phone: canceled' ($pt -and $pt.Status -eq 'canceled') "$($pt.Status) $($pt.Err)"
        $t = Wait-NewPc $before 'in' 30
        Check 'S5 PC: canceled' ($t -and $t.status -eq 'canceled') "$($t.status) $($t.error)"
        Check 'S5 PC: no .part left' (Wait-Until { @(Pc-Recv | Where-Object { $_.Name -like '*.part' }).Count -eq 0 } 10)
        $got = @(Pc-Recv)
        $wrong = @($got | Where-Object { (Pc-Sha $_.FullName) -ne $src[$_.Name] })
        Check 'S5 PC: what is left is complete and right' ($wrong.Count -eq 0 -and $got.Count -lt 16) "$($got.Count) of 16 left, $($wrong.Count) wrong"
    }

    # ---------------------------------------------------------------- S6 phone -> PC, the PC cancels
    if (Want 'S6') {
        Say 'S6: phone -> PC, 12 x 1 MB, canceled on the PC'
        Clear-Pc; Clear-Phone
        Make-PhoneFiles ((1..12 | ForEach-Object { 'e2e-s6-{0:D2}.bin:1000000' -f $_ }) -join ',')
        $before = Pc-Ids; $pbefore = Phone-Ids
        Share-Files ((1..12 | ForEach-Object { 'e2e-s6-{0:D2}.bin' -f $_ }) -join ',')
        $a = Wait-PcActive 'in' 2MB 90
        Check 'S6 the upload is running' ($null -ne $a)
        Invoke-Kin --cancel-transfers
        $t = Wait-NewPc $before 'in' 30
        Check 'S6 PC: canceled' ($t -and $t.status -eq 'canceled') "$($t.status) $($t.error)"
        $pt = Phone-NewTransfer $pbefore 'up' 30
        Check 'S6 phone: canceled by the PC, not failed' ($pt -and $pt.Status -eq 'canceled') "$($pt.Status) $($pt.Err)"
        Check 'S6 PC: no .part left' (Wait-Until { @(Pc-Recv | Where-Object { $_.Name -like '*.part' }).Count -eq 0 } 10)
    }

    # ---------------------------------------------------------------- S7 Wi-Fi off in the middle, both directions
    if (Want 'S7') {
        foreach ($dir in (1..$S7Runs | ForEach-Object { 'out', 'in' })) {
            $label = if ($dir -eq 'out') { 'PC -> phone' } else { 'phone -> PC' }
            Say "S7: $label, 10 x 3 MB, Wi-Fi off for 6 s in the middle"
            Clear-Pc; Clear-Phone
            $before = Pc-Ids; $pbefore = Phone-Ids
            $want = @{}
            if ($dir -eq 'out') {
                $paths = @()
                for ($i = 1; $i -le 10; $i++) { $p = Join-Path $PcFiles ('e2e-s7o-{0:D2}.bin' -f $i); Make-PcFile $p 3MB (400 + $i); $paths += $p; $want[(Split-Path $p -Leaf)] = Pc-Sha $p }
                Pc-Send $paths
            } else {
                Make-PhoneFiles ((1..10 | ForEach-Object { 'e2e-s7i-{0:D2}.bin:3000000' -f $_ }) -join ',')
                $want = Phone-SourceHashes
                Share-Files ((1..10 | ForEach-Object { 'e2e-s7i-{0:D2}.bin' -f $_ }) -join ',')
            }
            $a = Wait-PcActive $dir 4MB 90
            Check "S7 $label the transfer is running" ($null -ne $a)
            Wifi $false; Start-Sleep -Seconds 6; Wifi $true
            $t = Wait-NewPc $before $dir 300
            if (-not $t) {
                Say ("S7 $label did not finish; phone transfers: " + ((Phone-Transfers | ForEach-Object { "$($_.Dir) $($_.Status) $($_.Done)/$($_.Total) err=$($_.Err)" }) -join ' | '))
                Say ("phone log:`n" + ((Run-As 'tail -n 120 files/logs/kin.log') -replace "`r", ''))
            }
            Check "S7 $label done after the Wi-Fi came back" ($t -and $t.status -eq 'done' -and $t.files -eq 10) "$($t.status) $($t.error)"
            if ($dir -eq 'out') {
                $have = Phone-DirHashes '/sdcard/Download/Kin'
                $wrong = @($want.Keys | Where-Object { $have[$_] -ne $want[$_] })
                Check "S7 $label 10 files on the phone, byte for byte, no copies" ($wrong.Count -eq 0 -and $have.Count -eq 10) "$($have.Count) files, $($wrong.Count) wrong: $($wrong -join ', ')"
                if ($wrong.Count -gt 0) {
                    $sizes = (A shell "ls -l /sdcard/Download/Kin" | Out-String); Say "phone folder:`n$sizes"
                    foreach ($w in $wrong) {
                        $dst = Join-Path $Work "pulled-$w"
                        & $Adb -s $Serial pull "/sdcard/Download/Kin/$w" $dst 2>&1 | Out-Null
                        if (Test-Path $dst) {
                            $x = [IO.File]::ReadAllBytes($dst); $y = [IO.File]::ReadAllBytes((Join-Path $PcFiles $w)); $first = -1
                            for ($k = 0; $k -lt [Math]::Min($x.Length, $y.Length); $k++) { if ($x[$k] -ne $y[$k]) { $first = $k; break } }
                            $diffs = 0; for ($k = 0; $k -lt [Math]::Min($x.Length, $y.Length); $k++) { if ($x[$k] -ne $y[$k]) { $diffs++ } }
                            Say "DIFF $w : phone $($x.Length) bytes, PC $($y.Length) bytes, first difference at $first, $diffs bytes differ"
                        }
                    }
                    Say ("phone log:`n" + ((A logcat -d -s KinRelay | Select-Object -Last 80) -join "`n"))
                }
            } else {
                $got = @(Pc-Recv)
                $wrong = @($got | Where-Object { (Pc-Sha $_.FullName) -ne $want[$_.Name] })
                Check "S7 $label 10 files on the PC, byte for byte, no copies, no .part" ($wrong.Count -eq 0 -and $got.Count -eq 10 -and @($got | Where-Object { $_.Name -like '*.part' }).Count -eq 0) "$($got.Count) files, $($wrong.Count) wrong"
            }
        }
    }

    # ---------------------------------------------------------------- S8 the stall hunt (not in a default run)
    if ($Only.Count -gt 0 -and (Want 'S8')) {
        Say "S8: stall hunt, $S8Runs Wi-Fi cuts per direction, logs in $StallDir"
        New-Item -ItemType Directory -Force $StallDir | Out-Null
        Clear-Pc; Clear-Phone
        $outPaths = @(); $wantOut = @{}
        for ($i = 1; $i -le 10; $i++) { $p = Join-Path $PcFiles ('e2e-s8o-{0:D2}.bin' -f $i); Make-PcFile $p 3MB (500 + $i); $outPaths += $p; $wantOut[(Split-Path $p -Leaf)] = Pc-Sha $p }
        Make-PhoneFiles ((1..10 | ForEach-Object { 'e2e-s8i-{0:D2}.bin:3000000' -f $_ }) -join ',')
        $wantIn = Phone-SourceHashes
        $inNames = (1..10 | ForEach-Object { 'e2e-s8i-{0:D2}.bin' -f $_ }) -join ','
        if ($PingSeconds -gt 0) { Phone-Bcast 'SET_PING_INTERVAL' "--ei seconds $PingSeconds" 500 | Out-Null; Say "S8: the phone's WebSocket ping interval is $PingSeconds s from the next connection" }
        $rnd = New-Object Random $S8Seed
        $pcLog = Join-Path $Data 'logs\kin.log'
        $rows = New-Object System.Collections.Generic.List[object]
        for ($run = 1; $run -le $S8Runs; $run++) {
            foreach ($dir in $S8Dirs) {
                $tag = '{0:D3}-{1}' -f $run, $dir
                Wifi $true
                if (-not (Wait-Link 90)) { Say "S8 ${tag}: the link is not up before the run; toggling Wi-Fi"; Wifi $false; Start-Sleep 3; Wifi $true; Wait-Link 90 | Out-Null }
                Start-Sleep -Milliseconds 1500
                Clear-PcRecv
                if ($dir -eq 'out') { Clear-PhoneReceived }
                $before = Pc-Ids
                A logcat -c | Out-Null
                $pcOff = if (Test-Path $pcLog) { (Get-Item $pcLog).Length } else { 0 }
                $sw = [Diagnostics.Stopwatch]::StartNew()
                if ($dir -eq 'out') { Pc-Send $outPaths } else { Share-Files $inNames }
                $thr = @(2MB, 5MB, 9MB, 14MB, 20MB)[$rnd.Next(5)]
                $a = Wait-PcActive $dir $thr 90
                $cutMs = if ($run % 10 -eq 0 -and $S8CutMaxMs -ge 5000) { 20000 } else { $rnd.Next($S8CutMinMs, $S8CutMaxMs) }
                $mbAtCut = if ($a) { [Math]::Round($a.bytesDone / 1MB, 1) } else { -1 }
                $filesAtCut = if ($a) { $a.filesActive } else { -1 }
                Wifi $false; Start-Sleep -Milliseconds $cutMs; Wifi $true
                $tOn = $sw.Elapsed.TotalSeconds
                $t = $null; $diag = $false
                while (($sw.Elapsed.TotalSeconds - $tOn) -lt 240) {
                    $s = Get-Status
                    $t = @($s.transfers.recent) | Where-Object { $_ -and $_.id -notin $before -and $_.direction -eq $dir -and $_.status -in 'done', 'failed', 'canceled' } | Select-Object -First 1
                    if ($t) { break }
                    if (-not $diag -and ($sw.Elapsed.TotalSeconds - $tOn) -gt 45) {
                        $diag = $true
                        $d = @("STALL? $tag at +45 s after Wi-Fi came back")
                        $d += 'PC status: ' + (($s | ConvertTo-Json -Depth 6 -Compress) -replace '"(token|pairUrl)":"[^"]*"', '"$1":"x"')
                        $d += 'PC sockets: ' + ((netstat -ano | Select-String ":$Port\s" | ForEach-Object { $_.Line.Trim() }) -join ' | ')
                        $d += 'phone sockets: ' + ((A shell netstat -tn 2>&1 | Out-String) -replace "`r", '')
                        $d += 'phone STATE: ' + ((Phone-Bcast 'STATE' '' 1200 | Where-Object { $_ -match 'CMD STATE' } | Select-Object -Last 1))
                        $d += 'phone transfers: ' + ((Phone-Transfers | ForEach-Object { "$($_.Dir) $($_.Status) $($_.Done)/$($_.Total) err=$($_.Err)" }) -join ' | ')
                        $d | Set-Content -Encoding UTF8 (Join-Path $StallDir "$tag-diag.txt")
                        Say "S8 $tag still not done 45 s after the Wi-Fi came back: diagnostics in $StallDir\$tag-diag.txt"
                    }
                    Start-Sleep -Milliseconds 200
                }
                $recovery = [Math]::Round($sw.Elapsed.TotalSeconds - $tOn, 1)
                $done = [bool]($t -and $t.status -eq 'done' -and $t.files -eq 10)
                if (-not $t) {   # never finished: leave nothing running into the next run
                    Invoke-Kin --cancel-transfers
                    foreach ($x in @(Phone-Transfers | Where-Object { $_.Status -in 'active', 'queued' })) { Phone-Bcast 'CANCEL_TRANSFER' "--es id $($x.Id)" 300 | Out-Null }
                }
                # the bytes
                $bytesOk = $false
                if ($done) {
                    if ($dir -eq 'out') { $have = Phone-DirHashes '/sdcard/Download/Kin'; $bytesOk = ($have.Count -eq 10 -and @($wantOut.Keys | Where-Object { $have[$_] -ne $wantOut[$_] }).Count -eq 0) }
                    else { $got = @(Pc-Recv); $bytesOk = ($got.Count -eq 10 -and @($got | Where-Object { $_.Name -like '*.part' -or (Pc-Sha $_.FullName) -ne $wantIn[$_.Name] }).Count -eq 0) }
                }
                # the logs of this run
                $plog = @(A logcat -d -v threadtime)
                $plog | Set-Content -Encoding UTF8 (Join-Path $StallDir "$tag-phone-logcat.txt")
                (Run-As 'tail -n 400 files/logs/kin.log') -replace "`r", '' | Set-Content -Encoding UTF8 (Join-Path $StallDir "$tag-phone-kin.log")
                try {
                    $fs = [IO.File]::Open($pcLog, 'Open', 'Read', 'ReadWrite'); try { $null = $fs.Seek($pcOff, 'Begin'); $buf = New-Object byte[] ($fs.Length - $pcOff); $null = $fs.Read($buf, 0, $buf.Length) } finally { $fs.Dispose() }
                    [IO.File]::WriteAllBytes((Join-Path $StallDir "$tag-pc-kin.log"), $buf)
                } catch { }
                $ev = @(Get-PhoneTimeline $plog)
                $lost = $ev | Where-Object { $_.Msg -like 'Wi-Fi lost*' } | Select-Object -First 1
                $avail = if ($lost) { $ev | Where-Object { $_.Msg -like 'Wi-Fi available*' -and $_.T -gt $lost.T } | Select-Object -First 1 } else { $null }
                $conn = if ($avail) { $ev | Where-Object { $_.Msg -like 'connected to*' -and $_.T -gt $avail.T } | Select-Object -First 1 } else { $null }
                $fails = if ($avail -and $conn) { @($ev | Where-Object { $_.Msg -like 'PC not reachable*' -and $_.T -gt $avail.T -and $_.T -lt $conn.T }).Count } else { -1 }
                $row = [pscustomobject]@{
                    Tag = $tag; CutMs = $cutMs; MBAtCut = $mbAtCut; FilesAtCut = $filesAtCut; RecoverySec = $recovery; Done = $done; BytesOk = $bytesOk
                    LostToAvailSec = $(if ($lost -and $avail) { [Math]::Round(($avail.T - $lost.T).TotalSeconds, 1) } else { $null })
                    AvailToConnectedSec = $(if ($avail -and $conn) { [Math]::Round(($conn.T - $avail.T).TotalSeconds, 1) } else { $null })
                    FailedAttempts = $fails; Stalled = (-not $done) -or ($recovery -gt 45)
                }
                $rows.Add($row)
                Say ("S8 {0}: cut {1} ms at {2} MB ({3} files in flight): done {4} in {5} s after Wi-Fi back; lost->available {6} s, available->connected {7} s, failed attempts {8}, bytes {9}{10}" -f `
                    $tag, $cutMs, $mbAtCut, $filesAtCut, $done, $recovery, $row.LostToAvailSec, $row.AvailToConnectedSec, $fails, $bytesOk, $(if ($row.Stalled) { '   <<< STALL' } else { '' }))
                $rows | Export-Csv -NoTypeInformation -Encoding UTF8 (Join-Path $StallDir 'summary.csv')
            }
        }
        foreach ($dir in $S8Dirs) {
            $r = @($rows | Where-Object { $_.Tag -like "*-$dir" })
            $st = @($r | Where-Object { $_.Stalled })
            $rec = @($r | ForEach-Object { $_.RecoverySec } | Sort-Object)
            $med = if ($rec.Count) { $rec[[int][Math]::Floor($rec.Count / 2)] } else { $null }
            Check "S8 $dir no stall in $($r.Count) cuts (recovery > 45 s or not done)" ($st.Count -eq 0) ("median recovery $med s, max $($rec[-1]) s; stalled: $(($st | ForEach-Object { $_.Tag }) -join ', ')")
            Check "S8 $dir every batch that finished is byte for byte" (@($r | Where-Object { $_.Done -and -not $_.BytesOk }).Count -eq 0)
        }
    }

    # ---------------------------------------------------------------- S12 a silent peer (not in a default run)
    if ($Only.Count -gt 0 -and (Want 'S12')) {
        Add-Type -Namespace Kin12 -Name Native -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("ntdll.dll")] public static extern int NtSuspendProcess(System.IntPtr h);
[System.Runtime.InteropServices.DllImport("ntdll.dll")] public static extern int NtResumeProcess(System.IntPtr h);
'@
        New-Item -ItemType Directory -Force $StallDir | Out-Null
        Say "S12: a silent peer (the PC process frozen), cases $($S12Cases -join ', ')"
        Clear-Pc; Clear-Phone
        Make-PhoneFiles ((1..10 | ForEach-Object { 'e2e-s12-{0:D2}.bin:3000000' -f $_ }) -join ',')
        $wantIn = Phone-SourceHashes
        $inNames = (1..10 | ForEach-Object { 'e2e-s12-{0:D2}.bin' -f $_ }) -join ','
        $frozen = $null
        foreach ($case in $S12Cases) {
            $ping, $mode = $case -split ':'
            $ping = [int]$ping
            $limit = if ($ping -gt 0) { [Math]::Max(120, $ping * 3 + 90) } else { 620 }
            $tag = "s12-ping$ping-$mode"
            try {
                Clear-PcRecv
                Phone-Bcast 'SET_PING_INTERVAL' ("--ei seconds " + $(if ($ping -gt 0) { $ping } else { 240 })) 500 | Out-Null
                Wifi $false; Start-Sleep -Seconds 3; Wifi $true     # the interval applies from the next connection
                Check "S12 $tag the link is up with the interval set" (Wait-Link 90)
                Start-Sleep -Seconds 2
                $before = Pc-Ids
                if ($mode -eq 'up') {
                    Share-Files $inNames
                    $a = Wait-PcActive 'in' 6MB 90
                    Check "S12 $tag the upload is running" ($null -ne $a)
                }
                A logcat -c | Out-Null
                $procs = @(Get-DevKinProcesses)
                foreach ($pr in $procs) { [void][Kin12.Native]::NtSuspendProcess($pr.Handle) }
                $frozen = $procs
                $sw = [Diagnostics.Stopwatch]::StartNew()
                $detected = $null; $lines = @()
                while ($sw.Elapsed.TotalSeconds -lt $limit) {
                    Start-Sleep -Seconds 2
                    $lines = @(A logcat -d -v threadtime -s KinRelay)
                    $hit = $lines | Where-Object { $_ -match 'link: connection ended|link: disconnected' } | Select-Object -First 1
                    if ($hit) { $detected = [Math]::Round($sw.Elapsed.TotalSeconds, 0); break }
                }
                $tFrozen = [Math]::Round($sw.Elapsed.TotalSeconds, 0)
                foreach ($pr in $frozen) { try { [void][Kin12.Native]::NtResumeProcess($pr.Handle) } catch { } }
                $frozen = $null
                $lines | Set-Content -Encoding UTF8 (Join-Path $StallDir "$tag-phone-while-frozen.txt")
                Say ("S12 ${tag}: the phone's link ended after {0} s of silence (limit {1} s): {2}" -f $detected, $limit, (($lines | Where-Object { $_ -match 'link: (connection ended|disconnected)' } | Select-Object -First 1) -replace '^.*KinRelay: ', ''))
                Check "S12 $tag the phone noticed the silent PC (its link ended) within $limit s" ($null -ne $detected) "after $detected s"
                $sw2 = [Diagnostics.Stopwatch]::StartNew()
                Check "S12 $tag the phone is connected again after the PC thawed" (Wait-Link 240) ("{0:N0} s" -f $sw2.Elapsed.TotalSeconds)
                if ($mode -eq 'up') {
                    $t = Wait-NewPc $before 'in' 300
                    $pt = @(Phone-Transfers) | Where-Object { $_.Dir -eq 'up' } | Select-Object -Last 1
                    Say "S12 ${tag}: PC record $($t.status) $($t.files) files; phone: $($pt.Status) $($pt.Err); {0:N0} s after the thaw" -f $sw2.Elapsed.TotalSeconds
                    $got = @(Pc-Recv)
                    Check "S12 $tag the batch ends done with every file byte for byte (or the phone says failed, so a user Retry exists)" (($t -and $t.status -eq 'done' -and $got.Count -eq 10 -and @($got | Where-Object { $_.Name -like '*.part' -or (Pc-Sha $_.FullName) -ne $wantIn[$_.Name] }).Count -eq 0) -or ($pt -and $pt.Status -eq 'failed')) "PC $($t.status), phone $($pt.Status) $($pt.Err)"
                }
                (Run-As 'tail -n 300 files/logs/kin.log') -replace "`r", '' | Set-Content -Encoding UTF8 (Join-Path $StallDir "$tag-phone-kin.log")
            } finally {
                if ($frozen) { foreach ($pr in $frozen) { try { [void][Kin12.Native]::NtResumeProcess($pr.Handle) } catch { } }; $frozen = $null }
            }
        }
        Phone-Bcast 'SET_PING_INTERVAL' '--ei seconds 240' 300 | Out-Null
    }

    # ---------------------------------------------------------------- S14 a silent drop (not in a default run)
    if ($Only.Count -gt 0 -and (Want 'S14')) {
        New-Item -ItemType Directory -Force $StallDir | Out-Null
        Say "S14: a silent drop of $S14Seconds s (iptables on the rooted emulator), directions $($S14Dirs -join ', ')"
        & $Adb -s $Serial root 2>&1 | Out-Null
        Start-Sleep -Seconds 3; & $Adb -s $Serial wait-for-device 2>&1 | Out-Null
        Check 'S14 the emulator is rooted (iptables can be used)' ((A shell id | Out-String) -match 'uid=0')
        function Set-Blackhole([bool]$On) {
            $op = if ($On) { '-I' } else { '-D' }
            $pos = if ($On) { ' 1' } else { '' }
            A shell "iptables $op OUTPUT$pos -d 10.0.2.2 -j DROP; iptables $op INPUT$pos -s 10.0.2.2 -j DROP" 2>&1 | Out-Null
        }
        Clear-Pc; Clear-Phone
        $outPaths = @(); $wantOut = @{}
        for ($i = 1; $i -le 10; $i++) { $p = Join-Path $PcFiles ('e2e-s14o-{0:D2}.bin' -f $i); Make-PcFile $p 3MB (900 + $i); $outPaths += $p; $wantOut[(Split-Path $p -Leaf)] = Pc-Sha $p }
        Make-PhoneFiles ((1..10 | ForEach-Object { 'e2e-s14i-{0:D2}.bin:3000000' -f $_ }) -join ',')
        $wantIn = Phone-SourceHashes
        $inNames = (1..10 | ForEach-Object { 'e2e-s14i-{0:D2}.bin' -f $_ }) -join ','
        $holeOn = $false
        foreach ($dir in $S14Dirs) {
            foreach ($sec in @($S14Seconds -split ',' | ForEach-Object { [int]$_ })) {
                $tag = "s14-$dir-${sec}s"
                try {
                    Clear-PcRecv
                    if ($dir -eq 'out') { Clear-PhoneReceived }
                    Phone-Bcast 'SET_PING_INTERVAL' '--ei seconds 240' 300 | Out-Null
                    Check "S14 $tag the link is up" (Wait-Link 120)
                    Start-Sleep -Seconds 2
                    $before = Pc-Ids
                    if ($dir -eq 'out') { Pc-Send $outPaths } else { Share-Files $inNames }
                    $a = Wait-PcActive $dir 8MB 90
                    Check "S14 $tag the batch is running" ($null -ne $a)
                    A logcat -c | Out-Null
                    $pcLog = Join-Path $Data 'logs\kin.log'; $pcOff = (Get-Item $pcLog).Length
                    $sw = [Diagnostics.Stopwatch]::StartNew()
                    Set-Blackhole $true; $holeOn = $true
                    Start-Sleep -Seconds $sec
                    Set-Blackhole $false; $holeOn = $false
                    $tBack = $sw.Elapsed.TotalSeconds
                    $t = $null
                    while (($sw.Elapsed.TotalSeconds - $tBack) -lt $S14WaitSec) {
                        $st = Get-Status
                        $t = @($st.transfers.recent) | Where-Object { $_ -and $_.id -notin $before -and $_.direction -eq $dir -and $_.status -in 'done', 'failed', 'canceled' } | Select-Object -First 1
                        if ($t) { break }
                        Start-Sleep -Milliseconds 500
                    }
                    $after = [Math]::Round($sw.Elapsed.TotalSeconds - $tBack, 0)
                    $plog = @(A logcat -d -v threadtime)
                    $plog | Set-Content -Encoding UTF8 (Join-Path $StallDir "$tag-phone-logcat.txt")
                    try {
                        $fs = [IO.File]::Open($pcLog, 'Open', 'Read', 'ReadWrite'); try { $null = $fs.Seek($pcOff, 'Begin'); $buf = New-Object byte[] ($fs.Length - $pcOff); $null = $fs.Read($buf, 0, $buf.Length) } finally { $fs.Dispose() }
                        [IO.File]::WriteAllBytes((Join-Path $StallDir "$tag-pc-kin.log"), $buf)
                    } catch { }
                    $ev = @(Get-PhoneTimeline $plog)
                    $ended = $ev | Where-Object { $_.Msg -like 'connection ended*' } | Select-Object -First 1
                    $lost = $ev | Where-Object { $_.Msg -like 'Wi-Fi lost*' } | Select-Object -First 1
                    $first = if ($ev.Count) { $ev[0].T } else { $null }
                    $okRun = [bool]($t -and $t.status -eq 'done' -and $t.files -eq 10)
                    $bytesOk = $false
                    if ($okRun) {
                        if ($dir -eq 'out') { $have = Phone-DirHashes '/sdcard/Download/Kin'; $bytesOk = ($have.Count -eq 10 -and @($wantOut.Keys | Where-Object { $have[$_] -ne $wantOut[$_] }).Count -eq 0) }
                        else { $got = @(Pc-Recv); $bytesOk = ($got.Count -eq 10 -and @($got | Where-Object { $_.Name -like '*.part' -or (Pc-Sha $_.FullName) -ne $wantIn[$_.Name] }).Count -eq 0) }
                    }
                    $endedTxt = if ($ended) { ($ended.Msg -replace 'hello accepted for [0-9]+ ms', '') -replace '^(.{0,170}).*$', '$1' } else { 'the WebSocket never ended' }
                    Say "S14 ${tag}: the packets flowed again; the batch ended $($t.status) $after s later; 'Wi-Fi lost' seen: $([bool]$lost); WebSocket: $endedTxt"
                    $why = if ($phoneFailed) { ", the phone had failed it ($($phoneFailed.Err)) and only the user Retry finished it" } else { '' }
                    Check "S14 $tag the batch is done after the drop, byte for byte, without the user" ($okRun -and $bytesOk -and -not $phoneFailed) "$after s after the packets flowed again, $($t.status)$why"
                } finally {
                    if ($holeOn) { Set-Blackhole $false; $holeOn = $false }
                }
            }
        }
    }

    # ---------------------------------------------------------------- S13 the reconnect backoff (not in a default run)
    if ($Only.Count -gt 0 -and (Want 'S13')) {
        Add-Type -Namespace Kin13 -Name Native -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("ntdll.dll")] public static extern int NtSuspendProcess(System.IntPtr h);
[System.Runtime.InteropServices.DllImport("ntdll.dll")] public static extern int NtResumeProcess(System.IntPtr h);
'@
        New-Item -ItemType Directory -Force $StallDir | Out-Null
        Say "S13: the PC cannot answer for N seconds after the Wi-Fi came back: $S13Freezes s"
        Clear-Pc; Clear-Phone
        Make-PhoneFiles ((1..10 | ForEach-Object { 'e2e-s13-{0:D2}.bin:3000000' -f $_ }) -join ',')
        $wantIn = Phone-SourceHashes
        $inNames = (1..10 | ForEach-Object { 'e2e-s13-{0:D2}.bin' -f $_ }) -join ','
        $frozen = $null
        foreach ($fz in @($S13Freezes -split ',' | ForEach-Object { [int]$_ })) {
            $tag = "s13-freeze$fz"
            try {
                Clear-PcRecv
                Phone-Bcast 'SET_PING_INTERVAL' '--ei seconds 240' 300 | Out-Null
                Check "S13 $tag the link is up" (Wait-Link 120)
                Start-Sleep -Seconds 2
                $before = Pc-Ids
                Share-Files $inNames
                $a = Wait-PcActive 'in' 6MB 90
                Check "S13 $tag the upload is running" ($null -ne $a)
                A logcat -c | Out-Null
                Wifi $false; Start-Sleep -Seconds 3
                $procs = @(Get-DevKinProcesses)
                foreach ($pr in $procs) { [void][Kin13.Native]::NtSuspendProcess($pr.Handle) }   # frozen before the Wi-Fi is back
                $frozen = $procs
                Wifi $true
                Start-Sleep -Seconds $fz
                foreach ($pr in $frozen) { try { [void][Kin13.Native]::NtResumeProcess($pr.Handle) } catch { } }
                $frozen = $null
                $sw = [Diagnostics.Stopwatch]::StartNew()
                $t = $null
                while ($sw.Elapsed.TotalSeconds -lt 700) {
                    $s = Get-Status
                    $t = @($s.transfers.recent) | Where-Object { $_ -and $_.id -notin $before -and $_.direction -eq 'in' -and $_.status -in 'done', 'failed', 'canceled' } | Select-Object -First 1
                    if ($t) { break }
                    Start-Sleep -Milliseconds 500
                }
                $after = [Math]::Round($sw.Elapsed.TotalSeconds, 0)
                $plog = @(A logcat -d -v threadtime)
                $plog | Set-Content -Encoding UTF8 (Join-Path $StallDir "$tag-phone-logcat.txt")
                $ev = @(Get-PhoneTimeline $plog)
                $backoffs = @($ev | Where-Object { $_.Msg -like 'PC not reachable; next attempt in*' } | ForEach-Object { $_.Msg -replace 'PC not reachable; next attempt in ', '' }) -join ', '
                $okRun = [bool]($t -and $t.status -eq 'done' -and $t.files -eq 10)
                Say "S13 ${tag}: the PC was frozen $fz s from the moment the Wi-Fi was back; the batch was done $after s after the thaw (status $($t.status)); the phone's waits: $backoffs"
                Check "S13 $tag the batch ends done" $okRun "$after s after the thaw; waits $backoffs"
                $got = @(Pc-Recv)
                Check "S13 $tag byte for byte" ($got.Count -eq 10 -and @($got | Where-Object { $_.Name -like '*.part' -or (Pc-Sha $_.FullName) -ne $wantIn[$_.Name] }).Count -eq 0)
            } finally {
                if ($frozen) { foreach ($pr in $frozen) { try { [void][Kin13.Native]::NtResumeProcess($pr.Handle) } catch { } }; $frozen = $null }
            }
        }
    }

    # ---------------------------------------------------------------- S9 timings: 50 x 3.4 MB (not in a default run)
    if ($Only.Count -gt 0 -and (Want 'S9')) {
        Say "S9: 50 x 3.4 MB both directions, $S9Runs runs each"
        Clear-Pc; Clear-Phone
        $paths = @(); $wantOut = @{}
        for ($i = 1; $i -le 50; $i++) { $p = Join-Path $PcFiles ('e2e-s9o-{0:D2}.jpg' -f $i); Make-PcFile $p 3400000 (600 + $i); $paths += $p; $wantOut[(Split-Path $p -Leaf)] = Pc-Sha $p }
        Make-PhoneBulk 'e2e-s9i' 50 3400000 'jpg'
        $wantIn = Phone-SourceHashes
        $inNames = (1..50 | ForEach-Object { 'e2e-s9i-{0:D2}.jpg' -f $_ }) -join ','
        Check 'S9 the 50 files exist on the phone' ($wantIn.Count -eq 50)
        $timesOut = @(); $timesIn = @(); $peakOut = 0; $peakIn = 0
        for ($run = 1; $run -le $S9Runs; $run++) {
            Clear-PhoneReceived; Clear-PcRecv
            $before = Pc-Ids
            $sw = [Diagnostics.Stopwatch]::StartNew()
            Pc-Send $paths
            $peak = 0
            $t = $null
            while ($sw.Elapsed.TotalSeconds -lt 400) {
                $s = Get-Status
                foreach ($x in @($s.transfers.active)) { if ($x -and $x.filesActive -gt $peak) { $peak = $x.filesActive } }
                $t = @($s.transfers.recent) | Where-Object { $_ -and $_.id -notin $before -and $_.direction -eq 'out' -and $_.status -in 'done', 'failed', 'canceled' } | Select-Object -First 1
                if ($t) { break }
                Start-Sleep -Milliseconds 50
            }
            $sec = [Math]::Round($sw.Elapsed.TotalSeconds, 1)
            $have = Phone-DirHashes '/sdcard/Pictures/Kin'
            $missing = @($wantOut.Keys | Where-Object { $have[$_] -ne $wantOut[$_] })
            Check "S9 run $run PC -> phone 50 x 3.4 MB done, byte for byte" ($t -and $t.status -eq 'done' -and $missing.Count -eq 0 -and $have.Count -eq 50) "$sec s ($([Math]::Round(170 / $sec, 1)) MB/s), peak $peak files at once, $($missing.Count) wrong"
            $timesOut += $sec; $peakOut = [Math]::Max($peakOut, $peak)
            Clear-PcRecv
            $before = Pc-Ids
            $sw = [Diagnostics.Stopwatch]::StartNew()
            Share-Files $inNames
            $peak = 0; $t = $null
            while ($sw.Elapsed.TotalSeconds -lt 400) {
                $s = Get-Status
                foreach ($x in @($s.transfers.active)) { if ($x -and $x.filesActive -gt $peak) { $peak = $x.filesActive } }
                $t = @($s.transfers.recent) | Where-Object { $_ -and $_.id -notin $before -and $_.direction -eq 'in' -and $_.status -in 'done', 'failed', 'canceled' } | Select-Object -First 1
                if ($t) { break }
                Start-Sleep -Milliseconds 50
            }
            $sec = [Math]::Round($sw.Elapsed.TotalSeconds, 1)
            $got = @(Pc-Recv)
            $bad = @($got | Where-Object { $_.Name -like '*.part' -or (Pc-Sha $_.FullName) -ne $wantIn[$_.Name] })
            Check "S9 run $run phone -> PC 50 x 3.4 MB done, byte for byte" ($t -and $t.status -eq 'done' -and $bad.Count -eq 0 -and $got.Count -eq 50) "$sec s ($([Math]::Round(170 / $sec, 1)) MB/s), peak $peak files at once, $($bad.Count) wrong"
            $timesIn += $sec; $peakIn = [Math]::Max($peakIn, $peak)
        }
        Say ("S9 PC -> phone: {0} s (peak {1} at once); phone -> PC: {2} s (peak {3} at once)" -f ($timesOut -join ', '), $peakOut, ($timesIn -join ', '), $peakIn)
        Check 'S9 four files at once in both directions' ($peakOut -ge 4 -and $peakIn -ge 4) "peaks $peakOut / $peakIn"
    }

    # ---------------------------------------------------------------- S10 cancel in the middle, then Retry (a batch of several files)
    if (Want 'S10') {
        Say 'S10: phone -> PC, 16 x 2 MB, canceled on the phone'
        Clear-Pc; Clear-Phone
        Make-PhoneFiles ((1..16 | ForEach-Object { 'e2e-s10-{0:D2}.bin:2000000' -f $_ }) -join ',')
        $src = Phone-SourceHashes
        $before = Pc-Ids; $pbefore = Phone-Ids
        Share-Files ((1..16 | ForEach-Object { 'e2e-s10-{0:D2}.bin' -f $_ }) -join ',')
        $a = Wait-PcActive 'in' 6MB 90
        Check 'S10 the upload is running' ($null -ne $a)
        if ($a) { Phone-Bcast 'CANCEL_TRANSFER' "--es id $($a.id)" 300 | Out-Null }
        $pt = Phone-NewTransfer $pbefore 'up' 30
        Check 'S10 phone: canceled' ($pt -and $pt.Status -eq 'canceled') "$($pt.Status) $($pt.Err)"
        $t = Wait-NewPc $before 'in' 30
        Check 'S10 PC: canceled' ($t -and $t.status -eq 'canceled') "$($t.status) $($t.error)"
        Check 'S10 PC: no .part left' (Wait-Until { @(Pc-Recv | Where-Object { $_.Name -like '*.part' }).Count -eq 0 } 10)
        $kept = @(Pc-Recv)
        Check 'S10 the files saved before the cancel are complete and right' (@($kept | Where-Object { (Pc-Sha $_.FullName) -ne $src[$_.Name] }).Count -eq 0 -and $kept.Count -lt 16) "$($kept.Count) of 16 kept"
        # (a user-canceled upload has no Retry button: only a failed one does, see TransferEngine.retry. The failed batch is the next block.)
        Say 'S10: phone -> PC, 16 x 2 MB, the PC forgets the upload (24 h with no progress), the phone fails it (404), user Retry sends only what is missing'
        Clear-Pc; Clear-Phone
        Make-PhoneFiles ((1..16 | ForEach-Object { 'e2e-s10r-{0:D2}.bin:2000000' -f $_ }) -join ',')
        $src = Phone-SourceHashes
        $pcIds0 = @(Pc-Ids); $tids0 = @(Phone-Ids)
        Share-Files ((1..16 | ForEach-Object { 'e2e-s10r-{0:D2}.bin' -f $_ }) -join ',')
        $a = Wait-PcActive 'in' 8MB 90
        Check 'S10r the 16-file upload is running (several files in)' ($null -ne $a)
        $up = @(Phone-Transfers) | Where-Object { $_.Id -notin $tids0 -and $_.Dir -eq 'up' } | Select-Object -First 1
        Wifi $false
        Start-Sleep -Seconds 1
        Stop-DevKin 15
        Start-DevKin ($KinArgs + @('--dev-clock-offset', '90000')) | Out-Null    # a clock 25 hours ahead: the start-up sweep drops an upload with no progress for a day
        Check 'S10r the PC is up (clock +25 h)' (Wait-Until { @(Get-DevKinProcesses).Count -gt 0 } 30)
        Start-Sleep -Seconds 6
        Stop-DevKin 15
        Start-DevKin $KinArgs | Out-Null
        Check 'S10r the PC is up again (normal clock)' (Wait-Until { @(Get-DevKinProcesses).Count -gt 0 } 30)
        $kept = @(Pc-Recv | Where-Object { $_.Name -notlike '*.part' })
        Check 'S10r the PC dropped the unfinished files (no .part)' (@(Pc-Recv | Where-Object { $_.Name -like '*.part' }).Count -eq 0) "$($kept.Count) complete files stay"
        Wifi $true
        Check 'S10r the phone is back' (Wait-Link 90)
        Wait-Until { $script:rw = @(Phone-Transfers) | Where-Object { $_.Id -eq $up.Id }; $script:rw -and $script:rw.Status -in 'failed', 'canceled', 'done' } 60 | Out-Null
        Check 'S10r the phone failed the upload (the PC answered 404: not_found)' ($script:rw -and $script:rw.Status -eq 'failed' -and $script:rw.Err -eq 'not_found') "$($script:rw.Status) $($script:rw.Err)"
        $phIdsBefore = @(Phone-Ids); $pcIdsBefore = @(Pc-Ids)
        Phone-Bcast 'RETRY_TRANSFER' "--es id $($up.Id)" 1000 | Out-Null
        Wait-Until { $script:nr = @(Phone-Transfers) | Where-Object { $_.Id -notin $phIdsBefore -and $_.Dir -eq 'up' } | Select-Object -First 1; [bool]$script:nr } 30 | Out-Null
        $new = $script:nr
        Check 'S10r Retry made a NEW transfer with only the files that never got a 200' ($new -and $new.Id -ne $up.Id -and $new.Files -eq (16 - $kept.Count)) "files=$($new.Files) (expected $(16 - $kept.Count))"
        $t2 = Wait-NewPc $pcIdsBefore 'in' 180
        Check 'S10r the PC finished the retry (done)' ($t2 -and $t2.status -eq 'done' -and $t2.files -eq (16 - $kept.Count)) "$($t2.status) $($t2.files) files"
        $all = @(Pc-Recv)
        $bad = @($all | Where-Object { $_.Name -like '*.part' -or (Pc-Sha $_.FullName) -ne $src[$_.Name] })
        Check 'S10r all 16 files are there, byte for byte, no "(1)" copies' ($all.Count -eq 16 -and $bad.Count -eq 0 -and @($all | Where-Object { $_.Name -match ' \(\d+\)\.' }).Count -eq 0) "$($all.Count) files, $($bad.Count) wrong"

        Say 'S10: PC -> phone, 12 x 5 MB, canceled on the PC'
        Clear-Pc; Clear-Phone
        $paths = @(); $want = @{}
        for ($i = 1; $i -le 12; $i++) { $p = Join-Path $PcFiles ('e2e-s10o-{0:D2}.bin' -f $i); Make-PcFile $p 5MB (700 + $i); $paths += $p; $want[(Split-Path $p -Leaf)] = Pc-Sha $p }
        $before = Pc-Ids; $pbefore = Phone-Ids
        Pc-Send $paths
        $a = Wait-PcActive 'out' 2MB 90
        Check 'S10 the download is running' ($null -ne $a)
        Invoke-Kin --cancel-transfers
        $t = Wait-NewPc $before 'out' 30
        Check 'S10 PC: canceled' ($t -and $t.status -eq 'canceled') "$($t.status) $($t.error)"
        $pt = Phone-NewTransfer $pbefore 'down' 30
        Check 'S10 phone: canceled by the PC, not failed' ($pt -and $pt.Status -eq 'canceled') "$($pt.Status) $($pt.Err)"
        Start-Sleep -Seconds 2
        $rec = @(Phone-Received)
        Check 'S10 phone: no entry is left pending' (@($rec | Where-Object { $_.Pending -eq 1 }).Count -eq 0) "$($rec.Count) entries"
        $have = Phone-DirHashes '/sdcard/Download/Kin'
        Check 'S10 phone: what is left is complete and right' (@($have.Keys | Where-Object { $have[$_] -ne $want[$_] }).Count -eq 0 -and $have.Count -lt 12) "$($have.Count) of 12 left"
    }

    # ---------------------------------------------------------------- S11 the queue with a batch: the PC is down / the phone is offline
    if (Want 'S11') {
        Say 'S11: phone -> PC, 12 files shared while the PC is stopped; the PC starts afterwards'
        Clear-Pc; Clear-Phone
        Make-PhoneFiles ((1..12 | ForEach-Object { 'e2e-s11-{0:D2}.bin:{1}' -f $_, (300000 + $_ * 50000) }) -join ',')
        $src = Phone-SourceHashes
        $before = @(Pc-Ids)
        Stop-DevKin 15
        Check 'S11 the PC is stopped' (@(Get-DevKinProcesses).Count -eq 0)
        Start-Sleep -Seconds 3
        $pbefore = Phone-Ids
        Share-Files ((1..12 | ForEach-Object { 'e2e-s11-{0:D2}.bin' -f $_ }) -join ',')
        Start-Sleep -Seconds 8
        $q = @(Phone-Transfers) | Where-Object { $_.Id -notin $pbefore -and $_.Dir -eq 'up' } | Select-Object -First 1
        Check 'S11 the phone has the batch waiting (queued, not failed)' ($q -and $q.Status -in 'queued', 'active') "$($q.Status) $($q.Err)"
        Start-DevKin $KinArgs | Out-Null
        Check 'S11 the PC is up again' (Wait-Until { @(Get-DevKinProcesses).Count -gt 0 } 30)
        Check 'S11 the phone connects again by itself' (Wait-Link 180) ((Get-Status).state)
        $t = Wait-NewPc $before 'in' 180
        Check 'S11 the queued batch arrives: done, 12 files' ($t -and $t.status -eq 'done' -and $t.files -eq 12) "$($t.status) $($t.files) files $($t.error)"
        $got = @(Pc-Recv)
        $bad = @($got | Where-Object { $_.Name -like '*.part' -or (Pc-Sha $_.FullName) -ne $src[$_.Name] })
        Check 'S11 byte for byte, no copies' ($got.Count -eq 12 -and $bad.Count -eq 0) "$($got.Count) files, $($bad.Count) wrong"

        Say 'S11: PC -> phone, 12 files sent while the phone has no Wi-Fi'
        Clear-Pc; Clear-Phone
        $paths = @(); $want = @{}
        for ($i = 1; $i -le 12; $i++) { $p = Join-Path $PcFiles ('e2e-s11o-{0:D2}.jpg' -f $i); Make-PcFile $p (200000 + $i * 30000) (800 + $i); $paths += $p; $want[(Split-Path $p -Leaf)] = Pc-Sha $p }
        Wifi $false
        Start-Sleep -Seconds 5
        $before = Pc-Ids
        Pc-Send $paths
        Start-Sleep -Seconds 8
        Check 'S11 nothing has arrived while the phone is offline' ((Phone-DirHashes '/sdcard/Pictures/Kin').Count -eq 0)
        Wifi $true
        $t = Wait-NewPc $before 'out' 240
        Check 'S11 the queued batch arrives when the phone is back: done, 12 files' ($t -and $t.status -eq 'done' -and $t.files -eq 12) "$($t.status) $($t.files) files $($t.error)"
        $have = Phone-DirHashes '/sdcard/Pictures/Kin'
        Check 'S11 byte for byte, no copies' (@($want.Keys | Where-Object { $have[$_] -ne $want[$_] }).Count -eq 0 -and $have.Count -eq 12) "$($have.Count) files"
    }

    # ---------------------------------------------------------------- the phone and the logs after all that
    $crash = (A logcat -d | Out-String)
    Check 'no crash in logcat' ($crash -notmatch 'FATAL EXCEPTION' -and $crash -notmatch 'AndroidRuntime: Process: app\.kinrelay')
    $ph = Phone-Bcast 'STATE' '' 1500
    Check 'the wake lock is not held when idle' (($ph | Out-String) -match 'wakeLockHeld=(false|null)')
}
catch { Say "ERROR: $_"; $script:Failures.Add("exception: $_") }
finally {
    Say 'tearing down'
    try { if ($started.Emulator) { Clear-Phone } } catch { }
    if ($started.Kin) { Stop-DevKin 15 }    # --quit to our data directory; if it hangs, only our own process (program file and data directory checked), by ID
    if ($started.Emulator) { Invoke-EmuScript 'stop.ps1' @() 120 | Out-Null }
    $env:COMPUTERNAME = $script:OrigComputerName
    # after Kin has quit, so that the restored content is not synced to a phone
    if ($script:ClipBackup) { try { if (-not [bool](Restore-Clipboard $script:ClipBackup | Select-Object -Last 1)) { Say 'the clipboard could NOT be restored' } } catch { Say "restore failed: $_" } }
    if ($Failures.Count -eq 0 -and (Test-Path $Work)) { Remove-DevDataDir $Work }
    elseif (Test-Path $Work) { Say "data and logs kept in $Work" }
}
Write-Host ''
$script:Results.GetEnumerator() | ForEach-Object { Write-Host ("{0,-5} {1}" -f $_.Value, $_.Key) }
if ($script:Failures.Count -gt 0) { Write-Host "`nFAILED: $($script:Failures -join '; ')"; exit 1 }
Write-Host "`nOK: all $($script:Results.Count) checks passed on $Avd."
exit 0
