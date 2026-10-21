# Real-app check of milestone 1.2 on this PC: the release Kin.exe, a throwaway profile, the fake phone, and the real clipboard and hotkey.
#
#   powershell -ExecutionPolicy Bypass -File desktop\check-1-2.ps1 [-SkipHotkeyPress]
#
# What it does, in order: pairs the fake phone (example `fake_phone`) with a Kin that listens on 127.0.0.1 only, then
#   1. Set-Clipboard on the PC             -> the phone gets a `clipboard` message, acked; arrival time in status.json
#   2. a password-manager copy             -> not sent (status.json says why)
#   3. kin.exe --send-clipboard            -> the phone gets a `text` message
#   4. the global hotkey                   -> NOT pressed any more (see below); it checks that this Kin does not register the installed Kin's hotkey
#   5. the phone sends `text`              -> the PC clipboard is set, with \r\n line ends, and nothing is sent back
#   6. the phone sends a link              -> status.json says it is openable (the toast is off, so nothing opens)
#   7. a phone that never acks             -> after 5 s: no_reply, and the PC shows the phone as offline
#
# YOUR CLIPBOARD: every format of it is saved first (clipboard-backup.ps1) and put back at the end (also when something fails). The text this script
# puts on it is quiet (an installed Kin skips it), and the restore is too.
# **The hotkey is not pressed.** The installed Kin owns Ctrl+Alt+Shift+K, and a (synthetic) key press would make *that* Kin send the clipboard to the
# user's real phone, and these scripts never inject input anyway. A Kin that runs beside the installed one (its own --data-dir) does not register
# that key; step 3 (`Kin.exe --send-clipboard`) runs the same code as the hotkey. -SkipHotkeyPress is kept so old command lines work and does nothing.
# It runs beside the user's installed Kin like the other scripts (kin-dev.ps1): own data directory, every command with --data-dir, clipboard text quiet.
# Nothing is left running: Kin and the fake phone are stopped, and the profile in %TEMP% is deleted.
param(
    [string]$Kin = (Join-Path $PSScriptRoot 'src-tauri\target\release\Kin.exe'),
    [switch]$SkipHotkeyPress
)
# Only one of the Kin scripts (e2e, smoke, capture, check) may run at a time: they share the real clipboard, the Kin.exe slot and the emulator (tools\kin-lock.ps1).
. (Join-Path $PSScriptRoot '..\tools\kin-lock.ps1'); Enter-KinLock 'check-1-2'
. (Join-Path $PSScriptRoot '..\tools\kin-dev.ps1')   # a Kin of our own beside the installed one; every command carries --data-dir (kin-dev.ps1)
. (Join-Path $PSScriptRoot 'clipboard-backup.ps1')   # Set-QuietClipboardText
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_env.ps1')
Add-Type -AssemblyName System.Windows.Forms

$failures = 0
function Check([string]$what, [bool]$ok, [string]$detail = '') {
    if ($ok) { Write-Host "  PASS  $what" -ForegroundColor Green }
    else { Write-Host "  FAIL  $what  $detail" -ForegroundColor Red; $script:failures++ }
}
function WaitUntil([scriptblock]$cond, [double]$seconds = 8) {
    $end = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $end) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 50 }
    return [bool](& $cond)
}
function Status { Get-Content (Join-Path $data 'status.json') -Raw -Encoding UTF8 | ConvertFrom-Json }
function KinLog { Get-Content (Join-Path $data 'logs\kin.log') -Raw -Encoding UTF8 }
function PhoneLog { if (Test-Path $phoneOut) { Get-Content $phoneOut -Raw -Encoding UTF8 } else { '' } }
# This file has no non-ASCII characters in it (Windows PowerShell reads a file without a BOM as ANSI), so they are built from code points.
$uniWord = 'n' + [char]0xFC + [char]0xEF + 'code ' + [char]0x2713

# ----- the clipboard: save every format ------------------------------------------------------------------------------
$savedClipboard = Save-Clipboard | Select-Object -Last 1

