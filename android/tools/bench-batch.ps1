# Measures batches of files and one big file, in both directions, between the emulator and a dev desktop on this PC, for two debug APKs: the one
# before the v0.1.1 parallel-files change and the one after it. Both builds see the same PC, the same files and the same path.
#
# The emulator reaches the PC through its own NAT (10.0.2.2) at only about 2 MB/s, whatever the phone or the PC could do, so a batch of
# 50 x 3.4 MB is limited by that pipe and cannot tell two builds apart. The cases that can are the ones where the time is the cost per file,
# not the bytes: many small files. Both kinds are measured (see -Cases), and the report says which is which. Real-hardware numbers need
# a real phone on Wi-Fi (tools\speedtest).
#
#   android\tools\bench-batch.ps1                         # kin_api37, both variants, the default cases
#   android\tools\bench-batch.ps1 -Variants after         # only the new build
#
# Needs: the Kin.exe of desktop\build.ps1 (run as a test Kin beside the user's installed Kin: own --data-dir, tools\kin-dev.ps1), the "before" APK
# (-BeforeApk: a debug build of the committed 0.1.0 source, e.g. `git archive v0.1.0 android | tar -x -C <folder>` and gradlew assembleDebug there)
# and the "after" APK (android\app\build\outputs\apk\debug).
#
# What it never touches: the user's installed Kin and its data (the test Kin has its own --data-dir, single-instance lock and port; every command
# carries --data-dir; it is stopped with its own --quit and, if that fails, by process ID after its program file and data directory are checked),
# the Run key, SendTo, Downloads, the firewall and the clipboard (the test files are random bytes, so Kin's image-to-clipboard has nothing to
# decode; the clipboard is saved and put back anyway). No PC window opens (--dev-no-windows). The emulator is headless and silent.
param(
    [string]$Avd = 'kin_api37',
    [string]$Kin = (Join-Path $PSScriptRoot '..\..\desktop\src-tauri\target\release\Kin.exe'),
    [string]$BeforeApk = '',
    [string]$AfterApk = (Join-Path $PSScriptRoot '..\app\build\outputs\apk\debug\app-debug.apk'),
    # label, count, bytes per file, runs
    [object[]]$Cases = @(
        @{ Label = '50 x 20 KB'; Count = 50; Bytes = 20000; Runs = 5 },
        @{ Label = '50 x 100 KB'; Count = 50; Bytes = 100000; Runs = 5 },
        @{ Label = '50 x 3.4 MB'; Count = 50; Bytes = 3400000; Runs = 1 }
    ),
    [int]$BigMb = 30,
    [int]$Port = 49321,
    [string]$Work = '',
    [string]$OutJson = '',
    [string[]]$Variants = @('before', 'after')
)
. (Join-Path $PSScriptRoot '..\..\tools\kin-lock.ps1'); Enter-KinLock 'bench-batch'
. (Join-Path $PSScriptRoot '..\..\tools\kin-dev.ps1')
. (Join-Path $PSScriptRoot '..\..\desktop\clipboard-backup.ps1')
if (-not $Work) { $Work = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-bench-batch-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
if (-not $OutJson) { $OutJson = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-bench-result-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.json') }
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
$Serial = 'emulator-5554'
$Data = Join-Path $Work 'data'
$Recv = Join-Path $Work 'received'
$PcFiles = Join-Path $Work 'pc-files'
$script:Results = New-Object System.Collections.Generic.List[object]
$started = @{ Emulator = $false; Kin = $false }

function A { & $Adb -s $Serial @args }
function Say($m) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $m) }
function Wait-Until([scriptblock]$Cond, [double]$Seconds, [int]$PollMs = 250) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds $PollMs }
    return $false
}
function Invoke-EmuScript([string]$Script, [string[]]$ScriptArgs = @(), [int]$TimeoutSec = 300) {
    $out = Join-Path $env:TEMP "kin-bench-$Script.out"
    $p = Start-Process powershell -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $Emu $Script))) + $ScriptArgs)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; return $false }
    return $true
}
function Get-Status { try { Get-Content (Join-Path $Data 'status.json') -Raw -ErrorAction Stop | ConvertFrom-Json } catch { $null } }
function Phone-Cmd([string]$Command, [int]$WaitSec = 20) {
    A logcat -c | Out-Null
    A shell am broadcast -a "app.kinrelay.android.debug.$Command" -n "$Pkg/.debug.DebugCommandReceiver" | Out-Null
    $script:l = $null
    Wait-Until { $script:l = (A logcat -d -s KinRelay) | Where-Object { $_ -match "CMD $Command\b" } | Select-Object -Last 1; [bool]$script:l } $WaitSec | Out-Null
    return $script:l
}
function Pc-Ids { $s = Get-Status; @(@($s.transfers.active) + @($s.transfers.recent)) | Where-Object { $_ } | ForEach-Object { $_.id } }
function Wait-NewDone([string[]]$Before, [string]$Dir, [int]$Sec) {
    $script:nd = $null
    $script:maxFA = 0
    Wait-Until { $s = Get-Status; foreach ($x in @($s.transfers.active)) { if ($x -and $x.filesActive -gt $script:maxFA) { $script:maxFA = [int]$x.filesActive } }; $script:nd = @($s.transfers.recent) | Where-Object { $_ -and $_.id -notin $Before -and $_.direction -eq $Dir -and $_.status -in 'done', 'failed', 'canceled' } | Select-Object -First 1; [bool]$script:nd } $Sec 40 | Out-Null
    $script:nd
}
function Now-Ms { [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }
function Clear-PcReceived { Get-ChildItem $Recv -Force -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue }
function Phone-ReceivedCount {
    A logcat -c | Out-Null
    A shell am broadcast -a app.kinrelay.android.debug.RECEIVED -n "$Pkg/.debug.DebugCommandReceiver" | Out-Null
    Start-Sleep -Milliseconds 2500
    @((A logcat -d -s KinRelay) | Where-Object { $_ -match 'CMD RECEIVED (images|video|downloads) ' }).Count
}
# Deletes what the app saved in Pictures/Kin, Movies/Kin and Download/Kin, and waits until the app says none is left.
function Clear-PhoneReceived {
    for ($i = 0; $i -lt 4; $i++) {
        A shell am broadcast -a app.kinrelay.android.debug.DELETE_RECEIVED -n "$Pkg/.debug.DebugCommandReceiver" | Out-Null
        Start-Sleep -Seconds 3
        if ((Phone-ReceivedCount) -eq 0) { return }
    }
    Say 'NOTE: the phone still lists received files after four clean-ups'
}

function Add-Result($Variant, $Case, $Run, $ok, $t, $sw, $t0, $got) {
    $r = [pscustomobject]@{
        Variant = $Variant; Case = $Case; Run = $Run; Ok = [bool]$ok; Status = $t.status; Files = $t.files
        WallS = [math]::Round($sw.Elapsed.TotalSeconds, 2)
        PcRecordS = if ($t -and $t.finishedMs) { [math]::Round(($t.finishedMs - $t.startedMs) / 1000.0, 2) } else { $null }
        TapToStartS = if ($t) { [math]::Round(($t.startedMs - $t0) / 1000.0, 2) } else { $null }
        MaxFilesActive = $script:maxFA; PcFilesOnDisk = if ($got) { $got.Count } else { $null }; PcBytesOnDisk = if ($got) { ($got | Measure-Object Length -Sum).Sum } else { $null }
    }
    $script:Results.Add($r)
    Say ("{0} | {1} | run {2}: ok={3} wall {4} s, Kin's record {5} s, start after {6} s{7}" -f $Variant, $Case, $Run, $r.Ok, $r.WallS, $r.PcRecordS, $r.TapToStartS,
        $(if ($got) { ", $($got.Count) files / $($r.PcBytesOnDisk) bytes on the PC" } else { '' }))
}
function Measure-PhoneToPc([string]$Variant, [string]$Label, [string]$Names, [int]$ExpectFiles, [int]$Run) {
    Clear-PcReceived
    $before = @(Pc-Ids)
    $t0 = Now-Ms
    $sw = [Diagnostics.Stopwatch]::StartNew()
    A shell am broadcast -a app.kinrelay.android.debug.SEND_FILES -n "$Pkg/.debug.DebugCommandReceiver" --es files $Names | Out-Null
    $t = Wait-NewDone $before 'in' 300
    $sw.Stop()
    $got = @(Get-ChildItem $Recv -Recurse -File -ErrorAction SilentlyContinue)
    Add-Result $Variant $Label $Run ($t -and $t.status -eq 'done' -and $t.files -eq $ExpectFiles) $t $sw $t0 $got
}
function Measure-PcToPhone([string]$Variant, [string]$Label, [string[]]$Paths, [int]$Run) {
    Clear-PhoneReceived
    $before = @(Pc-Ids)
    $t0 = Now-Ms
    $sw = [Diagnostics.Stopwatch]::StartNew()
    Invoke-Kin --send @Paths
    $t = Wait-NewDone $before 'out' 300
    $sw.Stop()
    Add-Result $Variant $Label $Run ($t -and $t.status -eq 'done' -and $t.files -eq $Paths.Count) $t $sw $t0 $null
}

try {
    foreach ($p in 'EasyAntiCheat', 'EasyAntiCheat_EOS', 'EACLauncher', 'FortniteClient-Win64-Shipping') {
        if (Get-Process $p -ErrorAction SilentlyContinue) { Say "NOTE: $p is running; this script opens no window on the PC, so it goes on" }
    }
    $Kin = (Resolve-Path $Kin -ErrorAction Stop).Path
    foreach ($v in $Variants) { $apk = if ($v -eq 'before') { $BeforeApk } else { $AfterApk }; if (-not $apk -or -not (Test-Path $apk)) { throw "APK for '$v' is missing: '$apk' (-BeforeApk / -AfterApk)" } }
    Set-KinDev -Exe $Kin -DataDir $Data
    Assert-NoDevKinOnDataDir
    $script:ClipBackup = Save-Clipboard | Select-Object -Last 1

    # ---- files on the PC (random bytes, so nothing compresses)
    New-Item -ItemType Directory -Force $Data, $Recv, $PcFiles | Out-Null
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    $caseFiles = @{}
    foreach ($c in $Cases) {
        $tag = ($c.Label -replace '[^A-Za-z0-9]+', '-').Trim('-')
        $names = @(); $paths = @()
        for ($i = 1; $i -le $c.Count; $i++) {
            $n = 'kin-bench-{0}-{1:D2}.jpg' -f $tag, $i
            $b = New-Object byte[] $c.Bytes; $rng.GetBytes($b)
            [IO.File]::WriteAllBytes((Join-Path $PcFiles $n), $b)
            $names += $n; $paths += (Join-Path $PcFiles $n)
        }
        $caseFiles[$c.Label] = @{ Names = $names; Paths = $paths }
    }
    $bigBytes = $BigMb * 1MB
    $fs = [IO.File]::Create((Join-Path $PcFiles 'kin-bench-big.bin')); $chunk = New-Object byte[] (4MB); $left = $bigBytes
    while ($left -gt 0) { $rng.GetBytes($chunk); $n = [int][math]::Min($left, $chunk.Length); $fs.Write($chunk, 0, $n); $left -= $n }
    $fs.Dispose()
    Say "made the test files on the PC (and one of $BigMb MiB)"

    # ---- emulator
    Say "starting $Avd"
    $started.Emulator = $true
    # The adb server that start.ps1 leaves behind sometimes shows the new emulator as "offline" (seen on 2026-10-17): restart the server once, then look again.
    $emuUp = Invoke-EmuScript 'start.ps1' @('-Avd', $Avd)
    $isBooted = { ((A shell getprop sys.boot_completed 2>$null | Out-String).Trim() -eq '1') }
    if ($emuUp -and -not (& $isBooted)) { & $Adb kill-server 2>$null | Out-Null; Start-Sleep -Seconds 2; & $Adb start-server 2>$null | Out-Null; Wait-Until $isBooted 60 | Out-Null }
    if (-not $emuUp -or -not (& $isBooted)) { throw "emulator $Avd did not start" }
    A shell svc wifi enable | Out-Null
    # start.ps1 reuses whatever emulator is running: the AVD asked for must be the one running
    $runningAvd = ((A emu avd name | Out-String) -split "`r?`n" | Where-Object { $_ } | Select-Object -First 1)
    if ($runningAvd -and $runningAvd.Trim() -ne $Avd) { throw "the emulator that is running is '$($runningAvd.Trim())', not ${Avd}: stop it first (tools\emulator\stop.ps1)" }
    $sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    A shell am broadcast -a app.kinrelay.android.debug.DELETE_RECEIVED -n "$Pkg/.debug.DebugCommandReceiver" 2>$null | Out-Null
    Start-Sleep -Seconds 2
    A uninstall $Pkg 2>&1 | Out-Null
    @'
rm -rf /sdcard/Download/Kin /sdcard/Pictures/Kin /sdcard/Movies/Kin
content delete --uri content://media/external/file --where "_data LIKE '/storage/emulated/0/%/Kin/%'"
'@ | & $Adb -s $Serial shell | Out-Null

    # ---- the dev desktop: loopback only, its own data dir and port, no windows
    $started.Kin = $true
    $KinArgs = @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-port', "$Port",
        '--dev-received-dir', ('"{0}"' -f $Recv), '--dev-print-pair-url', '--dev-no-windows')
    Start-DevKin $KinArgs | Out-Null
    if (-not (Wait-Until { (Get-Status).state -eq 'not_paired' } 30)) { throw 'the dev desktop did not start (is its status.json in the data dir?)' }
    Say ("dev desktop up on 127.0.0.1:{0}" -f (Get-Status).port)

    $first = $true
    foreach ($variant in $Variants) {
        $apk = (Resolve-Path $(if ($variant -eq 'before') { $BeforeApk } else { $AfterApk })).Path
        Say "installing the $variant APK"
        $out = (& $Adb -s $Serial install -r -d $apk 2>&1 | Out-String)
        if ($out -notmatch 'Success') { throw "install failed: $out" }
        if ($sdk -ge 37) { A shell pm grant $Pkg android.permission.ACCESS_LOCAL_NETWORK | Out-Null }
        if ($sdk -ge 33) { A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null }
        A shell am start -n "$Pkg/.MainActivity" | Out-Null
        if ($first) {
            Invoke-Kin --dev-start-pairing
            $urlFile = Join-Path $Data 'pair-url.txt'
            Wait-Until { Test-Path $urlFile } 10 | Out-Null
            $url = (Get-Content $urlFile -Raw).Trim()
            $paired = $false
            for ($try = 1; $try -le 3 -and -not $paired; $try++) {
                A shell am start -a android.intent.action.VIEW -d "'$url'" --ez app.kinrelay.android.AUTO_CONFIRM true 2>$null | Out-Null
                $paired = Wait-Until { (Get-Status).state -eq 'phone_connected' } 25
            }
            if (-not $paired) { throw 'could not pair' }
            Say 'paired'
            $first = $false
        } else {
            Phone-Cmd 'CONNECT' | Out-Null
            if (-not (Wait-Until { (Get-Status).state -eq 'phone_connected' } 40)) { throw 'the phone did not reconnect after the install' }
        }
        # the files for the phone (made inside the app, deterministic bytes) stay across the install of the other build
        $spec = @('kin-bench-warm.bin:200000', "kin-bench-big.bin:$bigBytes")
        foreach ($c in $Cases) { $spec += ($caseFiles[$c.Label].Names | ForEach-Object { '{0}:{1}' -f $_, $c.Bytes }) }
        A shell am broadcast -a app.kinrelay.android.debug.MAKE_FILES -n "$Pkg/.debug.DebugCommandReceiver" --es spec ($spec -join ',') | Out-Null
        Start-Sleep -Seconds 8

        Say "---- $variant"
        # a first small transfer so that both sides have opened their connections and loaded their code
        $before = @(Pc-Ids)
        A shell am broadcast -a app.kinrelay.android.debug.SEND_FILES -n "$Pkg/.debug.DebugCommandReceiver" --es files 'kin-bench-warm.bin' | Out-Null
        Wait-NewDone $before 'in' 60 | Out-Null
        foreach ($c in $Cases) {
            $names = $caseFiles[$c.Label].Names; $paths = $caseFiles[$c.Label].Paths; $nameList = $names -join ','
            for ($run = 1; $run -le $c.Runs; $run++) {
                Measure-PhoneToPc $variant "phone to PC, $($c.Label)" $nameList $c.Count $run
                Measure-PcToPhone $variant "PC to phone, $($c.Label)" $paths $run
            }
        }
        Measure-PhoneToPc $variant "phone to PC, one $BigMb MiB file" 'kin-bench-big.bin' 1 1
        Measure-PcToPhone $variant "PC to phone, one $BigMb MiB file" @((Join-Path $PcFiles 'kin-bench-big.bin')) 1
        Clear-PhoneReceived
    }
}
catch { Say "ERROR: $_" }
finally {
    Say 'tearing down'
    if ($started.Kin) { Stop-DevKin 15 }
    if ($started.Emulator) {
        try { Clear-PhoneReceived } catch { }
        Invoke-EmuScript 'stop.ps1' @() 120 | Out-Null
    }
    $script:Results | ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 $OutJson
    Say "results: $OutJson"
    $env:COMPUTERNAME = $script:OrigComputerName
    if ($script:ClipBackup) { try { if (-not [bool](Restore-Clipboard $script:ClipBackup | Select-Object -Last 1)) { Say 'the clipboard could NOT be restored' } } catch { Say "restore failed: $_" } }
    Remove-DevDataDir $Work
}
Write-Host ''
$script:Results | Format-Table Variant, Case, Run, Ok, WallS, PcRecordS, TapToStartS, MaxFilesActive, PcFilesOnDisk -AutoSize | Out-String -Width 200 | Write-Host
