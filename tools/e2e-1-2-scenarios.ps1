# The milestone 1.2 scenarios (text, links, clipboard) for tools\e2e-1-1.ps1 -Milestone 1.2. Dot-sourced by that script, which owns
# the setup, the teardown and the helpers used here (A, Say, Check, Wait-Until, Get-Status, Phone-Cmd, Invoke-EmuScript, $Kin, $Data, $Pkg).
# Don't run it by itself: use tools\e2e-1-2.ps1.
#
# Real PC clipboard: it is saved before anything touches it (Save-PcClipboard) and put back in the teardown (Restore-PcClipboard),
# after Kin has quit so that the restored text is not synced to the phone.

$script:Timings = [ordered]@{}        # name -> text, printed at the end
$script:Texts = New-Object System.Collections.Generic.List[string]   # every text used, so the logs can be searched for them
$script:Run = Get-Date -Format 'HHmmss'
$script:SavedClip = $null
$script:ClipSaved = $false
$script:ClipBackupFile = Join-Path $env:TEMP 'kin-e2e-clipboard-backup.txt'

function Use-Text([string]$t) { $script:Texts.Add($t); $t }
function Esc-Cr([string]$s) { ($s -replace "`r", '\r' -replace "`n", '\n') }

# ---------------------------------------------------------------- the PC clipboard
# Every format is saved and put back (desktop\clipboard-backup.ps1: text in all its encodings, HTML, images, file lists, the password-manager
# marks). Since 1.3 this replaces the plain-text-only backup: a clipboard that holds a web page copy no longer stops the run.
# -DiscardClipboard is kept for the old callers and does nothing now.
if (-not (Get-Command Save-Clipboard -ErrorAction SilentlyContinue)) { . (Join-Path $Repo 'desktop\clipboard-backup.ps1') }   # defines Save-Clipboard, Restore-Clipboard, Get-ClipboardFormats
function Save-PcClipboard([bool]$Discard) {
    Add-Type -AssemblyName System.Windows.Forms
    $script:PcClipBackup = Save-Clipboard | Select-Object -Last 1
    $script:ClipSaved = $true
}
function Restore-PcClipboard {
    if (-not $script:ClipSaved) { return }
    $ok = $false
    try { $ok = [bool](Restore-Clipboard $script:PcClipBackup | Select-Object -Last 1) } catch { Say "restore failed: $_" }
    Check 'PC clipboard restored' $ok
    $script:ClipSaved = $false
}
function Pc-Clip { try { Get-Clipboard -Raw } catch { $null } }

# ---------------------------------------------------------------- reading the two sides
function Pc-Log { (Get-ChildItem (Join-Path $Data 'logs') -Filter 'kin*.log' -ErrorAction SilentlyContinue | Sort-Object Name | ForEach-Object { Get-Content $_.FullName -Raw }) -join "`n" }
function Pc-LogCount([string]$Pattern) { ([regex]::Matches((Pc-Log), $Pattern)).Count }
function Phone-Log { (A shell run-as $Pkg cat files/logs/kin.log | Out-String) }
function Phone-LogCount([string]$Pattern) { ([regex]::Matches((Phone-Log), $Pattern)).Count }

# The text on the phone's clipboard, read by a focused debug activity (the only way an app may read it on Android 10+).
function Phone-Clip {
    A logcat -c | Out-Null
    A shell am start -n "$Pkg/.debug.DebugClipboardActivity" | Out-Null
    $line = $null
    Wait-Until { $script:cl = (A logcat -d -s KinRelay) | Where-Object { $_ -match 'CMD CLIPBOARD' } | Select-Object -Last 1; [bool]$script:cl } 8 | Out-Null
    $line = $script:cl
    # 1.3 added `sensitive=<bool>` between the hash and the text
    if (-not $line -or $line -notmatch 'CMD CLIPBOARD len=(?<len>\d+) sha=(?<sha>\w+)(?: sensitive=(?<sens>\w+))? text="(?s)(?<text>.*)"\s*$') { return $null }
    $len = [int]$Matches['len']; $sha = $Matches['sha']; $sens = ($Matches['sens'] -eq 'true')
    # the debug hook escapes line ends as \n and \r and a backslash as \\
    $t = [regex]::Replace($Matches['text'], '\\(.)', { param($m) switch ($m.Groups[1].Value) { 'n' { "`n" } 'r' { "`r" } 't' { "`t" } default { $m.Groups[1].Value } } })
    [pscustomobject]@{ Len = $len; Sha = $sha; Text = $t; Sensitive = $sens }
}

