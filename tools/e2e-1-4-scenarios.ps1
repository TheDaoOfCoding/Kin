# The milestone 1.4 scenarios (queue, expiry, the 404 rules, sensitive text offline, the automatic clipboard offline, history parity,
# notifications, logs) for tools\e2e-1-1.ps1 -Milestone 1.4. Dot-sourced by that script, after e2e-1-2-scenarios.ps1 and e2e-1-3-scenarios.ps1,
# and it uses the helpers of all three (A, Say, Check, Wait-Until, Get-Status, Phone-Cmd, Phone-Bcast, Phone-Log, Pc-Log, Phone-Clip,
# Phone-Transfers, Phone-Received, Share-Files, Make-PhoneFiles, Make-PhoneBig, Wifi, Pc-Send, Restart-Kin, $Kin, $KinArgs, $Data, $Pkg, $Repo,
# $script:Timings, $script:Texts, $script:Names). Don't run it by itself: use tools\e2e-1-4.ps1.
#
# No non-ASCII character is written in this file (Windows PowerShell reads a BOM-less script as ANSI).
#
# What it touches: the real PC clipboard (saved by Save-AllClipboard, put back by Restore-AllClipboard after Kin has quit), a work folder under
# %LOCALAPPDATA%\Temp (removed at the end), the received folder inside the throwaway data dir, and on the phone files/debug-share, cache/exports
# and Pictures/Kin, Movies/Kin, Download/Kin (cleaned by Clear-Phone13). Kin.exe is stopped, started, frozen (NtSuspendProcess) and restarted
# with --dev-clock-offset; the phone app is stopped and frozen (SIGSTOP). A frozen process is always resumed in a finally block.

$script:W14 = Join-Path $env:LOCALAPPDATA ('Temp\kin-e2e14-' + $script:Run)
$script:Seen14 = @{}      # the ids of what each scenario made, for the parity check: id -> label
$script:SecretTexts = New-Object System.Collections.Generic.List[string]   # sensitive and automatic-clipboard texts: in no history, on no side
Add-Type -Namespace Kin14 -Name Native -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("ntdll.dll")] public static extern int NtSuspendProcess(System.IntPtr h);
[System.Runtime.InteropServices.DllImport("ntdll.dll")] public static extern int NtResumeProcess(System.IntPtr h);
'@

function Remove-Work14 {
    if (Test-Path -LiteralPath $script:W14) { try { [IO.Directory]::Delete($script:W14, $true) } catch { Say "could not remove $($script:W14): $_" } }
    # a freeze that a failed run left behind
    Resume-Kin14
}

