# A big phone photo with EXIF rotation against the real Kin.exe (the 1.3 review finding: a 12 MP JPEG with orientation 6 took over
# 400 s to reach the clipboard). A fake phone (examples\fake_phone.rs) uploads the photo; the script measures how long the `PUT` takes to
# answer and how long the picture needs to reach the clipboard, and checks the picture is upright (the four corners).
#
#   cd desktop
#   powershell -ExecutionPolicy Bypass -File check-rotation.ps1 [-Exe <path to Kin.exe>] [-Orientations '6,3,8'] [-Megapixels 12] [-TimeoutSeconds 60]
#
# What it does to your machine: starts the Kin.exe it is given (default: the release build) with `--dev-bind 127.0.0.1` (loopback only),
# a throwaway profile and a throwaway received-files folder under %LOCALAPPDATA%\Temp. The real clipboard is saved first in every format
# (clipboard-backup.ps1) and put back at the end, also after a failure. Everything it started is stopped and its folders deleted.
param(
    [string]$Exe = '',
    [string]$Orientations = '6,3,8',
    [double]$Megapixels = 12,
    [int]$TimeoutSeconds = 60
)
# Only one of the Kin scripts (e2e, smoke, capture, check) may run at a time: they share the real clipboard, the Kin.exe slot and the emulator (tools\kin-lock.ps1).
. (Join-Path $PSScriptRoot '..\tools\kin-lock.ps1'); Enter-KinLock 'check-rotation'
. (Join-Path $PSScriptRoot '..\tools\kin-dev.ps1')   # a Kin of our own beside the installed one; every command carries --data-dir (kin-dev.ps1)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
. (Join-Path $here 'clipboard-backup.ps1')
$kin = if ($Exe) { $Exe } else { Join-Path $here 'src-tauri\target\release\Kin.exe' }
$phoneExe = Join-Path $here 'src-tauri\target\debug\examples\fake_phone.exe'
$work = Join-Path $env:LOCALAPPDATA ('Temp\kin-rotation-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $work 'data'; $recv = Join-Path $work 'received'; $save = Join-Path $work 'phone-saved'; $src = Join-Path $work 'source'
$backup = $null
$rows = New-Object System.Collections.Generic.List[object]
$failures = 0
$p = $null   # the fake phone of the round that is running

function Wait-Until([scriptblock]$Cond, [double]$Seconds = 20) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds 25 }
    return $false
}
function Kin-Log { (Get-Content -LiteralPath (Join-Path $data 'logs\kin.log') -Raw -ErrorAction SilentlyContinue) }
function Stop-Kin { Stop-DevKin 10 }   # --quit to our data directory; a hung process of ours is stopped by its ID

# The four quadrants of the stored picture: red top left, green top right, blue bottom left, yellow bottom right, with noise on every pixel
# (+-20 per channel) so that the JPEG is as big as a real photo (about 5 MB for 12 MP) and not a few flat blocks.
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
public static class KinPhoto {
    public static void Fill(Bitmap bmp) {
        int w = bmp.Width, h = bmp.Height;
        BitmapData data = bmp.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.WriteOnly, PixelFormat.Format24bppRgb);
        Random rnd = new Random(1);
        byte[] row = new byte[Math.Abs(data.Stride)];
        byte[] noise = new byte[w * 3];
        for (int y = 0; y < h; y++) {
            rnd.NextBytes(noise);
            bool top = y < h / 2;
            for (int x = 0; x < w; x++) {
                bool left = x < w / 2;
                int r = top ? (left ? 255 : 0) : (left ? 0 : 255);
                int g = top ? (left ? 0 : 255) : (left ? 0 : 255);
                int b = top ? 0 : (left ? 255 : 0);
                int i = x * 3;
                row[i] = (byte)Math.Max(0, Math.Min(255, b + noise[i] % 41 - 20));
                row[i + 1] = (byte)Math.Max(0, Math.Min(255, g + noise[i + 1] % 41 - 20));
                row[i + 2] = (byte)Math.Max(0, Math.Min(255, r + noise[i + 2] % 41 - 20));
            }
            Marshal.Copy(row, 0, new IntPtr(data.Scan0.ToInt64() + (long)y * data.Stride), row.Length);
        }
        bmp.UnlockBits(data);
    }
}
'@
function Make-Photo([string]$path, [int]$w, [int]$h, [int]$orientation) {
    $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    [KinPhoto]::Fill($bmp)
    $pi = [System.Runtime.Serialization.FormatterServices]::GetUninitializedObject([System.Drawing.Imaging.PropertyItem])
    $pi.Id = 0x0112; $pi.Type = 3; $pi.Len = 2; $pi.Value = [byte[]]($orientation, 0); $bmp.SetPropertyItem($pi)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Jpeg); $bmp.Dispose()
}
function Colour-Name($c) {
    if ($c.R -gt 200 -and $c.G -lt 80 -and $c.B -lt 80) { return 'red' }
    if ($c.G -gt 200 -and $c.R -lt 80 -and $c.B -lt 80) { return 'green' }
    if ($c.B -gt 200 -and $c.R -lt 80 -and $c.G -lt 80) { return 'blue' }
    if ($c.R -gt 200 -and $c.G -gt 200 -and $c.B -lt 80) { return 'yellow' }
    return "?($($c.R),$($c.G),$($c.B))"
}
# What the upright picture shows in its corners (top left, top right, bottom right, bottom left) for each orientation.
$expected = @{
    1 = 'red,green,yellow,blue'
    3 = 'yellow,blue,red,green'
    6 = 'blue,red,green,yellow'
    8 = 'green,yellow,blue,red'
}