# The ack goes out before the phone writes the clipboard, so a read right after an ack may still see the old text: wait for it.
function Wait-PhoneClip([string]$Expected, [int]$Seconds = 6) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    do { $script:pcp = Phone-Clip } while (-not ($script:pcp -and $script:pcp.Text -ceq $Expected) -and $sw.Elapsed.TotalSeconds -lt $Seconds)
    $script:pcp
}

# One phone notification by its tag (the message id): title, text and the names of its action buttons.
function Phone-Notification([string]$Tag) {
    $d = A shell dumpsys notification --noredact | Out-String
    $r = ($d -split 'NotificationRecord\(') | Where-Object { $_ -match "tag=$([regex]::Escape($Tag)) " } | Select-Object -First 1
    # 1.4: the texts from the PC are one notification (id 3, no tag) that the app updates: "Text from <PC>" for one text, "N texts from <PC>" after that
    if (-not $r) { $r = ($d -split 'NotificationRecord\(') | Where-Object { $_ -match 'pkg=app\.kinrelay\.android ' -and $_ -match ' id=3 tag=null ' } | Select-Object -First 1 }
    if (-not $r) { return $null }
    $actions = @()
    if ($r -match '(?s)actions=\{(.*?)\}\s+extras') { $actions = @([regex]::Matches($Matches[1], '\[\d+\] "([^"]+)"') | ForEach-Object { $_.Groups[1].Value }) }
    $title = if ($r -match 'android\.title=String \(([^\r\n]*?)\)\r?\n') { $Matches[1] } else { '' }
    [pscustomobject]@{ Title = $title; Raw = $r; Actions = $actions }
}

# Taps the node with this exact text, found with uiautomator. Returns $false when it isn't on screen.
function Tap-UiText([string]$Text, [int]$Tries = 4) {
    for ($try = 0; $try -lt $Tries; $try++) {   # uiautomator now and then fails to dump ("null root node", "could not get idle state")
        A shell uiautomator dump /sdcard/kin-e2e-ui.xml 2>$null | Out-Null
        $x = A shell cat /sdcard/kin-e2e-ui.xml | Out-String
        if ($x -match ('text="{0}"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"' -f [regex]::Escape($Text))) {
            A shell input tap ([int](([int]$Matches[1] + [int]$Matches[3]) / 2)) ([int](([int]$Matches[2] + [int]$Matches[4]) / 2)) | Out-Null
            return $true
        }
        Start-Sleep -Milliseconds 700
    }
    return $false
}
function Ui-Texts { $x = A shell cat /sdcard/kin-e2e-ui.xml | Out-String; @([regex]::Matches($x, 'text="([^"]+)"') | ForEach-Object { $_.Groups[1].Value }) }

# ---------------------------------------------------------------- PC -> phone
# Copies the text on the PC like an app would, waits for the automatic `clipboard` to be acked. Returns the status object, with .WallMs.
function Copy-OnPc([string]$Text, [int]$TimeoutSec = 8) {
    $before = (Get-Status).lastSent.clipboard.id
    $sw = [Diagnostics.Stopwatch]::StartNew()
    Set-QuietClipboardText $Text
    $lc = $null
    Wait-Until { $script:lc = (Get-Status).lastSent.clipboard; $script:lc -and $script:lc.id -ne $before -and $script:lc.result } $TimeoutSec | Out-Null
    $ms = $sw.ElapsedMilliseconds
    $lc = $script:lc
    if ($lc) { $lc | Add-Member -NotePropertyName WallMs -NotePropertyValue $ms -Force }
    $lc
}
function Phone-SeenTexts {
    # 1.4: opening the app starts the text notification afresh (the next text is "Text from <PC>" again, not "2 texts from <PC>")
    A shell input keyevent KEYCODE_HOME | Out-Null
    A shell am start -n "$Pkg/.MainActivity" | Out-Null
    Start-Sleep -Milliseconds 800
}
function Hotkey-Send([int]$TimeoutSec = 8) {
    Phone-SeenTexts
    $before = (Get-Status).lastSent.text.id
    Invoke-Kin --send-clipboard
    Wait-Until { $script:lt = (Get-Status).lastSent.text; $script:lt -and $script:lt.id -ne $before -and $script:lt.result } $TimeoutSec | Out-Null
    $script:lt
}