$data = Join-Path ([IO.Path]::GetFullPath($env:TEMP)) "kin-check-1-2-$PID"
$phoneOut = Join-Path $data 'fake-phone.out.txt'
$phone2Out = Join-Path $data 'fake-phone2.out.txt'
$procs = @()
try {
    Remove-Item $data -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory $data | Out-Null
    Push-Location (Join-Path $PSScriptRoot 'src-tauri')
    $ErrorActionPreference = 'Continue'   # cargo writes its progress to stderr, which would count as an error otherwise
    cargo build --example fake_phone 2>&1 | Out-Null
    $built = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($built) { throw 'cargo build --example fake_phone failed' }
    Pop-Location
    $fake = Join-Path $PSScriptRoot 'src-tauri\target\debug\examples\fake_phone.exe'
    if (-not (Test-Path $Kin)) { throw "No $Kin. Build it first: desktop\build.ps1" }
    Set-KinDev -Exe $Kin -DataDir $data

    Write-Host "Starting Kin (data dir $data, loopback only)"
    $kinProc = Start-DevKin @('--data-dir', ('"{0}"' -f $data), '--dev-bind', '127.0.0.1', '--dev-print-pair-url', '--dev-no-windows')
    $procs += $kinProc
    if (-not (WaitUntil { (Test-Path (Join-Path $data 'status.json')) } 15)) { throw 'Kin did not start' }
    $s = Status
    Write-Host ("  hotkey: {0} registered={1} {2}" -f $s.hotkey.accelerator, $s.hotkey.registered, $s.hotkey.error)
    Check 'the settings are the defaults (sync on, Ctrl+Alt+Shift+K)' ($s.settings.autoClipboardSync -eq $true -and $s.settings.hotkey -eq 'Ctrl+Alt+Shift+K')

    # ----- 0. five wrong codes shut the pairing code down (carry-forward item (a) of the 1.1 review) --------------------------
    Write-Host '0. five wrong pairing codes'
    Invoke-Kin --dev-start-pairing
    if (-not (WaitUntil { Test-Path (Join-Path $data 'pair-url.txt') } 10)) { throw 'no pairing URL' }
    Check 'a pairing is active in status.json' (WaitUntil { (Status).pairing.active -eq $true } 5)
    $goodUrl = (Get-Content (Join-Path $data 'pair-url.txt')).Trim()
    $badUrl = $goodUrl -replace 'tk=[A-Za-z0-9_-]{22}', 'tk=ZZZZZZZZZZZZZZZZZZZZZZ'
    $ErrorActionPreference = 'Continue'   # the fake phone panics when pairing fails, which is what is wanted here
    foreach ($i in 1..5) { & $fake $badUrl --host 127.0.0.1 --seconds 1 2>&1 | Out-Null }
    $ErrorActionPreference = 'Stop'
    Check 'pairing.active is gone from status.json after the fifth wrong code' (WaitUntil { $null -eq (Status).pairing } 5)
    Check 'the log says the code no longer works' (WaitUntil { (KinLog) -match 'too many wrong codes' } 5)

    # ----- pair the fake phone ------------------------------------------------------------------------------------
    Invoke-Kin --dev-start-pairing
    Check 'a new code is active again' (WaitUntil { (Status).pairing.active -eq $true } 5)
    Start-Sleep -Milliseconds 300
    $url = (Get-Content (Join-Path $data 'pair-url.txt')).Trim()
    Check 'it is a different code' ($url -ne $goodUrl)
    $linkText = 'https://example.com/a?b=c&d=e'
    $phone = Start-Process $fake -ArgumentList ('"{0}"' -f $url), '--host', '127.0.0.1', '--name', '"Fake Phone"', '--seconds', '90',
        '--send-text', ('"{0}"' -f "from the phone`nsecond line"), '--send-text-after', '20', '--send-text', ('"{0}"' -f $linkText) `
        -PassThru -RedirectStandardOutput $phoneOut -RedirectStandardError (Join-Path $data 'fake-phone.err.txt')
    $procs += $phone
    Check 'the phone pairs and connects' (WaitUntil { (Status).state -eq 'phone_connected' } 15)
    Check 'pairing.active left status.json once the code was used' (WaitUntil { $null -eq (Status).pairing } 5)

    # ----- 1. automatic sync --------------------------------------------------------------------------------------
    Write-Host '1. Set-Clipboard on the PC -> clipboard message'
    Set-QuietClipboardText ('copied on the PC: ' + $uniWord)
    Check 'the phone received a clipboard message' (WaitUntil { (PhoneLog) -match 'received clipboard id=.* text=copied on the PC' } 5)
    Check 'status.json has the ack and the arrival time' (WaitUntil { (Status).lastSent.clipboard.result -eq 'acked' } 5)
    $sent = (Status).lastSent.clipboard
    Write-Host ("    tap to ack {0} ms, round trip {1} ms (target: under 1000 ms)" -f $sent.tapToAckMs, $sent.rttMs)
    Check 'tap to ack is under 1 s' ($sent.tapToAckMs -lt 1000) "$($sent.tapToAckMs)"

    # ----- 2. password manager ------------------------------------------------------------------------------------
    Write-Host '2. a password-manager copy -> not sent'
    $before = [regex]::Matches((PhoneLog), 'received clipboard').Count
    Invoke-Kin --dev-set-clipboard 'hunter2-secret' --dev-clipboard-mark exclude
    Check 'it is skipped, with the reason' (WaitUntil { (Status).clipboardSync.lastSkip.reason -like 'password_manager:*' } 5)
    Start-Sleep -Milliseconds 800
    Check 'the phone got nothing' (([regex]::Matches((PhoneLog), 'received clipboard').Count) -eq $before)
    Check 'the secret is not in the log' (-not ((KinLog) -match 'hunter2'))

    # ----- 3. kin.exe --send-clipboard ----------------------------------------------------------------------------
    Write-Host '3. kin.exe --send-clipboard -> text message'
    Set-QuietClipboardText 'sent with --send-clipboard'
    Start-Sleep -Milliseconds 700   # let the automatic sync finish first, so the two messages are not mixed up
    Invoke-Kin --send-clipboard
    Check 'the phone received a text message' (WaitUntil { (PhoneLog) -match 'received text id=.* isUrl=false sensitive=false text=sent with --send-clipboard' } 5)
    Check 'status.json has the explicit send' (WaitUntil { (Status).lastSent.text.result -eq 'acked' } 5)
    Write-Host ("    tap to ack {0} ms, round trip {1} ms" -f (Status).lastSent.text.tapToAckMs, (Status).lastSent.text.rttMs)

    # ----- 4. the real hotkey -------------------------------------------------------------------------------------
    Write-Host '4. the hotkey: not pressed (the installed Kin owns Ctrl+Alt+Shift+K; --send-clipboard in step 3 runs the same code)'
    Check 'a Kin beside the installed one does not register the installed Kin''s hotkey' (((Status).hotkey.registered -eq $false) -and (-not (Status).hotkey.error)) "registered=$((Status).hotkey.registered) error=$((Status).hotkey.error)"

    # ----- 5 and 6. the phone sends text --------------------------------------------------------------------------
    Write-Host '5. the phone sends text -> PC clipboard (no echo)'
    $clipboardMessages = [regex]::Matches((PhoneLog), 'received clipboard').Count
    Check 'the phone sent its text and it was acked' (WaitUntil { (PhoneLog) -match 'ack for text fake-text-1' } 40)
    Check 'the PC clipboard has the text, with Windows line ends' (WaitUntil { (Get-Clipboard -Raw) -eq "from the phone`r`nsecond line" } 5) "got: $([string](Get-Clipboard -Raw))"
    Invoke-Kin --dev-get-clipboard
    Check '--dev-get-clipboard writes the same text' (WaitUntil { (Test-Path (Join-Path $data 'dev-clipboard.txt')) -and ((Get-Content (Join-Path $data 'dev-clipboard.txt') -Raw -Encoding UTF8) -eq "from the phone`r`nsecond line") } 5)
    Start-Sleep -Milliseconds 1000
    Check 'nothing was sent back to the phone' (([regex]::Matches((PhoneLog), 'received clipboard').Count) -eq $clipboardMessages)
    Check 'status.json says it was the echo' ((Status).clipboardSync.lastSkip.reason -eq 'echo_of_phone_text')
    Write-Host '6. the phone sends a link'
    Check 'the link text was acked' (WaitUntil { (PhoneLog) -match 'ack for text fake-text-2' } 8)
    $r = (Status).lastReceived.text
    Check 'the link is marked openable' ($r.isUrl -eq $true -and $r.openable -eq $true -and $r.clipboardSet -eq $true)
    Check 'the link text is on the clipboard' (WaitUntil { (Get-Clipboard -Raw) -eq $linkText } 5)
    $log = KinLog
    Check 'the log never holds the clipboard text' (-not ($log -match ([regex]::Escape($uniWord) + '|second line|example\.com|sent with the hotkey|sent with --send-clipboard')))
    Check 'the log has the arrival times' ($log -match 'acked, tap to ack [\d.]+ ms \(round trip [\d.]+ ms\)')

    # ----- 7. a phone that never acks -----------------------------------------------------------------------------
    Write-Host '7. a phone that never acks'
    Stop-Process -Id $phone.Id -Force -ErrorAction SilentlyContinue
    Check 'the first phone is gone' (WaitUntil { (Status).state -eq 'phone_offline' } 10)
    Invoke-Kin --dev-start-pairing
    Start-Sleep -Milliseconds 300
    $url2 = (Get-Content (Join-Path $data 'pair-url.txt')).Trim()
    $phone2 = Start-Process $fake -ArgumentList ('"{0}"' -f $url2), '--host', '127.0.0.1', '--name', '"Silent Phone"', '--seconds', '40', '--no-ack' `
        -PassThru -RedirectStandardOutput $phone2Out -RedirectStandardError (Join-Path $data 'fake-phone2.err.txt')
    $procs += $phone2
    Check 'the second phone connects' (WaitUntil { (Status).state -eq 'phone_connected' } 15)
    Set-QuietClipboardText 'nobody acks this'
    Check 'it got the message' (WaitUntil { (Get-Content $phone2Out -Raw -Encoding UTF8) -match 'received clipboard' } 5)
    Check 'after 5 s the send is no_reply and the phone is offline' (WaitUntil { (Status).lastSent.clipboard.result -eq 'no_reply' -and (Status).state -eq 'phone_offline' } 9)
    Invoke-Kin --send-clipboard
    Check 'a send while the phone is offline fails with a visible notice (in the log, the toast is off)' (WaitUntil { (Status).lastSent.text.result -eq 'phone_offline' } 5)
    $log = KinLog
    Check 'the log has the toast line' ($log -match "toast: The phone isn't connected")

    Write-Host ''
    Write-Host 'status.json (the 1.2 part):'
    (Status) | Select-Object settings, hotkey, lastSent, lastReceived, clipboardSync | ConvertTo-Json -Depth 6
}
finally {
    if ((Get-Location).Path -like '*src-tauri') { Pop-Location }
    Stop-DevKin 8   # --quit to our data directory; a hung process of ours is stopped by its ID
    foreach ($p in $procs) { if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } }
    # Put the clipboard back.
    # Every format back (marked so that an installed Kin does not send the user's own text to the phone again: clipboard-backup.ps1).
    if ($savedClipboard) { $restored = Restore-Clipboard $savedClipboard; Write-Host ('the clipboard is back as it was (every format): {0}' -f [bool]$restored) }
    Remove-DevDataDir $data 5   # the data directory and the clipboard sink beside it
}
if ($failures) { Write-Host "`n$failures check(s) FAILED" -ForegroundColor Red; exit 1 } else { Write-Host "`nAll checks passed" -ForegroundColor Green }
