# The real-Windows check of milestone 2B.4 (presence, pause, ring, auto-sent screenshots, unlinking). ASCII only.
#
#   .\tools\check-2b4-desktop.ps1            (needs a debug build: cd desktop\src-tauri; cargo build; cargo build --examples)
#
# SILENT and SAFE by construction: a dev Kin beside the installed one (its own throwaway data directory, loopback only, --dev-no-windows so no window and no toast is ever
# shown, --dev-no-open so no browser), a fake phone, and a FAKE battery (--dev-battery). NOTHING MAKES A SOUND: a ring is a message that the fake phone prints.
#   1. presence: the PC's snapshot after the hello carries the fake battery (9 %, unplugged); the phone's battery shows in status.json and in the tray tooltip text;
#      a low phone battery gives exactly ONE low-battery warning, and no battery level of the phone is in the log
#   2. ring: `Kin.exe --ring` reaches the phone as ring.start (acked), the phone's ring.status makes the PC say ringing, `--ring-stop` sends ring.stop
#   3. pause: `Kin.exe --pause 1h` is told to the phone within a few seconds, shows in status.json and in the tooltip, is kept in settings.json; `--pause resume` clears it
#   4. the clipboard (2B.6: the dev Kin runs with --dev-clipboard-sink, so this is the sink's files and the user's real clipboard is never touched): a camera photo never goes to it; while PAUSED a manual text, a link and a screenshot arrive
#      quietly (nothing on the clipboard, no browser, counted in status.json); after a resume a screenshot goes on it WITH the three sensitive formats (read back)
#   5. unlink: `--dev-set unlinkDevice <id>` makes the phone find out, empties the device list and clears the pause
# It never touches the installed Kin (its process, its data, its AUMID, the Run key, SendTo, the firewall): it checks its PIDs before and after, kills only processes
# it started, and refuses to run while a game or an anti-cheat is running. It prints PASS/FAIL lines.
param([switch]$KeepEvidence)
$ErrorActionPreference = 'Stop'
$Repo = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'kin-dev.ps1')
. (Join-Path $Repo 'desktop\clipboard-backup.ps1')
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $cargoBin })) { $env:Path = "$cargoBin;$env:Path" }

$Target = Join-Path $Repo 'desktop\src-tauri\target\debug'
$KinExe = Join-Path $Target 'Kin.exe'
$Fake = Join-Path $Target 'examples\fake_phone.exe'
foreach ($f in $KinExe, $Fake) { if (-not (Test-Path $f)) { throw "$f is missing: cd desktop\src-tauri; cargo build; cargo build --examples" } }
Assert-NoGameRunning

$script:Fails = 0
function Check([string]$Name, [bool]$Ok, [string]$Detail = '') {
    if ($Ok) { Write-Host "PASS $Name" } else { Write-Host "FAIL $Name $Detail"; $script:Fails++ }
}
function Wait-Until([scriptblock]$Cond, [int]$Seconds, [string]$What) {
    $end = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $end) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds 300 }
    Write-Host "(timed out waiting for $What)"; return $false
}
function Read-Shared([string]$Path) {
    try {
        $fs = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
        try { $ms = New-Object IO.MemoryStream; $fs.CopyTo($ms); return [Text.Encoding]::GetEncoding(28591).GetString($ms.ToArray()) } finally { $fs.Dispose() }
    } catch { return '' }
}
function Get-Status { try { return (Read-Shared (Join-Path $Data 'status.json')) | ConvertFrom-Json } catch { return $null } }
function Start-Fake([string[]]$ArgList, [string]$OutName) {
    $out = Join-Path $Data $OutName
    $args2 = ($ArgList | ForEach-Object { if ($_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ } }) -join ' '
    return Start-Process $Fake -ArgumentList $args2 -PassThru -RedirectStandardOutput $out -RedirectStandardError (Join-Path $Data ($OutName + '.err')) -WindowStyle Hidden
}
function Fake-Text([string]$OutName) { return (Read-Shared (Join-Path $Data $OutName)) }
function Clip-Signature { return ((Get-ClipboardFormats | ForEach-Object { "$($_.Name):$($_.Bytes)" }) -join ',') }

