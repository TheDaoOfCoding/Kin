# The real-Windows check of milestone 2B.3 (media controls in both directions), run by the desktop coder and by the reviewer. ASCII only.
#
#   .\tools\check-2b3-desktop.ps1            (needs a debug build: cd desktop\src-tauri; cargo build; cargo build --examples)
#
# SILENT and SAFE by construction. It creates its OWN media session (examples\media_test_session.exe serve: a hidden window, SystemMediaTransportControls, status
# Paused, never Playing, no audio, a random nonce as the title), runs a dev Kin beside the installed one with --dev-media-aumid naming ONLY that session, and a fake
# phone. The dev Kin is a test run: it reads and controls no other session, its volume is a fake, and its own flyout session never shows Playing.
#   1. PC -> phone: the dev Kin reads the test session and the fake phone gets media.status, media.image and media.state (title = the nonce, state paused, the
#      controls of the test session as actions)
#   2. phone -> PC commands: the fake phone sends toggle, next and seek for that sid: they arrive at the TEST session (its own log of ButtonPressed and
#      PlaybackPositionChangeRequested); a volume level, a delta and a mute change the FAKE volume and come back as media.volume. The real master volume is read
#      (read only) before and after: unchanged.
#   3. phone -> PC: the fake phone's session shows in the Windows media flyout under the dev Kin's own AUMID: read back through GSMTC (media_test_session read
#      <that AUMID>, which refuses any other AUMID): title, artist, art, timeline, controls, and status Paused (a test run never shows Playing)
#   4. flyout -> phone: media_test_session control <that AUMID> toggle|next|seek sends a command to the dev Kin's OWN session only; the phone gets media.cmd
#   5. nothing of what plays is in the Kin log or in any file of the data directory; the session is gone from the flyout when the phone goes away
# It never touches the installed Kin (its process, its data, its AUMID): it checks its PID before and after. It never sends a command to a session it did not create,
# never changes the system volume or mute, and refuses to run while a game or an anti-cheat is running. It prints PASS/FAIL lines.
param([switch]$KeepEvidence)
$ErrorActionPreference = 'Stop'
$Repo = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'kin-dev.ps1')
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $cargoBin })) { $env:Path = "$cargoBin;$env:Path" }

$Target = Join-Path $Repo 'desktop\src-tauri\target\debug'
$KinExe = Join-Path $Target 'Kin.exe'
$Fake = Join-Path $Target 'examples\fake_phone.exe'
$Session = Join-Path $Target 'examples\media_test_session.exe'
foreach ($f in $KinExe, $Fake, $Session) { if (-not (Test-Path $f)) { throw "$f is missing: cd desktop\src-tauri; cargo build; cargo build --examples" } }
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
function Get-Sid([string]$Identity) {
    $sha = [Security.Cryptography.SHA256]::Create()
    $hash = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Identity))
    return (($hash[0..15] | ForEach-Object { $_.ToString('x2') }) -join '')
}
function Send-Session([string]$Line) {
    # a command to the test session's stdin; waits for its OK or ERROR (EVENT lines of presses may come in between)
    $script:Sess.StandardInput.WriteLine($Line)
    for ($i = 0; $i -lt 50; $i++) {
        $answer = $script:Sess.StandardOutput.ReadLine()
        if ($null -eq $answer) { return 'EOF' }
        if ($answer -notlike 'EVENT *') { return $answer }
    }
    return 'NOANSWER'
}

