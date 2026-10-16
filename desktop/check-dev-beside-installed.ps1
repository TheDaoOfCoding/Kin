# Proof that a test Kin runs BESIDE the user's installed Kin without touching it (v0.1.1, "Dev Kin beside the installed Kin").
#
#   powershell -ExecutionPolicy Bypass -File desktop\check-dev-beside-installed.ps1 [-Exe <Kin.exe of tauri build>] [-Port 49333] [-SkipClipboard]
#
# It starts two test Kins with throwaway data directories and non-default ports (loopback only, no windows), beside whatever Kin is installed
# and running, and checks that
#   * both start (their single-instance lock, toast identity, port and mDNS are their own), and the default port 47821 is not theirs;
#   * a command with --data-dir (--send, --quit) reaches the instance of that data directory and no other (the other test Kin, the installed Kin);
#   * text put on the real clipboard by a test Kin (--dev-set-clipboard) and by Set-QuietClipboardText carries the opt-out format that an
#     installed Kin skips (and the test Kin's own mark), and the test Kin itself reads it as ordinary text;
#   * the installed Kin is the same process at the end (ID, start time, command line), listens on the same port, its Run value and SendTo
#     shortcut are unchanged, and its log (read only, never written) shows nothing new but the clipboard changes it skipped.
# It never sends a command to the installed Kin, never stops it, never kills by name. The clipboard is saved in every format first and put back at
# the end. Exit code 0 only if every check passed.
param(
    [string]$Exe = (Join-Path $PSScriptRoot 'src-tauri\target\release\Kin.exe'),
    [int]$Port = 49333,
    [switch]$SkipClipboard
)
. (Join-Path $PSScriptRoot '..\tools\kin-lock.ps1'); Enter-KinLock 'check-dev-beside-installed'
. (Join-Path $PSScriptRoot '..\tools\kin-dev.ps1')
. (Join-Path $PSScriptRoot 'clipboard-backup.ps1')
$ErrorActionPreference = 'Stop'

$results = New-Object System.Collections.Generic.List[object]
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $results.Add([pscustomobject]@{ Name = $name; Ok = $ok })
    Write-Host ("  [{0}] {1}{2}" -f $(if ($ok) { 'ok' } else { 'FAIL' }), $name, $(if ($detail) { "  ($detail)" } else { '' }))
}
function Say([string]$t) { Write-Host "== $t" }
function Wait-Until([scriptblock]$Cond, [double]$Seconds = 20) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds 100 }
    return $false
}

$Exe = (Resolve-Path -LiteralPath $Exe -ErrorAction Stop).Path
$tempRoot = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) 'Temp'
$stamp = [guid]::NewGuid().ToString('N').Substring(0, 8)
$d1 = Join-Path $tempRoot "kin-beside-1-$stamp"
$d2 = Join-Path $tempRoot "kin-beside-2-$stamp"
$port1 = $Port; $port2 = $Port + 1
$backup = $null

