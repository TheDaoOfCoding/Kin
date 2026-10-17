# The real-Windows check of milestone 2B.2 (phone notifications on the PC), run by the desktop coder and by the reviewer. ASCII only.
#
#   .\tools\check-2b2-desktop.ps1            (needs a debug build: cd desktop\src-tauri; cargo build; cargo build --examples)
#
# What it does, with a real dev Kin beside the installed one (its own throwaway data directory, loopback only, --dev-toast-quiet: every toast goes to the
# Action Center with no popup and no sound), the fake phone, and the toast_probe tool:
#   1. the switch is on in settings.json: the dev Kin registers ITS OWN names (app.kinrelay.desktop.dev-<key>.phone and a CLSID made from the key) under HKCU
#   2. the fake phone posts notifications: Windows holds them (toast_probe history): tag, group, an input box and buttons, the private form, an update
#      that replaces a toast, a removal
#   3. a click is simulated the way the shell does it, with no input injection: COM creates the class and calls Activate with the button's arguments and the
#      reply box's text (toast_probe activate): the reply, a button and Copy reach Kin and the phone
#   4. Copy puts the code on the REAL clipboard as sensitive and it is gone after 60 s (the clipboard is saved first and restored at the end, every format)
#   5. a refused click, a dismissal (the same function the X calls), no notification text in the Kin log
#   6. quitting removes the toasts and EVERY registry key of the dev identity, including the two that Windows makes by itself (checked again a while later)
# It never touches the installed Kin (its process, its data, its AUMID app.kinrelay.desktop, its keys): it checks its PID before and after. It refuses to run
# while a game or an anti-cheat is running. It prints PASS/FAIL lines and never a notification text.
param([switch]$KeepEvidence, [int]$LaterCheckSeconds = 100)
$ErrorActionPreference = 'Stop'
$Repo = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'kin-dev.ps1')
. (Join-Path $Repo 'desktop\clipboard-backup.ps1')
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $cargoBin })) { $env:Path = "$cargoBin;$env:Path" }

$Target = Join-Path $Repo 'desktop\src-tauri\target\debug'
$KinExe = Join-Path $Target 'Kin.exe'
$Fake = Join-Path $Target 'examples\fake_phone.exe'
$Probe = Join-Path $Target 'examples\toast_probe.exe'
foreach ($f in $KinExe, $Fake, $Probe) { if (-not (Test-Path $f)) { throw "$f is missing: cd desktop\src-tauri; cargo build; cargo build --examples" } }
Assert-NoGameRunning

$script:Fails = 0
function Check([string]$Name, [bool]$Ok, [string]$Detail = '') {
    if ($Ok) { Write-Host "PASS $Name" } else { Write-Host "FAIL $Name $Detail"; $script:Fails++ }
}
function Read-Shared([string]$Path) {
    # a file that the running Kin may hold open (the database, the log): read it with sharing, as bytes
    try {
        $fs = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
        try { $ms = New-Object IO.MemoryStream; $fs.CopyTo($ms); return [Text.Encoding]::GetEncoding(28591).GetString($ms.ToArray()) } finally { $fs.Dispose() }
    } catch { return '' }
}
function Wait-Until([scriptblock]$Cond, [int]$Seconds, [string]$What) {
    $end = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $end) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds 300 }
    Write-Host "(timed out waiting for $What)"; return $false
}