# ---------------------------------------------------------------- phone -> PC
function Share-Intent([string]$Text) { A shell "am start -n $Pkg/.share.ShareActivity -a android.intent.action.SEND -t text/plain --es android.intent.extra.TEXT '$Text'" | Out-Null }
function Process-Text([string]$Text) { A shell "am start -n $Pkg/.share.ProcessTextActivity -a android.intent.action.PROCESS_TEXT -t text/plain --es android.intent.extra.PROCESS_TEXT '$Text'" | Out-Null }
function Clipboard-Alias { A shell am start -n "$Pkg/.debug.SendClipboardAlias" | Out-Null }
function Set-PhoneClip([string]$Text) { A logcat -c | Out-Null; A shell "am broadcast -a app.kinrelay.android.debug.SET_CLIPBOARD -n $Pkg/.debug.DebugCommandReceiver --es text '$Text'" | Out-Null; Start-Sleep -Milliseconds 500 }
# The "Send clipboard" button of the service notification, tapped for real: shade down, the notification expanded, a tap.
function Tap-NotificationAction {
    # This used to fail now and then: the shade opens at a speed that depends on the emulator, the first uiautomator dump of a just-opened shade can
    # be empty, and the service notification is sometimes collapsed. So: start from a quiet home screen, wait for the shade by looking (up to 5 s)
    # instead of for a fixed time, expand the notification's row when it is collapsed, and try five times with the shade closed in between.
    for ($try = 0; $try -lt 5; $try++) {
        A shell cmd statusbar collapse | Out-Null
        A shell input keyevent KEYCODE_HOME | Out-Null
        Start-Sleep -Milliseconds 700
        A shell cmd statusbar expand-notifications | Out-Null
        $found = Wait-Until { Tap-UiText 'Send clipboard' 1 } 5
        if ($found) { Start-Sleep -Milliseconds 300; A shell cmd statusbar collapse | Out-Null; return $true }
        # the service notification is collapsed: find its row and pull it down
        A shell uiautomator dump /sdcard/kin-e2e-ui.xml | Out-Null
        $x = A shell cat /sdcard/kin-e2e-ui.xml | Out-String
        if ($x -match 'text="Connected to [^"]*"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"' -or $x -match 'text="Kin"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"') {
            $y = [int]$Matches[2]
            A shell input swipe 600 $y 600 ($y + 320) 300 | Out-Null
            if (Wait-Until { Tap-UiText 'Send clipboard' 1 } 3) { Start-Sleep -Milliseconds 300; A shell cmd statusbar collapse | Out-Null; return $true }
        }
        A shell cmd statusbar collapse | Out-Null
        Start-Sleep -Seconds 1
    }
    return $false
}
# Runs a phone action and waits for the PC to receive a new `text`. Returns [pscustomobject]{ Got; WallMs; PhoneMs; Clip }.
function Send-From-Phone([scriptblock]$Action, [string]$Expect) {
    $before = (Get-Status).lastReceived.text.id
    $toasts = Pc-LogCount 'toast: Text from '
    $sw = [Diagnostics.Stopwatch]::StartNew()
    & $Action
    $got = Wait-Until { $script:r = (Get-Status).lastReceived.text; $script:r -and $script:r.id -ne $before } 10
    $ms = $sw.ElapsedMilliseconds
    $ph = Phone-Log
    $tap = if ($ph -match '(?s).*text sent \(\w+\)[^\n]*tap to ack (\d+) ms') { [int]$Matches[1] } else { $null }
    Start-Sleep -Milliseconds 400
    [pscustomobject]@{ Got = $got; WallMs = $ms; PhoneMs = $tap; Clip = (Pc-Clip); Status = $script:r; Toast = ((Pc-LogCount 'toast: Text from ') -gt $toasts) }
}
# Share one web link from the phone to a PC that knows `link` (2B.1): the PC logs "link <id> from device <d>: N characters, opened in the default browser" and, with
# --dev-no-open (every e2e Kin has it), the mocked opener logs that it would have opened it. Nothing starts a browser.
function Share-LinkFromPhone([string]$Url) {
    $opened = 'link \S+ from device \S+: \d+ characters, opened in the default browser'
    $mock = 'would have been opened in the browser'
    $o0 = Pc-LogCount $opened; $m0 = Pc-LogCount $mock
    Share-Intent $Url
    $got = Wait-Until { (Pc-LogCount $opened) -gt $o0 } 15
    [pscustomobject]@{ Got = $got; Mocked = ((Pc-LogCount $mock) -gt $m0) }
}
function Crlf([string]$s) { [regex]::Replace($s, "(?<!\r)\n", "`r`n") }