# ---------------------------------------------------------------- the PC
function Utc-Of([string]$LogLine) { if ($LogLine -match '^(\S+Z) ') { [DateTime]::Parse($Matches[1], [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::RoundtripKind) } else { $null } }
function Pc-LogLines([string]$Pattern) { @((Pc-Log) -split "`r?`n" | Where-Object { $_ -match $Pattern }) }
# What a window would show, written by Kin for scripts (--dev-dump-view). Asked for, and read once it exists.
function Pc-View([string]$Name) {
    $file = Join-Path $Data "view-$Name.json"
    [IO.File]::Delete($file)
    Invoke-Kin --dev-dump-view $Name
    if (-not (Wait-Until { Test-Path -LiteralPath $file } 10)) { return $null }
    Start-Sleep -Milliseconds 150
    Get-Content -LiteralPath $file -Raw -Encoding UTF8 | ConvertFrom-Json
}
function Pc-History { @((Pc-View 'history').items) }
function Pc-HistoryItem([string]$Id) { Pc-History | Where-Object { $_.id -eq $Id } | Select-Object -First 1 }
# What the history window's Cancel and Retry buttons do (--dev-history <cancel|retry> <id>); the outcome is in dev-history-result.json
function Pc-HistoryCmd([string]$Verb, [string]$Id) {
    $path = Join-Path $Data 'dev-history-result.json'
    [IO.File]::Delete($path)
    Invoke-Kin --dev-history $Verb $Id
    if (-not (Wait-Until { Test-Path -LiteralPath $path } 10)) { return $null }
    Start-Sleep -Milliseconds 150
    Get-Content -LiteralPath $path -Raw -Encoding UTF8 | ConvertFrom-Json
}
function Pc-QueueIds { @((Get-Status).queue.items | ForEach-Object { $_.id }) }
function Short([string]$Id) { $Id.Substring($Id.Length - 8) }   # the PC log and status.json call a transfer by the last 8 characters of its id

function Stop-Kin14 {
    Invoke-Kin --quit 2>$null
    Wait-Until { -not (Get-DevKinProcesses) } 15 | Out-Null
    Start-Sleep -Milliseconds 700
}
function Start-Kin14([string[]]$Extra = @()) {
    $port = (Get-Status).port
    Start-DevKin ($KinArgs + @('--dev-port', "$port") + $Extra) | Out-Null
    Wait-Until { (Get-Status).state -in 'phone_offline', 'phone_connected', 'not_paired' } 30 | Out-Null
}
# The phone is in its backoff after the PC went away: connect now
function Connect-Phone14 { Phone-Cmd 'CONNECT' | Out-Null }

$script:FrozenKin = $null
function Freeze-Kin14 { $p = Get-DevKinProcesses | Select-Object -First 1; if ($p) { [void][Kin14.Native]::NtSuspendProcess($p.Handle); $script:FrozenKin = $p } }
function Resume-Kin14 { if ($script:FrozenKin) { try { [void][Kin14.Native]::NtResumeProcess($script:FrozenKin.Handle) } catch { }; $script:FrozenKin = $null } }

# The phone goes away from the PC's point of view: Wi-Fi off, and the PC finds out the way it does in life (a send gets no ack), here with a ping
function Phone-Away14 {
    Wifi $false
    Start-Sleep -Milliseconds 800
    Invoke-Kin --ping
    Wait-Until { (Get-Status).state -eq 'phone_offline' } 25
}
function Phone-Back14([int]$Sec = 45) { Wifi $true; Wait-Until { (Get-Status).state -eq 'phone_connected' } $Sec }

# The explicit send of the hotkey (--send-clipboard), after the text is on the clipboard. Waits until Kin has read it (see check-1-4.ps1).
function Pc-SendText14([string]$Text, [int]$Sec = 20) {
    $t0 = (Get-Status).lastSent.text.at
    Set-QuietClipboardText $Text; Start-Sleep -Milliseconds 650
    Invoke-Kin --send-clipboard
    Wait-Until { $x = (Get-Status).lastSent.text; $x -and $x.at -ne $t0 } $Sec | Out-Null
    (Get-Status).lastSent.text
}
# --send a file and return the history item that appeared for it
function Pc-OfferFile14([string]$Path) {
    $known = @(Pc-History | ForEach-Object { $_.id })
    Invoke-Kin --send $Path
    Wait-Until { $script:nf = Pc-History | Where-Object { $_.direction -eq 'out' -and $_.kind -eq 'file' -and $_.id -notin $known } | Select-Object -First 1; [bool]$script:nf } 20 | Out-Null
    $script:nf
}

# ---------------------------------------------------------------- the phone
function Phone-Texts {
    # "CMD TEXTS count=N" then N "CMD TEXT <id> <dir> <status> attempts=.. src=.. sensitive=.. bytes=.. err=.. bodyKept=.."
    $lines = @(Phone-Bcast 'TEXTS' '' 1000)
    for ($i = 0; $i -lt 8; $i++) {
        $want = if (($lines -join "`n") -match 'CMD TEXTS count=(\d+)') { [int]$Matches[1] } else { -1 }
        if ($want -ge 0 -and @($lines | Where-Object { $_ -match 'CMD TEXT \S+ (up|down) ' }).Count -ge $want) { break }
        Start-Sleep -Milliseconds 600
        $lines = @(A logcat -d -s KinRelay)
    }
    foreach ($l in $lines) {
        if ($l -match 'CMD TEXT (\S+) (up|down) (\w+) attempts=(\d+) src=(\S+) sensitive=(\w+) bytes=(\d+) err=(\S*) bodyKept=(\w+)') {
            [pscustomobject]@{ Id = $Matches[1]; Dir = $Matches[2]; Status = $Matches[3]; Attempts = [int]$Matches[4]; Src = $Matches[5]; Sensitive = ($Matches[6] -eq 'true'); Bytes = [long]$Matches[7]; Err = $Matches[8]; BodyKept = ($Matches[9] -eq 'true') }
        }
    }
}
function Phone-TextIds { @(Phone-Texts | ForEach-Object { $_.Id }) }
# SEND_TEXT as the share sheet does it: the line "CMD SEND_TEXT result=..." comes after the connection wait (4 s) and the ack (5 s)
function Phone-SendText14([string]$Text, [bool]$Sensitive = $false, [int]$Sec = 20) {
    A logcat -c | Out-Null
    $extra = "--es text '$Text'" + $(if ($Sensitive) { ' --ez sensitive true' } else { '' })
    A shell "am broadcast -a app.kinrelay.android.debug.SEND_TEXT -n $Pkg/.debug.DebugCommandReceiver $extra" | Out-Null
    Wait-Until { $script:st14 = (A logcat -d -s KinRelay) | Where-Object { $_ -match 'CMD SEND_TEXT result=' } | Select-Object -Last 1; [bool]$script:st14 } $Sec | Out-Null
    "$script:st14"
}
function Phone-NewText([string[]]$Before, [string]$Dir = 'up', [int]$Sec = 10) {
    $script:pnx = $null
    Wait-Until { $script:pnx = @(Phone-Texts) | Where-Object { $_.Id -notin $Before -and $_.Dir -eq $Dir } | Select-Object -First 1; [bool]$script:pnx } $Sec | Out-Null
    $script:pnx
}
function Phone-Text([string]$Id) { @(Phone-Texts) | Where-Object { $_.Id -eq $Id } | Select-Object -First 1 }
function Phone-Transfer([string]$Id) { @(Phone-Transfers) | Where-Object { $_.Id -eq $Id } | Select-Object -First 1 }
# The new upload row, once it is `queued` (a share starts it at once; with no connection it waits for one for a few seconds and then goes to the queue)
function Phone-NewUpload([string[]]$Before, [int]$Sec = 40) {
    $script:pnu = $null
    Wait-Until { $script:pnu = @(Phone-Transfers) | Where-Object { $_.Id -notin $Before -and $_.Dir -eq 'up' } | Select-Object -First 1; $script:pnu -and $script:pnu.Status -ne 'active' } $Sec | Out-Null
    $script:pnu
}
function Show-PhoneLink14 { Say 'phone log (link, queue, transfer lines):'; (Phone-Log) -split "`r?`n" | Where-Object { $_ -match ' link: | queue: | transfer: ' } | Select-Object -Last 25 | ForEach-Object { Say "  $_" } }
function Phone-Pid { ((A shell pidof $Pkg | Out-String).Trim() -split '\s+')[0] }
function Phone-Freeze14 { $pid14 = Phone-Pid; if ($pid14) { A shell run-as $Pkg kill -STOP $pid14 | Out-Null }; $script:FrozenPhone = $pid14 }
function Phone-Resume14 { if ($script:FrozenPhone) { A shell run-as $Pkg kill -CONT $script:FrozenPhone | Out-Null; $script:FrozenPhone = $null } }
$script:FrozenPhone = $null
function Phone-AppRecords {
    $d = A shell dumpsys notification --noredact | Out-String
    @(($d -split 'NotificationRecord\(') | Where-Object { $_ -match 'pkg=app\.kinrelay\.android ' })
}
function Rec-Title([string]$Rec) { if ($Rec -match 'android\.title=String \(([^\r\n]*?)\)\r?\n') { $Matches[1] } else { '' } }
function Phone-Screenshot([string]$Name, [switch]$Shade) {
    if (-not $script:ShotDir14) { return }
    if ($Shade) { A shell cmd statusbar expand-notifications | Out-Null; Start-Sleep -Seconds 2 }
    New-Item -ItemType Directory -Force $script:ShotDir14 | Out-Null
    $shot = Join-Path $script:ShotDir14 $Name
    Invoke-EmuScript 'screenshot.ps1' @('-Out', ('"{0}"' -f $shot)) 60 | Out-Null
    if ($Shade) { A shell cmd statusbar collapse | Out-Null }
}
$script:ShotDir14 = $ScreenshotDir   # an ignored temp folder unless -UpdateScreens was given (e2e-1-1.ps1 decides)

# Brings the phone app to its Home tab ("Kin") whatever screen the earlier scenarios left it on: the Debug screen (its "Back" button), the
# setup screen of the first run, a dialog, another tab, or another app on top. Returns $false if the tabs could not be reached.
function Open-PhoneHomeTab {
    A shell input keyevent KEYCODE_HOME | Out-Null
    A shell am start -n "$Pkg/.MainActivity" | Out-Null
    Start-Sleep -Milliseconds 1200
    for ($i = 0; $i -lt 8; $i++) {
        A shell uiautomator dump /sdcard/kin-e2e-ui.xml 2>$null | Out-Null
        $t = Ui-Texts
        $back = "$([char]0x2039) Back"   # (not written out: this file has no byte order mark, and PowerShell 5.1 would read the character as ANSI)
        if ($t -contains $back) { Tap-UiText $back 1 | Out-Null; Start-Sleep -Milliseconds 800; continue }
        if ($t -contains 'Keep Kin connected') { A shell input keyevent KEYCODE_BACK | Out-Null; Start-Sleep -Milliseconds 800; continue }
        if (($t -contains 'Kin') -and (($t -contains 'History') -or ($t -contains 'Debug'))) {
            Tap-UiText 'Kin' 1 | Out-Null   # the first tab: explicitly Home, whichever tab was selected
            Start-Sleep -Milliseconds 700
            return $true
        }
        A shell input keyevent KEYCODE_BACK | Out-Null
        Start-Sleep -Milliseconds 800
        A shell am start -n "$Pkg/.MainActivity" | Out-Null
        Start-Sleep -Milliseconds 800
    }
    return $false
}

# The receiving side's copy of a file, by name
function Pc-RecvNames { @(Pc-Recv | ForEach-Object { $_.Name }) }
function Phone-DownloadNames { @(Phone-Received | ForEach-Object { $_.Name }) }

# ================================================================ the scenarios
# A scenario that dies (a throw, a missing item) is recorded as a failed check and the run goes on with the next one. What it may have left behind
# (a frozen process, Wi-Fi off, Kin stopped) is put right first, so the next scenario starts from the usual state.
function Scenario-Failed([string]$Name, $ErrorRecord) {
    $where = "$($ErrorRecord.InvocationInfo.ScriptName | Split-Path -Leaf):$($ErrorRecord.InvocationInfo.ScriptLineNumber)"
    Check "$Name finished without an error" $false ("$($ErrorRecord.Exception.Message) ($where)")
    try {
        Resume-Kin14; Phone-Resume14
        # Always a fresh Kin without a clock offset, never "start it only if it is not running": a scenario that died while Kin ran with
        # --dev-clock-offset (S5, S7b) would leave every later scenario on a shifted clock (expiry, the 24 hour windows, "last seen").
        Stop-Kin14
        Start-Kin14
        Wifi $true
        Connect-Phone14
        Wait-Until { (Get-Status).state -eq 'phone_connected' } 45 | Out-Null
    } catch { Say "${Name}: putting things right failed: $_" }
}
function Run-Milestone14 {
    New-Item -ItemType Directory -Force $script:W14 | Out-Null
    $n = $script:Run
    $phoneName = (Get-Status).devices[0].name
    Wifi $true
    Phone-Bcast 'DELETE_RECEIVED' '' 1200 | Out-Null

    # ---------------------------------------------------------------- S1: PC -> phone, the phone's Wi-Fi is off
    if (Want 'S1') { try {
        Say 'S1: PC -> phone with the phone offline (a text, a file, a text), then Wi-Fi on'
        $f1 = Join-Path $script:W14 'e2e14-s1-file.bin'; Make-PcFile $f1 300000 61
        $sha1 = Pc-Sha $f1
        $script:Names.Add('e2e14-s1-file.bin')
        $tx1 = Use-Text "e2e14-$n s1 first text from the PC"
        $tx2 = Use-Text "e2e14-$n s1 second text, a bit longer than the first"
        $ids0 = @(Phone-TextIds); $tids0 = @(Phone-Ids)
        $away = Phone-Away14
        Check 'S1 the PC noticed the phone is away (a ping got no ack)' $away
        $r1 = Pc-SendText14 $tx1
        Check 'S1 the first text was not delivered but queued (result phone_offline, queued true)' ($r1 -and $r1.result -eq 'phone_offline' -and $r1.queued -eq $true) "$($r1.result) queued=$($r1.queued)"
        $o = Pc-OfferFile14 $f1
        Check 'S1 the file was queued by --send (history: out, file, queued)' ($o -and $o.status -eq 'queued') "$($o.status)"
        $r2 = Pc-SendText14 $tx2
        Check 'S1 the second text was queued too' ($r2 -and $r2.queued -eq $true) "$($r2.result)"
        $st = Get-Status
        Check 'S1 status.json shows the 3 queued items in creation order (text, file, text)' ($st.queue.count -eq 3 -and (@($st.queue.items | ForEach-Object { $_.kind }) -join ',') -eq 'text,file,text' -and $st.queue.items[0].createdMs -le $st.queue.items[1].createdMs -and $st.queue.items[1].createdMs -le $st.queue.items[2].createdMs) ((@($st.queue.items | ForEach-Object { $_.kind }) -join ',') + " count=$($st.queue.count)")
        Check 'S1 the queue in status.json has no text and no file name' (-not ((ConvertTo-Json $st.queue -Depth 6) -match 's1 first|s1 second|e2e14-s1-file'))
        Check 'S1 the tray says it' (([string]$st.tray.status) -match 'offline' -and ([string]$st.tray.status) -match '3 queued') "$($st.tray.status)"
        $hist = Pc-History
        $textItems = @($hist | Where-Object { $_.kind -eq 'text' -and $_.status -eq 'queued' })
        $tid1 = ($textItems | Where-Object { $_.text -eq $tx1 }).id; $tid2 = ($textItems | Where-Object { $_.text -eq $tx2 }).id
        Check 'S1 the history shows both texts as queued' ($tid1 -and $tid2)
        foreach ($i in $tid1, $tid2, $o.id) { if ($i) { $script:Seen14[$i] = 'S1' } }
        Check 'S1 nothing reached the phone while it was away' (@(Phone-TextIds | Where-Object { $_ -in @($tid1, $tid2) }).Count -eq 0 -and (@(Phone-Ids | Where-Object { $_ -eq $o.id }).Count -eq 0))

        $helloBefore = (Pc-LogLines 'phone connected: ').Count
        $swWifi = [Diagnostics.Stopwatch]::StartNew()
        $utcWifi = [DateTime]::UtcNow
        Wifi $true
        $acks = @{}
        Wait-Until {
            $s = Get-Status
            if (-not $acks['q'] -and $s.queue.count -eq 0) { $acks['q'] = $swWifi.Elapsed.TotalSeconds }
            if (-not $acks['t2'] -and $s.lastSent.text.id -eq $tid2 -and $s.lastSent.text.result -eq 'acked') { $acks['t2'] = $swWifi.Elapsed.TotalSeconds }
            [bool]$acks['q']
        } 60 | Out-Null
        $done = Wait-Until { (Pc-History | Where-Object { $_.id -eq $o.id }).status -eq 'done' } 45
        $acks['file'] = $swWifi.Elapsed.TotalSeconds
        Check 'S1 the queue emptied and the file offer finished (done) after Wi-Fi came back' ($acks['q'] -and $done) ("queue empty {0:N2} s after svc wifi enable, file done {1:N2} s" -f $acks['q'], $acks['file'])
        $helloLine = Pc-LogLines 'phone connected: ' | Select-Object -Skip $helloBefore -First 1
        $utcHello = Utc-Of $helloLine
        $ackLine = Pc-LogLines ("queued text $(Short $tid2): sent and acked") | Select-Object -Last 1
        $offerDone = Pc-LogLines ("the phone has all the files of offer $(Short $o.id)") | Select-Object -Last 1
        $sentLine = Pc-LogLines 'queue: 3 item\(s\) sent after the reconnect' | Select-Object -Last 1
        $dHelloAck = if ($utcHello -and $ackLine) { ((Utc-Of $ackLine) - $utcHello).TotalMilliseconds } else { $null }
        $dHelloDone = if ($utcHello -and $offerDone) { ((Utc-Of $offerDone) - $utcHello).TotalMilliseconds } else { $null }
        $dHelloSent = if ($utcHello -and $sentLine) { ((Utc-Of $sentLine) - $utcHello).TotalMilliseconds } else { $null }
        $dWifiHello = if ($utcHello) { ($utcHello - $utcWifi).TotalSeconds } else { $null }
        Check 'S1 the whole queue went out within 2 s of the hello (3 items sent), the offer done within 5 s' ($null -ne $dHelloSent -and $dHelloSent -lt 2000 -and $null -ne $dHelloDone -and $dHelloDone -lt 5000) ("hello -> queue sent {0:N0} ms, hello -> second text acked {1:N0} ms, hello -> file done {2:N0} ms" -f $dHelloSent, $dHelloAck, $dHelloDone)
        Check 'S1 within seconds of Wi-Fi coming on (svc wifi enable to the whole queue delivered under 15 s)' ($acks['q'] -lt 15 -and $acks['file'] -lt 15) ("{0:N1} s (the emulator needs about 3 s of it to bring Wi-Fi up)" -f $acks['file'])
        $script:Timings['S1 svc wifi enable -> hello on the PC'] = "{0:N2} s" -f $dWifiHello
        $script:Timings['S1 svc wifi enable -> queue empty (3 items acked, file offer acked)'] = "{0:N2} s" -f $acks['q']
        $script:Timings['S1 svc wifi enable -> file offer done (the phone has the file)'] = "{0:N2} s" -f $acks['file']
        $script:Timings['S1 hello -> the 3 queued items sent / second text acked / file offer done'] = "{0:N0} ms / {1:N0} ms / {2:N0} ms" -f $dHelloSent, $dHelloAck, $dHelloDone
        # order, on the PC (what it sent) and on the phone (what it received)
        $pcLog = Pc-Log
        $ia = $pcLog.IndexOf("queued text $(Short $tid1): sent and acked"); $ib = $pcLog.IndexOf("offer $(Short $o.id) to device"); $ic = $pcLog.IndexOf("queued text $(Short $tid2): sent and acked")
        Check 'S1 the PC sent them in creation order (text, offer, text)' ($ia -ge 0 -and $ib -gt $ia -and $ic -gt $ib) "$ia $ib $ic"
        $ph = Phone-Log
        $pa = [regex]::Match($ph, "text from '[^']*' len=$($tx1.Length) ").Index; $pb = $ph.IndexOf("offer $($o.id) accepted"); $pcix = [regex]::Match($ph, "text from '[^']*' len=$($tx2.Length) ").Index
        Check 'S1 the phone received them in that order (text, offer, text)' ($pa -gt 0 -and $pb -gt $pa -and $pcix -gt $pb) "$pa $pb $pcix"
        Wait-Until { (Phone-Transfer $o.id).Status -in 'done', 'failed', 'canceled' } 30 | Out-Null
        $pt = Phone-Transfer $o.id
        Check 'S1 the phone downloaded the file (done) under the same ID' ($pt -and $pt.Status -eq 'done') "$($pt.Status) $($pt.Err)"
        $hh = Phone-DirHashes '/sdcard/Download/Kin'
        Check 'S1 SHA-256 of the file on the phone equals the PC file' ($hh['e2e14-s1-file.bin'] -eq $sha1)
        $pc2 = Wait-PhoneClip $tx2
        Check 'S1 the phone clipboard has the last text' ($pc2 -and $pc2.Text -ceq $tx2)
        $pts = @(Phone-Texts)
        Check 'S1 the phone history has both texts (down, done) under the PC ids' ((($pts | Where-Object { $_.Id -eq $tid1 }).Status -eq 'done') -and (($pts | Where-Object { $_.Id -eq $tid2 }).Status -eq 'done'))
        $pcH = Pc-History
        Check 'S1 the PC history says done for all three' ((@($pcH | Where-Object { $_.id -in @($tid1, $tid2, $o.id) -and $_.status -eq 'done' }).Count) -eq 3)
        Check 'S1 the PC clipboard sync did not send a stale clipboard on the reconnect (no clipboard message after the hello)' ((Pc-LogLines 'clipboard \S+ sent to device' | Select-Object -Skip 0 | Where-Object { (Utc-Of $_) -gt $utcHello }).Count -eq 0)
    } catch { Scenario-Failed 'S1' $_ } }

    # ---------------------------------------------------------------- S2: phone -> PC, the PC is stopped
    if (Want 'S2') { try {
        Say 'S2: phone -> PC with Kin.exe stopped (a text, then a shared file), then Kin.exe started'
        Make-PhoneFiles 'e2e14-s2-file.bin:250000'
        $src = Phone-SourceHashes
        $script:Names.Add('e2e14-s2-file.bin')
        $tx = Use-Text "e2e14-$n s2 text from the phone"
        $ids0 = @(Phone-TextIds); $tids0 = @(Phone-Ids)
        Stop-Kin14
        Check 'S2 Kin.exe is stopped' (-not (Get-DevKinProcesses))
        Check 'S2 the phone notices (not connected)' (Wait-Until { (Phone-State)['link'] -notlike 'Connected*' } 20)
        $res = Phone-SendText14 $tx
        Check 'S2 SEND_TEXT answers Queued(...) with the PC away' ($res -match 'result=Queued\(') $res
        $txRow = Phone-NewText $ids0 'up' 10
        Check 'S2 the text is in the phone queue (up, queued, 0 attempts or 1)' ($txRow -and $txRow.Status -eq 'queued') "$($txRow.Status)"
        Share-Files 'e2e14-s2-file.bin'
        $upRow = Phone-NewUpload $tids0
        Check 'S2 the shared file is queued (up, queued: it waits for the connection first, then goes to the queue)' ($upRow -and $upRow.Status -eq 'queued') "$($upRow.Status)"
        if ($txRow) { $script:Seen14[$txRow.Id] = 'S2' }; if ($upRow) { $script:Seen14[$upRow.Id] = 'S2' }
        Start-Sleep -Seconds 2
        $swK = [Diagnostics.Stopwatch]::StartNew()
        Start-Kin14
        Connect-Phone14
        Check 'S2 the phone connected to the new Kin.exe' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 60) ("{0:N1} s" -f $swK.Elapsed.TotalSeconds)
        if ($swK.Elapsed.TotalSeconds -gt 15) { Show-PhoneLink14 }
        $gotText = Wait-Until { (Get-Status).lastReceived.text.id -eq $txRow.Id } 30
        $tArrive = $swK.Elapsed.TotalSeconds
        Check 'S2 the text arrived at the PC (lastReceived.text has the phone message id)' $gotText
        $pcRow = $null
        Wait-Until { $script:pr = Pc-HistoryItem $upRow.Id; $script:pr -and $script:pr.status -in 'done', 'failed', 'canceled' } 60 | Out-Null
        $pcRow = $script:pr
        $tFile = $swK.Elapsed.TotalSeconds
        Check 'S2 the file arrived (PC history: in, done) under the phone transfer id' ($pcRow -and $pcRow.status -eq 'done') "$($pcRow.status) $($pcRow.error)"
        $recv = Join-Path $script:RecvDir 'e2e14-s2-file.bin'
        Check 'S2 SHA-256 of the file on the PC equals the phone source' ((Test-Path -LiteralPath $recv) -and (Pc-Sha $recv) -eq $src['e2e14-s2-file.bin'])
        Check 'S2 the PC clipboard has the text' ((Pc-Clip) -ceq $tx)
        $ht = Pc-HistoryItem $txRow.Id
        Check 'S2 in order: the text came first, then the file (PC history times)' ($ht -and $pcRow -and $ht.createdMs -le $pcRow.createdMs) "text $($ht.createdMs), file $($pcRow.createdMs)"
        Check 'S2 the phone says done for both' ((Phone-Text $txRow.Id).Status -eq 'done' -and (Phone-Transfer $upRow.Id).Status -eq 'done')
        $script:Timings['S2 Kin.exe started + phone CONNECT -> text at the PC / file done'] = "{0:N1} s / {1:N1} s" -f $tArrive, $tFile
    } catch { Scenario-Failed 'S2' $_ } }

    # ---------------------------------------------------------------- S3: the queue survives a restart
    if (Want 'S3') { try {
        Say 'S3: the queue survives a restart of Kin.exe and a force-stop of the phone app'
        # 3a: PC side
        $f = Join-Path $script:W14 'e2e14-s3-pc.bin'; Make-PcFile $f 200000 62
        $shaPc = Pc-Sha $f
        $script:Names.Add('e2e14-s3-pc.bin')
        $txPc = Use-Text "e2e14-$n s3 text queued on the PC across a restart"
        Check 'S3a the phone is away' (Phone-Away14)
        $r = Pc-SendText14 $txPc
        $o = Pc-OfferFile14 $f
        Check 'S3a a text and a file are queued on the PC' ($r.queued -eq $true -and $o.status -eq 'queued')
        $tid = (Pc-History | Where-Object { $_.text -eq $txPc }).id
        $script:Seen14[$tid] = 'S3a'; $script:Seen14[$o.id] = 'S3a'
        Stop-Kin14
        Start-Kin14
        $st = Get-Status
        Check 'S3a after the restart both are still queued (status.json queue.count 2)' ($st.queue.count -eq 2) "count=$($st.queue.count)"
        $hq = Pc-History | Where-Object { $_.id -in @($tid, $o.id) }
        Check 'S3a ... and the history still shows them as queued, with the same ids' (@($hq | Where-Object { $_.status -eq 'queued' }).Count -eq 2)
        Wifi $true
        Check 'S3a the phone is back' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 45)
        Check 'S3a both are delivered (done) under the same ids' (Wait-Until { (@(Pc-History | Where-Object { $_.id -in @($tid, $o.id) -and $_.status -eq 'done' })).Count -eq 2 } 45)
        $hh = Phone-DirHashes '/sdcard/Download/Kin'
        Check 'S3a the file on the phone is intact' ($hh['e2e14-s3-pc.bin'] -eq $shaPc)
        Check 'S3a the text is on the phone' ((Wait-PhoneClip $txPc) -ne $null)

        # 3b: phone side
        Make-PhoneFiles 'e2e14-s3-ph.bin:220000'
        $src = Phone-SourceHashes
        $script:Names.Add('e2e14-s3-ph.bin')
        $txPh = Use-Text "e2e14-$n s3 text queued on the phone across a force-stop"
        $ids0 = @(Phone-TextIds); $tids0 = @(Phone-Ids)
        Wifi $false
        Start-Sleep -Seconds 2
        $res = Phone-SendText14 $txPh
        Check 'S3b the phone queued the text (Wi-Fi off)' ($res -match 'Queued\(') $res
        $txRow = Phone-NewText $ids0 'up' 10
        Share-Files 'e2e14-s3-ph.bin'
        $upRow = Phone-NewUpload $tids0
        Check 'S3b a text and a file are queued on the phone' ($txRow -and $txRow.Status -eq 'queued' -and $upRow -and $upRow.Status -eq 'queued') "$($txRow.Status) $($upRow.Status)"
        $script:Seen14[$txRow.Id] = 'S3b'; $script:Seen14[$upRow.Id] = 'S3b'
        A shell am force-stop $Pkg | Out-Null
        Start-Sleep -Seconds 1
        A shell am start -n "$Pkg/.MainActivity" | Out-Null
        Start-Sleep -Seconds 3
        $tx2 = Phone-Text $txRow.Id; $up2 = Phone-Transfer $upRow.Id
        # (the relaunch starts the upload again, so it may say "active" while it waits for the PC: what counts is that it is not lost, failed or sent)
        Check 'S3b after force-stop and relaunch both are still there, waiting, same ids' ($tx2 -and $tx2.Status -eq 'queued' -and $up2 -and $up2.Status -in 'queued', 'active') "$($tx2.Status) $($up2.Status)"
        Check 'S3b ... and nothing reached the PC meanwhile' ((Pc-HistoryItem $upRow.Id) -eq $null -and (Pc-HistoryItem $txRow.Id) -eq $null)
        Wifi $true
        Check 'S3b the text and the file arrive at the PC' ((Wait-Until { (Get-Status).lastReceived.text.id -eq $txRow.Id } 60) -and (Wait-Until { (Pc-HistoryItem $upRow.Id).status -eq 'done' } 60))
        $recv = Join-Path $script:RecvDir 'e2e14-s3-ph.bin'
        Check 'S3b the file on the PC is intact' ((Test-Path -LiteralPath $recv) -and (Pc-Sha $recv) -eq $src['e2e14-s3-ph.bin'])
        Check 'S3b the phone says done for both' ((Wait-Until { (Phone-Text $txRow.Id).Status -eq 'done' -and (Phone-Transfer $upRow.Id).Status -eq 'done' } 20))
    } catch { Scenario-Failed 'S3' $_ } }

    # ---------------------------------------------------------------- S4: cancel a queued item, each side
    if (Want 'S4') { try {
        Say 'S4: cancel a queued item on each side'
        $f = Join-Path $script:W14 'e2e14-s4-pc.bin'; Make-PcFile $f 150000 63
        $script:Names.Add('e2e14-s4-pc.bin')
        $txPc = Use-Text "e2e14-$n s4 text that is canceled on the PC"
        $keep = Use-Text "e2e14-$n s4 text that is kept on the PC"
        Check 'S4 the phone is away' (Phone-Away14)
        $r = Pc-SendText14 $txPc
        $o = Pc-OfferFile14 $f
        $r2 = Pc-SendText14 $keep
        $hist = Pc-History
        $tidC = ($hist | Where-Object { $_.text -eq $txPc }).id; $tidK = ($hist | Where-Object { $_.text -eq $keep }).id
        Check 'S4 three items queued on the PC' ((Get-Status).queue.count -eq 3)
        $c1 = Pc-HistoryCmd 'cancel' $tidC; $c2 = Pc-HistoryCmd 'cancel' $o.id
        Check 'S4 the PC canceled the text and the file (the history Cancel button)' ($c1.ok -and $c2.ok) "$($c1.message) / $($c2.message)"
        $hist = Pc-History
        Check 'S4 the history says canceled for both, queued for the third' ((($hist | Where-Object { $_.id -eq $tidC }).status -eq 'canceled') -and (($hist | Where-Object { $_.id -eq $o.id }).status -eq 'canceled') -and (($hist | Where-Object { $_.id -eq $tidK }).status -eq 'queued'))
        Check 'S4 the queue holds one item' ((Get-Status).queue.count -eq 1)
        $script:Seen14[$tidC] = 'S4'; $script:Seen14[$o.id] = 'S4'; $script:Seen14[$tidK] = 'S4'
        $ids0 = @(Phone-TextIds); $tids0 = @(Phone-Ids)
        Wifi $true
        Check 'S4 the phone is back' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 45)
        Check 'S4 the kept text arrives' (Wait-Until { (Pc-HistoryItem $tidK).status -eq 'done' } 30)
        Start-Sleep -Seconds 4
        $pids = @(Phone-TextIds); $ptids = @(Phone-Ids)
        Check 'S4 the canceled text never arrived at the phone' ($tidC -notin $pids -and $tidK -in $pids)
        Check 'S4 the canceled file never arrived at the phone (no transfer, no file)' ($o.id -notin $ptids -and 'e2e14-s4-pc.bin' -notin @(Phone-DownloadNames))
        Check 'S4 the PC log has no offer or text send for the canceled ones' ((Pc-LogLines "offer $(Short $o.id) to device").Count -eq 0 -and (Pc-LogLines "text $tidC sent").Count -eq 0)

        # phone side
        Make-PhoneFiles 'e2e14-s4-ph.bin:150000'
        $script:Names.Add('e2e14-s4-ph.bin')
        $txPh = Use-Text "e2e14-$n s4 text canceled on the phone"
        $ids0 = @(Phone-TextIds); $tids0 = @(Phone-Ids)
        $lastText = (Get-Status).lastReceived.text.id
        $pcIds0 = @(Pc-History | ForEach-Object { $_.id })
        Wifi $false
        Start-Sleep -Seconds 2
        $res = Phone-SendText14 $txPh
        $txRow = Phone-NewText $ids0 'up' 10
        Share-Files 'e2e14-s4-ph.bin'
        $upRow = Phone-NewUpload $tids0
        Check 'S4 phone: a text and a file queued' ($txRow.Status -eq 'queued' -and $upRow.Status -eq 'queued')
        $l = Phone-Bcast 'CANCEL_TEXT' "--es id $($txRow.Id)" 800
        Check 'S4 phone: CANCEL_TEXT canceled the queued text' (($l -join "`n") -match 'CMD CANCEL_TEXT \S+ canceled=true')
        Phone-Bcast 'CANCEL_TRANSFER' "--es id $($upRow.Id)" 800 | Out-Null
        Check 'S4 phone: the queued upload is canceled' (Wait-Until { (Phone-Transfer $upRow.Id).Status -eq 'canceled' } 10) "$((Phone-Transfer $upRow.Id).Status)"
        Check 'S4 phone: the text says canceled' ((Phone-Text $txRow.Id).Status -eq 'canceled')
        $script:Seen14[$txRow.Id] = 'S4p'; $script:Seen14[$upRow.Id] = 'S4p'
        Wifi $true
        Check 'S4 phone: back on the PC' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 45)
        Start-Sleep -Seconds 6
        $pcNow = @(Pc-History | ForEach-Object { $_.id })
        Check 'S4 phone: neither the text nor the file ever reached the PC' ($txRow.Id -notin $pcNow -and $upRow.Id -notin $pcNow -and (Get-Status).lastReceived.text.id -eq $lastText -and 'e2e14-s4-ph.bin' -notin (Pc-RecvNames))
        Check 'S4 phone: still canceled after the reconnect (nothing woke them up)' ((Phone-Text $txRow.Id).Status -eq 'canceled' -and (Phone-Transfer $upRow.Id).Status -eq 'canceled')
    } catch { Scenario-Failed 'S4' $_ } }

    # ---------------------------------------------------------------- S5: expiry
    if (Want 'S5') { try {
        Say 'S5: expiry (the PC with --dev-clock-offset, the phone with EXPIRE --el ahead)'
        $f = Join-Path $script:W14 'e2e14-s5-pc.bin'; Make-PcFile $f 120000 64
        $script:Names.Add('e2e14-s5-pc.bin')
        $txPc = Use-Text "e2e14-$n s5 text that expires on the PC"
        Check 'S5 the phone is away' (Phone-Away14)
        $r = Pc-SendText14 $txPc
        $o = Pc-OfferFile14 $f
        $tid = (Pc-History | Where-Object { $_.text -eq $txPc }).id
        Check 'S5 a text and a file are queued on the PC' ((Get-Status).queue.count -eq 2)
        $script:Seen14[$tid] = 'S5'; $script:Seen14[$o.id] = 'S5'
        Stop-Kin14
        Start-Kin14 @('--dev-clock-offset', '604801')   # a clock 7 days and one second ahead: the start-up sweep fails what waited 7 days
        $hist = $null
        Wait-Until { $script:hx = Pc-History | Where-Object { $_.id -in @($tid, $o.id) }; @($script:hx | Where-Object { $_.status -eq 'failed' }).Count -eq 2 } 20 | Out-Null
        $hist = @($script:hx)
        Check 'S5 PC: both queued items are failed with "expired" (history)' (@($hist | Where-Object { $_.status -eq 'failed' -and $_.error -eq 'expired' }).Count -eq 2) (($hist | ForEach-Object { "$($_.kind) $($_.status) $($_.error)" }) -join '; ')
        Check 'S5 PC: the queue is empty and the expired items offer Retry (canRetry)' ((Get-Status).queue.count -eq 0 -and @($hist | Where-Object { $_.canRetry }).Count -eq 2)
        Stop-Kin14
        Start-Kin14
        $ids0 = @(Phone-TextIds); $tids0 = @(Phone-Ids)
        Wifi $true
        Check 'S5 PC: the phone is back' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 45)
        Start-Sleep -Seconds 5
        Check 'S5 PC: the expired items were never sent' ($tid -notin @(Phone-TextIds) -and $o.id -notin @(Phone-Ids) -and 'e2e14-s5-pc.bin' -notin @(Phone-DownloadNames) -and (Pc-LogLines "offer $(Short $o.id) to device").Count -eq 0)
        $hist = Pc-History | Where-Object { $_.id -in @($tid, $o.id) }
        Check 'S5 PC: still failed/expired after the restart without the offset' (@($hist | Where-Object { $_.status -eq 'failed' -and $_.error -eq 'expired' }).Count -eq 2)

        # phone side
        Make-PhoneFiles 'e2e14-s5-ph.bin:120000'
        $script:Names.Add('e2e14-s5-ph.bin')
        $txPh = Use-Text "e2e14-$n s5 text that expires on the phone"
        $ids0 = @(Phone-TextIds); $tids0 = @(Phone-Ids)
        $pcIds0 = @(Pc-History | ForEach-Object { $_.id })
        $lastText = (Get-Status).lastReceived.text.id
        Wifi $false
        Start-Sleep -Seconds 2
        Phone-SendText14 $txPh | Out-Null
        $txRow = Phone-NewText $ids0 'up' 10
        Share-Files 'e2e14-s5-ph.bin'
        $upRow = Phone-NewUpload $tids0
        $script:Seen14[$txRow.Id] = 'S5p'; $script:Seen14[$upRow.Id] = 'S5p'
        $l = Phone-Bcast 'EXPIRE' '--el ahead 603000000' 800
        Check 'S5 phone: EXPIRE with a clock 30 minutes short of 7 days ahead expires nothing' (($l -join "`n") -match 'CMD EXPIRE expired=0') (($l | Where-Object { $_ -match 'CMD EXPIRE' }) -join '')
        Check 'S5 phone: both are still queued' ((Phone-Text $txRow.Id).Status -eq 'queued' -and (Phone-Transfer $upRow.Id).Status -eq 'queued')
        $l = Phone-Bcast 'EXPIRE' '--el ahead 604800000' 800
        Check 'S5 phone: EXPIRE --el ahead 604800000 (7 days) expired both' (($l -join "`n") -match 'CMD EXPIRE expired=2') (($l | Where-Object { $_ -match 'CMD EXPIRE' }) -join '')
        $tx2 = Phone-Text $txRow.Id; $up2 = Phone-Transfer $upRow.Id
        Check 'S5 phone: the text and the file are failed with "expired"' ($tx2.Status -eq 'failed' -and $tx2.Err -eq 'expired' -and $up2.Status -eq 'failed' -and $up2.Err -eq 'expired') "$($tx2.Status) $($tx2.Err) / $($up2.Status) $($up2.Err)"
        Wifi $true
        Check 'S5 phone: back on the PC' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 45)
        Start-Sleep -Seconds 6
        $pcNow = @(Pc-History | ForEach-Object { $_.id })
        Check 'S5 phone: the expired items never reached the PC' ($txRow.Id -notin $pcNow -and $upRow.Id -notin $pcNow -and (Get-Status).lastReceived.text.id -eq $lastText -and 'e2e14-s5-ph.bin' -notin (Pc-RecvNames))
        Check 'S5 phone: still failed/expired' ((Phone-Text $txRow.Id).Err -eq 'expired' -and (Phone-Transfer $upRow.Id).Err -eq 'expired')
    } catch { Scenario-Failed 'S5' $_ } }

    # ---------------------------------------------------------------- S6: no ack -> re-queued, delivered once
    if (Want 'S6') { try {
        Say 'S6: the receiver does not ack (frozen process): the item goes back to the queue and arrives once'
        try {
            # 6a PC -> phone, the phone app is frozen (SIGSTOP)
            $txA = Use-Text "e2e14-$n s6a text sent to a frozen phone"
            $pidA = Phone-Pid
            Check 'S6a the phone app has a process' ([bool]$pidA)
            Phone-Freeze14
            $r = Pc-SendText14 $txA 25
            Check 'S6a no ack in 5 s: the text is queued, not failed (no_reply, queued true)' ($r -and $r.queued -eq $true -and $r.result -in 'no_reply', 'phone_offline') "$($r.result) queued=$($r.queued)"
            $tid = $r.id
            $script:Seen14[$tid] = 'S6a'
            Check 'S6a the PC marked the phone offline' (Wait-Until { (Get-Status).state -eq 'phone_offline' } 15)
            $hi = Pc-HistoryItem $tid
            Check 'S6a the history shows the text queued (not failed)' ($hi -and $hi.status -eq 'queued') "$($hi.status) $($hi.error)"
            Phone-Resume14
            Check 'S6a the phone came back and the text is delivered (history done)' (Wait-Until { (Pc-HistoryItem $tid).status -eq 'done' } 60) "$((Pc-HistoryItem $tid).status)"
            Start-Sleep -Seconds 3
            $rows = @(Phone-Texts | Where-Object { $_.Id -eq $tid })
            Check 'S6a exactly one copy on the phone (de-duplicated by the message id, not a second history row)' ($rows.Count -eq 1 -and $rows[0].Status -eq 'done') "$($rows.Count)"
            Check 'S6a the phone log shows the text arrived once ("text from" with that length), and a repeat was acked' (([regex]::Matches((Phone-Log), "text from '[^']*' len=$($txA.Length) ")).Count -eq 1) "$(([regex]::Matches((Phone-Log), "text from '[^']*' len=$($txA.Length) ")).Count)"

            # 6b PC -> phone, an offer to the frozen phone
            $f = Join-Path $script:W14 'e2e14-s6-offer.bin'; Make-PcFile $f 400000 65
            $shaF = Pc-Sha $f
            $script:Names.Add('e2e14-s6-offer.bin')
            Phone-Freeze14
            $o = Pc-OfferFile14 $f
            Check 'S6b the offer to a frozen phone is in the history (queued or being offered)' ($o -and $o.status -in 'queued', 'active') "$($o.status)"
            Check 'S6b the PC marks the phone offline after 5 s without an ack' (Wait-Until { (Get-Status).state -eq 'phone_offline' } 25)
            $hi = Pc-HistoryItem $o.id
            Check 'S6b the offer is queued, not failed' ($hi.status -eq 'queued') "$($hi.status) $($hi.error)"
            $script:Seen14[$o.id] = 'S6b'
            Phone-Resume14
            $swB = [Diagnostics.Stopwatch]::StartNew()
            Check 'S6b delivered after the phone is back (history done)' (Wait-Until { (Pc-HistoryItem $o.id).status -eq 'done' } 90) "$((Pc-HistoryItem $o.id).status)"
            $script:Timings['S6b frozen phone resumed -> offer done on the PC'] = "{0:N1} s" -f $swB.Elapsed.TotalSeconds
            Say "  PC log, offer $(Short $o.id):"; Pc-LogLines (Short $o.id) | Select-Object -Last 12 | ForEach-Object { Say "    $_" }
            Say '  phone log, transfer and link lines:'; (Phone-Log) -split "`r?`n" | Where-Object { $_ -match "$($o.id)| link: " } | Select-Object -Last 14 | ForEach-Object { Say "    $_" }
            Start-Sleep -Seconds 2
            $names = @(Phone-DownloadNames)
            Check 'S6b one copy of the file on the phone, no "(1)" copy' (@($names | Where-Object { $_ -like 'e2e14-s6-offer*' }).Count -eq 1) (($names | Where-Object { $_ -like 'e2e14-s6-offer*' }) -join ', ')
            $hh = Phone-DirHashes '/sdcard/Download/Kin'
            Check 'S6b the file is intact' ($hh['e2e14-s6-offer.bin'] -eq $shaF)
            Check 'S6b the phone has exactly one transfer row for the offer id' (@(Phone-Transfers | Where-Object { $_.Id -eq $o.id }).Count -eq 1)

            # 6c phone -> PC, Kin.exe frozen
            $txC = Use-Text "e2e14-$n s6c text sent to a frozen PC"
            $ids0 = @(Phone-TextIds)
            $toasts0 = Pc-LogCount 'toast: Text from '
            Freeze-Kin14
            $res = Phone-SendText14 $txC $false 25
            Check 'S6c no ack in 5 s: the phone answers Queued(afterNoReply=true)' ($res -match 'Queued\(' -and $res -match 'afterNoReply=true') $res
            $row = Phone-NewText $ids0 'up' 10
            Check 'S6c the text is queued on the phone, not failed, with 1 attempt' ($row -and $row.Status -eq 'queued' -and $row.Attempts -eq 1) "$($row.Status) attempts=$($row.Attempts)"
            Resume-Kin14
            Check 'S6c the PC got it after it was resumed' (Wait-Until { (Get-Status).lastReceived.text.id -eq $row.Id } 40)
            Connect-Phone14
            Check 'S6c the phone sent it again and finished it (done, 2 attempts)' (Wait-Until { $x = Phone-Text $row.Id; $x.Status -eq 'done' } 60) "$((Phone-Text $row.Id).Status) attempts=$((Phone-Text $row.Id).Attempts)"
            Start-Sleep -Seconds 2
            $ph = @(Pc-History | Where-Object { $_.id -eq $row.Id })
            Check 'S6c the PC history has it once, and the PC showed one toast for it (no duplicate)' ($ph.Count -eq 1 -and ((Pc-LogCount 'toast: Text from ') - $toasts0) -eq 1) "rows=$($ph.Count) toasts=$((Pc-LogCount 'toast: Text from ') - $toasts0)"
            $script:Seen14[$row.Id] = 'S6c'

            # 6d phone -> PC, an upload; Kin.exe frozen when the phone starts it
            # (an upload has no ack of its own: the POST waits; the phone's WebSocket is not involved. Nothing to re-queue: covered by S7.)
        } finally {
            Resume-Kin14
            Phone-Resume14
        }
    } catch { Scenario-Failed 'S6' $_ } }

    # ---------------------------------------------------------------- S7: the PC cancels / forgets an upload while the phone is offline
    # (a) the PC's stored cancel reaches the phone after the reconnect (changelog 52): the phone ends canceled (peer_canceled), not failed (not_found).
    # (b) the 404 path: the PC forgets the upload (24 hours with no progress: --dev-clock-offset) and sends no cancel, so the phone's automatic resume
    #     gets a 404 and fails it (not_found); a user Retry sends only the files without a 200, under a new ID.
    if (Want 'S7') { try {
        Say 'S7a: the PC cancels a phone upload while the phone is offline; the stored cancel reaches the phone after the reconnect'
        Make-PhoneFiles 'e2e14-r1.bin:200000,e2e14-r2.bin:200000'
        Make-PhoneBig 'e2e14-r3.bin' 80   # big enough that a fast emulator (API 29 does 15 MB/s) is still sending when the PC cancels
        foreach ($nm in 'e2e14-r1.bin', 'e2e14-r2.bin', 'e2e14-r3.bin') { $script:Names.Add($nm) }
        $tids0 = @(Phone-Ids); $pcIds0 = @(Pc-Ids)
        Share-Files 'e2e14-r1.bin,e2e14-r2.bin,e2e14-r3.bin'
        $a = Wait-PcActive 'in' (4MB + 400000) 120
        Check 'S7a the 3-file upload is running; the two small files are in' ($null -ne $a) "$($a.bytesDone) of $($a.totalBytes)"
        $up = @(Phone-Transfers) | Where-Object { $_.Id -notin $tids0 -and $_.Dir -eq 'up' } | Select-Object -First 1
        Wifi $false
        Start-Sleep -Seconds 1
        Invoke-Kin --cancel-transfers
        $pcT = Wait-NewPcTransfer $pcIds0 'in' 40
        Check 'S7a the PC canceled the transfer (the phone is offline)' ($pcT -and $pcT.status -eq 'canceled') "$($pcT.status) $($pcT.error)"
        $script:Seen14[$up.Id] = 'S7a'
        $partial = @(Pc-Recv | Where-Object { $_.Name -like '*.part' })
        Check 'S7a the PC has no .part file left; the two complete files stay, the third is gone' ($partial.Count -eq 0 -and (Test-Path -LiteralPath (Join-Path $script:RecvDir 'e2e14-r1.bin')) -and (Test-Path -LiteralPath (Join-Path $script:RecvDir 'e2e14-r2.bin')) -and -not (Test-Path -LiteralPath (Join-Path $script:RecvDir 'e2e14-r3.bin'))) ((Pc-RecvNames | Where-Object { $_ -like 'e2e14-r*' }) -join ', ')
        $pcIdsBefore = @(Pc-Ids); $phIdsBefore = @(Phone-Ids)
        $cancelsBefore = (Pc-LogLines 'cancel message\(s\) are sent again after the hello').Count
        Wifi $true
        Check 'S7a the phone is back' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 45)
        Wait-Until { $script:rw = Phone-Transfer $up.Id; $script:rw -and $script:rw.Status -in 'failed', 'canceled', 'done' } 40 | Out-Null
        # The phone's automatic resume may get its 404 (failed, not_found) a moment before the cancel message: the cancel then turns it into canceled.
        Wait-Until { $script:rw = Phone-Transfer $up.Id; $script:rw -and $script:rw.Status -eq 'canceled' } 15 | Out-Null
        $row = $script:rw
        Check 'S7a the phone shows the upload as canceled (peer_canceled), not failed (not_found): the PC''s stored cancel reached it' ($row -and $row.Status -eq 'canceled' -and $row.Err -eq 'peer_canceled') "$($row.Status) $($row.Err)"
        Check 'S7a the PC sent the stored cancel again after the hello and the phone acked it' ((Pc-LogLines 'cancel message\(s\) are sent again after the hello').Count -gt $cancelsBefore -and (Pc-LogLines "cancel message for $(Short $up.Id): acked").Count -ge 1)
        Start-Sleep -Seconds 5
        Check 'S7a nothing was sent again: no new transfer on either side, no file written, no .part' ((@(Pc-Ids).Count -eq $pcIdsBefore.Count) -and (@(Phone-Ids).Count -eq $phIdsBefore.Count) -and (@(Pc-Recv | Where-Object { $_.Name -like '*.part' -or $_.Name -like 'e2e14-r3*' }).Count -eq 0))

        # ---- S7b: the PC forgets the upload (the 24 hours), the phone's resume gets a 404, a user Retry sends what is missing
        Say 'S7b: the PC forgets a phone upload (24 hours with no progress) while the phone is offline: 404, failed (not_found), user Retry'
        Make-PhoneFiles 'e2e14-x1.bin:200000,e2e14-x2.bin:200000'
        Make-PhoneBig 'e2e14-x3.bin' 80
        foreach ($nm in 'e2e14-x1.bin', 'e2e14-x2.bin', 'e2e14-x3.bin') { $script:Names.Add($nm) }
        $src = Phone-SourceHashes
        $tids0 = @(Phone-Ids); $pcIds0 = @(Pc-Ids)
        Share-Files 'e2e14-x1.bin,e2e14-x2.bin,e2e14-x3.bin'
        $a = Wait-PcActive 'in' (4MB + 400000) 120
        Check 'S7b the 3-file upload is running; the two small files are in' ($null -ne $a) "$($a.bytesDone) of $($a.totalBytes)"
        $up = @(Phone-Transfers) | Where-Object { $_.Id -notin $tids0 -and $_.Dir -eq 'up' } | Select-Object -First 1
        Wifi $false
        Start-Sleep -Seconds 1
        Stop-Kin14
        Start-Kin14 @('--dev-clock-offset', '90000')   # a clock 25 hours ahead: the start-up sweep drops an upload with no progress for a day
        $pcRow = $null
        Wait-Until { $script:pr = Pc-HistoryItem $up.Id; $script:pr -and $script:pr.status -eq 'failed' } 20 | Out-Null
        $pcRow = $script:pr
        Check 'S7b PC: the upload with no progress for 24 hours is failed with "expired" (history), its .part gone, no cancel kept for the phone' ($pcRow -and $pcRow.status -eq 'failed' -and $pcRow.error -eq 'expired' -and @(Pc-Recv | Where-Object { $_.Name -like '*.part' }).Count -eq 0) "$($pcRow.status) $($pcRow.error)"
        Stop-Kin14
        Start-Kin14
        $script:Seen14[$up.Id] = 'S7b'
        $pcIdsBefore = @(Pc-Ids); $phIdsBefore = @(Phone-Ids)
        Wifi $true
        Check 'S7b the phone is back' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 45)
        Wait-Until { $script:rw = Phone-Transfer $up.Id; $script:rw -and $script:rw.Status -in 'failed', 'canceled', 'done' } 40 | Out-Null
        $row = $script:rw
        Check 'S7b automatic resume: the POST got 404, so the phone failed the transfer (not_found) and did not send again' ($row -and $row.Status -eq 'failed' -and $row.Err -eq 'not_found') "$($row.Status) $($row.Err)"
        Check 'S7b the PC answered 404 because the ID was used before (its log)' ((Pc-LogLines "upload $(Short $up.Id): that ID was used before, answered 404").Count -ge 1)
        Start-Sleep -Seconds 5
        Check 'S7b no new transfer on either side, no file written, no .part' ((@(Pc-Ids).Count -eq $pcIdsBefore.Count) -and (@(Phone-Ids).Count -eq $phIdsBefore.Count) -and (@(Pc-Recv | Where-Object { $_.Name -like '*.part' -or $_.Name -like 'e2e14-x3*' }).Count -eq 0))
        # user Retry
        $l = Phone-Bcast 'RETRY_TRANSFER' "--es id $($up.Id)" 1000
        Wait-Until { $script:nr = @(Phone-Transfers) | Where-Object { $_.Id -notin $phIdsBefore -and $_.Dir -eq 'up' } | Select-Object -First 1; [bool]$script:nr } 30 | Out-Null
        $new = $script:nr
        Check 'S7b user Retry made a NEW transfer id with only the one file without a 200' ($new -and $new.Id -ne $up.Id -and $new.Files -eq 1) "$($new.Id) files=$($new.Files)"
        $newDone = Wait-Until { (Phone-Transfer $new.Id).Status -eq 'done' } 120
        Check 'S7b the retry finished (done) on the phone' $newDone "$((Phone-Transfer $new.Id).Status) $((Phone-Transfer $new.Id).Err)"
        $pcNew = Wait-NewPcTransfer $pcIdsBefore 'in' 60
        Check 'S7b the PC got the new transfer: 1 file, done, under the new id' ($pcNew -and $pcNew.id -eq $new.Id -and $pcNew.status -eq 'done' -and $pcNew.files -eq 1) "$($pcNew.id) $($pcNew.status) files=$($pcNew.files)"
        $oldAgain = Phone-Transfer $up.Id
        Check 'S7b the old row stays failed (no second retry from it)' ($oldAgain.Status -eq 'failed')
        $script:Seen14[$new.Id] = 'S7bretry'
        $names = Pc-RecvNames
        Check 'S7b the PC folder has the three files once each and no "name (1)" copy' (@($names | Where-Object { $_ -match ' \(\d+\)' }).Count -eq 0 -and @($names | Where-Object { $_ -like 'e2e14-x*' }).Count -eq 3) (($names | Where-Object { $_ -like 'e2e14-x*' }) -join ', ')
        $bad = @('e2e14-x1.bin', 'e2e14-x2.bin', 'e2e14-x3.bin' | Where-Object { $p = Join-Path $script:RecvDir $_; -not ((Test-Path -LiteralPath $p) -and (Pc-Sha $p) -eq $src[$_]) })
        Check 'S7b all three files are byte-identical to the phone sources' ($bad.Count -eq 0) ($bad -join ', ')
    } catch { Scenario-Failed 'S7' $_ } }

    # ---------------------------------------------------------------- S8: a sensitive text while the peer is offline
    if (Want 'S8') { try {
        Say 'S8: a sensitive text while the peer is offline is not queued (both directions)'
        $secPc = Use-Text "e2e14-$n s8 pc secret words here"
        $script:SecretTexts.Add($secPc)
        Check 'S8 the phone is away' (Phone-Away14)
        $ids0 = @(Phone-TextIds)
        $q0 = (Get-Status).queue.count
        Invoke-Kin --dev-set-clipboard $secPc --dev-clipboard-mark exclude
        Start-Sleep -Milliseconds 1200
        $t0 = (Get-Status).lastSent.text.at
        Invoke-Kin --send-clipboard
        Wait-Until { $x = (Get-Status).lastSent.text; $x -and $x.at -ne $t0 } 15 | Out-Null
        $r = (Get-Status).lastSent.text
        Check 'S8 PC: the sensitive text was not sent and not queued (result phone_offline, sensitive true, queued false)' ($r -and $r.sensitive -eq $true -and $r.result -eq 'phone_offline' -and -not $r.queued) "$($r.result) sensitive=$($r.sensitive) queued=$($r.queued)"
        Check 'S8 PC: the queue is unchanged' ((Get-Status).queue.count -eq $q0)
        Check 'S8 PC: the history has no item with that text, and the status has no length' ((@(Pc-History | Where-Object { $_.text -eq $secPc }).Count -eq 0) -and $null -eq $r.chars)
        Check 'S8 PC: the user is told ("never queued ... not sent" toast in the log)' ((Pc-Log) -match 'toast: The phone isn.t connected\. A text marked sensitive is never queued, so it was not sent')
        Wifi $true
        Check 'S8 PC: the phone is back' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 45)
        Start-Sleep -Seconds 6
        $pcl = Phone-Clip
        Check 'S8 PC: it never reached the phone (no text row, clipboard without it)' (@(Phone-TextIds | Where-Object { $_ -notin $ids0 }).Count -eq 0 -and (-not $pcl -or $pcl.Text -cne $secPc))
        Check 'S8 PC: the queue is still empty after the reconnect' ((Get-Status).queue.count -eq $q0)

        # phone -> PC
        $secPh = Use-Text "e2e14-$n s8 phone secret words here"
        $script:SecretTexts.Add($secPh)
        $ids0 = @(Phone-TextIds)
        $lastText = (Get-Status).lastReceived.text.id
        Wifi $false
        Start-Sleep -Seconds 2
        $res = Phone-SendText14 $secPh $true
        Check 'S8 phone: SEND_TEXT with sensitive answers NotSent(...), not Queued' ($res -match 'result=NotSent\(') $res
        Check 'S8 phone: nothing was stored (no new row in the text history)' (@(Phone-TextIds | Where-Object { $_ -notin $ids0 }).Count -eq 0)
        Check 'S8 phone: the user got a notification "Not sent: <PC> offline"' (Wait-Until { (Phone-Notifs) -match 'Not sent: ' } 10)
        Wifi $true
        Check 'S8 phone: back on the PC' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 45)
        Start-Sleep -Seconds 6
        Check 'S8 phone: it never reached the PC, and nothing was sent by itself after the reconnect' ((Get-Status).lastReceived.text.id -eq $lastText -and @(Phone-TextIds | Where-Object { $_ -notin $ids0 }).Count -eq 0 -and (Pc-Clip) -cne $secPh)
        # a sensitive text over an open connection still works
        $secOn = Use-Text "e2e14-$n s8 phone secret sent online"
        $script:SecretTexts.Add($secOn)
        $res = Phone-SendText14 $secOn $true
        Check 'S8 phone: a sensitive text over an open connection is delivered' ($res -match 'result=Delivered\(') $res
        Check 'S8 phone: ... and it is in no history on either side' (@(Phone-Texts | Where-Object { $_.Sensitive }).Count -eq 0 -and @(Pc-History | Where-Object { $_.text -eq $secOn }).Count -eq 0)
    } catch { Scenario-Failed 'S8' $_ } }

    # ---------------------------------------------------------------- S9: the automatic clipboard while the phone is offline
    if (Want 'S9') { try {
        Say 'S9: the PC copies something while the phone is offline: not queued, not sent stale'
        $auto = Use-Text "e2e14-$n s9 copied while the phone was away"
        $script:SecretTexts.Add($auto)
        $later = Use-Text "e2e14-$n s9 copied after the phone is back"
        Check 'S9 the phone is away' (Phone-Away14)
        $q0 = (Get-Status).queue.count; $s0 = Get-Status
        $ids0 = @(Phone-TextIds)
        Set-QuietClipboardText $auto
        Start-Sleep -Milliseconds 2500
        $s1 = Get-Status
        Check 'S9 the copy was not queued (queue.count unchanged), and nothing is in the history' ($s1.queue.count -eq $q0 -and @(Pc-History | Where-Object { $_.text -eq $auto }).Count -eq 0) "queue $q0 -> $($s1.queue.count)"
        Check 'S9 status.json says it was dropped because the phone is offline (clipboard result phone_offline, queued false)' ($s1.lastSent.clipboard.result -in 'phone_offline', 'no_reply' -and -not $s1.lastSent.clipboard.queued) "$($s1.lastSent.clipboard.result)"
        Wifi $true
        Check 'S9 the phone is back' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 45)
        Start-Sleep -Seconds 6
        $pcl = Phone-Clip
        Check 'S9 on the reconnect the old copy was not sent (the phone clipboard does not have it)' (-not $pcl -or $pcl.Text -cne $auto)
        Check 'S9 ... and the PC says nothing was sent after the hello' ((Get-Status).clipboardSync.sent -eq $s0.clipboardSync.sent -and (Get-Status).queue.count -eq $q0 -and @(Phone-TextIds | Where-Object { $_ -notin $ids0 }).Count -eq 0)
        $lc = Copy-OnPc $later
        Check 'S9 control: a copy made while the phone is connected is sent (acked)' ($lc -and $lc.result -eq 'acked') "$($lc.result)"
        $pcl = Wait-PhoneClip $later
        Check 'S9 control: and the phone clipboard has it' ($pcl -and $pcl.Text -ceq $later)
    } catch { Scenario-Failed 'S9' $_ } }

    # ---------------------------------------------------------------- S10: history parity
    if (Want 'S10') { try {
        Say 'S10: the PC history and the phone history agree'
        $pcH = @(Pc-History)
        $phT = @(Phone-Texts); $phF = @(Phone-Transfers)
        $phone = @{}
        foreach ($t in $phT) { $phone[$t.Id] = [pscustomobject]@{ Kind = 'text'; Dir = $t.Dir; Status = $t.Status; Err = $t.Err } }
        foreach ($t in $phF) { $phone[$t.Id] = [pscustomobject]@{ Kind = 'file'; Dir = $t.Dir; Status = $t.Status; Err = $t.Err } }
        $rows = New-Object System.Collections.Generic.List[object]
        $mismatch = @()
        $pcOnly = @()
        foreach ($p in $pcH) {
            $q = $phone[$p.id]
            if ($q) {
                $dirOk = (($p.direction -eq 'out' -and $q.Dir -eq 'down') -or ($p.direction -eq 'in' -and $q.Dir -eq 'up'))
                $stOk = ($p.status -eq $q.Status)
                $rows.Add([pscustomobject]@{ Id = (Short $p.id); Kind = $p.kind; Dir = $p.direction; PC = "$($p.status)$(if ($p.error) { '/' + $p.error })"; Phone = "$($q.Status)$(if ($q.Err) { '/' + $q.Err })"; Same = ($dirOk -and $stOk) })
                if (-not ($dirOk -and $stOk)) { $mismatch += "$(Short $p.id) $($p.kind): PC $($p.status)/$($p.error), phone $($q.Status)/$($q.Err)" }
            } elseif ($p.status -eq 'done') { $pcOnly += "$(Short $p.id) $($p.kind) done on the PC only" }
        }
        $phOnly = @($phone.Keys | Where-Object { $id = $_; -not ($pcH | Where-Object { $_.id -eq $id }) -and $phone[$id].Status -eq 'done' })
        $rows | Format-Table -AutoSize | Out-String | ForEach-Object { $_ -split "`r?`n" } | Where-Object { $_.Trim() } | ForEach-Object { Say "  $_" }
        # Changelog 52: a transfer the PC canceled while the phone was offline is canceled on both sides (the stored cancel reaches the phone), and one the PC
        # forgot (24 hours) is failed on both (the phone through the 404). No exception is left.
        Check 'S10 every item both sides know has the same direction and status (the S7a upload is canceled on both, the S7b upload failed on both)' ($mismatch.Count -eq 0) ($mismatch -join '; ')
        Check 'S10 nothing that is done on one side is missing on the other' ($pcOnly.Count -eq 0 -and $phOnly.Count -eq 0) (($pcOnly + $phOnly) -join '; ')
        $secretsPc = @($pcH | Where-Object { $t = $_.text; $t -and (@($script:SecretTexts | Where-Object { $t -like "*$($_.Substring(0, [Math]::Min(25, $_.Length)))*" }).Count -gt 0) })
        Check 'S10 no sensitive text and no automatic clipboard text is in the PC history' ($secretsPc.Count -eq 0)
        Check 'S10 no sensitive row in the phone history (and no body kept for one)' (@($phT | Where-Object { $_.Sensitive }).Count -eq 0 -and @($phT | Where-Object { $_.Src -eq 'clipboard' -and $_.Dir -eq 'down' }).Count -eq 0)
        $script:Timings['S10 items compared (on both sides)'] = "$($rows.Count) ($(@($rows | Where-Object { $_.Same }).Count) identical)"
    } catch { Scenario-Failed 'S10' $_ } }

    # ---------------------------------------------------------------- S11: notification burst
    if (Want 'S11') { try {
        Say "S11: $BurstTexts texts and $BurstFiles transfers in a burst, PC -> phone"
        Reset-PhoneNotifications
        Check 'S11 the phone is away while the PC queues the texts' (Phone-Away14)
        $prefix = "e2e14-$n s11 burst text"
        for ($i = 1; $i -le $BurstTexts; $i++) { $t = Use-Text ("$prefix {0:D3}" -f $i); Pc-SendText14 $t 15 | Out-Null }
        Check "S11 $BurstTexts texts are queued on the PC" ((Get-Status).queue.count -eq $BurstTexts) "count=$((Get-Status).queue.count)"
        $sw = [Diagnostics.Stopwatch]::StartNew()
        Wifi $true
        Check 'S11 the phone is back and the queue is delivered' ((Wait-Until { (Get-Status).state -eq 'phone_connected' } 45) -and (Wait-Until { (Get-Status).queue.count -eq 0 } 60))
        $script:Timings["S11 svc wifi enable -> $BurstTexts queued texts all acked"] = "{0:N1} s" -f $sw.Elapsed.TotalSeconds
        Start-Sleep -Seconds 4
        $recs = @(Phone-AppRecords)
        $texts = @($recs | Where-Object { (Rec-Title $_) -match '^(\d+) texts from ' })
        $count = if ($texts.Count) { [int]([regex]::Match((Rec-Title $texts[0]), '^(\d+) texts').Groups[1].Value) } else { 0 }
        Check "S11 one grouped notification '$BurstTexts texts from <PC>' (the count says $count)" ($texts.Count -eq 1 -and $count -ge $BurstTexts) "records=$($texts.Count) count=$count"
        Check 'S11 the phone shows at most 15 notifications of ours in all (the cap), not one per text' ($recs.Count -le 15) "$($recs.Count) records"
        $script:Timings['S11 notifications of the app after the text burst'] = "$($recs.Count) (cap 15; Android's own limit is 50)"
        Phone-Screenshot '11-text-burst.png' -Shade

        # the transfer storm: BurstFiles single-file offers, back to back
        $before = @(Phone-Ids)
        $files = 1..$BurstFiles | ForEach-Object { $p = Join-Path $script:W14 ('e2e14-storm-{0:D2}.bin' -f $_); Make-PcFile $p 20000 (80 + $_); $script:Names.Add((Split-Path $p -Leaf)); $p }
        foreach ($p in $files) { Invoke-Kin --send $p }
        Check "S11 the phone downloaded all $BurstFiles files" (Wait-Until { @(Phone-Transfers | Where-Object { $_.Id -notin $before -and $_.Dir -eq 'down' -and $_.Status -eq 'done' }).Count -ge $BurstFiles } 120) "$(@(Phone-Transfers | Where-Object { $_.Id -notin $before -and $_.Status -eq 'done' }).Count)"
        Start-Sleep -Seconds 3
        $recs = @(Phone-AppRecords)
        $sum = @($recs | Where-Object { (Rec-Title $_) -match 'file transfers|files from your PC' })
        Check 'S11 the transfers are grouped: a summary notification exists' ($sum.Count -ge 1) (($recs | ForEach-Object { Rec-Title $_ }) -join ' | ')
        Check 'S11 still at most 15 notifications of ours after the storm' ($recs.Count -le 15) "$($recs.Count) records"
        $script:Timings["S11 notifications of the app after $BurstFiles transfers"] = "$($recs.Count) (cap 15)"
        Phone-Screenshot '11-transfer-storm.png' -Shade
    } catch { Scenario-Failed 'S11' $_ } }

    # ---------------------------------------------------------------- SX: a probe, run only with -Only SX
    # A file is shared while Kin.exe is stopped, and Kin.exe is back within the upload's wait for a connection: how long does the phone take?
    if ($Only -contains 'SX') {
        Say 'SX: share while Kin.exe is stopped, start it again at once'
        Make-PhoneFiles 'e2e14-sx.bin:100000'
        $script:Names.Add('e2e14-sx.bin')
        $tids0 = @(Phone-Ids)
        Stop-Kin14
        Wait-Until { (Phone-State)['link'] -notlike 'Connected*' } 20 | Out-Null
        Share-Files 'e2e14-sx.bin'
        Start-Sleep -Seconds 1
        $swX = [Diagnostics.Stopwatch]::StartNew()
        Start-Kin14
        Connect-Phone14
        $okX = Wait-Until { (Get-Status).state -eq 'phone_connected' } 90
        Check 'SX the phone is connected again' $okX ("{0:N1} s after Kin.exe was started" -f $swX.Elapsed.TotalSeconds)
        $script:Timings['SX share while the PC is down, PC back after 1 s -> phone connected'] = "{0:N1} s" -f $swX.Elapsed.TotalSeconds
        Show-PhoneLink14
        $upX = Phone-NewUpload $tids0
        Check 'SX the file arrives' (Wait-Until { (Pc-HistoryItem $upX.Id).status -eq 'done' } 60)
    }

    # ---------------------------------------------------------------- S13: logs and Export logs
    if (Want 'S13') { try {
        Say 'S13: logs'
        $pcAll = Pc-Log
        # The debug build's command hooks echo what they show (file names in TRANSFER and RECEIVED lines): those `debug: CMD` lines are the
        # harness talking to itself and are in no release build, so the ordinary lines are what is checked.
        $phAll = ((Phone-Log) -split "`r?`n" | Where-Object { $_ -notmatch ' debug: CMD ' }) -join "`n"
        $lc = (A logcat -d -s KinRelay | Where-Object { $_ -notmatch 'CMD ' } | Out-String)
        $status = Get-Content (Join-Path $Data 'status.json') -Raw
        $leaks = @()
        foreach ($t in $script:Texts) {
            $word = ($t -split ' ' | Select-Object -Skip 1 | Select-Object -First 4) -join ' '
            foreach ($pair in @(@('PC log', $pcAll), @('phone log', $phAll), @('logcat', $lc), @('status.json', $status))) {
                if ($pair[1].Contains($t) -or ($word.Length -gt 12 -and $pair[1].Contains($word))) { $leaks += "$($pair[0]): '$($t.Substring(0, [Math]::Min(30, $t.Length)))'" }
            }
        }
        Check 'S13 no text content in the PC log, phone log, logcat or status.json (sensitive, automatic and ordinary texts)' ($leaks.Count -eq 0) ($leaks -join '; ')
        $tok = ([regex]::Match((Get-Content (Join-Path $Data 'pair-url.txt') -Raw), 'tk=([^&\s]+)')).Groups[1].Value
        Check 'S13 no token, tk= or private key anywhere' ($tok.Length -eq 22 -and -not ($pcAll + $phAll + $lc + $status).Contains($tok) -and ($pcAll + $phAll + $lc) -notmatch 'PRIVATE KEY|tk=')
        $pcNames = @($script:Names | Where-Object { $nm = $_; $pcAll.Contains($nm) })
        $phNames = @($script:Names | Where-Object { $nm = $_; $phAll.Contains($nm) -or $lc.Contains($nm) })
        Check 'S13 no file name in the PC log (hashes only, toasts included)' ($pcNames.Count -eq 0) (($pcNames | Select-Object -First 5) -join ', ')
        Check 'S13 no file name in the phone log or logcat' ($phNames.Count -eq 0) (($phNames | Select-Object -First 5) -join ', ')
        $script:Timings['S13 file names found in the logs (names used in 1.4)'] = "PC log $($pcNames.Count) of $($script:Names.Count), phone $($phNames.Count) of $($script:Names.Count)"
        if ((Want 'S1') -and (Want 'S2')) {
            Check 'S13 the PC log and the phone log carry ids, hashes and counts (the queue lines are there)' (($pcAll -match 'queued text \w+: sent and acked') -and ($phAll -match 'queue: text \S+ delivered'))
        } else {
            Say 'S13: the queue lines are not checked (S1 and S2 did not both run)'
        }

        # PC: Export logs (the developer hook does what the button does, without the dialog)
        $zip = Join-Path $script:W14 'pc-logs.zip'
        Invoke-Kin --dev-export-logs $zip
        Wait-Until { Test-Path -LiteralPath $zip } 15 | Out-Null
        Start-Sleep -Milliseconds 500
        Check 'S13 PC: Export logs wrote a zip' (Test-Path -LiteralPath $zip) "$zip"
        if (Test-Path -LiteralPath $zip) {
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $z = [IO.Compression.ZipFile]::OpenRead($zip)
            $entries = @($z.Entries | ForEach-Object { $_.FullName })
            $all = ($z.Entries | ForEach-Object { $s = New-Object IO.StreamReader($_.Open()); try { $s.ReadToEnd() } finally { $s.Dispose() } }) -join "`n"
            $z.Dispose()
            Say "  PC export entries: $($entries -join ', ')"
            $bad = @()
            $pcName = (Get-Status).pcName
            if ($all -match '\b(?:\d{1,3}\.){3}\d{1,3}\b') { $bad += "IP address $($Matches[0])" }
            if ($pcName -and $all.Contains($pcName)) { $bad += 'PC name' }
            if ($phoneName -and $all.Contains($phoneName)) { $bad += 'phone name' }
            if ($all.Contains($env:USERNAME) -or $all.Contains($env:USERPROFILE)) { $bad += 'Windows user name or profile path' }
            foreach ($nm in $script:Names) { if ($all.Contains($nm)) { $bad += "file name $nm" } }
            foreach ($t in $script:Texts) { if ($all.Contains($t)) { $bad += 'a text' } }
            if ($all.Contains($tok) -or $all -match 'PRIVATE KEY|tk=') { $bad += 'token or key' }
            Check 'S13 PC: the export is redacted (no IP address, PC or phone name, user name, file name, text, token or key)' ($bad.Count -eq 0) ($bad -join '; ')
            Check 'S13 PC: the export has the log, status.json, debug.json and a README' (($entries | Where-Object { $_ -like '*kin*.log' }).Count -ge 1 -and 'status.json' -in $entries -and 'debug.json' -in $entries -and ($entries | Where-Object { $_ -like 'README*' }))
        }

        # phone: Settings > Debug > Export logs, then the zip is read from cache/exports
        # This step used to fail after the earlier scenarios: they leave the app on whatever screen they last drove (a detail screen, the setup
        # screen, a dialog), and the old code assumed the tabs were showing. Now: go to the Home tab first (BACK until the tabs are there), then to
        # Settings, scroll until "Debug" is on screen, and look at the exports folder to see that a new zip came, instead of waiting a fixed time.
        $listing = ''
        for ($attempt = 1; $attempt -le 2 -and $listing -notmatch 'kin-logs-.*\.zip'; $attempt++) {
            $before = @(([regex]::Matches((Run-As 'ls cache/exports'), 'kin-logs-[^\s]+\.zip') | ForEach-Object { $_.Value }))
            if (-not (Open-PhoneHomeTab)) { Say "S13: could not get the phone to its Home tab (attempt $attempt)" }
            $ok = Tap-UiText 'Settings'
            $ok = $ok -and (Wait-Until { A shell input swipe 540 1700 540 600 300 | Out-Null; Start-Sleep -Milliseconds 500; A shell uiautomator dump /sdcard/kin-e2e-ui.xml 2>$null | Out-Null; (Ui-Texts) -contains 'Debug' } 45)   # 2B.1 to 2B.5 made Settings six screens long: "Debug" is at the very bottom
            $ok = $ok -and (Tap-UiText 'Debug')
            $ok = $ok -and (Wait-Until { A shell uiautomator dump /sdcard/kin-e2e-ui.xml 2>$null | Out-Null; (Ui-Texts) -contains 'Export logs' } 8)
            $ok = $ok -and (Tap-UiText 'Export logs')
            Wait-Until { $script:ls = Run-As 'ls cache/exports'; @([regex]::Matches($script:ls, 'kin-logs-[^\s]+\.zip') | Where-Object { $_.Value -notin $before }).Count -gt 0 } 15 | Out-Null
            $listing = $script:ls
            if ($listing -notmatch 'kin-logs-.*\.zip') { Say "S13: no export zip yet after attempt $attempt (taps ok: $ok)" }
        }
        Check 'S13 phone: Export logs made a zip in cache/exports' ($listing -match 'kin-logs-.*\.zip') "$($listing.Trim())"
        $zipName = ([regex]::Match($listing, 'kin-logs-[^\s]+\.zip')).Value
        if ($zipName) {
            $local = Join-Path $script:W14 'phone-logs.zip'
            # exec-out is binary-safe, and Start-Process hands its output handle to the process as it is
            Start-Process $Adb -ArgumentList @('-s', $Serial, 'exec-out', 'run-as', $Pkg, 'cat', "cache/exports/$zipName") -RedirectStandardOutput $local -Wait -NoNewWindow
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $z = [IO.Compression.ZipFile]::OpenRead($local)
            $entries = @($z.Entries | ForEach-Object { $_.FullName })
            $all = ($z.Entries | ForEach-Object { $s = New-Object IO.StreamReader($_.Open()); try { $s.ReadToEnd() } finally { $s.Dispose() } }) -join "`n"
            $z.Dispose()
            Say "  phone export entries: $($entries -join ', ')"
            $all = ($all -split "`r?`n" | Where-Object { $_ -notmatch ' debug: CMD ' }) -join "`n"   # the debug build's own command echo, as above
            $bad = @()
            foreach ($nm in $script:Names) { if ($all.Contains($nm)) { $bad += "file name $nm" } }
            foreach ($t in $script:Texts) { if ($all.Contains($t)) { $bad += 'a text' } }
            if ($all.Contains($tok) -or $all -match 'PRIVATE KEY|tk=') { $bad += 'token or key' }
            Check 'S13 phone: the export has no file name, text, token or key' ($bad.Count -eq 0) ($bad -join '; ')
            Check 'S13 phone: the export has the log and state.txt' ($entries -contains 'state.txt' -and ($entries | Where-Object { $_ -like 'kin*.log*' }).Count -ge 1) ($entries -join ', ')
            $script:Timings['S13 phone export entries'] = ($entries -join ', ')
        }
        Run-As 'rm -rf cache/exports' | Out-Null
        A shell input keyevent KEYCODE_BACK | Out-Null; A shell input keyevent KEYCODE_BACK | Out-Null
    } catch { Scenario-Failed 'S13' $_ } }

    A shell am start -n "$Pkg/.MainActivity" | Out-Null
    Start-Sleep -Seconds 1
    if (Tap-UiText 'History') { Start-Sleep -Seconds 1; Phone-Screenshot '99-history-end.png' }
}
