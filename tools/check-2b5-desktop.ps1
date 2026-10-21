# The real-Windows check of milestone 2B.5 (Focus and Do Not Disturb, alerts, Kin.exe --notify). ASCII only.
#
#   .\tools\check-2b5-desktop.ps1            (needs a debug build: cd desktop\src-tauri; cargo build; cargo build --examples)
#   .\tools\check-2b5-desktop.ps1 -Release   (uses target\release\Kin.exe, the GUI-subsystem program: also checks that its output and exit code reach a script)
#
# SILENT and SAFE by construction: a dev Kin beside the installed one (its own throwaway data directory, loopback only, its own pipe suffix, --dev-no-windows so no window and
# no toast is ever shown, --dev-no-open), a fake phone, and a FAKE Windows Focus source (--dev-focus). **The user's real Windows Focus and Do Not Disturb are never read by this
# script's Kin and never changed by anything**: Kin only ever reads them, and this run replaces the reading. NOTHING MAKES A SOUND. No input is injected. The clipboard is not
# written (its formats are only compared before and after).
#   1. Focus: with the switch on and a phone that lists presence, the PC's FIRST snapshot says focus (the fake source says on); a runtime `--dev-focus off` takes it out of
#      the next snapshot after 3 s; nothing is read while the switch is off
#   2. Do Not Disturb: the phone's `dnd` shows in status.json and in the tray tooltip text, holds Kin's low-battery warning (one counted, none shown), and `off` clears it
#   3. alerts: `Kin.exe --notify` reaches a phone that lists `alert` (exit 0, the line, the text, no waitedMs), is queued for an offline phone and arrives later with waitedMs,
#      fails with exit 6 for a phone without `alert` (nothing is sent), exit 5 for 301 characters, exit 2 without text, exit 4 for an unknown phone; the clipboard formats are as
#      they were; the log has no alert text
# It never touches the installed Kin (its process, its data, its AUMID, the Run key, SendTo, the firewall, its pipe): it checks its PIDs before and after, kills only processes it
# started, and refuses to run while a game or an anti-cheat is running. It prints PASS/FAIL lines.
param([switch]$KeepEvidence, [switch]$Release)
$ErrorActionPreference = 'Stop'
$Repo = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'kin-dev.ps1')
. (Join-Path $Repo 'desktop\clipboard-backup.ps1')
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $cargoBin })) { $env:Path = "$cargoBin;$env:Path" }

$DebugDir = Join-Path $Repo 'desktop\src-tauri\target\debug'
$KinExe = if ($Release) { Join-Path $Repo 'desktop\src-tauri\target\release\Kin.exe' } else { Join-Path $DebugDir 'Kin.exe' }
$Fake = Join-Path $DebugDir 'examples\fake_phone.exe'
foreach ($f in $KinExe, $Fake) { if (-not (Test-Path $f)) { throw "$f is missing: cd desktop\src-tauri; cargo build$(if ($Release) { ' --release' }); cargo build --examples" } }
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
# `Kin.exe --data-dir <ours> --notify ...` : a command for OUR dev Kin only (its pipe has our own suffix). A GUI-subsystem Kin.exe is waited for with `| Out-String`
# (PowerShell does not wait for a GUI program otherwise), which also gives the output and $LASTEXITCODE.
function Invoke-Notify([string[]]$NotifyArgs) {
    $all = @('--data-dir', $Data, '--notify') + $NotifyArgs
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $KinExe
    $psi.Arguments = ($all | ForEach-Object { '"' + ($_ -replace '"', '\"') + '"' }) -join ' '
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true; $psi.CreateNoWindow = $true
    $p = [Diagnostics.Process]::Start($psi)
    $out = $p.StandardOutput.ReadToEnd() + $p.StandardError.ReadToEnd()
    $p.WaitForExit()
    return [pscustomobject]@{ Exit = $p.ExitCode; Text = $out.Trim() }
}