# ================================================================ scenario 10, before the pairing
# An external link must show the confirmation and must not pair until it is confirmed.
function Test-ConfirmationScreen([string]$Url, $Pc) {
    Say 'scenario 10: pairing link without auto-confirm'
    A shell am start -a android.intent.action.VIEW -d "'$Url'" 2>$null | Out-Null
    $shown = Wait-Until { A shell uiautomator dump /sdcard/kin-e2e-ui.xml | Out-Null; (Ui-Texts) -contains 'Pair with this PC?' } 15
    Check 'S10 link without AUTO_CONFIRM shows the confirmation screen' $shown
    $short = $Pc.shortId.Substring(0, 4) + '-' + $Pc.shortId.Substring(4)
    $named = Wait-Until { A shell uiautomator dump /sdcard/kin-e2e-ui.xml | Out-Null; $script:uit = Ui-Texts; ($script:uit -contains $Pc.pcName) -and ($script:uit -contains "PC ID: $short") } 10
    Check 'S10 screen shows the PC name and its short ID' $named "$($Pc.pcName) $short (screen: $($script:uit -join ' | '))"
    $shot = $null
    if ($ScreenshotDir) {
        New-Item -ItemType Directory -Force $ScreenshotDir | Out-Null
        $shot = Join-Path $ScreenshotDir '01-confirm-pairing.png'
        Invoke-EmuScript 'screenshot.ps1' @('-Out', ('"{0}"' -f $shot)) 60 | Out-Null
        Check 'S10 screenshot taken' ((Test-Path $shot) -and (Get-Item $shot).Length -gt 5000) $shot
    }
    Start-Sleep -Seconds 4
    $s = Get-Status
    Check 'S10 nothing paired while the screen is up' ($s.state -eq 'not_paired' -and @($s.devices).Count -eq 0 -and $s.pairing.active)
    Check 'S10 Cancel is on the screen and works' (Tap-UiText 'Cancel')
    Start-Sleep -Seconds 2
    $s = Get-Status
    Check 'S10 still not paired after Cancel (the code is still valid)' ($s.state -eq 'not_paired' -and @($s.devices).Count -eq 0 -and $s.pairing.active)
}
# After the unpair: the same link, confirmed by a real tap on Confirm.
function Test-ConfirmByTap {
    Say 'scenario 10: confirm by tap'
    $urlFile = Join-Path $Data 'pair-url.txt'
    try { [IO.File]::Delete($urlFile) } catch { }   # the first pairing's URL is still there (Remove-Item trips over the 8.3 %TEMP% path)
    Invoke-Kin --dev-start-pairing
    Wait-Until { Test-Path $urlFile } 10 | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    A shell am start -a android.intent.action.VIEW -d "'$url'" 2>$null | Out-Null
    $shown = Wait-Until { A shell uiautomator dump /sdcard/kin-e2e-ui.xml | Out-Null; (Ui-Texts) -contains 'Confirm' } 15
    Check 'S10 confirmation screen again after the unpair' $shown
    Check 'S10 tap on Confirm' (Tap-UiText 'Confirm')
    Check 'S10 paired after Confirm' (Wait-Until { (Get-Status).state -eq 'phone_connected' } 40)
}