$installedBefore = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
$Data = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-2b3-real-' + $PID)
New-Item -ItemType Directory -Force $Data | Out-Null
Set-KinDev -Exe $KinExe -DataDir $Data
$script:Sess = $null; $kin = $null; $fakeProc = $null; $KinAumid = $null; $TestAumid = $null
$events = Join-Path $Data 'session-events.txt'
try {
    $volBefore = (& $Session volume-read) -join ' '
    Write-Host "INFO the real master volume, read only: $volBefore"
    # --- the test session: its own, silent
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $Session
    $psi.Arguments = "serve --log `"$events`""
    $psi.UseShellExecute = $false; $psi.CreateNoWindow = $true; $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true
    $script:Sess = [Diagnostics.Process]::Start($psi)
    $ready = $script:Sess.StandardOutput.ReadLine()
    Check 'the test session is up (READY with its own AUMID and a nonce title)' ($ready -match '^READY aumid=(app\.kinrelay\.dev\.mediatest-\d+) title=(mediatest-[0-9a-f]{12})$') $ready
    $TestAumid = $Matches[1]; $Nonce = $Matches[2]
    $PcSid = Get-Sid $TestAumid

    '{ "mediaShare": true, "mediaShow": true, "hotkey": "", "autoClipboardSync": false }' | Set-Content (Join-Path $Data 'settings.json') -Encoding ASCII
    $kin = Start-DevKin @('--data-dir', ('"{0}"' -f $Data),'--dev-bind', '127.0.0.1', '--dev-print-pair-url', '--dev-toast-quiet', '--dev-no-open', '--dev-no-windows', '--dev-media-aumid', $TestAumid)
    $log = Join-Path $Data 'logs\kin.log'
    $up = Wait-Until { (Test-Path $log) -and ((Get-Content $log -Raw -ErrorAction SilentlyContinue) -match "sharing the PC's media with the phone") } 30 'the media reader to start'
    Check 'the dev Kin started sharing the PC media, as a TEST run' ($up -and ((Get-Content $log -Raw) -match 'sharing the PC''s media with the phone \(test run\)'))
    if ((Get-Content $log -Raw) -match '(app\.kinrelay\.desktop\.dev-[0-9a-f]+)') { $KinAumid = $Matches[1] }
    Check 'the dev Kin''s own AUMID is a dev one, never the installed Kin''s' ($KinAumid -like 'app.kinrelay.desktop.dev-*')

    Invoke-Kin --dev-start-pairing | Out-Null
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 'the pairing URL' | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    $state = '{\"sid\":\"@sid\",\"app\":\"ZQPlayer\",\"state\":\"playing\",\"title\":\"ZQPhoneSong\",\"artist\":\"ZQPhoneArtist\",\"album\":\"ZQAlbum\",\"durationMs\":337000,\"positionMs\":42000,\"actions\":[\"play\",\"pause\",\"next\",\"prev\",\"seek\"],\"artHash\":\"@art\",\"volume\":{\"level\":35,\"muted\":false}}'
    $c1 = '{\"sid\":\"' + $PcSid + '\",\"cmd\":\"toggle\"}'
    $c2 = '{\"sid\":\"' + $PcSid + '\",\"cmd\":\"next\"}'
    $c3 = '{\"sid\":\"' + $PcSid + '\",\"cmd\":\"seek\",\"seekMs\":90000}'
    $v1 = '{\"cmd\":\"volume\",\"level\":30}'
    $v2 = '{\"cmd\":\"volume\",\"delta\":5}'
    $v3 = '{\"cmd\":\"volume\",\"mute\":true}'
    $fakeOut = Join-Path $Data 'fake-out.txt'
    $fakeArgs = "`"$url`" --host 127.0.0.1 --seconds 75 --media --media-status true,true,granted --media-image --media-state `"$state`" --media-cmd `"$c1`" --media-cmd `"$c2`" --media-cmd `"$c3`" --media-cmd `"$v1`" --media-cmd `"$v2`" --media-cmd `"$v3`""
    $fakeProc = Start-Process $Fake -ArgumentList $fakeArgs -PassThru -RedirectStandardOutput $fakeOut -RedirectStandardError (Join-Path $Data 'fake-err.txt') -WindowStyle Hidden

    # --- 1. PC -> phone
    Wait-Until { (Get-Content $fakeOut -Raw -ErrorAction SilentlyContinue) -match 'received media.volume level=36 muted=true' } 40 'the phone to send everything and the PC to answer' | Out-Null
    $fakeText = Get-Content $fakeOut -Raw
    Check 'the PC told the phone its switches (media.status)' ($fakeText -match 'received media.status show=true share=true')
    Check 'the phone got the TEST session (the nonce, paused, the controls of the test session) and nothing of any other session' ($fakeText -match "received media.state sid=$PcSid state=paused app=\S+ title=$Nonce artist=Kin test artist" -and ([regex]::Matches($fakeText, 'received media.state')).Count -ge 1)
    Check 'the cover of the test session came as a media.image before its state' ($fakeText -match 'received media.image kind=art hash=[0-9a-f]{32}' -and $fakeText.IndexOf('received media.image') -lt $fakeText.IndexOf('received media.state'))
    # --- 2. phone -> PC commands
    Wait-Until { (Test-Path $events) -and ((Get-Content $events -Raw -ErrorAction SilentlyContinue) -match 'seek 90000') } 15 'the commands to reach the test session' | Out-Null
    $ev = @(Get-Content $events -ErrorAction SilentlyContinue)
    Check 'the phone''s toggle, next and seek reached ONLY the test session (ButtonPressed Play, Next, a seek to 90000 ms)' (($ev -contains 'button play') -and ($ev -contains 'button next') -and ($ev -contains 'seek 90000')) ($ev -join ' | ')
    Check 'the volume commands came back as the FAKE volume (30, then 36 = three 2 % steps up, then muted)' ($fakeText -match 'received media.volume level=30 muted=false' -and $fakeText -match 'received media.volume level=36 muted=false' -and $fakeText -match 'received media.volume level=36 muted=true')
    $volAfter = (& $Session volume-read) -join ' '
    Check 'the REAL master volume is unchanged (read only before and after)' ($volBefore -eq $volAfter) "$volBefore -> $volAfter"

    # --- 3. phone -> PC: the flyout session of the dev Kin, read back through GSMTC (this tool refuses any AUMID that is not app.kinrelay.*)
    $read = (& $Session read $KinAumid) -join ' '
    Check 'the phone''s session is in the Windows flyout under the dev Kin''s AUMID (title, artist, a thumbnail, the timeline)' ($read -match 'title=ZQPhoneSong artist=ZQPhoneArtist' -and $read -match 'durationMs=337000' -and $read -match 'thumbnailBytes=[1-9]') $read
    Check 'its status is Paused (5): a test run never shows Playing' ($read -match 'status=5 ') $read
    Check 'its buttons are the ones the phone listed (play, pause, next, prev, position)' ($read -match 'controls=\S*play' -and $read -match 'controls=\S*next' -and $read -match 'controls=\S*prev' -and $read -match 'controls=\S*position') $read
    # --- 4. flyout -> phone
    $r1 = (& $Session control $KinAumid toggle) -join ' '
    $r2 = (& $Session control $KinAumid next) -join ' '
    $r3 = (& $Session control $KinAumid seek 120000) -join ' '
    Check 'the commands reached the dev Kin''s own session' (($r1 -eq 'RESULT true') -and ($r2 -eq 'RESULT true') -and ($r3 -eq 'RESULT true')) "$r1 / $r2 / $r3"
    $got = Wait-Until { (Get-Content $fakeOut -Raw) -match 'received media.cmd id=\S+ sid=\S+ cmd=seek seekMs=120000' } 10 'the flyout commands'
    $fakeText = Get-Content $fakeOut -Raw
    Check 'the phone got them as media.cmd (play, next, seek 120000) for the session it showed' ($got -and $fakeText -match 'cmd=play seekMs=null' -and $fakeText -match 'cmd=next' -and $fakeText -match 'sid=\S+ cmd=seek seekMs=120000')
    # (the tool says why on stderr: PowerShell 5.1 turns that into an error record, so this one call runs with errors not stopping the script)
    $savedPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    # Refusals only: these AUMIDs are refused BEFORE any session is looked at (the second is the installed Kin's own: reading it would show the user's real phone).
    $wrong = & $Session read 'MSEdge' 2>&1 | Out-String
    $wrongCode = $LASTEXITCODE
    $wrongInstalled = & $Session read 'app.kinrelay.desktop' 2>&1 | Out-String
    $wrongInstalledCode = $LASTEXITCODE
    $ErrorActionPreference = $savedPreference
    Check 'the tool refuses an AUMID that is not a test session or a dev Kin (no other session can be read or controlled with it)' ($wrongCode -ne 0 -and $wrong -match 'refused') "$wrongCode $wrong"
    Check 'the tool refuses the installed Kin''s own AUMID (app.kinrelay.desktop): it would show the user''s real phone' ($wrongInstalledCode -ne 0 -and $wrongInstalled -match 'refused' -and $wrongInstalled -cnotmatch 'SESSION') "$wrongInstalledCode $wrongInstalled"

    # --- the status and the privacy of the run
    Start-Sleep -Seconds 1
    $status = Get-Content (Join-Path $Data 'status.json') -Raw | ConvertFrom-Json
    Check 'status.json says it is a test run, reading, with numbers and words only' ($status.media.view.mode -eq 'test' -and $status.media.view.reading -eq $true -and $status.media.counts.sent.states -ge 1 -and $status.media.counts.received.states -ge 1)
    $logText = Get-Content $log -Raw
    $leaks = @($Nonce, 'ZQPhoneSong', 'ZQPhoneArtist', 'ZQAlbum', 'ZQPlayer', $TestAumid, 'Kin test artist') | Where-Object { $logText.Contains($_) }
    Check 'no title, artist, album, app name or AUMID of what plays is in the Kin log' ($leaks.Count -eq 0) ($leaks -join ',')
    $filesWithText = @(Get-ChildItem $Data -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch '\\(fake-|session-events)' } | Where-Object { $c = Read-Shared $_.FullName; $c.Contains('ZQPhoneSong') -or $c.Contains($Nonce) -or $c.Contains('ZQPhoneArtist') })
    Check 'no title or artist is in any file of the data directory' ($filesWithText.Count -eq 0) (($filesWithText | ForEach-Object { $_.Name }) -join ',')

    # --- the phone goes away: the session leaves the flyout
    Stop-OwnProcess $fakeProc $Fake
    Start-Sleep -Seconds 3
    $after = (& $Session read $KinAumid) -join ' '
    Write-Host "INFO after the phone left, the dev Kin's session reads: $after"
    Check 'the phone''s session is gone from the flyout when the phone is' ($after -eq 'NOSESSION' -or $after -notmatch 'title=ZQPhoneSong') $after

    # --- the real-Windows test of Kin's flyout session (tests\media_surface_real.rs: #[ignore] in `cargo test`): its own hidden window and AUMID, Paused, no audio,
    # read back by its own AUMID only. Debug build only (never --release: that would replace Kin.exe).
    # The dev Kin is stopped first: cargo test relinks target\debug\Kin.exe, and Windows refuses to replace a running program file (os error 5).
    Stop-DevKin
    Push-Location (Join-Path $Repo 'desktop\src-tauri')
    try {
        $savedPreference = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        $surfaceTest = & cargo test --test media_surface_real -- --ignored 2>&1 | Out-String
        $surfaceCode = $LASTEXITCODE
        $ErrorActionPreference = $savedPreference
    } finally { Pop-Location }
    Check 'the flyout session test (media_surface_real, ignored by default) passes: shown, read back by its own AUMID, gone on lock, back on unlock' ($surfaceCode -eq 0 -and $surfaceTest -match 'test result: ok\. 1 passed') $surfaceTest
}
catch {
    Write-Host ('FAIL the run stopped with an error: ' + $_)
    $script:Fails++
}
finally {
    if ($fakeProc) { Stop-OwnProcess $fakeProc $Fake }
    Stop-DevKin
    if ($script:Sess) {
        try { $script:Sess.StandardInput.WriteLine('exit') } catch { }
        if (-not $script:Sess.WaitForExit(5000)) { Stop-OwnProcess $script:Sess $Session }
    }
}
$installedAfter = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
Check 'the installed Kin was not started, stopped or restarted (same PIDs)' (($installedBefore -join ',') -eq ($installedAfter -join ',')) "$($installedBefore -join ',') -> $($installedAfter -join ',')"
$leftProcs = @(Get-DevKinProcesses) + @(Get-Process -Name fake_phone, media_test_session -ErrorAction SilentlyContinue | Where-Object { ($fakeProc -and $_.Id -eq $fakeProc.Id) -or ($script:Sess -and $_.Id -eq $script:Sess.Id) })
Check 'no process of this run is left' ($leftProcs.Count -eq 0)
$volEnd = (& $Session volume-read) -join ' '
Check 'the real master volume is still what it was at the start' ($volBefore -eq $volEnd) "$volBefore -> $volEnd"
if (-not $KeepEvidence) { Remove-DevDataDir $Data } else { Write-Host "evidence kept in $Data" }
if ($script:Fails -gt 0) { Write-Host "$script:Fails check(s) FAILED"; exit 1 }
Write-Host 'all checks passed'