# ----- the installed Kin: what it looks like before (read only) ---------------------------------------------------------------------------
function Get-InstalledKin {
    # A Kin.exe that is not one of ours: the one in Program Files, or any other with no --data-dir (the user's own profile).
    @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" -ErrorAction SilentlyContinue | Where-Object {
        $_.ExecutablePath -and ($_.ExecutablePath -ine $Exe) -and (($_.CommandLine -notmatch '--data-dir') -or ($_.ExecutablePath -like (Join-Path $env:ProgramFiles 'Kin\*')))
    })
}
function Get-InstalledSnapshot {
    $procs = @(Get-InstalledKin | ForEach-Object { [pscustomobject]@{ Id = $_.ProcessId; Started = $_.CreationDate.ToString('o'); Path = $_.ExecutablePath; CommandLine = $_.CommandLine } })
    $run = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue).PSObject.Properties | Where-Object { $_.Name -eq 'Kin' } | ForEach-Object { "$($_.Name)=$($_.Value)" }
    $lnk = Join-Path $env:APPDATA 'Microsoft\Windows\SendTo\Kin.lnk'
    $listening = @(Get-NetTCPConnection -State Listen -LocalPort 47821 -ErrorAction SilentlyContinue | ForEach-Object { $_.OwningProcess } | Sort-Object -Unique)
    [pscustomobject]@{
        Processes = $procs
        RunValue = ($run -join '|')
        SendTo = $(if (Test-Path -LiteralPath $lnk) { (Get-Item -LiteralPath $lnk).LastWriteTimeUtc.ToString('o') } else { 'none' })
        ListeningOn47821 = ($listening -join ',')
    }
}
$installedLog = Join-Path $env:APPDATA 'Kin\logs\kin.log'
function Read-InstalledLogSince([string]$UtcStamp) {
    # Read only (the installed Kin keeps writing: share mode ReadWrite). Lines are 'yyyy-MM-ddTHH:mm:ss.fffZ LEVEL module: message'.
    if (-not (Test-Path -LiteralPath $installedLog)) { return @() }
    $fs = [IO.File]::Open($installedLog, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    try { $text = (New-Object IO.StreamReader($fs)).ReadToEnd() } finally { $fs.Dispose() }
    @($text -split "`n" | Where-Object { $_ -match '^\d{4}-' -and ($_.Substring(0, 24) -gt $UtcStamp) })
}
function Get-KinLogText([string]$dir) { Get-Content -LiteralPath (Join-Path $dir 'logs\kin.log') -Raw -ErrorAction SilentlyContinue }
function Get-KinStatus([string]$dir) { try { Get-Content -LiteralPath (Join-Path $dir 'status.json') -Raw -ErrorAction Stop | ConvertFrom-Json } catch { $null } }
function Count-Matches([string]$text, [string]$pattern) { if (-not $text) { 0 } else { [regex]::Matches($text, $pattern).Count } }

$before = Get-InstalledSnapshot
$t0 = [DateTime]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'")
Say ("installed Kin before: {0} process(es) {1}; listens on 47821: {2}" -f $before.Processes.Count, (($before.Processes | ForEach-Object { $_.Id }) -join ','), $(if ($before.ListeningOn47821) { "pid $($before.ListeningOn47821)" } else { 'nobody' }))
if ($before.Processes.Count -eq 0) { Write-Host 'NOTE: no installed Kin is running now, so "beside the installed Kin" is only partly tested (the two test Kins still run beside each other).' }

try {
    New-Item -ItemType Directory -Force -Path $d1, $d2 | Out-Null
    if (-not $SkipClipboard) { $backup = Save-Clipboard | Select-Object -Last 1 }
    $common = @('--dev-bind', '127.0.0.1', '--dev-print-pair-url', '--dev-no-windows')

    # ----- two test Kins ----------------------------------------------------------------------------------------------------------------------
    Say 'two test Kins, each with its own data directory and port'
    Set-KinDev -Exe $Exe -DataDir $d1
    $p1 = Start-DevKin (@('--data-dir', ('"{0}"' -f $d1), '--dev-port', "$port1", '--dev-received-dir', ('"{0}"' -f (Join-Path $d1 'received'))) + $common)
    Check 'test Kin 1 started (it did not hand its launch to another Kin and exit)' (Wait-Until { (Get-KinStatus $d1).state -eq 'not_paired' } 30)
    Set-KinDev -Exe $Exe -DataDir $d2
    $p2 = Start-DevKin (@('--data-dir', ('"{0}"' -f $d2), '--dev-port', "$port2", '--dev-received-dir', ('"{0}"' -f (Join-Path $d2 'received'))) + $common)
    Check 'test Kin 2 started beside it (its own single-instance lock)' (Wait-Until { (Get-KinStatus $d2).state -eq 'not_paired' } 30)
    Start-Sleep -Milliseconds 500
    $s1 = Get-KinStatus $d1; $s2 = Get-KinStatus $d2
    Check 'both run, as two processes of their own' (-not $p1.HasExited -and -not $p2.HasExited -and $p1.Id -ne $p2.Id)
    Check 'they listen on loopback, on the ports they were given' (($s1.bind -eq '127.0.0.1' -and $s1.port -eq $port1) -and ($s2.bind -eq '127.0.0.1' -and $s2.port -eq $port2)) "ports $($s1.port), $($s2.port)"
    Check 'neither took the installed Kin''s port 47821' (($s1.port -ne 47821) -and ($s2.port -ne 47821))
    $log1 = Get-KinLogText $d1; $log2 = Get-KinLogText $d2
    Check 'the log says: an instance beside the installed Kin, with an identifier of its own' (($log1 -match 'instance dev-[0-9a-f]{12} \(beside the installed Kin\)') -and ($log1 -match 'single-instance name app\.kinrelay\.desktop\.dev-[0-9a-f]{12}'))
    $k1 = [regex]::Match($log1, 'instance (dev-[0-9a-f]{12})').Groups[1].Value; $k2 = [regex]::Match($log2, 'instance (dev-[0-9a-f]{12})').Groups[1].Value
    Check 'the two have different keys' ($k1 -and $k2 -and $k1 -ne $k2) "$k1 / $k2"
    Check 'neither advertises on the network (mDNS)' (($log1 -match 'mDNS: not advertising') -and ($log2 -match 'mDNS: not advertising'))
    Check 'the Run key and SendTo are not touched (a developer run: stand-in file)' (($log1 -match 'start with Windows: not a normal release run') -and ($log1 -match 'integration: not a normal release run'))
    Check 'the hotkey of the installed Kin is not registered by a test Kin' (($log1 -match 'hotkey: .* is the installed Kin''s, not registered') -and ($s1.hotkey.registered -eq $false))

    # ----- commands go to the instance of their data directory -----------------------------------------------------------------------------
    Say 'a command with --data-dir reaches that instance only'
    Set-KinDev -Exe $Exe -DataDir $d1
    $file = Join-Path $d1 'hello.txt'; [IO.File]::WriteAllText($file, "hello`n")
    $c1 = Count-Matches (Get-KinLogText $d1) 'command: send files'; $c2 = Count-Matches (Get-KinLogText $d2) 'command: send files'
    $tCmd = [DateTime]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'")
    Invoke-Kin --send $file
    Check '--send reached test Kin 1' (Wait-Until { (Count-Matches (Get-KinLogText $d1) 'command: send files') -eq ($c1 + 1) } 10)
    Start-Sleep -Milliseconds 500
    Check '--send did not reach test Kin 2' ((Count-Matches (Get-KinLogText $d2) 'command: send files') -eq $c2)
    $newInstalled = @(Read-InstalledLogSince $tCmd | Where-Object { $_ -match 'command:' })
    Check 'the installed Kin''s log has no command line in it' ($newInstalled.Count -eq 0)

    # ----- the clipboard ------------------------------------------------------------------------------------------------------------------
    if (-not $SkipClipboard) {
        Say 'text on the real clipboard from a test Kin is marked, so an installed Kin skips it'
        $tClip = [DateTime]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'")
        foreach ($how in 'dev-set-clipboard', 'Set-QuietClipboardText') {
            $text = "kin-devtest-$how-$stamp"
            if ($how -eq 'dev-set-clipboard') { Invoke-Kin --dev-set-clipboard $text } else { Set-QuietClipboardText $text }
            Start-Sleep -Milliseconds 1500
            $names = @(Get-ClipboardFormats | ForEach-Object { $_.Name })
            Check "($how) the clipboard carries ExcludeClipboardContentFromMonitorProcessing (an installed Kin skips it)" ($names -contains 'ExcludeClipboardContentFromMonitorProcessing') ($names -join ',')
            Check "($how) ... and KinRelayDevTestContent (a test Kin reads it as ordinary text)" ($names -contains 'KinRelayDevTestContent')
            $reason = [string](Get-KinStatus $d1).clipboardSync.lastSkip.reason
            Check "($how) the test Kin did not treat it as a password-manager copy" ($reason -notlike 'password_manager:*') "last skip: $reason"
        }
        Start-Sleep -Milliseconds 1000
        $seen = @(Read-InstalledLogSince $tClip)
        $skipped = @($seen | Where-Object { $_ -match 'clipboard changed: not sent, the source app marked it ExcludeClipboardContentFromMonitorProcessing' })
        $others = @($seen | Where-Object { ($_ -notmatch 'clipboard changed: not sent, the source app marked it') -and ($_ -notmatch 'kin_lib::ws: (websocket (opened|ended)|phone (connected|disconnected))') })
        Write-Host ("    the installed Kin's log since then: {0} 'not sent, the source app marked it' line(s), {1} other line(s)" -f $skipped.Count, $others.Count)
        Check 'the installed Kin, if its sync is on, skipped the marked content (and none of it was sent)' (($skipped.Count -ge 1) -or ($before.Processes.Count -eq 0) -or ($seen.Count -eq 0)) "$($skipped.Count) skip line(s)"
        Check 'the installed Kin''s log has nothing else new (no send, no toast, no command)' ($others.Count -eq 0) (($others | ForEach-Object { ($_ -replace '^\S+ ', '').Substring(0, [math]::Min(60, ($_ -replace '^\S+ ', '').Length)) }) -join ' | ')
        if ($backup) { $ok = Restore-Clipboard $backup; $backup = $null; Check 'the clipboard is back as it was' ([bool]$ok) }
    }

    # ----- --quit goes to its own instance ------------------------------------------------------------------------------------------------------
    Say '--quit reaches the instance of its data directory and no other'
    Set-KinDev -Exe $Exe -DataDir $d2
    Stop-DevKin 10
    Check 'test Kin 2 quit' (Wait-Until { $p2.HasExited } 10)
    Check 'test Kin 1 is still running (the quit was not for it)' (-not $p1.HasExited)
    $nowInstalled = Get-InstalledSnapshot
    Check 'the installed Kin is still running (the quit was not for it)' ($nowInstalled.Processes.Count -eq $before.Processes.Count)
    Set-KinDev -Exe $Exe -DataDir $d1
    Stop-DevKin 10
    Check 'test Kin 1 quit' (Wait-Until { $p1.HasExited } 10)
    Check 'no process of ours is left' (@(Get-DevKinProcesses).Count -eq 0)

    # ----- the installed Kin, after ------------------------------------------------------------------------------------------------------------
    Say 'the installed Kin is unchanged'
    $after = Get-InstalledSnapshot
    Check 'same processes (ID, start time, command line)' ((ConvertTo-Json @($before.Processes) -Compress) -eq (ConvertTo-Json @($after.Processes) -Compress))
    Check 'it still listens on 47821, as the same process' ($after.ListeningOn47821 -eq $before.ListeningOn47821)
    Check 'its Run value is as it was' ($after.RunValue -eq $before.RunValue)
    Check 'its SendTo shortcut is as it was' ($after.SendTo -eq $before.SendTo)
}
catch {
    Check 'the check ran to the end' $false $_.Exception.Message
}
finally {
    foreach ($d in $d1, $d2) { if (Test-Path -LiteralPath $d) { Set-KinDev -Exe $Exe -DataDir $d; Stop-DevKin 8 } }
    if ($backup) { try { [void](Restore-Clipboard $backup) } catch { Write-Host "clipboard restore failed: $_" } }
    Start-Sleep -Milliseconds 300
    foreach ($d in $d1, $d2) { if ([IO.Directory]::Exists($d)) { try { [IO.Directory]::Delete($d, $true) } catch { Write-Host "could not remove $d" } } }
}
$failed = @($results | Where-Object { -not $_.Ok })
Write-Host ("`n{0} checks, {1} failed" -f $results.Count, $failed.Count)
if ($failed.Count) { $failed | ForEach-Object { Write-Host "  FAILED: $($_.Name)" }; exit 1 }
exit 0
