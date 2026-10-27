# Screenshots of the windows of Kin (the main window on its three pages Home, History and Settings, the debug window and the pairing window) from the real Kin.exe, as small PNGs.
#
#   cd desktop; powershell -ExecutionPolicy Bypass -File capture-windows.ps1 [-Out <folder>] [-Width 560] [-UpdateScreens]
#
# It starts Kin.exe in developer mode (loopback only, throwaway profile and received folder, toasts only logged), gives it a fake phone and a
# few transfers so the windows have something to show (a text each way, a file each way, one failed send, one text waiting in the queue), opens
# the main window (titled "Kin"; --drop, --history and --settings switch its page in place) and the other windows, and captures each visible frame (the screen is used: the window
# is brought to the front, so don't touch the mouse for a minute).
# The real clipboard is saved first and put back at the end (clipboard-backup.ps1). Everything it started is stopped and deleted.
# The QR code of the pairing window holds the address of the PC, so Kin gets --dev-qr-address 10.0.2.2 (the emulator's name for the PC, not a real
# address): without it the picture would carry the LAN address of the PC it was taken on.
# The main window is destroyed again after its three pictures (and the other windows after theirs), and the memory of Kin.exe is measured before and after (the idle-memory claim of 1.4).
# Without -Out and -UpdateScreens the pictures go to an ignored folder in the temp area: a run never overwrites the kept pictures of
# notes\screens (and an -Out that points there is redirected the same way) unless -UpdateScreens says so.
param([string]$Out = '', [int]$Width = 560, [switch]$UpdateScreens)
# Only one of the Kin scripts (e2e, smoke, capture, check) may run at a time: they share the real clipboard, the Kin.exe slot and the emulator (tools\kin-lock.ps1).
. (Join-Path $PSScriptRoot '..\tools\kin-lock.ps1'); Enter-KinLock 'capture-windows'
. (Join-Path $PSScriptRoot '..\tools\kin-dev.ps1')   # a Kin of our own beside the installed one; every command carries --data-dir (kin-dev.ps1)
$ErrorActionPreference = 'Stop'
$tracked = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\notes\screens'))
$temp = Join-Path $env:LOCALAPPDATA 'Temp\kin-capture-windows'
if (-not $Out) { $Out = if ($UpdateScreens) { Join-Path $PSScriptRoot '..\notes\screens\desktop-windows' } else { $temp } }
$Out = [IO.Path]::GetFullPath($Out)
if (-not $UpdateScreens -and $Out.StartsWith($tracked, [StringComparison]::OrdinalIgnoreCase)) { Write-Host "NOTE: $Out is tracked; the pictures go to $temp (pass -UpdateScreens to write there)"; $Out = $temp }
$here = $PSScriptRoot
. (Join-Path $here 'clipboard-backup.ps1')
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class W32 {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
}
'@
[void][W32]::SetProcessDPIAware()
$kin = Join-Path $here 'src-tauri\target\release\Kin.exe'
$phoneExe = Join-Path $here 'src-tauri\target\debug\examples\fake_phone.exe'
# The windows show the received folder and the PC's name, and the pictures are committed: a neutral folder (not under the user's profile) and a neutral
# computer name (Kin reads COMPUTERNAME), so nothing in a picture names the person or their PC. The folder is removed at the end.
$work = 'C:\Kin-demo'
$origComputerName = $env:COMPUTERNAME
if (Test-Path -LiteralPath $work) { throw "$work exists: remove it first (this script deletes it at the end)" }
$env:COMPUTERNAME = 'DESK-PC'
$data = Join-Path $work 'data'; $recv = Join-Path $work 'received'; $save = Join-Path $work 'phone-saved'; $src = Join-Path $work 'source'
$phones = New-Object System.Collections.Generic.List[object]
$backup = $null
function Say([string]$t) { Write-Host "== $t" }
function Wait-Until([scriptblock]$Cond, [int]$Seconds = 20) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds 100 }
    return $false
}
function Status { Get-Content -LiteralPath (Join-Path $data 'status.json') -Raw | ConvertFrom-Json }
# OUR Kin.exe (never the installed one) and every process below it (WebView2 starts its own), with their private working sets added up.
function Mem {
    $root = @(Get-DevKinProcesses | Select-Object -First 1)
    if (-not $root) { return [pscustomobject]@{ Processes = 0; PrivateWorkingSetMB = 0 } }
    $all = @(Get-CimInstance Win32_Process)
    $ids = New-Object System.Collections.Generic.List[int]; $ids.Add([int]$root[0].Id)
    for ($i = 0; $i -lt $ids.Count; $i++) { foreach ($c in ($all | Where-Object { $_.ParentProcessId -eq $ids[$i] })) { if (-not $ids.Contains([int]$c.ProcessId)) { $ids.Add([int]$c.ProcessId) } } }
    $perf = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object { $ids.Contains([int]$_.IDProcess) })
    [pscustomobject]@{ Processes = $ids.Count; PrivateWorkingSetMB = [math]::Round((($perf | Measure-Object WorkingSetPrivate -Sum).Sum) / 1MB, 1) }
}
function New-Phone([string[]]$PhoneArgs, [int]$Seconds = 120) {
    Invoke-Kin --dev-start-pairing
    if (-not (Wait-Until { (Test-Path -LiteralPath (Join-Path $data 'pair-url.txt')) } 10)) { throw 'no pairing URL' }
    Start-Sleep -Milliseconds 400
    $url = (Get-Content -LiteralPath (Join-Path $data 'pair-url.txt')).Trim()
    $log = Join-Path $work ("phone{0}.log" -f $phones.Count)
    $a = @($url, '--host', '127.0.0.1', '--seconds', "$Seconds", '--save-dir', ('"{0}"' -f $save), '--name', '"Pixel 8"', '--identity-file', ('"{0}"' -f (Join-Path $work 'id.json'))) + $PhoneArgs
    $p = Start-Process $phoneExe -ArgumentList $a -PassThru -RedirectStandardOutput $log -NoNewWindow
    $o = [pscustomobject]@{ Process = $p; Log = $log }; $phones.Add($o); $o
}
function Phone-Log($phone) { (Get-Content -LiteralPath $phone.Log -Raw -ErrorAction SilentlyContinue) }
function Make-File([string]$path, [int]$bytes) { $b = New-Object byte[] $bytes; (New-Object Random 7).NextBytes($b); [IO.File]::WriteAllBytes($path, $b) }
function Shot([string]$title, [string]$file, [int]$SwitchWaitMs = 0) {
    $h = [IntPtr]::Zero
    $found = Wait-Until { $w = Get-DevKinProcesses | Where-Object { $_.MainWindowTitle -eq $title } | Select-Object -First 1; if ($w) { $script:h = $w.MainWindowHandle; $true } else { $false } } 15
    if (-not $found) { throw "no window titled $title" }
    if ($title -ne 'Pair new phone') { Close-Window 'Pair new phone' }   # a pairing opens its "New device paired" notice: keep it out of the picture
    Start-Sleep -Milliseconds $SwitchWaitMs   # a window that is open already is told to switch its page: let the page switch before the picture
    [void][W32]::ShowWindow($script:h, 9); [void][W32]::SetForegroundWindow($script:h)
    Start-Sleep -Milliseconds 2200
    $r = New-Object W32+RECT
    if ([W32]::DwmGetWindowAttribute($script:h, 9, [ref]$r, 16) -ne 0) { [void][W32]::GetWindowRect($script:h, [ref]$r) }
    $w = $r.Right - $r.Left; $hgt = $r.Bottom - $r.Top
    $bmp = New-Object Drawing.Bitmap $w, $hgt
    $g = [Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($r.Left, $r.Top, 0, 0, $bmp.Size); $g.Dispose()
    $scale = [math]::Min(1.0, $Width / $w)
    $small = New-Object Drawing.Bitmap ([int]($w * $scale)), ([int]($hgt * $scale))
    $g = [Drawing.Graphics]::FromImage($small); $g.InterpolationMode = 'HighQualityBicubic'; $g.DrawImage($bmp, 0, 0, $small.Width, $small.Height); $g.Dispose()
    $small.Save((Join-Path $Out $file), [Drawing.Imaging.ImageFormat]::Png); $small.Dispose(); $bmp.Dispose()
    Write-Host ("  {0}: {1}x{2}, {3} KB" -f $file, $small.Width, $small.Height, [math]::Round((Get-Item (Join-Path $Out $file)).Length / 1KB))
}
function Close-Window([string]$title) {
    $w = Get-DevKinProcesses | Where-Object { $_.MainWindowTitle -eq $title } | Select-Object -First 1   # a window of OUR Kin only: the installed Kin may have a window of the same name open
    if ($w) { [void]$w.CloseMainWindow() }
    Wait-Until { -not (Get-DevKinProcesses | Where-Object { $_.MainWindowTitle -eq $title }) } 8 | Out-Null
}

try {
    foreach ($f in $kin, $phoneExe) { if (-not (Test-Path -LiteralPath $f)) { throw "$f is missing: build it first" } }
    Assert-NoGameRunning   # this script shows windows and brings them to the front
    Set-KinDev -Exe $kin -DataDir $data
    New-Item -ItemType Directory -Force -Path $work, $src, $save, $Out | Out-Null
    $backup = Save-Clipboard
    Make-File (Join-Path $src 'Holiday photo.jpg') 2400000
    Make-File (Join-Path $src 'Quarterly report.pdf') 800000
    Make-File (Join-Path $src 'notes.txt') 4200
    Make-File (Join-Path $src 'Holiday video.mp4') 5200000
    Start-DevKin @('--data-dir', ('"{0}"' -f $data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-received-dir', ('"{0}"' -f $recv), '--dev-print-pair-url', '--dev-no-toasts') | Out-Null
    if (-not (Wait-Until { Test-Path -LiteralPath (Join-Path $data 'status.json') } 15)) { throw 'Kin did not start' }
    Start-Sleep -Seconds 2
    $before = Mem
    Say ("memory before any window: {0} MB private working set, {1} process(es)" -f $before.PrivateWorkingSetMB, $before.Processes)

    Say 'giving the windows something to show'
    # a phone that sends a text and a photo, and takes what the PC sends
    $p1 = New-Phone @('--send-text', '"Meeting moved to 10:30, room 4"', '--send-text-after', '2', '--upload', ('"{0}"' -f (Join-Path $src 'Holiday photo.jpg')), '--upload-after', '3') 25
    if (-not (Wait-Until { (Phone-Log $p1) -match 'upload done' } 30)) { throw 'the phone did not finish' }
    Start-Sleep -Milliseconds 500
    Invoke-Kin --send (Join-Path $src 'Quarterly report.pdf') (Join-Path $src 'notes.txt')
    Wait-Until { (Phone-Log $p1) -match 'downloaded notes.txt' } 20 | Out-Null
    Invoke-Kin --dev-set-clipboard 'Wi-Fi password for the guests: sunflower-42'; Start-Sleep -Milliseconds 700
    Invoke-Kin --send-clipboard
    Wait-Until { (Phone-Log $p1) -match 'received text id=\S+ isUrl=False sensitive=False text=Wi-Fi' } 10 | Out-Null
    Start-Sleep -Seconds 1
    $p1.Process.Kill(); Start-Sleep -Seconds 1
    # a phone that refuses what it is sent (its disk is full): one failed send
    $p2 = New-Phone @('--refuse-offer', 'disk_full') 12
    if (-not (Wait-Until { (Status).state -eq 'phone_connected' } 15)) { throw 'the second phone did not connect' }
    Invoke-Kin --send (Join-Path $src 'Holiday video.mp4')
    Wait-Until { (Phone-Log $p2) -match 'refusing the offer' } 10 | Out-Null
    Start-Sleep -Seconds 1
    $p2.Process.Kill()
    if (-not (Wait-Until { (Status).state -eq 'phone_offline' } 20)) { throw 'the phone is still connected' }
    # the phone is away: a text waits in the queue
    Invoke-Kin --dev-set-clipboard 'See you at the station at 6'; Start-Sleep -Milliseconds 700
    Invoke-Kin --send-clipboard
    if (-not (Wait-Until { (Status).queue.count -ge 1 } 10)) { throw 'nothing is queued' }
    # connect again so the debug window has a live connection, and a ping for the latency
    $p3 = New-Phone @() 100
    if (-not (Wait-Until { (Status).state -eq 'phone_connected' } 15)) { throw 'the phone did not come back' }
    Start-Sleep -Seconds 3
    Invoke-Kin --ping
    Start-Sleep -Seconds 2
    # the phone goes away again, and two sends wait in the queue (a text and a file), so the history shows them as queued
    $p3.Process.Kill()
    if (-not (Wait-Until { (Status).state -eq 'phone_offline' } 20)) { throw 'the phone is still connected' }
    Invoke-Kin --dev-set-clipboard 'Dinner at 8? I booked the corner table'; Start-Sleep -Milliseconds 700
    Invoke-Kin --send-clipboard
    Invoke-Kin --send (Join-Path $src 'notes.txt')
    if (-not (Wait-Until { (Status).queue.count -ge 2 } 10)) { throw 'two sends are not queued' }

    Say 'windows (one at a time: a window is opened, photographed and closed)'
    # the "New device paired" notice that every pairing opens must not be in front of the pictures
    Close-Window 'Pair new phone'
    # The main window: one window, three pages. The first command builds it (on that page); the next two tell the open window to switch.
    $first = $true
    foreach ($s in @(@('--history', '01-history.png'), @('--settings', '02-settings.png'), @('--drop', '04-drop.png'))) {
        Invoke-Kin $s[0]
        Shot 'Kin' $s[1] $(if ($first) { 0 } else { 1200 })
        $first = $false
    }
    Close-Window 'Kin'
    foreach ($s in @(@('--debug-window', 'Debug', '03-debug.png'), @('--pair', 'Pair new phone', '05-pairing.png'))) {
        Invoke-Kin $s[0]
        Shot $s[1] $s[2]
        Close-Window $s[1]
    }
    Start-Sleep -Seconds 3
    $after = Mem
    Say ("memory after opening and closing every window: {0} MB private working set, {1} process(es) (before: {2} MB, {3})" -f $after.PrivateWorkingSetMB, $after.Processes, $before.PrivateWorkingSetMB, $before.Processes)
    Write-Host ("MEMORY_BEFORE_MB={0} MEMORY_AFTER_MB={1} PROCESSES_AFTER={2}" -f $before.PrivateWorkingSetMB, $after.PrivateWorkingSetMB, $after.Processes)
}
finally {
    $phones | ForEach-Object { if (-not $_.Process.HasExited) { $_.Process.Kill() } }
    Stop-DevKin 10   # --quit to our data directory; a hung process of ours is stopped by its ID
    if ($backup) { Restore-Clipboard $backup }
    $env:COMPUTERNAME = $origComputerName
    Remove-DevDataDir $work   # (the web views of the windows hold files in it for a moment after Kin has quit)
}