try {
    foreach ($f in $kin, $phoneExe) { if (-not (Test-Path -LiteralPath $f)) { throw "$f is missing: build it first" } }
    Set-KinDev -Exe $kin -DataDir $data
    New-Item -ItemType Directory -Force -Path $work, $src, $save | Out-Null
    $backup = Save-Clipboard
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    Write-Host "Kin.exe: $kin"
    Write-Host "work folder: $work"

    foreach ($o in ($Orientations -split '[, ]+' | Where-Object { $_ } | ForEach-Object { [int]$_ })) {
        # Stored sideways (as a phone does): for 6 and 8 the picture is stored with the long side across.
        $w = [int][math]::Round([math]::Sqrt($Megapixels * 1e6 * 4 / 3)); $h = [int][math]::Round($Megapixels * 1e6 / $w)
        $photo = Join-Path $src "photo-o$o.jpg"
        Make-Photo $photo $w $h $o
        $size = (Get-Item -LiteralPath $photo).Length
        # (nothing to clear: Kin runs with the clipboard sink, which holds only what this Kin wrote, and the real clipboard is never touched.)

        if (Test-Path -LiteralPath $data) { [IO.Directory]::Delete($data, $true) }
        New-Item -ItemType Directory -Force -Path $recv | Out-Null
        Get-ChildItem -LiteralPath $recv | Remove-Item -Force
        Start-DevKin @('--data-dir', ('"{0}"' -f $data), '--dev-bind', '127.0.0.1', '--dev-received-dir', ('"{0}"' -f $recv), '--dev-print-pair-url', '--dev-no-windows') | Out-Null
        if (-not (Wait-Until { Test-Path -LiteralPath (Join-Path $data 'status.json') } 15)) { throw 'Kin did not start' }
        Invoke-Kin --dev-start-pairing
        if (-not (Wait-Until { Test-Path -LiteralPath (Join-Path $data 'pair-url.txt') } 10)) { throw 'no pairing URL' }
        Start-Sleep -Milliseconds 400
        $url = (Get-Content -LiteralPath (Join-Path $data 'pair-url.txt')).Trim()
        $log = Join-Path $work "phone-o$o.log"
        $phoneSeconds = $TimeoutSeconds + 20
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $p = Start-Process $phoneExe -ArgumentList @($url, '--host', '127.0.0.1', '--seconds', "$phoneSeconds", '--save-dir', ('"{0}"' -f $save), '--upload', ('"{0}"' -f $photo), '--upload-after', '1') -PassThru -RedirectStandardOutput $log -NoNewWindow

        $putDone = $null; $onClipboard = $null
        while ($sw.Elapsed.TotalSeconds -lt $TimeoutSeconds + 5) {
            if (-not $putDone -and ((Get-Content -LiteralPath $log -Raw -ErrorAction SilentlyContinue) -match 'upload done')) { $putDone = $sw.Elapsed.TotalSeconds }
            if (-not $onClipboard -and ((Kin-Log) -match 'is on the clipboard')) { $onClipboard = $sw.Elapsed.TotalSeconds }
            if ($putDone -and $onClipboard) { break }
            Start-Sleep -Milliseconds 20
        }
        $upright = ''
        $dims = ''
        if ($onClipboard) {
            # (another program can hold the clipboard for a moment: try a few times)
            $img = $null
            foreach ($try in 1..10) { $img = try { Get-PcClipboardImage } catch { $null }; if ($img) { break }; Start-Sleep -Milliseconds 100 }
            if ($img) {
                $dims = "$($img.Width) x $($img.Height)"
                $i = 100
                $upright = (Colour-Name $img.GetPixel($i, $i)), (Colour-Name $img.GetPixel($img.Width - $i, $i)), (Colour-Name $img.GetPixel($img.Width - $i, $img.Height - $i)), (Colour-Name $img.GetPixel($i, $img.Height - $i)) -join ','
                $img.Dispose()
            }
        }
        $ok = ($onClipboard -ne $null) -and ($upright -eq $expected[$o])
        if (-not $ok) { $failures++ }
        $row = [pscustomobject]@{
            Orientation = $o; Pixels = "$w x $h"; FileMB = [math]::Round($size / 1MB, 1)
            PutAnswered_s = if ($putDone) { [math]::Round($putDone, 2) } else { "not within $TimeoutSeconds s" }
            OnClipboard_s = if ($onClipboard) { [math]::Round($onClipboard, 2) } else { "not within $TimeoutSeconds s" }
            Picture = $dims; Corners = $upright; Upright = $ok
        }
        $rows.Add($row)
        Write-Host ($row | Format-List | Out-String).Trim()
        Write-Host ''
        if (-not $p.HasExited) { $p.Kill() }
        Stop-Kin
    }
}
finally {
    Stop-OwnProcess $p      # the fake phone this script started (by ID, never by name)
    Stop-Kin
    if ($backup) { $ok = Restore-Clipboard $backup; Write-Host ("the clipboard is back as it was (every format): {0}" -f [bool]$ok) }
    Start-Sleep -Milliseconds 300
    if ([IO.Directory]::Exists($work)) { try { [IO.Directory]::Delete($work, $true) } catch { } }
    Write-Host ("nothing left running: {0}; work folder gone: {1}" -f ((@(Get-DevKinProcesses).Count -eq 0) -and (-not $p -or $p.HasExited)), (-not (Test-Path -LiteralPath $work)))
}
Write-Host ''
$rows | Format-Table -AutoSize | Out-String | Write-Host
if ($failures) { Write-Host "$failures case(s) failed"; exit 1 }