# ================================================================ scenarios 1 to 6 and 11, in the middle
function Run-Milestone12 {
    $n = $script:Run
    $sync0 = (Get-Status).clipboardSync.sent

    # ---- 1. PC copy -> phone clipboard, under 1 s
    Say 'scenario 1: PC copy -> phone clipboard'
    $t1 = Use-Text "e2e-$n pc copy one"
    $lc = Copy-OnPc $t1
    Check 'S1 clipboard message acked' ($lc -and $lc.result -eq 'acked' -and $lc.trigger -eq 'clipboard') "$($lc.result)"
    Check 'S1 tap to ack under 1 s' ($lc -and $lc.tapToAckMs -lt 1000 -and $lc.WallMs -lt 1000) ("tap to ack {0} ms, Set-Clipboard to ack {1} ms, round trip {2} ms" -f $lc.tapToAckMs, $lc.WallMs, $lc.rttMs)
    $script:Timings['S1 PC copy -> phone clipboard (tap to ack, includes the 300 ms debounce)'] = "$($lc.tapToAckMs) ms (wall $($lc.WallMs) ms)"
    $pc = Wait-PhoneClip $t1
    Check 'S1 phone clipboard has the text' ($pc -and $pc.Text -ceq $t1) "len=$($pc.Len)"
    Check 'S1 exactly one clipboard message for the copy' (((Get-Status).clipboardSync.sent - $sync0) -eq 1)

    # ---- 2. password manager marks: not sent
    Say 'scenario 2: password-manager copy'
    foreach ($mark in 'exclude', 'no-cloud') {
        $secret = Use-Text "e2e-$n secret $mark"
        $s0 = Get-Status
        Invoke-Kin --dev-set-clipboard $secret --dev-clipboard-mark $mark
        Start-Sleep -Milliseconds 1500   # debounce 300 ms, then the read
        $s1 = Get-Status
        Check "S2 --dev-clipboard-mark $mark is not sent" ($s1.clipboardSync.sent -eq $s0.clipboardSync.sent -and $s1.clipboardSync.skipped -gt $s0.clipboardSync.skipped -and "$($s1.clipboardSync.lastSkip.reason)" -like 'password_manager:*') "$($s1.clipboardSync.lastSkip.reason)"
    }
    $pc = Phone-Clip
    Check 'S2 the phone clipboard still has the old text' ($pc -and $pc.Text -ceq $t1)

    # ---- 3. the hotkey path (--send-clipboard runs the same code as the hotkey)
    Say 'scenario 3: hotkey path'
    $url3 = Use-Text "https://example.com/e2e?run=$n&k=hotkey"
    Copy-OnPc $url3 | Out-Null
    $tx = Hotkey-Send
    Check 'S3 hotkey path: text acked' ($tx -and $tx.result -eq 'acked' -and $tx.chars -eq $url3.Length) "tap to ack $($tx.tapToAckMs) ms"
    $script:Timings['S3 hotkey -> phone text (tap to ack)'] = "$($tx.tapToAckMs) ms"
    $pc = Wait-PhoneClip $url3
    Check 'S3 phone clipboard set' ($pc -and $pc.Text -ceq $url3) $(if ($pc) { "read len=$($pc.Len), expected $($url3.Length)" } else { 'no clipboard line from the phone' })
    $nt = $null; Wait-Until { $script:nt = Phone-Notification $tx.id; [bool]$script:nt } 8 | Out-Null; $nt = $script:nt
    Check 'S3 phone notification posted ("Text from ...")' ($nt -and $nt.Title -like 'Text from *') "$($nt.Title)"
    Check 'S3 the URL notification has an Open action' ($nt -and $nt.Actions -contains 'Open') "$($nt.Actions -join ',')"
    $plain3 = Use-Text "e2e-$n plain words from the hotkey"
    Copy-OnPc $plain3 | Out-Null
    $tx = Hotkey-Send
    $nt = $null; Wait-Until { $script:nt = Phone-Notification $tx.id; [bool]$script:nt } 8 | Out-Null; $nt = $script:nt
    # Since 2B.1 every text notification has a Copy button ("Copy latest" when grouped); only a link would have Open.
    Check 'S3 plain text: notification without an Open action (a Copy button is there since 2B.1)' ($tx.result -eq 'acked' -and $nt -and $nt.Actions -notcontains 'Open' -and @($nt.Actions | Where-Object { $_ -notmatch '^Copy' }).Count -eq 0) "$($nt.Actions -join ',')"

    # ---- 4. phone "Send to PC": share, PROCESS_TEXT, clipboard alias, notification action. 5. loop prevention on the PC side
    Say 'scenario 4: phone -> PC'
    $paths = @(
        @{ Name = 'share intent'; Wall = $true; Text = (Use-Text "e2e-$n shared words"); Do = { Share-Intent "e2e-$n shared words" } },
        @{ Name = 'PROCESS_TEXT'; Wall = $true; Text = (Use-Text "e2e-$n selected words"); Do = { Process-Text "e2e-$n selected words" } },
        @{ Name = 'clipboard alias'; Text = (Use-Text "e2e-$n alias clip"); Do = { Set-PhoneClip "e2e-$n alias clip"; Clipboard-Alias } },
        @{ Name = 'notification action'; Text = (Use-Text "e2e-$n notification clip"); Do = { Set-PhoneClip "e2e-$n notification clip"; if (-not (Tap-NotificationAction)) { Say 'could not find the Send clipboard button' } } }
    )
    foreach ($p in $paths) {
        $s0 = (Get-Status).clipboardSync
        $echo0 = Pc-LogCount 'not sent back'
        $r = Send-From-Phone $p.Do $p.Text
        Check "S4 $($p.Name): the PC received it" $r.Got
        Check "S4 $($p.Name): the PC clipboard has the text (Ctrl+V)" ($r.Clip -ceq $p.Text) "len=$($r.Clip.Length)"
        Check "S4 $($p.Name): a toast was logged" $r.Toast
        # the adb start to status.json time only means something when adb starts the activity itself; the two other paths include setting the clipboard and the taps
        $script:Timings["S4 $($p.Name) -> PC clipboard (phone tap to ack$(if ($p.Wall) { ' / adb start to status.json' }))"] = "$($r.PhoneMs) ms$(if ($p.Wall) { " / $($r.WallMs) ms" })"
        Start-Sleep -Milliseconds 1200
        $s1 = (Get-Status).clipboardSync
        Check "S5 $($p.Name): the PC did not send it back" ($s1.sent -eq $s0.sent -and (Pc-LogCount 'not sent back') -gt $echo0 -and $s1.lastSkip.reason -eq 'echo_of_phone_text') "sent $($s0.sent) -> $($s1.sent), $($s1.lastSkip.reason)"
    }

    # ---- 5. loop prevention on the phone side
    Say 'scenario 5: the phone does not bounce the PC clipboard back'
    $t5 = Use-Text "e2e-$n set by the pc"
    Copy-OnPc $t5 | Out-Null
    $received0 = (Get-Status).lastReceived.text.id
    $sent0 = Phone-LogCount 'text sent \('
    Clipboard-Alias
    Start-Sleep -Milliseconds 2500
    Check 'S5 the phone skipped the send of what the PC just set' ((Phone-Log) -match 'send skipped \(CLIPBOARD\)')
    Check 'S5 nothing came back to the PC' ((Get-Status).lastReceived.text.id -eq $received0 -and (Phone-LogCount 'text sent \(') -eq $sent0)
    $pc = Wait-PhoneClip $t5
    Check 'S5 the phone clipboard is still the PC text' ($pc -and $pc.Text -ceq $t5)

    # ---- 6. URL handling, both ways
    Say 'scenario 6: links'
    $urls = @(
        @{ Text = "https://example.org/e2e?run=$n"; Open = $true },
        @{ Text = "HTTP://Example.ORG/e2e-$n"; Open = $true },
        @{ Text = "javascript:alert($n)"; Open = $false },
        @{ Text = "file:///C:/Windows/System32/calc-$n.exe"; Open = $false },
        @{ Text = "intent://e2e$n#Intent;scheme=http;end"; Open = $false },
        @{ Text = "ms-msdt:/id e2e$n"; Open = $false }
    )
    foreach ($u in $urls) {
        $txt = Use-Text $u.Text
        $toasts = Pc-LogCount 'click opens the link'
        $script:CurrentText = $txt
        if ($u.Open) {
            # Since 2B.1 a phone that shares one web link sends a `link` message (not a `text`), and the PC opens it by itself (the setting is on by default);
            # this run's Kin has --dev-no-open, so nothing starts a browser: the PC logs the decision and what the opener would have done.
            $lr = Share-LinkFromPhone $txt
            Check "S6 phone -> PC [$txt]: a link message, opened by the PC (mocked opener), no text toast" ($lr.Got -and $lr.Mocked) "opened-line=$($lr.Got) mocked-opener=$($lr.Mocked)"
        } else {
            $r = Send-From-Phone { Share-Intent $script:CurrentText } $txt
            $opens = ((Pc-LogCount 'click opens the link') -gt $toasts)
            Check "S6 phone -> PC [$txt]: isUrl/openable as expected, link toast only for http(s)" ($r.Got -and $r.Status.isUrl -eq $u.Open -and $r.Status.openable -eq $u.Open -and $opens -eq $u.Open) "isUrl=$($r.Status.isUrl) openable=$($r.Status.openable) linkToast=$opens"
        }
        Copy-OnPc $txt | Out-Null
        $tx = Hotkey-Send
        $nt = $null; Wait-Until { $script:nt = Phone-Notification $tx.id; [bool]$script:nt } 8 | Out-Null; $nt = $script:nt
        $hasOpen = [bool]($nt -and $nt.Actions -contains 'Open')
        Check "S6 PC -> phone [$txt]: Open action only for http(s)" ($tx.result -eq 'acked' -and $nt -and $hasOpen -eq $u.Open) "actions=$($nt.Actions -join ',')"
    }

    # ---- 8. large text, PC -> phone. Since 1.3 (protocol.md, "Text messages") a sender sends a `text` or `clipboard` message only up to 128 KiB
    # (131,072 bytes of UTF-8): a longer clipboard is not sent (`too_large`), a longer hotkey text goes as a .txt file (1.3's S10c checks the file).
    # The receivers' own limit is still 1 MiB, but no sender uses it any more, so there is nothing to send above 128 KiB as a message.
    Say 'scenario 8: large text'
    $lc = Copy-OnPc ('c' * 131072) 15
    Check 'S8 clipboard of exactly 128 KiB: acked' ($lc -and $lc.result -eq 'acked') "tap to ack $($lc.tapToAckMs) ms"
    Check 'S8 ... and written to the phone clipboard' (Wait-Until { (Phone-Log) -match 'clipboard from the PC len=131072 sha=\w+ written=true' } 8)
    $tx = Hotkey-Send 15
    Check 'S8 hotkey text of exactly 128 KiB is sent as a text message and acked' ($tx -and $tx.result -eq 'acked' -and $tx.chars -eq 131072 -and -not $tx.asFile) "tap to ack $($tx.tapToAckMs) ms asFile=$($tx.asFile)"
    $script:Timings['S8 hotkey text of 131,072 characters (tap to ack)'] = "$($tx.tapToAckMs) ms"
    Check 'S8 ... and the phone copied it' (Wait-Until { (Phone-Log) -match "text from '$($script:PcName)' len=131072 sha=\w+ isUrl=false openable=false written=true" } 8)
    $lc = Copy-OnPc ('c' * 131073) 15
    Check 'S8 a clipboard over 128 KiB is not sent (too_large)' ($lc -and $lc.result -eq 'too_large') "$($lc.result)"
    $tx = Hotkey-Send 15
    Check 'S8 a hotkey text over 128 KiB goes as a .txt file offer instead' ($tx -and $tx.result -eq 'acked' -and $tx.asFile -eq $true) "result=$($tx.result) asFile=$($tx.asFile)"
    # ---- 11. logs
    Say 'scenario 11: logs'
    $pcAll = Pc-Log
    $phAll = Phone-Log
    # `CMD CLIPBOARD` is the debug activity that prints the clipboard to logcat on purpose; every other line must be free of text
    $lc = (A logcat -d -s KinRelay | Where-Object { $_ -notmatch 'CMD CLIPBOARD' } | Out-String)
    $status = Get-Content (Join-Path $Data 'status.json') -Raw
    $leaks = @()
    foreach ($t in $script:Texts) {
        $word = ($t -split ' ' | Select-Object -Skip 1 | Select-Object -First 3) -join ' '
        foreach ($pair in @(@('PC log', $pcAll), @('phone log', $phAll), @('logcat', $lc), @('status.json', $status))) {
            if ($pair[1].Contains($t) -or ($word.Length -gt 8 -and $pair[1].Contains($word))) { $leaks += "$($pair[0]): '$t'" }
        }
    }
    Check 'S11 no clipboard or text content in either log, logcat or status.json' ($leaks.Count -eq 0) ($leaks -join '; ')
    $tok = ([regex]::Match((Get-Content (Join-Path $Data 'pair-url.txt') -Raw), 'tk=([^&\s]+)')).Groups[1].Value
    Check 'S11 no token, tk= or key in the 1.2 logs' ($tok.Length -eq 22 -and -not ($pcAll + $phAll + $lc).Contains($tok) -and ($pcAll + $phAll + $lc) -notmatch 'PRIVATE KEY|tk=')
    Check 'S11 the logs do carry lengths and hashes' (($pcAll -match 'hash [0-9a-f]{8}') -and ($phAll -match 'sha=[0-9a-f]{8}'))
}
