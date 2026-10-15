# The milestone 1.3 scenarios (files in both directions, and the 1.2 review's text rules) for tools\e2e-1-1.ps1 -Milestone 1.3.
# Dot-sourced by that script, after e2e-1-2-scenarios.ps1, and it uses the helpers of both (A, Say, Check, Wait-Until, Get-Status,
# Phone-Cmd, Phone-Log, Pc-Log, Phone-Clip, Phone-Notification, $Kin, $KinArgs, $Data, $Pkg, $Repo). Don't run it by itself: use tools\e2e-1-3.ps1.
#
# No non-ASCII character is written in this file (Windows PowerShell reads a BOM-less script as ANSI): names are built from [char] codes.
#
# What it touches: the real PC clipboard (every format is saved first by Save-AllClipboard and put back by Restore-AllClipboard, after Kin
# has quit), a work folder under %LOCALAPPDATA%\Temp (removed at the end), the received folder inside the throwaway data dir, and on the phone
# `files/debug-share`, `Pictures/Kin`, `Movies/Kin` and `Download/Kin` (cleaned by Clear-Phone13).

if (-not (Get-Command Save-Clipboard -ErrorAction SilentlyContinue)) { . (Join-Path $Repo 'desktop\clipboard-backup.ps1') }   # Save-Clipboard, Restore-Clipboard, Get-ClipboardFormats (e2e-1-2-scenarios.ps1 loads it first)
[Console]::OutputEncoding = [Text.Encoding]::UTF8     # adb prints file names in UTF-8
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

$script:W13 = Join-Path $env:LOCALAPPDATA ('Temp\kin-e2e13-' + $script:Run)   # source files on the PC
$script:RecvDir = Join-Path $Data 'received'
$script:ClipBackup = $null
$script:Names = New-Object System.Collections.Generic.List[string]   # names used, to count how many log lines carry a file name

function Want([string]$Id) { $ids = @($Only | ForEach-Object { $_ -split ',' } | Where-Object { $_ }); $ids.Count -eq 0 -or $ids -contains $Id }   # -File passes "S1,S2" as one string

# ---------------------------------------------------------------- the PC clipboard, every format
function Save-AllClipboard { $script:ClipBackup = Save-Clipboard | Select-Object -Last 1 }
function Restore-AllClipboard {
    if ($null -eq $script:ClipBackup) { return }
    $ok = $false
    try { $ok = [bool](Restore-Clipboard $script:ClipBackup | Select-Object -Last 1) } catch { Say "restore failed: $_" }
    Check 'PC clipboard restored (every format)' $ok
    $script:ClipBackup = $null
}

# ---------------------------------------------------------------- small helpers
function Esc([string]$s) { -join ($s.ToCharArray() | ForEach-Object { if ([int]$_ -gt 126 -or [int]$_ -lt 32) { '\u{0:X4}' -f [int]$_ } else { [string]$_ } }) }
function U([int]$cp) { [string][char]$cp }
function Cjk([int]$n) { (U 0x4E2D) * $n }
function Long([string]$Path) { if ($Path.StartsWith('\\?\')) { $Path } else { '\\?\' + $Path } }
# .NET, not Get-FileHash: PowerShell's providers don't open a \\?\ path (the only way to reach a file called CON.txt)
function Pc-Sha([string]$Path) {
    $fs = [IO.File]::OpenRead((Long $Path))
    try { ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($fs)) -replace '-', '').ToLower() } finally { $fs.Dispose() }
}
function Pc-Recv { @(Get-ChildItem -LiteralPath $script:RecvDir -Force -ErrorAction SilentlyContinue) }
function Pc-Parts { @(Pc-Recv | Where-Object { $_.Name -like '*.part' }) }
function Make-PcFile([string]$Path, [long]$Bytes, [int]$Seed = 7) {
    $rnd = New-Object Random $Seed
    $buf = New-Object byte[] (1MB)
    $fs = [IO.File]::Create((Long $Path))
    try { $left = $Bytes; while ($left -gt 0) { $rnd.NextBytes($buf); $n = [int][Math]::Min($left, $buf.Length); $fs.Write($buf, 0, $n); $left -= $n } } finally { $fs.Dispose() }
}
function Make-PcJpeg([string]$Path, [int]$W, [int]$H) {
    $bmp = New-Object System.Drawing.Bitmap $W, $H
    $rnd = New-Object Random 11
    for ($y = 0; $y -lt $H; $y += 4) { for ($x = 0; $x -lt $W; $x += 4) { $bmp.SetPixel($x, $y, [System.Drawing.Color]::FromArgb($rnd.Next(256), $rnd.Next(256), $rnd.Next(256))) } }
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Jpeg); $bmp.Dispose()
}

