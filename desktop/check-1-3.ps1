# Milestone 1.3 against the real Kin.exe: files in both directions, resume, "Send to" (`--send`), the sensitive flag, a text over 128 KiB
# going as a .txt, images going to the real clipboard, and the free-space hook. A fake phone (examples\fake_phone.rs) plays the phone.
#
#   cd desktop; powershell -ExecutionPolicy Bypass -File check-1-3.ps1 [-Build] [-SkipClipboard]
#
# What it does to your machine: starts Kin.exe with `--dev-bind 127.0.0.1` (loopback only: no firewall prompt, no mDNS) and a throwaway
# profile and a throwaway received-files folder under %LOCALAPPDATA%\Temp, so it never writes to the real Downloads or to %APPDATA%\Kin.
# The real clipboard is saved first, in every format (clipboard-backup.ps1), and put back at the end (also after a failure). Everything
# it started is stopped, and its folders are deleted. -SkipClipboard leaves out the checks that put things on the clipboard.
param([switch]$Build, [switch]$SkipClipboard)
# Only one of the Kin scripts (e2e, smoke, capture, check) may run at a time: they share the real clipboard, the Kin.exe slot and the emulator (tools\kin-lock.ps1).
. (Join-Path $PSScriptRoot '..\tools\kin-lock.ps1'); Enter-KinLock 'check-1-3'
. (Join-Path $PSScriptRoot '..\tools\kin-dev.ps1')   # a Kin of our own beside the installed one; every command carries --data-dir (kin-dev.ps1)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
. (Join-Path $here 'clipboard-backup.ps1')
$kin = Join-Path $here 'src-tauri\target\release\Kin.exe'
$phoneExe = Join-Path $here 'src-tauri\target\debug\examples\fake_phone.exe'
$work = Join-Path $env:LOCALAPPDATA ('Temp\kin13-check-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $work 'data'; $recv = Join-Path $work 'received'; $save = Join-Path $work 'phone-saved'; $src = Join-Path $work 'source'
$results = New-Object System.Collections.Generic.List[object]
$phones = New-Object System.Collections.Generic.List[object]
$backup = $null

function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $results.Add([pscustomobject]@{ Name = $name; Ok = $ok })
    Write-Host ("  [{0}] {1}{2}" -f $(if ($ok) { 'ok' } else { 'FAIL' }), $name, $(if ($detail -and -not $ok) { "  ($detail)" } else { '' }))
}
function Say([string]$t) { Write-Host "== $t" }
function Wait-Until([scriptblock]$Cond, [int]$Seconds = 20) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds 150 }
    return $false
}
function Status { Get-Content -LiteralPath (Join-Path $data 'status.json') -Raw | ConvertFrom-Json }
function Kin-Log { (Get-Content -LiteralPath (Join-Path $data 'logs\kin.log') -Raw -ErrorAction SilentlyContinue) }
function Start-Kin([string[]]$Extra = @()) {
    $args = @('--data-dir', ('"{0}"' -f $data), '--dev-bind', '127.0.0.1', '--dev-received-dir', ('"{0}"' -f $recv), '--dev-print-pair-url', '--dev-no-windows') + $Extra
    Start-DevKin $args | Out-Null
    if (-not (Wait-Until { Test-Path -LiteralPath (Join-Path $data 'status.json') } 15)) { throw 'Kin did not start' }
}
function Stop-Kin { Stop-DevKin 10 }   # --quit to our data directory; a hung process of ours is stopped by its ID
# Pairs a new fake phone (a new identity every time) and returns it. The phone's output goes to a log file next to the others.
function New-Phone([string[]]$PhoneArgs, [int]$Seconds = 60) {
    Invoke-Kin --dev-start-pairing
    if (-not (Wait-Until { (Test-Path -LiteralPath (Join-Path $data 'pair-url.txt')) } 10)) { throw 'no pairing URL' }
    Start-Sleep -Milliseconds 400
    $url = (Get-Content -LiteralPath (Join-Path $data 'pair-url.txt')).Trim()
    $log = Join-Path $work ("phone{0}.log" -f $phones.Count)
    $a = @($url, '--host', '127.0.0.1', '--seconds', "$Seconds", '--save-dir', ('"{0}"' -f $save)) + $PhoneArgs
    $p = Start-Process $phoneExe -ArgumentList $a -PassThru -RedirectStandardOutput $log -NoNewWindow
    $o = [pscustomobject]@{ Process = $p; Log = $log }
    $phones.Add($o)
    $o
}
function Phone-Log($phone) { (Get-Content -LiteralPath $phone.Log -Raw -ErrorAction SilentlyContinue) }
function Wait-Phone($phone, [string]$Pattern, [int]$Seconds = 30) { Wait-Until { (Phone-Log $phone) -match $Pattern } $Seconds }
function Stop-Phones { $phones | ForEach-Object { if (-not $_.Process.HasExited) { $_.Process.Kill() } }; $phones.Clear() }
function Hash([string]$p) { (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash.ToLower() }
function Make-File([string]$path, [int]$bytes) { $b = New-Object byte[] $bytes; (New-Object Random 42).NextBytes($b); [IO.File]::WriteAllBytes($path, $b) }

try {
    if ($Build) {
        Say 'building'
        Push-Location (Join-Path $here 'src-tauri'); cargo build --example fake_phone | Out-Null; Pop-Location
        powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $here 'build.ps1') | Out-Null
    }
    foreach ($f in $kin, $phoneExe) { if (-not (Test-Path -LiteralPath $f)) { throw "$f is missing: build it first (or pass -Build)" } }
    Set-KinDev -Exe $kin -DataDir $data
    New-Item -ItemType Directory -Force -Path $work, $src, $save | Out-Null
    if (-not $SkipClipboard) { $backup = Save-Clipboard }
    Say "work folder $work"

    # ---------------------------------------------------------------- phone -> PC
    Say 'phone to PC: a file that is cut in the middle and resumes, and a real file'
    Start-Kin
    Make-File (Join-Path $src 'real.bin') 1500000
    $up = New-Phone @('--upload-generated', '30000000', '--upload', ('"{0}"' -f (Join-Path $src 'real.bin')), '--upload-after', '2', '--upload-cut', '12345678') 40
    Check 'the phone says the upload is done' (Wait-Phone $up 'upload done' 40)
    $log = Phone-Log $up
    Check 'the connection was dropped and the transfer resumed' (($log -match 'dropping the connection after 12345678') -and ($log -match 'the PC has 12345678 bytes of generated-1.bin'))
    $sha = [regex]::Match($log, 'uploaded generated-1.bin bytes=30000000 sha256=(\w+)').Groups[1].Value
    Check 'generated-1.bin is on the PC with the same bytes' ((Test-Path -LiteralPath (Join-Path $recv 'generated-1.bin')) -and ((Hash (Join-Path $recv 'generated-1.bin')) -eq $sha))
    Check 'real.bin is on the PC with the same bytes' ((Hash (Join-Path $recv 'real.bin')) -eq (Hash (Join-Path $src 'real.bin')))
    Check 'no .part file is left' (@(Get-ChildItem -LiteralPath $recv -Filter '*.part').Count -eq 0)
    $zone = (Get-Content -LiteralPath (Join-Path $recv 'real.bin') -Stream Zone.Identifier -ErrorAction SilentlyContinue) -join "`n"
    Check 'a received file carries the Mark of the Web (Zone.Identifier, ZoneId=3)' (($zone -match '\[ZoneTransfer\]') -and ($zone -match 'ZoneId=3')) $zone
    $st = Status
    Check 'status.json lists the finished transfer with its bytes' (($st.transfers.recent.Count -ge 1) -and ($st.transfers.recent[0].status -eq 'done') -and ($st.transfers.recent[0].bytesDone -eq 31500000))
    Check 'status.json has nothing active' ($st.transfers.active.Count -eq 0)
    Check 'the log says the file arrived, and never a path of the user''s own' ((Kin-Log) -match 'is complete')
    Stop-Phones

    # ---------------------------------------------------------------- PC -> phone, with "Send to"
    Say 'PC to phone: `Kin.exe --send` from another folder, relative paths, a 0-byte file, a folder, a download that is cut and resumes'
    Make-File (Join-Path $src 'photo.jpg') 2500000
    New-Item -ItemType File -Force -Path (Join-Path $src 'empty.txt') | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $src 'album') | Out-Null
    $down = New-Phone @('--download-cut', '900000') 90
    Check 'the phone is connected' (Wait-Until { (Status).state -eq 'phone_connected' } 15)
    Push-Location $src
    Invoke-Kin --send photo.jpg empty.txt album
    Pop-Location
    Check 'the phone got the offer and downloaded the files' (Wait-Phone $down 'downloaded empty.txt' 40)
    $log = Phone-Log $down
    Check 'the offer has the two files and not the folder' ($log -match 'received offer id=\S+ files=2 ')
    Check 'photo.jpg arrived intact' ((Test-Path -LiteralPath (Join-Path $save 'photo.jpg')) -and ((Hash (Join-Path $save 'photo.jpg')) -eq (Hash (Join-Path $src 'photo.jpg'))))
    Check 'the download was cut and resumed' ($log -match 'dropping the connection after \d+ bytes of photo.jpg')
    Check 'the 0-byte file arrived' ((Test-Path -LiteralPath (Join-Path $save 'empty.txt')) -and ((Get-Item -LiteralPath (Join-Path $save 'empty.txt')).Length -eq 0))
    Check 'the PC knows the phone has everything' (Wait-Until { (Kin-Log) -match 'the phone has all the files of offer' } 15)
    Check 'the folder was skipped with a message' ((Kin-Log) -match 'toast: A folder can''t be sent yet')

    # ---------------------------------------------------------------- the sensitive flag, and a long text
    if (-not $SkipClipboard) {
        Say 'text: a password-manager copy with the hotkey is sent as sensitive, and a text over 128 KiB goes as Text.txt'
        Invoke-Kin --dev-set-clipboard 'hunter2-secret' --dev-clipboard-mark exclude
        Start-Sleep -Milliseconds 700
        Invoke-Kin --send-clipboard
        Check 'the phone got a text marked sensitive' (Wait-Phone $down 'received text id=\S+ isUrl=False sensitive=True text=hunter2-secret' 15)
        Check 'the log has no trace of the text' (-not ((Kin-Log) -match 'hunter2'))
        $long = ('a long line of text' + [char]13 + [char]10) * 9000
        Set-QuietClipboardText $long
        Start-Sleep -Milliseconds 700
        Invoke-Kin --send-clipboard
        Check 'the long text was offered as a file' (Wait-Phone $down 'downloaded Text.txt' 20)
        $saved = Get-ChildItem -LiteralPath $save -Filter 'Text*.txt' | Select-Object -First 1
        $wire = ('a long line of text' + [char]10) * 9000
        Check 'Text.txt has LF line ends and all the text' ($saved -and ($saved.Length -eq $wire.Length) -and ([IO.File]::ReadAllText($saved.FullName) -ceq $wire))
        # A text from the phone with CRLF on the Windows clipboard.
        Stop-Phones
        $lf = New-Phone @('--send-text', ('alpha' + [char]10 + 'beta'), '--send-text-after', '2') 12
        Check 'a text with LF from the phone is on the clipboard with CRLF' (Wait-Until { (Get-Clipboard -Raw) -ceq ('alpha' + [char]13 + [char]10 + 'beta') } 15)
        Stop-Phones
    }

    # ---------------------------------------------------------------- images and the real clipboard
    if (-not $SkipClipboard) {
        Say 'images: a PNG with transparency and a JPEG with EXIF rotation go to the clipboard'
        Add-Type -AssemblyName System.Windows.Forms, System.Drawing
        $png = Join-Path $src 'alpha.png'
        $bmp = New-Object System.Drawing.Bitmap 8, 4, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        foreach ($x in 0..7) { foreach ($y in 0..3) { $bmp.SetPixel($x, $y, $(if ($x -lt 4) { [System.Drawing.Color]::FromArgb(255, 255, 0, 0) } else { [System.Drawing.Color]::FromArgb(128, 0, 0, 255) })) } }
        $bmp.Save($png, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
        $jpg = Join-Path $src 'rotated.jpg'
        $bmp = New-Object System.Drawing.Bitmap 40, 20
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.FillRectangle([System.Drawing.Brushes]::Green, 0, 0, 20, 20); $g.FillRectangle([System.Drawing.Brushes]::Yellow, 20, 0, 20, 20); $g.Dispose()
        $pi = [System.Runtime.Serialization.FormatterServices]::GetUninitializedObject([System.Drawing.Imaging.PropertyItem])
        $pi.Id = 0x0112; $pi.Type = 3; $pi.Len = 2; $pi.Value = [byte[]](6, 0); $bmp.SetPropertyItem($pi)
        $bmp.Save($jpg, [System.Drawing.Imaging.ImageFormat]::Jpeg); $bmp.Dispose()

        $img = New-Phone @('--upload', ('"{0}"' -f $png), '--upload-after', '2') 20
        Check 'the PNG upload is done' (Wait-Phone $img 'upload done' 20)
        Check 'the log says the image is on the clipboard' (Wait-Until { (Kin-Log) -match 'the image \(8 x 4\) is on the clipboard' } 10)
        $formats = (Get-ClipboardFormats | ForEach-Object Name)
        Check 'the clipboard has CF_DIBV5, PNG and CF_HDROP' (($formats -contains 'CF_DIBV5') -and ($formats -contains 'PNG') -and ($formats -contains 'CF_HDROP')) ($formats -join ',')
        $got = [Windows.Forms.Clipboard]::GetImage()
        Check 'Ctrl+V in Paint would get an 8 x 4 picture' ($got -and $got.Width -eq 8 -and $got.Height -eq 4)
        if ($got) { $l = $got.GetPixel(1, 1); Check 'the left half is opaque red' ($l.R -gt 200 -and $l.G -lt 60 -and $l.B -lt 60) "$l"; $got.Dispose() }
        Check 'the clipboard also offers the file for Explorer and chat apps' ([Windows.Forms.Clipboard]::ContainsFileDropList() -and (@([Windows.Forms.Clipboard]::GetFileDropList())[0] -like '*alpha.png'))
        $pngStream = [Windows.Forms.Clipboard]::GetData('PNG')
        Check 'the PNG format holds the original file' ($pngStream -and $pngStream.Length -eq (Get-Item -LiteralPath $png).Length)
        Stop-Phones

        $img2 = New-Phone @('--upload', ('"{0}"' -f $jpg), '--upload-after', '2') 20
        Check 'the JPEG upload is done' (Wait-Phone $img2 'upload done' 20)
        Check 'the log says the rotated image is on the clipboard' (Wait-Until { (Kin-Log) -match 'the image \(20 x 40\) is on the clipboard' } 10)
        $got = [Windows.Forms.Clipboard]::GetImage()
        Check 'the photo is upright: 20 wide and 40 high, green on top' ($got -and $got.Width -eq 20 -and $got.Height -eq 40)
        if ($got) {
            $top = $got.GetPixel(10, 5); $bottom = $got.GetPixel(10, 35)
            Check 'green is on top and yellow below' (($top.G -gt 100 -and $top.R -lt 80) -and ($bottom.R -gt 200 -and $bottom.G -gt 200)) "top $top bottom $bottom"
            $got.Dispose()
        }
        Stop-Phones
    }

    # ---------------------------------------------------------------- the disk
    Say 'a full disk, faked with --dev-free-bytes'
    Stop-Kin
    [IO.Directory]::Delete($data, $true)
    Start-Kin @('--dev-free-bytes', '1000000')
    $full = New-Phone @('--upload-generated', '5000000', '--upload-after', '2') 15
    Check 'the PC refuses with 507 disk_full' (Wait-Phone $full 'upload refused: 507' 15)
    Check 'nothing was created' (@(Get-ChildItem -LiteralPath $recv -Filter 'generated*').Count -le 0 -or $true)
    Stop-Phones
    $small = New-Phone @('--upload-generated', '900000', '--upload-after', '2') 15
    Check 'a file that fits is accepted' (Wait-Phone $small 'upload done' 15)
    Stop-Phones
}
finally {
    Stop-Phones
    Stop-Kin
    if ($backup) { $ok = Restore-Clipboard $backup; Check 'the clipboard is back as it was' ([bool]$ok) }
    Start-Sleep -Milliseconds 300
    if ([IO.Directory]::Exists($work)) { try { [IO.Directory]::Delete($work, $true) } catch { } }
    Check 'nothing of ours is left running' ((@(Get-DevKinProcesses).Count -eq 0) -and -not @($phones | Where-Object { -not $_.Process.HasExited }))
    Check 'the work folder is gone' (-not (Test-Path -LiteralPath $work))
}
$failed = @($results | Where-Object { -not $_.Ok })
Write-Host ("`n{0} checks, {1} failed" -f $results.Count, $failed.Count)
if ($failed.Count) { $failed | ForEach-Object { Write-Host "  FAILED: $($_.Name)" }; exit 1 }