$installedBefore = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
$Data = Join-Path $env:TEMP ("kin-2b2-real-" + $PID)
New-Item -ItemType Directory -Force $Data | Out-Null
Set-KinDev -Exe $KinExe -DataDir $Data
$N1 = '54892d1f759b11f6762ff1bd98cda7df'; $N2 = '36460390475e793c9a9e06dd5e2d421f'; $N3 = 'a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4'; $N4 = 'b1b2c3d4b1b2c3d4b1b2c3d4b1b2c3d4'
$Aumid = $null; $Clsid = $null; $kin = $null; $fakeProc = $null; $backup = $null
try {
    # the switch is on from the start; no hotkey, no automatic clipboard sync (the clipboard is only touched by Copy)
    '{ "mirrorNotifications": true, "hotkey": "", "autoClipboardSync": false }' | Set-Content (Join-Path $Data 'settings.json') -Encoding ASCII
    $kin = Start-DevKin @('--data-dir', $Data, '--dev-bind', '127.0.0.1', '--dev-print-pair-url', '--dev-toast-quiet', '--dev-no-open', '--dev-no-windows')
    $log = Join-Path $Data 'logs\kin.log'
    $ready = Wait-Until { (Test-Path $log) -and ((Get-Content $log -Raw -ErrorAction SilentlyContinue) -match 'activator class') } 30 'phone notifications to start'
    Check 'the dev Kin started with phone notifications on' $ready
    $line = (Get-Content $log | Where-Object { $_ -match 'phone notifications are on' } | Select-Object -First 1)
    if ($line -match 'toasts under (\S+), activator class (\{[0-9A-F-]+\})') { $Aumid = $Matches[1]; $Clsid = $Matches[2] }
    Check 'the identity is the dev one, never the installed Kin''s' ($Aumid -like 'app.kinrelay.desktop.dev-*.phone' -and $Aumid -ne 'app.kinrelay.desktop.phone')
    $reg = & $Probe registry $Aumid $Clsid
    Check 'the CLSID and the AppUserModelId key exist while it runs' (($reg -match 'EXISTS .*Classes\\CLSID').Count -eq 1 -and ($reg -match 'EXISTS .*AppUserModelId').Count -eq 1)
    cmd /c 'reg query "HKCU\Software\Classes\AppUserModelId\app.kinrelay.desktop.phone" >nul 2>nul'
    Check 'the installed Kin''s phone AUMID was not written' ($LASTEXITCODE -ne 0)

    Invoke-Kin --dev-start-pairing | Out-Null
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 'the pairing URL' | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    $p1 = '{\"nid\":\"' + $N1 + '\",\"app\":\"ZQApp\",\"iconHash\":\"@icon\",\"title\":\"ZQTitle\",\"text\":\"ZQText one\",\"reply\":{\"i\":1,\"hint\":\"ZQHint\"},\"code\":\"482913\",\"actions\":[{\"i\":2,\"label\":\"ZQButton\"}]}'
    $p2 = '{\"nid\":\"' + $N2 + '\",\"app\":\"ZQBank\",\"title\":\"ZQSecretTitle\",\"text\":\"ZQSecretText\",\"sensitive\":true,\"reply\":{\"i\":0},\"actions\":[{\"i\":1,\"label\":\"ZQSecretButton\"}]}'
    $p3 = '{\"nid\":\"' + $N3 + '\",\"app\":\"ZQApp\",\"title\":\"ZQTitle3\",\"text\":\"first\",\"importance\":2}'
    $p3b = '{\"nid\":\"' + $N3 + '\",\"app\":\"ZQApp\",\"title\":\"ZQTitle3\",\"text\":\"second, replaces the first\",\"importance\":2}'
    $p4 = '{\"nid\":\"' + $N4 + '\",\"app\":\"ZQApp\",\"title\":\"ZQTitle4\",\"text\":\"to be removed\"}'
    $fakeOut = Join-Path $Data 'fake-out.txt'
    $fakeArgs = "`"$url`" --host 127.0.0.1 --seconds 150 --notif --notif-status --notif-icon --notif-post `"$p1`" --notif-post `"$p2`" --notif-post `"$p3`" --notif-post `"$p4`" --notif-remove $N4 --notif-post `"$p3b`""
    $fakeProc = Start-Process $Fake -ArgumentList $fakeArgs -PassThru -RedirectStandardOutput $fakeOut -RedirectStandardError (Join-Path $Data 'fake-err.txt') -WindowStyle Hidden

    # the phone sends one message a second: status, icon, four posts, a removal and the update of N3 (the eighth, `fake-notif.post-8`)
    Wait-Until { (Get-Content $fakeOut -Raw -ErrorAction SilentlyContinue) -match 'sent notif.post id=fake-notif.post-8' } 40 'the phone to send everything' | Out-Null
    $three = Wait-Until { (@(& $Probe history $Aumid) -match '^3 toast').Count -eq 1 } 20 'three toasts in the history'
    Start-Sleep -Seconds 1
    $hist = @(& $Probe history $Aumid)
    Check 'Windows holds exactly the three toasts (N1, N2, N3 replaced in place, N4 removed again)' $three ($hist -join ' | ')
    $t1 = @($hist | Where-Object { $_ -match "tag=$N1 " })[0]; $t2 = @($hist | Where-Object { $_ -match "tag=$N2 " })[0]; $t3 = @($hist | Where-Object { $_ -match "tag=$N3 " })[0]
    Check 'N1: tag, the phone''s group, an input box and 3 buttons (Send, Copy, the phone''s)' ($t1 -match 'group=kin-[A-Z2-7]{8} ' -and $t1 -match 'buttons=3 input=true private=False')
    Check 'N2 (sensitive): the private form, no input box, no buttons' ($t2 -match 'buttons=0 input=false private=True')
    Check 'N3 (updated): one toast' ($null -ne $t3)
    Check 'N4 (removed by the phone) is gone' (-not ($hist -match "tag=$N4 "))
    $short = ([regex]::Match($t1, 'group=kin-([A-Z2-7]{8})')).Groups[1].Value
    $fakeText = Get-Content $fakeOut -Raw
    Check 'the phone got nothing back for its posts (ephemeral, no acks)' (-not ($fakeText -match 'ack for'))

    # --- clicks, the way the shell does them: COM CoCreateInstance of the class, then Activate (no input injection)
    $args1 = "kin=1&d=$short&n=$N1&a=reply&i=1"
    $r = & $Probe activate $Clsid $Aumid $args1 'reply=ZQReply'
    Check 'a reply through the COM class reached Kin and the phone (reply text, the hint as label, the index)' (Wait-Until { (Get-Content $fakeOut -Raw) -match "received notif.action id=\S+ nid=$N1 i=1 label=ZQHint text=ZQReply" } 10 'the reply') ($r -join ' ')
    & $Probe activate $Clsid $Aumid "kin=1&d=$short&n=$N1&a=btn&i=2" | Out-Null
    Check 'a phone button through the COM class reached the phone (index 2, its own label)' (Wait-Until { (Get-Content $fakeOut -Raw) -match "received notif.action id=\S+ nid=$N1 i=2 label=ZQButton text=" } 10 'the button')
    & $Probe activate $Clsid $Aumid "kin=1&d=$short&n=$N2&a=btn&i=1" | Out-Null
    & $Probe activate $Clsid $Aumid "kin=1&d=ZZZZZZZZ&n=$N1&a=btn&i=2" | Out-Null
    & $Probe activate $Clsid $Aumid "kin=1&d=$short&n=$N1&a=btn&i=9" | Out-Null
    & $Probe activate $Clsid $Aumid "not our arguments" | Out-Null
    Start-Sleep -Seconds 1
    $fakeText = Get-Content $fakeOut -Raw
    Check 'clicks that are not ours (private toast button, unknown phone, button never offered, nonsense) sent nothing to the phone' (([regex]::Matches($fakeText, 'received notif.action')).Count -eq 2)
    # status.json is rewritten in the background after a change: look until it has caught up
    Wait-Until { try { $st = Get-Content (Join-Path $Data 'status.json') -Raw | ConvertFrom-Json; ($st.mirror.counts.actions.sent -eq 2 -and $st.mirror.counts.actions.refused -ge 4) } catch { $false } } 10 'status.json to catch up' | Out-Null
    $status = Get-Content (Join-Path $Data 'status.json') -Raw | ConvertFrom-Json
    Check 'status.json counts the refusals and the actions, with numbers only' ($status.mirror.counts.actions.refused -ge 4 -and $status.mirror.counts.actions.sent -eq 2 -and $status.mirror.counts.shown -eq 4 -and $status.mirror.counts.replaced -eq 1 -and $status.mirror.counts.removed -eq 1) ($status.mirror.counts | ConvertTo-Json -Compress)

    # --- Copy: the real clipboard, saved first and restored at the end
    $backup = Save-Clipboard
    & $Probe activate $Clsid $Aumid "kin=1&d=$short&n=$N1&a=copy" | Out-Null
    $copied = Wait-Until { try { (Get-Clipboard -Raw -ErrorAction SilentlyContinue) -eq '482913' } catch { $false } } 10 'the code on the clipboard'
    Check 'Copy put the code on the clipboard' $copied
    $formats = @(Get-ClipboardFormats | ForEach-Object { $_.Name })
    Check 'with the sensitive formats (kept out of the clipboard history and the cloud clipboard, and out of Kin''s own sync)' (($formats -contains 'ExcludeClipboardContentFromMonitorProcessing') -and ($formats -contains 'CanIncludeInClipboardHistory') -and ($formats -contains 'CanUploadToCloudClipboard')) ($formats -join ',')
    Start-Sleep -Seconds 63
    $left = try { Get-Clipboard -Raw -ErrorAction SilentlyContinue } catch { $null }
    Check 'after 60 seconds the code is off the clipboard' ($left -ne '482913')

    # --- the X: the same function the Dismissed handler calls
    Invoke-Kin --dev-toast dismiss $N1 | Out-Null
    Check 'dismissing a toast on the PC reached the phone as notif.dismiss' (Wait-Until { (Get-Content $fakeOut -Raw) -match "received notif.dismiss nid=$N1" } 10 'the dismissal')

    # --- nothing of a notification in the log
    $logText = Get-Content $log -Raw
    $leaks = @('ZQApp', 'ZQBank', 'ZQTitle', 'ZQText', 'ZQSecret', 'ZQHint', 'ZQReply', 'ZQButton', '482913', $N1, $N2, $N3, $N4) | Where-Object { $logText.Contains($_) }
    Check 'no notification text, reply, code or nid is in the Kin log' ($leaks.Count -eq 0) ($leaks -join ',')
    $filesWithText = @(Get-ChildItem $Data -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch '\\fake-' } | Where-Object { $c = Read-Shared $_.FullName; $c.Contains('ZQTitle') -or $c.Contains('ZQSecret') -or $c.Contains('ZQReply') })
    Check 'no notification text is in any file of the data directory' ($filesWithText.Count -eq 0) (($filesWithText | ForEach-Object { $_.Name }) -join ',')
    $iconFiles = @(Get-ChildItem (Join-Path $Data 'notif-icons') -File -ErrorAction SilentlyContinue)
    Check 'the icon cache holds the one icon, by its hash' ($iconFiles.Count -eq 1 -and $iconFiles[0].Name -match '^[0-9a-f]{32}\.png$')
}
catch {
    Write-Host ('FAIL the run stopped with an error: ' + $_)
    $script:Fails++
}
finally {
    if ($backup) { try { Restore-Clipboard $backup; Write-Host 'the clipboard was restored' } catch { Write-Host "restoring the clipboard failed: $_" } }
    if ($fakeProc) { Stop-OwnProcess $fakeProc $Fake }
    Stop-DevKin
}
if ($Aumid) {
    Start-Sleep -Seconds 2
    $h = @(& $Probe history $Aumid)
    Check 'quitting removed the toasts' (($h -match '^0 toast').Count -eq 1) ($h -join ' | ')
    $reg = @(& $Probe registry $Aumid $Clsid)
    Check 'quitting removed every registry key of the dev identity (CLSID, AppUserModelId, and the two Windows made)' (($reg -match 'EXISTS').Count -eq 0) ($reg -join ' | ')
    Write-Host ("waiting {0} s to look again (Windows may add its PushNotifications\Backup key a while after a toast)" -f $LaterCheckSeconds)
    Start-Sleep -Seconds $LaterCheckSeconds
    $reg2 = @(& $Probe registry $Aumid $Clsid)
    if (($reg2 -match 'EXISTS').Count -gt 0) {
        Write-Host ('NOTE a key came back: ' + ($reg2 -join ' | ') + ' - removing it')
        foreach ($k in @("Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\$Aumid", "Software\Microsoft\Windows\CurrentVersion\PushNotifications\Backup\$Aumid", "Software\Classes\AppUserModelId\$Aumid", "Software\Classes\CLSID\$Clsid")) { cmd /c "reg delete `"HKCU\$k`" /f >nul 2>nul" }
    }
    Check 'a while later no registry key of the dev identity is back' (($reg2 -match 'EXISTS').Count -eq 0) ($reg2 -join ' | ')
}
$installedAfter = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
Check 'the installed Kin was not started, stopped or restarted (same PIDs)' (($installedBefore -join ',') -eq ($installedAfter -join ',')) "$($installedBefore -join ',') -> $($installedAfter -join ',')"
$leftProcs = @(Get-DevKinProcesses) + @(Get-Process -Name fake_phone -ErrorAction SilentlyContinue | Where-Object { $fakeProc -and $_.Id -eq $fakeProc.Id })
Check 'no process of this run is left' ($leftProcs.Count -eq 0)
if (-not $KeepEvidence) { Remove-DevDataDir $Data } else { Write-Host "evidence kept in $Data" }
if ($script:Fails -gt 0) { Write-Host "$script:Fails check(s) FAILED"; exit 1 }
Write-Host 'all checks passed'