# ---------------------------------------------------------------- the phone
function Phone-Bcast([string]$Command, [string]$Extras = '', [int]$Wait = 1200) {
    A logcat -c | Out-Null
    A shell "am broadcast -a app.kinrelay.android.debug.$Command -n $Pkg/.debug.DebugCommandReceiver $Extras" | Out-Null
    Start-Sleep -Milliseconds $Wait
    @(A logcat -d -s KinRelay)
}
# A shell script (ASCII, any quoting) run as the app: it is piped in, because a script in the command line can't carry quotes through adb.exe
function Run-As([string]$Script) { ($Script | & $Adb -s $Serial shell "run-as $Pkg sh" | Out-String) }
# name -> sha256 of every file of a folder on the phone (the shell can read /sdcard)
function Phone-DirHashes([string]$Dir) {
    $h = @{}
    $out = (A shell "cd '$Dir' 2>/dev/null && sha256sum *" | Out-String)
    foreach ($l in ($out -split "`r?`n")) { if ($l -match '^([0-9a-f]{64})  (.+)$') { $h[$Matches[2]] = $Matches[1] } }
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
# A big random file made on the phone itself (MAKE_FILES keeps a file in memory, so it is not for hundreds of MB)
function Make-PhoneBig([string]$Name, [int]$Mb) {
    Run-As "mkdir -p files/debug-share && head -c $($Mb * 1048576) /dev/urandom > files/debug-share/$Name" | Out-Null
}
function Share-Files([string]$Names) { A shell "am start -n $Pkg/.debug.DebugShareActivity --es files '$Names'" | Out-Null }
function Phone-Transfers {
    # "CMD TRANSFERS count=N" comes first, then N lines; the database can be slow to answer while a transfer is writing
    $lines = @(Phone-Bcast 'TRANSFERS' '' 1000)
    for ($i = 0; $i -lt 8; $i++) {
        $want = if (($lines -join "`n") -match 'CMD TRANSFERS count=(\d+)') { [int]$Matches[1] } else { -1 }
        if ($want -ge 0 -and @($lines | Where-Object { $_ -match 'CMD TRANSFER \S+ (up|down) ' }).Count -ge $want) { break }
        Start-Sleep -Milliseconds 700
        $lines = @(A logcat -d -s KinRelay)
    }
    foreach ($l in $lines) {
        # 1.4 added ` retriedAs=<id or null>` and then ` userRetry= started= owed= completeAcked=`: a field that is added later must not break the parse
        if ($l -match 'CMD TRANSFER (\S+) (up|down) (\w+) (\d+)/(\d+) files=(\d+) names=(.*) err=(\S*)(?: retriedAs=(\S*))?(?: .*)?$') {
            [pscustomobject]@{ Id = $Matches[1]; Dir = $Matches[2]; Status = $Matches[3]; Done = [long]$Matches[4]; Total = [long]$Matches[5]; Files = [int]$Matches[6]; Names = $Matches[7]; Err = $Matches[8]; RetriedAs = $Matches[9] }
        }
    }
}
function Phone-Received {
    # the command prints one line per entry (or "none") with no end marker, and a query right after a big download can be slow: wait until the output stops growing
    $lines = @(Phone-Bcast 'RECEIVED' '' 1500)
    for ($i = 0; $i -lt 6; $i++) {
        $more = @(A logcat -d -s KinRelay)
        if ($more.Count -eq $lines.Count -and ($more | Where-Object { $_ -match 'CMD RECEIVED' })) { break }
        $lines = $more; Start-Sleep -Milliseconds 700
    }
    foreach ($l in $lines) {
        if ($l -match "CMD RECEIVED (images|video|downloads) id=(\d+) name='(.*)' size=(\d+) mime=(\S*) path=(\S+) pending=(\d)") {
            [pscustomobject]@{ Kind = $Matches[1]; Name = $Matches[3]; Size = [long]$Matches[4]; Mime = $Matches[5]; Path = $Matches[6]; Pending = [int]$Matches[7] }
        }
    }
}
function Phone-NewTransfer([string[]]$Before, [string]$Dir, [int]$Sec = 90) {
    $script:pnt = $null
    Wait-Until { $script:pnt = @(Phone-Transfers) | Where-Object { $_.Id -notin $Before -and $_.Dir -eq $Dir -and $_.Status -in 'done', 'failed', 'canceled' } | Select-Object -First 1; [bool]$script:pnt } $Sec | Out-Null
    $script:pnt
}
# For a failed check: what the phone logged about transfers lately
function Show-PhoneTransferLog { Say 'phone transfer log (last lines):'; (Phone-Log) -split "`r?`n" | Where-Object { $_ -match ' transfer:| share:| link: ' } | Select-Object -Last 25 | ForEach-Object { Say "  $_" } }
function Phone-Ids {@(Phone-Transfers | ForEach-Object { $_.Id }) }
function Phone-Notifs { (A shell dumpsys notification --noredact | Out-String) }
function Wifi([bool]$On) { A shell svc wifi $(if ($On) { 'enable' } else { 'disable' }) | Out-Null }

# ---------------------------------------------------------------- the PC
function Pc-Ids { $s = Get-Status; @(@($s.transfers.active) + @($s.transfers.recent)) | Where-Object { $_ } | ForEach-Object { $_.id } }
function Wait-NewPcTransfer([string[]]$Before, [string]$Dir, [int]$Sec = 90) {
    $script:npt = $null
    Wait-Until { $s = Get-Status; $script:npt = @($s.transfers.recent) | Where-Object { $_ -and $_.id -notin $Before -and ($Dir -eq '' -or $_.direction -eq $Dir) } | Select-Object -First 1; [bool]$script:npt } $Sec | Out-Null
    $script:npt
}
function Wait-PcActive([string]$Dir, [long]$MinBytes, [int]$Sec = 60) {
    $script:pac = $null
    Wait-Until { $s = Get-Status; $script:pac = @($s.transfers.active) | Where-Object { $_ -and $_.direction -eq $Dir -and $_.bytesDone -ge $MinBytes } | Select-Object -First 1; [bool]$script:pac } $Sec | Out-Null
    $script:pac
}
function Pc-Send([string[]]$Paths) { & $Kin --send @Paths }
function Restart-Kin([string[]]$Extra = @()) {
    $port = (Get-Status).port
    & $Kin --quit 2>$null
    Wait-Until { -not (Get-Process Kin -ErrorAction SilentlyContinue) } 15 | Out-Null
    Start-Sleep -Milliseconds 700
    Start-Process $Kin -ArgumentList ($KinArgs + @('--dev-port', "$port") + $Extra) | Out-Null
    Wait-Until { (Get-Status).state -in 'phone_offline', 'phone_connected' } 30 | Out-Null
    Phone-Cmd 'CONNECT' | Out-Null   # the phone is in its backoff after the close: connect now
    Wait-Until { (Get-Status).state -eq 'phone_connected' } 60
}
function Set-PcClip([string]$Text) { Set-Clipboard -Value $Text; Start-Sleep -Milliseconds 700 }

# Android allows an app 50 notifications at once and silently drops the next ones (seen on API 29 after a long run: a text from the PC got no
# notification). A force-stop removes them all; the service is started again and the phone connects at once.
function Reset-PhoneNotifications {
    A shell am force-stop $Pkg | Out-Null
    Start-Sleep -Seconds 1
    # a foreground start: Android 12 and later refuse to start the foreground service from a broadcast while the app is in the background
    A shell am start -n "$Pkg/.MainActivity" | Out-Null
    Start-Sleep -Seconds 3
    Phone-Cmd 'CONNECT' | Out-Null
    # the PC keeps saying phone_connected for a while after the phone's process died, so ask the phone itself
    Wait-Until { (Phone-State)['link'] -like 'Connected*' } 60 | Out-Null
    Start-Sleep -Seconds 2
}

# ---------------------------------------------------------------- the clean-up
function Clear-Phone13 {
    Wifi $true   # a scenario that failed half way may have left it off, and the emulator snapshot would keep that
    Phone-Bcast 'DELETE_RECEIVED' '' 1500 | Out-Null
    Run-As 'rm -rf files/debug-share cache/queue' | Out-Null
    $left = @(Phone-Received | Where-Object { $_.Name -like 'e2e*' -or $_.Name -like 'Kin text*' -or $_.Name -eq 'Text.txt' })
    Say ("phone cleaned: {0} test file(s) left in Pictures, Movies and Download" -f $left.Count)
}
function Remove-Work13 {
    if (Test-Path -LiteralPath (Long $script:W13)) { try { [IO.Directory]::Delete((Long $script:W13), $true) } catch { Say "could not remove $($script:W13): $_" } }
}

# ================================================================ the scenarios
function Run-Milestone13 {
    New-Item -ItemType Directory -Force $script:W13 | Out-Null
    $n = $script:Run
    $big = $BigMb
    $phoneName = (Get-Status).devices[0].name

    # ---------------------------------------------------------------- S1, S2: phone -> PC
    if (Want 'S1') {
        Say 'S1: phone -> PC, one photo'
        Make-PhoneFiles 'e2e-photo.jpg@1600x1200'
        $src = Phone-SourceHashes
        $before = Pc-Ids; $pbefore = Phone-Ids
        $sw = [Diagnostics.Stopwatch]::StartNew()
        Share-Files 'e2e-photo.jpg'
        $t = Wait-NewPcTransfer $before 'in' 60
        Check 'S1 the PC finished the transfer' ($t -and $t.status -eq 'done') "$($t.status) $($t.error)"
        $f = Join-Path $script:RecvDir 'e2e-photo.jpg'
        Check 'S1 the photo is in the received folder' (Test-Path -LiteralPath $f)
        Check 'S1 SHA-256 equals the phone source' ((Test-Path -LiteralPath $f) -and (Pc-Sha $f) -eq $src['e2e-photo.jpg']) "$($src['e2e-photo.jpg'])"
        Check 'S1 no .part file left' ((Pc-Parts).Count -eq 0)
        $pt = Phone-NewTransfer $pbefore 'up' 30
        Check 'S1 the phone says done' ($pt -and $pt.Status -eq 'done') "$($pt.Status) $($pt.Err)"
        Check 'S1 a toast was logged' (Wait-Until { (Pc-LogCount 'toast: 1 file from ') -ge 1 } 10)
        $log = Pc-Log
        Check 'S1 the file finished exactly once on the PC (the phone still sends its empty PUT only when complete)' ((([regex]::Matches($log, 'file 1 is complete')).Count) -eq 1)
        $formats = @(Get-ClipboardFormats | ForEach-Object { $_.Name })
        Check 'S1 the PC clipboard has CF_DIBV5 and CF_HDROP' (($formats -contains 'CF_DIBV5') -and ($formats -contains 'CF_HDROP')) ($formats -join ',')
        $img = $null; try { $img = [Windows.Forms.Clipboard]::GetImage() } catch { }
        Check 'S1 Ctrl+V would paste a 1600 x 1200 picture' ($img -and $img.Width -eq 1600 -and $img.Height -eq 1200) $(if ($img) { "$($img.Width)x$($img.Height)" } else { 'no image' })
        if ($img) { $img.Dispose() }
        $drop = $null; try { $drop = @([Windows.Forms.Clipboard]::GetFileDropList()) } catch { }
        Check 'S1 the clipboard file list points at the received file' ($drop.Count -eq 1 -and $drop[0] -eq $f) "$drop"
        $script:Timings['S1 phone -> PC one photo (share to PC done)'] = "{0:N1} s" -f $sw.Elapsed.TotalSeconds
    }
    if (Want 'S2') {
        Say 'S2: phone -> PC, 50 photos'
        $names = 1..50 | ForEach-Object { 'e2e-batch-{0:D2}.jpg' -f $_ }
        Make-PhoneFiles (($names | ForEach-Object { "$_@800x600" }) -join ',')
        $src = Phone-SourceHashes
        $before = Pc-Ids; $pbefore = Phone-Ids
        $sw = [Diagnostics.Stopwatch]::StartNew()
        Share-Files ($names -join ',')
        $t = Wait-NewPcTransfer $before 'in' 180
        $secs = $sw.Elapsed.TotalSeconds
        Check 'S2 the PC finished the 50-file transfer' ($t -and $t.status -eq 'done' -and $t.files -eq 50) "$($t.status) files=$($t.files) $($t.error)"
        $bad = @($names | Where-Object { $p = Join-Path $script:RecvDir $_; -not ((Test-Path -LiteralPath $p) -and (Pc-Sha $p) -eq $src[$_]) })
        Check 'S2 all 50 hashes match, names unchanged' ($bad.Count -eq 0) ($bad -join ', ')
        Check 'S2 no .part file left' ((Pc-Parts).Count -eq 0)
        Check 'S2 nothing else in the folder but the photo of S1 and the 50' ((@(Pc-Recv | Where-Object { $_.Name -like 'e2e-batch-*' }).Count) -eq 50)
        $pt = Phone-NewTransfer $pbefore 'up' 30
        Check 'S2 the phone says done, 50 files' ($pt -and $pt.Status -eq 'done' -and $pt.Files -eq 50) "$($pt.Status) $($pt.Files)"
        Check 'S2 one toast, "50 files from"' (Wait-Until { (Pc-LogCount 'toast: 50 files from ') -ge 1 } 10)
        $mb = (($names | ForEach-Object { (Get-Item -LiteralPath (Join-Path $script:RecvDir $_)).Length } | Measure-Object -Sum).Sum) / 1MB
        $script:Timings['S2 phone -> PC 50 photos'] = "{0:N1} s, {1:N1} MB, {2:N2} MB/s (emulator, not representative)" -f $secs, $mb, ($mb / $secs)
    }

    # ---------------------------------------------------------------- S3: phone -> PC, a big file with Wi-Fi off and on
    if (Want 'S3') {
        Say "S3: phone -> PC, $big MB with Wi-Fi off and on"
        Make-PhoneBig 'e2e-big3.bin' $big
        $src = Phone-SourceHashes
        $before = Pc-Ids; $pbefore = Phone-Ids
        $sw = [Diagnostics.Stopwatch]::StartNew()
        Share-Files 'e2e-big3.bin'
        $a = Wait-PcActive 'in' ([long]($big * 1MB * 0.25)) 180
        Check 'S3 the upload is running and a quarter is in' ($null -ne $a) "$($a.bytesDone) of $($a.totalBytes)"
        Wifi $false
        Start-Sleep -Seconds 6
        $kept = [long](@((Get-Status).transfers.active) | Where-Object { $_ -and $_.direction -eq 'in' } | Select-Object -First 1).bytesDone
        Check 'S3 the PC kept what arrived (more than a quarter, not everything)' ($kept -ge ($big * 1MB * 0.25) -and $kept -lt ($big * 1MB)) "$kept bytes"
        Check 'S3 the phone noticed the drop (and waits for the PC)' (Wait-Until { (Phone-Log) -match 'upload \S+: (the connection ended; waiting for the PC|\w+Exception)' } 20)
        Wifi $true
        $t = Wait-NewPcTransfer $before 'in' 300
        $secs = $sw.Elapsed.TotalSeconds
        Check 'S3 the transfer finished after the drop' ($t -and $t.status -eq 'done') "$($t.status) $($t.error)"
        # the PC logs where a resumed upload carries on ("file 1 continues from byte N of M"): it must be what it kept, not 0
        $from = if ((Pc-Log) -match 'upload \S+: file 1 continues from byte (\d+) of (\d+)') { [long]$Matches[1] } else { -1 }
        Check 'S3 it resumed from the offset the PC kept (not from 0)' ($from -ge $kept -and $from -lt ($big * 1MB)) "kept $kept, continued from $from"
        $f = Join-Path $script:RecvDir 'e2e-big3.bin'
        Check 'S3 SHA-256 equals the phone source' ((Test-Path -LiteralPath $f) -and (Pc-Sha $f) -eq $src['e2e-big3.bin'])
        Check 'S3 no .part file left' ((Pc-Parts).Count -eq 0)
        $script:Timings["S3 phone -> PC $big MB with one Wi-Fi drop (includes about 10 s of drop)"] = "{0:N1} s, {1:N2} MB/s overall (emulator, not representative)" -f $secs, ($big / $secs)
    }

    # ---------------------------------------------------------------- S4: PC -> phone, one photo
    if (Want 'S4') {
        Say 'S4: PC -> phone, one photo'
        $p = Join-Path $script:W13 'e2e-pc-photo.jpg'
        Make-PcJpeg $p 1280 960
        $sha = Pc-Sha $p; $size = (Get-Item -LiteralPath $p).Length
        $before = Pc-Ids; $pbefore = Phone-Ids
        $sw = [Diagnostics.Stopwatch]::StartNew()
        Pc-Send @($p)
        $pt = Phone-NewTransfer $pbefore 'down' 90
        Check 'S4 the phone finished the download' ($pt -and $pt.Status -eq 'done') "$($pt.Status) $($pt.Err)"
        $rec = @(Phone-Received) | Where-Object { $_.Name -eq 'e2e-pc-photo.jpg' }
        Check 'S4 it is in MediaStore Images, in Pictures/Kin/, IS_PENDING cleared' ($rec -and $rec.Kind -eq 'images' -and $rec.Path -eq 'Pictures/Kin/' -and $rec.Pending -eq 0) "$($rec.Kind) $($rec.Path) pending=$($rec.Pending)"
        Check 'S4 MediaStore knows the size' ($rec -and $rec.Size -eq $size) "$($rec.Size) vs $size"
        $h = Phone-DirHashes '/sdcard/Pictures/Kin'
        Check 'S4 SHA-256 on the phone equals the PC file' ($h['e2e-pc-photo.jpg'] -eq $sha)
        $t = Wait-NewPcTransfer $before 'out' 30
        Check 'S4 the PC received complete (the transfer is done)' ($t -and $t.status -eq 'done') "$($t.status) $($t.error)"
        Check 'S4 the PC log says the phone has all the files' ((Pc-Log) -match 'the phone has all the files of offer')
        Check 'S4 the phone showed "Photo from <PC>"' ((Phone-Notifs) -match 'Photo from ')
        $script:Timings['S4 PC -> phone one photo (send to done)'] = "{0:N1} s" -f $sw.Elapsed.TotalSeconds
    }

    # ---------------------------------------------------------------- S5: PC -> phone, a mixed batch
    if (Want 'S5') {
        Say 'S5: PC -> phone, a mixed batch'
        $d = Join-Path $script:W13 'mix'; New-Item -ItemType Directory -Force $d | Out-Null
        Make-PcJpeg (Join-Path $d 'e2e-mix.jpg') 800 600
        Make-PcFile (Join-Path $d 'e2e-mix.mp4') 3000000 21
        [IO.File]::WriteAllText((Join-Path $d 'e2e-mix.pdf'), ('%PDF-1.4 ' + ('pdf body ' * 5000)))
        Make-PcFile (Join-Path $d 'e2e-mix.zip') 250000 22
        $expect = @(
            @{ Name = 'e2e-mix.jpg'; Kind = 'images'; Path = 'Pictures/Kin/'; Dir = '/sdcard/Pictures/Kin' },
            @{ Name = 'e2e-mix.mp4'; Kind = 'video'; Path = 'Movies/Kin/'; Dir = '/sdcard/Movies/Kin' },
            @{ Name = 'e2e-mix.pdf'; Kind = 'downloads'; Path = 'Download/Kin/'; Dir = '/sdcard/Download/Kin' },
            @{ Name = 'e2e-mix.zip'; Kind = 'downloads'; Path = 'Download/Kin/'; Dir = '/sdcard/Download/Kin' }
        )
        $before = Pc-Ids; $pbefore = Phone-Ids
        Pc-Send ($expect | ForEach-Object { Join-Path $d $_.Name })
        $pt = Phone-NewTransfer $pbefore 'down' 90
        Check 'S5 the phone finished the download of 4 files' ($pt -and $pt.Status -eq 'done' -and $pt.Files -eq 4) "$($pt.Status) $($pt.Files) $($pt.Err)"
        $rec = @(Phone-Received)
        foreach ($e in $expect) {
            $r = $rec | Where-Object { $_.Name -eq $e.Name } | Select-Object -First 1
            $h = Phone-DirHashes $e.Dir
            $want = Pc-Sha (Join-Path $d $e.Name)
            Check "S5 $($e.Name): $($e.Path), MediaStore $($e.Kind), not pending, hash equal" ($r -and $r.Kind -eq $e.Kind -and $r.Path -eq $e.Path -and $r.Pending -eq 0 -and $h[$e.Name] -eq $want) "$($r.Kind) $($r.Path) pending=$($r.Pending)"
        }
        $t = Wait-NewPcTransfer $before 'out' 30
        Check 'S5 the PC received complete' ($t -and $t.status -eq 'done' -and $t.files -eq 4) "$($t.status)"
        Check 'S5 the phone showed "4 files from <PC>"' ((Phone-Notifs) -match '4 files from ')
    }

    # ---------------------------------------------------------------- S6: PC -> phone, a big file with Wi-Fi off and on
    if (Want 'S6') {
        Say "S6: PC -> phone, $big MB with Wi-Fi off and on"
        $p = Join-Path $script:W13 'e2e-big6.bin'
        Make-PcFile $p ($big * 1MB) 31
        $sha = Pc-Sha $p
        $before = Pc-Ids; $pbefore = Phone-Ids
        $sw = [Diagnostics.Stopwatch]::StartNew()
        Pc-Send @($p)
        $a = Wait-PcActive 'out' ([long]($big * 1MB * 0.25)) 180
        Check 'S6 the download is running and a quarter is across' ($null -ne $a) "$($a.bytesDone) of $($a.totalBytes)"
        Wifi $false
        Start-Sleep -Seconds 6
        Check 'S6 the phone noticed the drop (and waits for the PC)' (Wait-Until { (Phone-Log) -match 'download \S+: (the connection ended; waiting for the PC|\w+Exception)' } 20)
        $kept = [long](@((Get-Status).transfers.active) | Where-Object { $_ -and $_.direction -eq 'out' } | Select-Object -First 1).bytesDone
        Wifi $true
        $t = Wait-NewPcTransfer $before 'out' 300
        $secs = $sw.Elapsed.TotalSeconds
        Check 'S6 the PC saw complete after the drop' ($t -and $t.status -eq 'done') "$($t.status) $($t.error)"
        # the PC logs a ranged GET ("file 1 is served from byte N of M"): N is what the phone had saved, so not 0 and not more than the PC had handed over
        $from = if ((Pc-Log) -match 'offer \S+: file 1 is served from byte (\d+) of (\d+)') { [long]$Matches[1] } else { -1 }
        Check 'S6 the phone resumed with a Range from the bytes it had (not from 0)' ($from -gt 0 -and $from -le $kept) "the PC had handed over $kept, the Range started at $from"
        $h = Phone-DirHashes '/sdcard/Download/Kin'
        Check 'S6 SHA-256 on the phone equals the PC file' ($h['e2e-big6.bin'] -eq $sha)
        $rec = @(Phone-Received) | Where-Object { $_.Name -eq 'e2e-big6.bin' }
        Check 'S6 not pending, size right' ($rec -and $rec.Pending -eq 0 -and $rec.Size -eq ($big * 1MB)) "$($rec.Size)"
        $script:Timings["S6 PC -> phone $big MB with one Wi-Fi drop (includes about 10 s of drop)"] = "{0:N1} s, {1:N2} MB/s overall (emulator, not representative)" -f $secs, ($big / $secs)
    }

    # ---------------------------------------------------------------- S7: cancel from each side, in both directions
    if (Want 'S7') {
        Say 'S7: cancel'
        $cancelMb = 100   # big enough that a cancel always arrives mid-transfer, also where the emulator is faster (API 29)
        # 7a phone -> PC, canceled on the phone
        Make-PhoneBig 'e2e-cancel7a.bin' $cancelMb
        $before = Pc-Ids; $pbefore = Phone-Ids
        Share-Files 'e2e-cancel7a.bin'
        $a = Wait-PcActive 'in' 3MB 90
        $ptx = @(Phone-Transfers) | Where-Object { $_.Id -notin $pbefore -and $_.Dir -eq 'up' -and $_.Status -eq 'active' } | Select-Object -First 1
        Check 'S7a the upload is running on both sides' ($a -and $ptx) ""
        if ($ptx) { Phone-Bcast 'CANCEL_TRANSFER' "--es id $($ptx.Id)" 600 | Out-Null }
        $pt = Phone-NewTransfer $pbefore 'up' 30
        Check 'S7a phone: canceled' ($pt -and $pt.Status -eq 'canceled') "$($pt.Status) $($pt.Err)"
        $t = Wait-NewPcTransfer $before 'in' 30
        Check 'S7a PC: canceled' ($t -and $t.status -eq 'canceled') "$($t.status) $($t.error)"
        Check 'S7a PC: no .part and no partial file left' (Wait-Until { @(Pc-Recv | Where-Object { $_.Name -like 'e2e-cancel7a*' }).Count -eq 0 } 10)
        Check 'S7a phone: the progress notification is gone' (Wait-Until { -not ((Phone-Notifs) -match 'e2e-cancel7a') } 10)
        # 7b phone -> PC, canceled on the PC
        Make-PhoneBig 'e2e-cancel7b.bin' $cancelMb
        $before = Pc-Ids; $pbefore = Phone-Ids
        Share-Files 'e2e-cancel7b.bin'
        $a = Wait-PcActive 'in' 3MB 90
        Check 'S7b the upload is running' ($null -ne $a)
        & $Kin --cancel-transfers
        $t = Wait-NewPcTransfer $before 'in' 30
        Check 'S7b PC: canceled' ($t -and $t.status -eq 'canceled') "$($t.status) $($t.error)"
        $pt = Phone-NewTransfer $pbefore 'up' 30
        Check 'S7b phone: canceled by the PC, not failed' ($pt -and $pt.Status -eq 'canceled') "$($pt.Status) $($pt.Err)"
        Check 'S7b PC: no .part and no partial file left' (Wait-Until { @(Pc-Recv | Where-Object { $_.Name -like 'e2e-cancel7b*' }).Count -eq 0 } 10)
        Check 'S7b phone: the progress notification is gone' (Wait-Until { -not ((Phone-Notifs) -match 'e2e-cancel7b') } 10)
        # 7c PC -> phone, canceled on the PC
        $p = Join-Path $script:W13 'e2e-cancel7c.bin'; Make-PcFile $p ($cancelMb * 1MB) 41
        $before = Pc-Ids; $pbefore = Phone-Ids
        Pc-Send @($p)
        $a = Wait-PcActive 'out' 3MB 90
        Check 'S7c the download is running' ($null -ne $a)
        & $Kin --cancel-transfers
        $t = Wait-NewPcTransfer $before 'out' 30
        Check 'S7c PC: canceled' ($t -and $t.status -eq 'canceled') "$($t.status) $($t.error)"
        $pt = Phone-NewTransfer $pbefore 'down' 30
        Check 'S7c phone: canceled' ($pt -and $pt.Status -eq 'canceled') "$($pt.Status) $($pt.Err)"
        Check 'S7c phone: the partial file is gone from MediaStore' (Wait-Until { @(Phone-Received | Where-Object { $_.Name -like 'e2e-cancel7c*' }).Count -eq 0 } 10)
        Check 'S7c phone: the progress notification is gone' (Wait-Until { -not ((Phone-Notifs) -match 'e2e-cancel7c') } 10)
        # 7d PC -> phone, canceled on the phone
        $p = Join-Path $script:W13 'e2e-cancel7d.bin'; Make-PcFile $p ($cancelMb * 1MB) 42
        $before = Pc-Ids; $pbefore = Phone-Ids
        Pc-Send @($p)
        $a = Wait-PcActive 'out' 3MB 90
        $ptx = @(Phone-Transfers) | Where-Object { $_.Id -notin $pbefore -and $_.Dir -eq 'down' -and $_.Status -eq 'active' } | Select-Object -First 1
        Check 'S7d the download is running on both sides' ($a -and $ptx)
        if ($ptx) { Phone-Bcast 'CANCEL_TRANSFER' "--es id $($ptx.Id)" 600 | Out-Null }
        $pt = Phone-NewTransfer $pbefore 'down' 30
        Check 'S7d phone: canceled' ($pt -and $pt.Status -eq 'canceled') "$($pt.Status) $($pt.Err)"
        $t = Wait-NewPcTransfer $before 'out' 30
        Check 'S7d PC: canceled by the phone' ($t -and $t.status -eq 'canceled') "$($t.status) $($t.error)"
        Check 'S7d phone: the partial file is gone from MediaStore' (Wait-Until { @(Phone-Received | Where-Object { $_.Name -like 'e2e-cancel7d*' }).Count -eq 0 } 10)
        Check 'S7d PC: nothing is still active' (@((Get-Status).transfers.active).Count -eq 0)
    }

    # ---------------------------------------------------------------- S8: errors
    if (Want 'S8') {
        Say 'S8: disk_full and source_changed'
        # disk_full: the PC pretends to have 1,000,000 bytes free
        $ok = Restart-Kin @('--dev-free-bytes', '1000000')
        Check 'S8 Kin restarted with --dev-free-bytes and the phone is back' $ok
        Make-PhoneFiles 'e2e-full.bin:3000000,e2e-fits.bin:900000'
        $src = Phone-SourceHashes
        $pbefore = Phone-Ids; $before = Pc-Ids
        Share-Files 'e2e-full.bin'
        $pt = Phone-NewTransfer $pbefore 'up' 40
        Check 'S8 disk_full: the phone shows the transfer as failed with disk_full' ($pt -and $pt.Status -eq 'failed' -and $pt.Err -eq 'disk_full') "$($pt.Status) $($pt.Err)"
        Check 'S8 disk_full: the phone says why in a notification' (Wait-Until { (Phone-Notifs) -match 'enough free space' } 10)
        $failedLines = ([regex]::Matches((Phone-Log), 'upload \S+ failed: disk_full')).Count
        $refusals = Pc-LogCount 'not enough free space for 3000000 bytes'
        Start-Sleep -Seconds 20
        $all = @(Phone-Transfers)
        $again = @($all | Where-Object { $_.Dir -eq 'up' -and $_.Id -notin $pbefore })
        if ($again.Count -ne 1) { Say "  phone transfers now: $(($all | ForEach-Object { "$($_.Dir) $($_.Status) $($_.Err) $($_.Id)" }) -join ' | '); before: $($pbefore -join ',')" }
        $nFailed = ([regex]::Matches((Phone-Log), 'upload \S+ failed: disk_full')).Count
        $nRefused = Pc-LogCount 'not enough free space for 3000000 bytes'
        $created = @(Pc-Recv | Where-Object { $_.Name -like 'e2e-full*' }).Count
        Check 'S8 disk_full: no retry (one failed transfer, the PC refused once and still once, nothing created on the PC)' ($again.Count -eq 1 -and $again[0].Status -eq 'failed' -and $nFailed -eq $failedLines -and $refusals -eq 1 -and $nRefused -eq 1 -and $created -eq 0) "transfers=$($again.Count) status=$($again[0].Status) failed lines $failedLines->$nFailed, PC refusals $refusals->$nRefused, files created $created"
        $pbefore = Phone-Ids
        Share-Files 'e2e-fits.bin'
        $pt = Phone-NewTransfer $pbefore 'up' 40
        Check 'S8 disk_full: a file that fits is taken' ($pt -and $pt.Status -eq 'done') "$($pt.Status) $($pt.Err)"
        Check 'S8 disk_full: ... and arrives intact' ((Test-Path -LiteralPath (Join-Path $script:RecvDir 'e2e-fits.bin')) -and (Pc-Sha (Join-Path $script:RecvDir 'e2e-fits.bin')) -eq $src['e2e-fits.bin'])
        $ok = Restart-Kin @()
        Check 'S8 Kin restarted without the hook and the phone is back' $ok

        # source_changed: the file changes on the PC while the phone's download is cut
        $p = Join-Path $script:W13 'e2e-changed.bin'
        Make-PcFile $p ($big * 1MB) 51
        $before = Pc-Ids; $pbefore = Phone-Ids
        Pc-Send @($p)
        $a = Wait-PcActive 'out' ([long]($big * 1MB * 0.2)) 180
        Check 'S8 source_changed: the download is running' ($null -ne $a)
        Wifi $false
        Start-Sleep -Seconds 5
        # a different size (and modified time). The PC still has the file open for the old connection, so share it
        $fs = [IO.File]::Open($p, [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite); $fs.WriteByte(1); $fs.Dispose()
        Wifi $true
        $pt = Phone-NewTransfer $pbefore 'down' 120
        Check 'S8 source_changed: the phone fails the transfer with source_changed' ($pt -and $pt.Status -eq 'failed' -and $pt.Err -eq 'source_changed') "$($pt.Status) $($pt.Err)"
        $t = Wait-NewPcTransfer $before 'out' 40
        Check 'S8 source_changed: the PC got the cancel (reason error)' ($t -and $t.status -eq 'canceled') "$($t.status) $($t.error)"
        Check 'S8 source_changed: the phone deleted its partial file' (Wait-Until { @(Phone-Received | Where-Object { $_.Name -like 'e2e-changed*' }).Count -eq 0 } 10)
        Check 'S8 source_changed: the phone says why in a notification' ((Phone-Notifs) -match 'changed on ')
        Say 'S8: an expired offer was not run (offers last 24 hours and no hook shortens that; the desktop test offers_e2e covers it)'
    }

    # ---------------------------------------------------------------- S9: tricky file names, both ways
    if (Want 'S9') {
        Say 'S9: file names'
        # phone -> PC. Raw names use %uXXXX; for one code point (the debug hooks decode them); the wanted names follow protocol.md, "File names".
        $dec = { param($s) [regex]::Replace($s, '%u([0-9A-Fa-f]{4,6});', { param($m) [char]::ConvertFromUtf32([Convert]::ToInt32($m.Groups[1].Value, 16)) }) }
        $cjk83 = ('%u4E2D;' * 83) + '.txt'
        $cases = @(
            @{ Raw = 'CON.txt'; Want = '_CON.txt' },
            @{ Raw = 'rtl%u202E;gpj.exe'; Want = 'rtl_gpj.exe' },
            @{ Raw = $cjk83; Want = ((Cjk 65) + '.txt') },
            @{ Raw = 'tail.txt.%u0020;'; Want = 'tail.txt' },
            @{ Raw = 'dir%u005C;evil.txt'; Want = 'evil.txt' },
            @{ Raw = 'we%u003C;ir%u003E;d%u0022;%u007C;.txt'; Want = 'we_ir_d__.txt' },
            @{ Raw = 'q%u003A;a%u003F;b%u002A;.txt'; Want = 'q_a_b_.txt' },
            @{ Raw = 'Com1.tar.gz'; Want = '_Com1.tar.gz' },
            @{ Raw = 'lpt%u00B9;.txt'; Want = ('_lpt' + (U 0xB9) + '.txt') },
            @{ Raw = '.hidden'; Want = '.hidden' },
            @{ Raw = 'e2e-zero.txt'; Want = 'e2e-zero.txt'; Size = 0 }   # a 0-byte file: a POST that creates it, a PUT with Content-Length 0 that finishes it
        )
        $spec = ($cases | ForEach-Object { "$($_.Raw):$(if ($null -ne $_.Size) { $_.Size } else { 2000 })" }) -join ','
        Make-PhoneFiles $spec
        $src = Phone-SourceHashes
        $before = Pc-Ids
        Share-Files (($cases | ForEach-Object { $_.Raw }) -join ',')
        $t = Wait-NewPcTransfer $before 'in' 60
        Check 'S9 phone -> PC: the transfer finished' ($t -and $t.status -eq 'done' -and $t.files -eq $cases.Count) "$($t.status) files=$($t.files)"
        $have = @(Pc-Recv | ForEach-Object { Esc $_.Name })
        $missing = @($cases | Where-Object { (Esc $_.Want) -notin $have } | ForEach-Object { "$(Esc (& $dec $_.Raw)) -> wanted $(Esc $_.Want)" })
        Check 'S9 phone -> PC: every name is what protocol.md says' ($missing.Count -eq 0) ($missing -join '; ')
        $bad = @(); $known = 0
        foreach ($c in $cases) {
            $want = $src[(& $dec $c.Raw)]
            $p = Join-Path $script:RecvDir $c.Want
            if (-not $want) { continue }
            $known++
            if (-not ((Test-Path -LiteralPath $p) -and (Pc-Sha $p) -eq $want)) { $bad += (Esc $c.Want) }
        }
        Check 'S9 phone -> PC: contents match (for the names whose source hash could be read)' ($known -ge 8 -and $bad.Count -eq 0) "checked $known; bad: $($bad -join ', ')"
        Check 'S9 phone -> PC: the 300-byte-class CJK name was cut to at most 200 bytes' (@(Pc-Recv | Where-Object { [Text.Encoding]::UTF8.GetByteCount($_.Name) -gt 200 }).Count -eq 0)
        # collisions: the same names again, in three more transfers
        Make-PhoneFiles 'dup.txt:300,DUP.TXT:301'
        $src = Phone-SourceHashes
        foreach ($nm in 'dup.txt', 'dup.txt', 'DUP.TXT') {
            $before = Pc-Ids
            Share-Files $nm
            Wait-NewPcTransfer $before 'in' 40 | Out-Null
        }
        $have = @(Pc-Recv | ForEach-Object { $_.Name })
        Check 'S9 phone -> PC: collisions give dup.txt, dup (1).txt, DUP (2).TXT (case-insensitive, suffix before the extension)' (($have -ccontains 'dup.txt') -and ($have -ccontains 'dup (1).txt') -and ($have -ccontains 'DUP (2).TXT')) (($have | Where-Object { $_ -like 'dup*' -or $_ -like 'DUP*' }) -join ', ')

        # PC -> phone
        $dn = Join-Path $script:W13 'names'; New-Item -ItemType Directory -Force $dn | Out-Null
        $pcCases = @(
            @{ Disk = 'CON.txt'; Want = '_CON.txt' },
            @{ Disk = ('rtl' + (U 0x202E) + 'fdp.txt'); Want = 'rtl_fdp.txt' },
            @{ Disk = ((Cjk 100) + '.txt'); Want = ((Cjk 65) + '.txt') },
            @{ Disk = 'tail2.txt.'; Want = 'tail2.txt' },
            @{ Disk = 'Com1.tar.gz'; Want = '_Com1.tar.gz' },
            @{ Disk = 'plain name.txt'; Want = 'plain name.txt' },
            @{ Disk = 'e2e-zero.txt'; Want = 'e2e-zero.txt'; Empty = $true }
        )
        $paths = @()
        foreach ($c in $pcCases) {
            $path = Join-Path $dn $c.Disk
            [IO.File]::WriteAllText((Long $path), $(if ($c.Empty) { '' } else { "contents of $($c.Want.Length) $($c.Want.GetHashCode())" }))   # \\?\ lets Windows create CON.txt and a trailing dot
            $paths += (Long $path)
        }
        $dup = Join-Path $dn 'dup2.txt'; [IO.File]::WriteAllText($dup, 'dup two')
        $pbefore = Phone-Ids
        Pc-Send $paths
        $pt = Phone-NewTransfer $pbefore 'down' 60
        Check 'S9 PC -> phone: the transfer finished' ($pt -and $pt.Status -eq 'done' -and $pt.Files -eq $pcCases.Count) "$($pt.Status) files=$($pt.Files) $($pt.Err)"
        $rec = @(Phone-Received)
        $got = @($rec | ForEach-Object { Esc $_.Name })
        $missing = @($pcCases | Where-Object { (Esc $_.Want) -notin $got } | ForEach-Object { "$(Esc $_.Disk) -> wanted $(Esc $_.Want)" })
        Check 'S9 PC -> phone: every name is what protocol.md says' ($missing.Count -eq 0) ($missing -join '; ')
        $h = Phone-DirHashes '/sdcard/Download/Kin'
        $bad = @($pcCases | Where-Object { $h[$_.Want] -ne (Pc-Sha (Long (Join-Path $dn $_.Disk))) } | ForEach-Object { Esc $_.Want })
        Check 'S9 PC -> phone: contents match' ($bad.Count -eq 0) ($bad -join ', ')
        foreach ($i in 1, 2) {
            $pbefore = Phone-Ids
            Pc-Send @($dup)
            Phone-NewTransfer $pbefore 'down' 40 | Out-Null
        }
        $got = @(Phone-Received | ForEach-Object { $_.Name })
        Check 'S9 PC -> phone: a name sent twice gives dup2.txt and dup2 (1).txt' (($got -ccontains 'dup2.txt') -and ($got -ccontains 'dup2 (1).txt')) (($got | Where-Object { $_ -like 'dup2*' }) -join ', ')
        foreach ($c in $pcCases) { $script:Names.Add($c.Want) }
        $script:Names.Add('dup2.txt')
    }

    # ---------------------------------------------------------------- S10: the text rules
    if (Want 'S10') {
        Say 'S10: text'
        Reset-PhoneNotifications
        # a. sensitive, PC -> phone
        $secA = Use-Text "e2e-$n s3cretpcphone words here"
        & $Kin --dev-set-clipboard $secA --dev-clipboard-mark exclude
        Start-Sleep -Milliseconds 1500
        $textsBefore = Phone-LogCount "(?<!sensitive )text from '"
        $tx = Hotkey-Send
        Check 'S10a PC -> phone: the hotkey sent it, acked, flagged sensitive' ($tx -and $tx.result -eq 'acked' -and $tx.sensitive -eq $true) "$($tx.result) sensitive=$($tx.sensitive)"
        Check 'S10a PC -> phone: no length in the PC status (chars is null)' ($null -eq $tx.chars)
        $nt = $null; Wait-Until { $script:nt = Phone-Notification $tx.id; [bool]$script:nt } 25 | Out-Null; $nt = $script:nt
        Check 'S10a PC -> phone: the notification is "Text from <PC>" with no preview and no Open' ($nt -and $nt.Title -like 'Text from *' -and -not $nt.Raw.Contains('s3cretpcphone') -and $nt.Actions.Count -eq 0) "$($nt.Title) actions=$($nt.Actions -join ',')"
        $pc = Wait-PhoneClip $secA
        Check 'S10a PC -> phone: the phone clipboard has the text' ($pc -and $pc.Text -ceq $secA)
        Check 'S10a PC -> phone: the phone clipboard is marked sensitive (Android 13 and later, where the flag exists)' ($pc -and ($sdk -lt 33 -or $pc.Sensitive))
        Check 'S10a PC -> phone: the phone log has one line without length or hash, and no ordinary "text from" line' (((Phone-LogCount "(?<!sensitive )text from '") -eq $textsBefore) -and ((Phone-Log) -match "sensitive text from '[^']*' written=\w+ \(its content is not logged\)"))
        # b. sensitive, phone -> PC
        $secB = Use-Text "e2e-$n s3cretphonepc words here"
        $script:CurrentSecret = $secB
        $r = Send-From-Phone { Phone-Bcast 'SEND_TEXT' "--es text '$($script:CurrentSecret)' --ez sensitive true" 200 | Out-Null } $secB
        Check 'S10b phone -> PC: arrived, flagged sensitive, no length in the status' ($r.Got -and $r.Status.sensitive -eq $true -and $null -eq $r.Status.chars) "sensitive=$($r.Status.sensitive) chars=$($r.Status.chars)"
        Check 'S10b phone -> PC: the PC clipboard has the text' ($r.Clip -ceq $secB)
        $fmts = @(Get-ClipboardFormats | ForEach-Object { $_.Name })
        Check 'S10b phone -> PC: the clipboard carries the three opt-out formats' (($fmts -contains 'ExcludeClipboardContentFromMonitorProcessing') -and ($fmts -contains 'CanIncludeInClipboardHistory') -and ($fmts -contains 'CanUploadToCloudClipboard')) ($fmts -join ',')
        Check 'S10b phone -> PC: a toast "Text from" without a preview' ($r.Toast -and -not ((Pc-Log) -match 'toast: Text from [^\r\n]*preview'))
        # c. a text over 128 KiB becomes a .txt file, both ways
        $line = 'x' * 61
        $crlf = (($line + "`r`n") * 3300)
        $wire = (($line + "`n") * 3300)
        $wireBytes = [Text.Encoding]::UTF8.GetBytes($wire)
        $wireSha = ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($wireBytes)) -replace '-', '').ToLower()
        Use-Text ($line + ' ' + $line) | Out-Null
        Set-PcClip $crlf
        $pbefore = Phone-Ids
        & $Kin --send-clipboard
        $pt = Phone-NewTransfer $pbefore 'down' 60
        Check 'S10c PC -> phone: a 200 KiB text came as a file download' ($pt -and $pt.Status -eq 'done' -and $pt.Files -eq 1) "$($pt.Status) $($pt.Names) err=$($pt.Err)"
        if (-not ($pt -and $pt.Status -eq 'done')) { Show-PhoneTransferLog }
        $h = Phone-DirHashes '/sdcard/Download/Kin'
        $tn = if ($pt) { $pt.Names } else { '' }
        Check 'S10c PC -> phone: the file is a .txt with LF line ends, the wire form, byte for byte' ($tn -like '*.txt' -and $h[$tn] -eq $wireSha) "$tn $($h[$tn]) vs $wireSha"
        # MediaStore re-reads the size in the background after IS_PENDING goes to 0, so give it a moment
        $rec = $null
        Wait-Until { $script:rec10 = @(Phone-Received) | Where-Object { $_.Name -eq $tn } | Select-Object -First 1; $script:rec10 -and $script:rec10.Size -eq $wireBytes.Length } 20 | Out-Null
        $rec = $script:rec10
        Check 'S10c PC -> phone: MediaStore says text/plain, size = wire size' ($rec -and $rec.Mime -like 'text/plain*' -and $rec.Size -eq $wireBytes.Length) "$($rec.Mime) $($rec.Size) vs $($wireBytes.Length)"
        $pcBefore = Pc-Ids
        Run-As "mkdir -p files/debug-share; yes abcdefghij0123456789 | head -n 9000 > files/debug-share/e2e-long200.txt" | Out-Null
        $longWire = ('abcdefghij0123456789' + "`n") * 9000
        $longSha = ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes($longWire))) -replace '-', '').ToLower()
        Use-Text 'abcdefghij0123456789' | Out-Null
        $clipBefore = Pc-Clip
        Phone-Bcast 'SEND_TEXT' '--es textfile e2e-long200.txt' 300 | Out-Null
        $t = Wait-NewPcTransfer $pcBefore 'in' 60
        Check 'S10c phone -> PC: a 198,000-byte text arrived as a file transfer' ($t -and $t.status -eq 'done' -and $t.files -eq 1) "$($t.status) files=$($t.files)"
        $nf = @(Pc-Recv | Where-Object { $_.Name -like 'Kin text*.txt' }) | Select-Object -First 1
        Check 'S10c phone -> PC: a .txt, byte for byte the wire form' ($nf -and (Pc-Sha $nf.FullName) -eq $longSha) "$($nf.Name)"
        Check 'S10c phone -> PC: it was not put on the PC clipboard' ((Pc-Clip) -ceq $clipBefore -or ((Pc-Clip) -notlike '*abcdefghij0123456789*'))
        # d. CRLF and LF
        Run-As 'mkdir -p files/debug-share; printf "a\r\nb\r\nc" > files/debug-share/e2e-crlf.txt; printf "a\nb\nc" > files/debug-share/e2e-lf.txt; printf "p\r\r\nq" > files/debug-share/e2e-cr.txt' | Out-Null
        Use-Text 'e2e-lf-marker' | Out-Null
        foreach ($case in @(@('e2e-crlf.txt', "a`r`nb`r`nc"), @('e2e-lf.txt', "a`r`nb`r`nc"), @('e2e-cr.txt', "p`r`nq"))) {
            $script:CurrentFile = $case[0]
            $r = Send-From-Phone { Phone-Bcast 'SEND_TEXT' "--es textfile $($script:CurrentFile)" 200 | Out-Null } $case[1]
            Check "S10d phone -> PC: $($case[0]) arrives with CRLF on the Windows clipboard" ($r.Got -and $r.Clip -ceq $case[1]) "got $(Esc $r.Clip)"
        }
        foreach ($case in @(@("x`r`ny`r`nz", "x`ny`nz"), @("p`r`r`nq", "p`r`nq"), @("lone`rcr", "lone`rcr"))) {
            Set-PcClip $case[0]
            $tx = Hotkey-Send
            $pc = Wait-PhoneClip $case[1]
            Check "S10d PC -> phone: $(Esc $case[0]) goes as LF ($(Esc $case[1]))" ($tx.result -eq 'acked' -and $pc -and $pc.Text -ceq $case[1]) "phone has $(Esc $pc.Text)"
        }
        # round trip: the phone's LF text comes back from the PC as the same wire text
        $script:CurrentFile = 'e2e-lf.txt'
        $r = Send-From-Phone { Phone-Bcast 'SEND_TEXT' "--es textfile $($script:CurrentFile)" 200 | Out-Null } "a`r`nb`r`nc"
        Start-Sleep -Milliseconds 1500
        $tx = Hotkey-Send
        $pc = Wait-PhoneClip "a`nb`nc"
        Check 'S10d round trip: LF on the phone -> CRLF on the PC -> LF on the phone again' ($tx.result -eq 'acked' -and $pc -and $pc.Text -ceq "a`nb`nc") "phone has $(Esc $pc.Text)"
        # e. openable URLs
        $urls = @(
            @{ T = 'http://example.com:8080/e2e-ok'; Open = $true },
            @{ T = 'http://.'; Open = $false },
            @{ T = 'http://example.com:/x'; Open = $false },
            @{ T = 'http://example.com:0/x'; Open = $false },
            @{ T = 'https://example.com:65536/x'; Open = $false },
            @{ T = 'https://[]/x'; Open = $false },
            @{ T = ('http://example.com/' + ('a' * 8200)); Open = $false }
        )
        foreach ($u in $urls) {
            $label = if ($u.T.Length -gt 40) { "$($u.T.Substring(0, 24))... ($($u.T.Length) chars)" } else { $u.T }
            Use-Text $u.T | Out-Null
            $script:CurrentText = $u.T
            $opens0 = Pc-LogCount 'click opens the link'
            $r = Send-From-Phone { Share-Intent $script:CurrentText } $u.T
            $opens = ((Pc-LogCount 'click opens the link') -gt $opens0)
            Check "S10e phone -> PC [$label]: isUrl/openable $($u.Open), link toast $($u.Open)" ($r.Got -and $r.Status.isUrl -eq $u.Open -and $r.Status.openable -eq $u.Open -and $opens -eq $u.Open) "isUrl=$($r.Status.isUrl) openable=$($r.Status.openable) toast=$opens"
            Set-PcClip $u.T
            $tx = Hotkey-Send
            $nt = $null; Wait-Until { $script:nt = Phone-Notification $tx.id; [bool]$script:nt } 25 | Out-Null; $nt = $script:nt
            $hasOpen = [bool]($nt -and $nt.Actions -contains 'Open')
            Check "S10e PC -> phone [$label]: Open action $($u.Open)" ($tx.result -eq 'acked' -and $nt -and $hasOpen -eq $u.Open) "result=$($tx.result) notification found=$([bool]$nt) actions=$($nt.Actions -join ',')"
        }
    }

    # ---------------------------------------------------------------- S12: logs
    if (Want 'S12') {
        Say 'S12: logs'
        $pcAll = Pc-Log
        $phAll = Phone-Log
        $lc = (A logcat -d -s KinRelay | Where-Object { $_ -notmatch 'CMD CLIPBOARD|CMD RECEIVED|CMD TRANSFER|CMD MAKE_FILES|CMD SEND_TEXT' } | Out-String)
        $status = Get-Content (Join-Path $Data 'status.json') -Raw
        $leaks = @()
        foreach ($t in $script:Texts) {
            $word = ($t -split ' ' | Select-Object -Skip 1 | Select-Object -First 3) -join ' '
            foreach ($pair in @(@('PC log', $pcAll), @('phone log', $phAll), @('logcat', $lc), @('status.json', $status))) {
                if ($pair[1].Contains($t) -or ($word.Length -gt 8 -and $pair[1].Contains($word))) { $leaks += "$($pair[0]): '$($t.Substring(0, [Math]::Min(30, $t.Length)))'" }
            }
        }
        Check 'S12 no clipboard or text content in the logs, logcat or status.json' ($leaks.Count -eq 0) ($leaks -join '; ')
        $tok = ([regex]::Match((Get-Content (Join-Path $Data 'pair-url.txt') -Raw), 'tk=([^&\s]+)')).Groups[1].Value
        Check 'S12 no token, tk= or private key anywhere' ($tok.Length -eq 22 -and -not ($pcAll + $phAll + $lc + $status).Contains($tok) -and ($pcAll + $phAll + $lc) -notmatch 'PRIVATE KEY|tk=')
        # The one place a file name reaches the PC log is the toast text "Sending <name> to <phone>" of a single-file offer (the log repeats every toast)
        $pcHit = @($script:Names | Where-Object { $nm = $_; @(($pcAll -split "`r?`n") | Where-Object { $_.Contains($nm) -and $_ -notmatch 'toast: Sending ' }).Count -gt 0 })
        $pcNames = $pcHit.Count
        $phNames = @($script:Names | Where-Object { $phAll.Contains($_) }).Count
        if ($pcHit.Count) { foreach ($l in ($pcAll -split "`r?`n")) { foreach ($nm in $pcHit) { if ($l.Contains($nm)) { Say "  PC log line with a name: $l" } } } }
        $script:Timings['S12 file names found in the logs (names used in S9; the PC log should have none)'] = "PC log: $pcNames of $($script:Names.Count), phone log: $phNames of $($script:Names.Count)"
        Check 'S12 the PC log carries no file names' ($pcNames -eq 0)
        Check 'S12 the logs carry ids, sizes and counts (the transfer lines are there)' (($pcAll -match 'file \d+ is complete') -and ($phAll -match 'upload \S+ done: \d+ files'))
    }
}