$installedBefore = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
$Data = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-2b4-real-' + $PID)
New-Item -ItemType Directory -Force $Data | Out-Null
Set-KinDev -Exe $KinExe -DataDir $Data
$kin = $null; $fakeProc = $null; $backup = $null
$identity = Join-Path $Data 'fake-identity.json'
$png = Join-Path $Data 'Screenshot_check.png'
Copy-Item (Join-Path $Repo 'protocol\fixtures\media-art-16.png') $png
try {
    $backup = Save-Clipboard
    Write-Host 'INFO the real clipboard is not used (the dev Kin runs with --dev-clipboard-sink: its clipboard is files, read through Get-ClipboardFormats)'
    '{ "hotkey": "", "autoClipboardSync": false }' | Set-Content (Join-Path $Data 'settings.json') -Encoding ASCII
    $kin = Start-DevKin @('--data-dir', ('"{0}"' -f $Data),'--dev-bind', '127.0.0.1', '--dev-print-pair-url', '--dev-toast-quiet', '--dev-no-open', '--dev-no-windows', '--dev-battery', '9:unplugged')
    $log = Join-Path $Data 'logs\kin.log'
    Wait-Until { (Test-Path $log) -and ((Read-Shared $log) -match 'Kin .* started') } 30 'the dev Kin' | Out-Null
    Invoke-Kin --dev-start-pairing | Out-Null
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 'the pairing URL' | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()

    # --- 1 to 3: one phone that has presence and ring, and says things on a schedule (one a second from the 8th second)
    $low = '{"battery":{"level":14,"charging":false}}'
    $lower = '{"battery":{"level":13,"charging":false}}'
    $fakeProc = Start-Fake @($url, '--host', '127.0.0.1', '--name', 'ZQ Phone', '--identity-file', $identity, '--seconds', '60', '--presence', '--ring', '--send-text-after', '8', '--presence-send', $low, '--ring-status', 'ringing', '--presence-send', $lower, '--ring-status', 'stopped,timeout') 'fake1.txt'
    $got = Wait-Until { (Fake-Text 'fake1.txt') -match 'received presence battery=' } 20 'the PC''s snapshot'
    $t = Fake-Text 'fake1.txt'
    Check 'the PC sent its snapshot after the hello, with the FAKE battery (9 %, unplugged) and no pause' ($got -and $t -match 'received presence battery=\{"charging":false,"level":9\} pause=null') $t
    Invoke-Kin --ring | Out-Null
    $got = Wait-Until { (Fake-Text 'fake1.txt') -match 'received ring.start id=\S+' } 10 'the ring.start'
    Check 'Kin.exe --ring reached the phone as a ring.start (the fake phone acks it and makes no sound)' $got (Fake-Text 'fake1.txt')
    $st = Get-Status
    Check 'status.json says a ring was requested' ($st -and $st.ring.counts.started -ge 1)
    $got = Wait-Until { $s = Get-Status; $s -and $s.presence.phone.level -eq 14 } 30 'the phone''s battery'
    Check 'the phone''s battery (14 %) is in status.json (numbers are fine there)' $got
    $st = Get-Status
    Check 'the tray tooltip text has it' ($st.tray.tooltip -match '14%') $st.tray.tooltip
    $got = Wait-Until { $s = Get-Status; $s -and $s.ring.state -eq 'ringing' } 15 'ring.status ringing'
    Check 'the phone''s ring.status ringing made the PC say ringing' $got
    Invoke-Kin --ring-stop | Out-Null
    $got = Wait-Until { (Fake-Text 'fake1.txt') -match 'received ring.stop id=\S+' } 10 'the ring.stop'
    Check 'Kin.exe --ring-stop reached the phone as a ring.stop' $got
    $got = Wait-Until { $s = Get-Status; $s -and $s.presence.phone.level -eq 13 } 15 'the second battery'
    Check 'the second snapshot (13 %) replaced the first' $got
    Start-Sleep -Seconds 1
    $logText = Read-Shared $log
    $warnings = ([regex]::Matches($logText, 'low battery warning for ZQ Phone')).Count
    Check 'exactly ONE low-battery warning for the whole episode (14 % then 13 %)' ($warnings -eq 1) "count=$warnings"
    Check 'no battery level of the phone is in the log (not 14, not 13, not as % or as level)' ($logText -notmatch '(?i)\b1[34]\s?%' -and $logText -notmatch 'level["= :]+1[34]\b') ''

    # --- pause (the phone is still connected: the fake phone runs for 60 s)
    Invoke-Kin --pause 1h | Out-Null
    $got = Wait-Until { (Fake-Text 'fake1.txt') -match 'received presence battery=\S* pause=\{"remainingMs":\d+\}' } 8 'the pause to be told'
    Check 'the pause was told to the phone within a few seconds, as the time left' $got (Fake-Text 'fake1.txt')
    $st = Get-Status
    Check 'status.json and the tooltip say paused' ($st.pause.on -eq $true -and $st.tray.tooltip -match 'paused') "$($st.pause.on) / $($st.tray.tooltip)"
    $settings = (Read-Shared (Join-Path $Data 'settings.json')) | ConvertFrom-Json
    Check 'the pause is kept in settings.json (so it survives a restart)' ($settings.pause.until -gt 0)
    Invoke-Kin --pause resume | Out-Null
    $got = Wait-Until { ([regex]::Matches((Fake-Text 'fake1.txt'), 'pause=null')).Count -ge 2 } 8 'the resume to be told'
    Check 'a resume was told to the phone (a snapshot without a pause)' $got
    $st = Get-Status
    Check 'status.json says not paused again' ($st.pause.on -eq $false)
    Stop-OwnProcess $fakeProc $Fake

    # --- 4. the clipboard (the sink's: Get-ClipboardFormats reads what Kin wrote)
    $before = Clip-Signature
    $fakeProc = Start-Fake @($url, '--host', '127.0.0.1', '--name', 'ZQ Phone', '--identity-file', $identity, '--skip-pairing', '--seconds', '14', '--presence', '--upload', $png, '--upload-origin', 'camera', '--upload-after', '2') 'fake2.txt'
    $got = Wait-Until { (Fake-Text 'fake2.txt') -match 'uploaded Screenshot_check.png' } 25 'the camera upload'
    Check 'a camera photo arrived and was saved' $got (Fake-Text 'fake2.txt')
    Start-Sleep -Seconds 1
    Check 'a camera photo NEVER goes to the clipboard (every format and size is as it was)' ((Clip-Signature) -eq $before)
    Stop-OwnProcess $fakeProc $Fake

    Invoke-Kin --pause resumed | Out-Null
    $textArgs = @($url, '--host', '127.0.0.1', '--name', 'ZQ Phone', '--identity-file', $identity, '--skip-pairing', '--seconds', '16', '--presence', '--send-text-after', '2', '--send-text', 'ZQ quiet words', '--send-link', 'https://example.com/zq-quiet', '--upload', $png, '--upload-origin', 'screenshot', '--upload-after', '4')
    $fakeProc = Start-Fake $textArgs 'fake3.txt'
    $got = Wait-Until { $s = Get-Status; $s -and $s.pause.quietArrivals -ge 3 } 30 'three quiet arrivals'
    Check 'a text, a link and a screenshot arrived while paused and were counted as quiet arrivals' $got ((Get-Status).pause | ConvertTo-Json -Compress)
    $logText = Read-Shared $log
    Check 'the log says they were handled quietly and nothing was opened' (([regex]::Matches($logText, 'arrived while paused, handled quietly')).Count -ge 3 -and $logText -notmatch 'opened in the default browser')
    Check 'the clipboard is untouched by a paused PC (every format and size as it was)' ((Clip-Signature) -eq $before)
    Stop-OwnProcess $fakeProc $Fake

    Invoke-Kin --pause resume | Out-Null
    $fakeProc = Start-Fake @($url, '--host', '127.0.0.1', '--name', 'ZQ Phone', '--identity-file', $identity, '--skip-pairing', '--seconds', '14', '--presence', '--upload', $png, '--upload-origin', 'screenshot', '--upload-after', '2') 'fake4.txt'
    $got = Wait-Until { (Clip-Signature) -ne $before } 25 'the screenshot on the clipboard'
    Check 'after the resume a screenshot goes on the clipboard' $got
    $names = @(Get-ClipboardFormats | ForEach-Object { $_.Name })
    Check 'with an image format (CF_DIBV5) and the three sensitive formats: ExcludeClipboardContentFromMonitorProcessing, CanIncludeInClipboardHistory, CanUploadToCloudClipboard' (($names -contains 'CF_DIBV5') -and ($names -contains 'ExcludeClipboardContentFromMonitorProcessing') -and ($names -contains 'CanIncludeInClipboardHistory') -and ($names -contains 'CanUploadToCloudClipboard')) ($names -join ',')
    Stop-OwnProcess $fakeProc $Fake

    # --- 5. unlink
    $fakeProc = Start-Fake @($url, '--host', '127.0.0.1', '--name', 'ZQ Phone', '--identity-file', $identity, '--skip-pairing', '--seconds', '25', '--presence') 'fake5.txt'
    Wait-Until { $s = Get-Status; $s -and $s.state -eq 'phone_connected' } 15 'the phone to connect' | Out-Null
    $id = (Get-Status).devices[0].deviceId
    Invoke-Kin --pause resumed | Out-Null
    Invoke-Kin --dev-set unlinkDevice $id | Out-Null
    $got = Wait-Until { (Fake-Text 'fake5.txt') -match 'the PC unpaired this phone' } 10 'the unpair'
    Check 'the phone found out at once (an unpair message)' $got (Fake-Text 'fake5.txt')
    $got = Wait-Until { $s = Get-Status; $s -and @($s.devices).Count -eq 0 } 10 'the device list to be empty'
    Check 'the device list is empty and the pause was cleared with the last phone' ($got -and (Get-Status).pause.on -eq $false)
    $res = (Read-Shared (Join-Path $Data 'dev-set-result.json')) | ConvertFrom-Json
    Check 'the unlink said it removed the device' ($res.ok -eq $true -and $res.message -match 'removed: true') ($res | ConvertTo-Json -Compress)
    Stop-OwnProcess $fakeProc $Fake
}
catch {
    Write-Host ('FAIL the run stopped with an error: ' + $_)
    $script:Fails++
}
finally {
    if ($fakeProc) { Stop-OwnProcess $fakeProc $Fake }
    Stop-DevKin
    if ($backup) { try { Restore-Clipboard $backup; Write-Host 'the clipboard was restored' } catch { Write-Host "restoring the clipboard failed: $_" } }
}
$installedAfter = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
Check 'the installed Kin was not started, stopped or restarted (same PIDs)' (($installedBefore -join ',') -eq ($installedAfter -join ',')) "$($installedBefore -join ',') -> $($installedAfter -join ',')"
$leftProcs = @(Get-DevKinProcesses) + @(Get-Process -Name fake_phone -ErrorAction SilentlyContinue | Where-Object { $fakeProc -and $_.Id -eq $fakeProc.Id })
Check 'no process of this run is left' ($leftProcs.Count -eq 0)
if (-not $KeepEvidence) { Remove-DevDataDir $Data } else { Write-Host "evidence kept in $Data" }
if ($script:Fails -gt 0) { Write-Host "$script:Fails check(s) FAILED"; exit 1 }
Write-Host 'all checks passed'