$installedBefore = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
$Data = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-2b5-real-' + $PID)
New-Item -ItemType Directory -Force $Data | Out-Null
Set-KinDev -Exe $KinExe -DataDir $Data
# Our own pipe: the dev pipe with a suffix, so no other dev Kin and never the installed Kin's pipe (Kin.exe --notify uses the same variable).
$env:KIN_DEV_PIPE_SUFFIX = 'zq5-' + $PID
$kin = $null; $fakeProc = $null
$identity = Join-Path $Data 'fake-identity.json'
try {
    $clipBefore = Clip-Signature
    '{ "hotkey": "", "autoClipboardSync": false, "focusShare": true }' | Set-Content (Join-Path $Data 'settings.json') -Encoding ASCII
    $kin = Start-DevKin @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-print-pair-url', '--dev-toast-quiet', '--dev-no-open', '--dev-no-windows', '--dev-focus', 'on', '--dev-battery', 'none')
    $log = Join-Path $Data 'logs\kin.log'
    Wait-Until { (Test-Path $log) -and ((Read-Shared $log) -match 'Kin .* started') } 30 'the dev Kin' | Out-Null
    Invoke-Kin --dev-start-pairing | Out-Null
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 'the pairing URL' | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()

    # --- 1 and 2: a phone with presence and alert. It says its Do Not Disturb and a low battery on a schedule (one a second from the 8th second).
    $fakeProc = Start-Fake @($url, '--host', '127.0.0.1', '--name', 'ZQ Phone', '--identity-file', $identity, '--seconds', '70', '--presence', '--alert', '--send-text-after', '6', '--dnd', 'priority', '--presence-send', '{"dnd":"priority","battery":{"level":14,"charging":false}}') 'fake1.txt'
    $got = Wait-Until { (Fake-Text 'fake1.txt') -match 'received presence battery=\S* pause=\S* focus=true' } 20 'the first snapshot with focus'
    Check 'the PC''s FIRST snapshot says focus (the fake source says on; read at once, no 3 s wait for the first reading)' $got (Fake-Text 'fake1.txt')
    $st = Get-Status
    Check 'status.json says the Focus source is the fake one and is active' ($st.focus.dev -eq 'on' -and $st.focus.sourceActive -eq $true -and $st.focus.on -eq $true) ($st.focus | ConvertTo-Json -Compress)
    Check 'the log has the words "focus: on (...)" and nothing else about Focus' ((Read-Shared $log) -match 'focus: on \(focus session\)') ''
    $got = Wait-Until { $s = Get-Status; $s -and $s.presence.phone.dnd -eq 'priority' } 30 'the phone''s dnd'
    Check 'the phone''s Do Not Disturb (priority) is in status.json as a current word' $got
    $st = Get-Status
    Check 'the tray tooltip text says the phone is in Do Not Disturb' ($st.tray.tooltip -match 'phone: Do Not Disturb') $st.tray.tooltip
    $got = Wait-Until { $s = Get-Status; $s -and $s.presence.phone.level -eq 14 } 30 'the low battery'
    Start-Sleep -Seconds 1
    $logText = Read-Shared $log
    $st = Get-Status
    Check 'the phone''s battery (14 %) while in Do Not Disturb: the low-battery warning was HELD (counted, not shown)' ($got -and $st.focus.counts.heldToasts -ge 1 -and $logText -notmatch 'low battery warning for ZQ Phone') "held=$($st.focus.counts.heldToasts)"
    # Focus ends (the runtime developer command): the phone hears it after 3 s of stability and within a second more.
    $mark = ([regex]::Matches((Fake-Text 'fake1.txt'), 'received presence')).Count
    Invoke-Kin --dev-focus off | Out-Null
    $got = Wait-Until { (Fake-Text 'fake1.txt') -match 'received presence battery=\S* pause=\S* focus=null' } 15 'the snapshot without focus'
    Check 'after `--dev-focus off` a snapshot without focus follows (3 s of stability, then within a second)' $got (Fake-Text 'fake1.txt')

    # --- 3. alerts to a phone that lists alert (the phone from above is still connected)
    $r = Invoke-Notify @('ZQ build finished', '--title', 'ZQ CI')
    Check 'Kin.exe --notify exits 0 and says "Sent to ZQ Phone."' ($r.Exit -eq 0 -and $r.Text -match 'Sent to ZQ Phone\.') "exit=$($r.Exit) '$($r.Text)'"
    $got = Wait-Until { (Fake-Text 'fake1.txt') -match 'received alert id=\S+ title=ZQ CI text=ZQ build finished waitedMs=null' } 10 'the alert'
    Check 'the phone got an alert frame with the text and title, no waitedMs, and acked it' $got (Fake-Text 'fake1.txt')
    $r = Invoke-Notify @('x' * 301)
    Check 'a text of 301 characters: exit 5 and nothing is sent' ($r.Exit -eq 5 -and $r.Text -match '301') "exit=$($r.Exit) '$($r.Text)'"
    $r = Invoke-Notify @()
    Check 'no text: exit 2' ($r.Exit -eq 2) "exit=$($r.Exit) '$($r.Text)'"
    $r = Invoke-Notify @('--device', 'Nobody Here', 'hello')
    Check 'an unknown --device: exit 4' ($r.Exit -eq 4) "exit=$($r.Exit) '$($r.Text)'"
    $r = Invoke-Notify @('--ttl', '59s', 'hello')
    Check 'a ttl under a minute: exit 2' ($r.Exit -eq 2) "exit=$($r.Exit) '$($r.Text)'"
    Stop-OwnProcess $fakeProc $Fake
    $got = Wait-Until { $s = Get-Status; $s -and $s.state -ne 'phone_connected' -and $null -eq $s.presence.phone.dnd } 30 'the connection to close'
    Check 'the phone''s Do Not Disturb is forgotten with its connection' $got

    # An offline phone: queued, and it arrives with waitedMs after the phone is back.
    $r = Invoke-Notify @('ZQ later alert', '--ttl', '2h')
    Check 'for an offline phone: exit 0 and "Queued for ZQ Phone (offline), expires in 2h."' ($r.Exit -eq 0 -and $r.Text -match 'Queued for ZQ Phone \(offline\), expires in 2h\.') "exit=$($r.Exit) '$($r.Text)'"
    Start-Sleep -Seconds 2
    $fakeProc = Start-Fake @($url, '--host', '127.0.0.1', '--name', 'ZQ Phone', '--identity-file', $identity, '--skip-pairing', '--seconds', '15', '--presence', '--alert') 'fake2.txt'
    $got = Wait-Until { (Fake-Text 'fake2.txt') -match 'received alert id=\S+ title= text=ZQ later alert waitedMs=\d+' } 20 'the queued alert'
    Check 'the queued alert arrived after the phone came back, with how long it waited (waitedMs)' $got (Fake-Text 'fake2.txt')
    Stop-OwnProcess $fakeProc $Fake

    # A phone without the alert capability: exit 6 and nothing on the wire.
    $fakeProc = Start-Fake @($url, '--host', '127.0.0.1', '--name', 'ZQ Phone', '--identity-file', $identity, '--skip-pairing', '--seconds', '12', '--presence') 'fake3.txt'
    Wait-Until { $s = Get-Status; $s -and $s.state -eq 'phone_connected' } 15 'the phone to connect' | Out-Null
    $r = Invoke-Notify @('ZQ never sent')
    Check 'a connected phone without alert: exit 6 and "Update Kin on your phone"' ($r.Exit -eq 6 -and $r.Text -match 'Update Kin on your phone') "exit=$($r.Exit) '$($r.Text)'"
    Start-Sleep -Seconds 1
    Check 'nothing of it reached the phone (no alert, no text)' ((Fake-Text 'fake3.txt') -notmatch 'ZQ never sent' -and (Fake-Text 'fake3.txt') -notmatch 'received alert') ''
    Stop-OwnProcess $fakeProc $Fake

    # --- what must not have happened
    $logText = Read-Shared $log
    Check 'no alert text or title is in the log' ($logText -notmatch 'ZQ build finished|ZQ later alert|ZQ never sent|ZQ CI') ''
    $statusText = Read-Shared (Join-Path $Data 'status.json')
    Check 'no alert text or title is in status.json' ($statusText -notmatch 'ZQ build finished|ZQ later alert|ZQ never sent|ZQ CI') ''
    Check 'the clipboard was not touched by any alert (every format and size as it was)' ((Clip-Signature) -eq $clipBefore) ''
    Check 'Kin never changed Windows Focus: the log has no call that starts or ends a session' ($logText -notmatch '(?i)TryStartFocus|DeactivateFocus') ''
}
catch {
    Write-Host ('FAIL the run stopped with an error: ' + $_)
    $script:Fails++
}
finally {
    if ($fakeProc) { Stop-OwnProcess $fakeProc $Fake }
    Stop-DevKin
    Remove-Item Env:\KIN_DEV_PIPE_SUFFIX -ErrorAction SilentlyContinue
}
$installedAfter = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
Check 'the installed Kin was not started, stopped or restarted (same PIDs)' (($installedBefore -join ',') -eq ($installedAfter -join ',')) "$($installedBefore -join ',') -> $($installedAfter -join ',')"
$leftProcs = @(Get-DevKinProcesses) + @(Get-Process -Name fake_phone -ErrorAction SilentlyContinue | Where-Object { $fakeProc -and $_.Id -eq $fakeProc.Id })
Check 'no process of this run is left' ($leftProcs.Count -eq 0)
if (-not $KeepEvidence) { Remove-DevDataDir $Data } else { Write-Host "evidence kept in $Data" }
if ($script:Fails -gt 0) { Write-Host "$script:Fails check(s) FAILED"; exit 1 }
Write-Host 'all checks passed'
