# Drops a file on the real drop window, as a person would: a real OLE drag-and-drop (`DoDragDrop` with a file list) from a small window
# of this script's own onto the Kin drop window, with the mouse moved by the script. The fake phone must then get an `offer` for the
# file and download it. (`check-1-3.ps1` covers everything else; this is its own script because it moves the mouse pointer for about
# a second, so don't touch the mouse while it runs.)
#
#   cd desktop; powershell -STA -ExecutionPolicy Bypass -File check-dragdrop.ps1
#
# Loopback only, a throwaway profile and received folder under %LOCALAPPDATA%\Temp; everything it starts is stopped and deleted.
param()
$ErrorActionPreference = 'Stop'
if ([Threading.Thread]::CurrentThread.GetApartmentState() -ne 'STA') { throw 'run it with powershell -STA (OLE drag and drop needs a single-threaded apartment)' }
$here = $PSScriptRoot
$kin = Join-Path $here 'src-tauri\target\release\Kin.exe'
$phoneExe = Join-Path $here 'src-tauri\target\release\examples\fake_phone.exe'
if (-not (Test-Path $phoneExe)) { $phoneExe = Join-Path $here 'src-tauri\target\debug\examples\fake_phone.exe' }
$work = Join-Path $env:LOCALAPPDATA ('Temp\kin13-dnd-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $work 'data'; $recv = Join-Path $work 'received'; $save = Join-Path $work 'phone-saved'
$ok = $true
function Check([string]$n, [bool]$c) { Write-Host ("  [{0}] {1}" -f $(if ($c) { 'ok' } else { 'FAIL' }), $n); if (-not $c) { $script:ok = $false } }
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class Mouse6 {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  public static void Down() { mouse_event(2, 0, 0, 0, UIntPtr.Zero); }
  public static void Up() { mouse_event(4, 0, 0, 0, UIntPtr.Zero); }
}
'@
try {
    [IO.Directory]::CreateDirectory($work) | Out-Null
    if (Get-Process Kin -ErrorAction SilentlyContinue) { throw 'Kin.exe is already running: quit it first' }
    Start-Process $kin -ArgumentList '--data-dir', ('"{0}"' -f $data), '--dev-bind', '127.0.0.1', '--dev-received-dir', ('"{0}"' -f $recv), '--dev-print-pair-url'
    for ($i = 0; $i -lt 50 -and -not (Test-Path -LiteralPath (Join-Path $data 'status.json')); $i++) { Start-Sleep -Milliseconds 200 }
    & $kin --dev-start-pairing; Start-Sleep -Seconds 1
    $url = (Get-Content -LiteralPath (Join-Path $data 'pair-url.txt')).Trim()
    $log = Join-Path $work 'phone.log'
    Start-Process $phoneExe -ArgumentList $url, '--host', '127.0.0.1', '--seconds', '90', '--save-dir', ('"{0}"' -f $save) -RedirectStandardOutput $log -NoNewWindow
    for ($i = 0; $i -lt 50 -and -not ((Get-Content -LiteralPath (Join-Path $data 'status.json') -Raw) -match 'phone_connected'); $i++) { Start-Sleep -Milliseconds 200 }
    # Pairing opens the "New device paired" window in the middle of the screen, where the drop window opens too. Close it, so that the drop
    # lands on the drop window.
    Start-Sleep -Seconds 2
    $paired = [Windows.Automation.AutomationElement]::RootElement.FindFirst([Windows.Automation.TreeScope]::Children, (New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::NameProperty), 'Pair new phone'))
    if ($paired) { $paired.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close(); Start-Sleep -Milliseconds 800 }
    $file = Join-Path $work 'dropped.txt'
    [IO.File]::WriteAllText($file, "dropped on the drop window`n")
    & $kin --drop
    $win = $null
    for ($i = 0; $i -lt 40 -and -not $win; $i++) {
        $win = [Windows.Automation.AutomationElement]::RootElement.FindFirst([Windows.Automation.TreeScope]::Children, (New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::NameProperty), 'Send to phone'))
        if (-not $win) { Start-Sleep -Milliseconds 250 }
    }
    Check 'the drop window is open' ([bool]$win)
    Start-Sleep -Seconds 2   # the page has to load before it listens for the drop (a person needs more than that to get the file there)
    $r = $win.Current.BoundingRectangle
    $target = @([int]($r.X + $r.Width / 2), [int]($r.Y + 130))   # the dashed zone
    # A small window of ours, in the top left corner of the screen, that the drag starts from.
    $form = New-Object Windows.Forms.Form
    $form.FormBorderStyle = 'None'; $form.StartPosition = 'Manual'; $form.Location = New-Object Drawing.Point 20, 20; $form.Size = New-Object Drawing.Size 120, 60
    $form.TopMost = $true; $form.BackColor = [Drawing.Color]::SteelBlue; $form.ShowInTaskbar = $false
    $null = $form.Show(); [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 300
    $data_object = New-Object Windows.Forms.DataObject ([Windows.Forms.DataFormats]::FileDrop), ([string[]]@($file))
    # Button down on our own window, then DoDragDrop (it runs a loop until the button is released), with the pointer moved by a timer.
    # Windows sometimes ends the drag at once when the button press hasn't reached the window yet, so it is tried again (up to three times).
    $effect = 'None'
    for ($attempt = 1; $attempt -le 3 -and $effect -eq 'None'; $attempt++) {
        # A click first, so that the window of ours has the focus and the mouse (a drag from a window that has neither ends at once).
        $null = [Mouse6]::SetForegroundWindow($form.Handle); $null = $form.Activate(); [Windows.Forms.Application]::DoEvents()
        $null = [Mouse6]::SetCursorPos(60, 40); Start-Sleep -Milliseconds 300
        [Mouse6]::Down(); Start-Sleep -Milliseconds 100; [Mouse6]::Up(); Start-Sleep -Milliseconds 300
        [Windows.Forms.Application]::DoEvents()
        [Mouse6]::Down(); Start-Sleep -Milliseconds 200
        $script:steps = 0
        $timer = New-Object Windows.Forms.Timer; $timer.Interval = 30
        $timer.Add_Tick({
            $script:steps++
            $t = [math]::Min(1.0, $script:steps / 25.0)
            $null = [Mouse6]::SetCursorPos([int](60 + ($target[0] - 60) * $t), [int](40 + ($target[1] - 40) * $t))
            if ($script:steps -eq 40) { [Mouse6]::Up() }
        })
        $timer.Start()
        $effect = [string]$form.DoDragDrop($data_object, [Windows.Forms.DragDropEffects]::Copy)
        $timer.Stop(); [Mouse6]::Up()
        Write-Host "  attempt ${attempt}: the drag ended with effect $effect after $($script:steps) timer ticks"
        if ($effect -eq 'None') { Start-Sleep -Milliseconds 500 }
    }
    $form.Close()
    $got = $false
    for ($i = 0; $i -lt 40 -and -not $got; $i++) { $got = ((Get-Content -LiteralPath $log -Raw) -match 'downloaded dropped.txt'); if (-not $got) { Start-Sleep -Milliseconds 250 } }
    Check 'the phone was offered the dropped file and downloaded it' $got
    if (-not $got) {
        Write-Host '  --- the end of the PC log:'; Get-Content -LiteralPath (Join-Path $data 'logs\kin.log') -ErrorAction SilentlyContinue | Select-Object -Last 8 | ForEach-Object { Write-Host "    $_" }
        Write-Host '  --- the end of the phone log:'; Get-Content -LiteralPath $log -ErrorAction SilentlyContinue | Select-Object -Last 5 | ForEach-Object { Write-Host "    $_" }
    }
    Check 'the downloaded bytes are the file''s' ((Test-Path (Join-Path $save 'dropped.txt')) -and ((Get-FileHash $file).Hash -eq (Get-FileHash (Join-Path $save 'dropped.txt')).Hash))
}
finally {
    Get-Process fake_phone -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    if (Get-Process Kin -ErrorAction SilentlyContinue) { & $kin --quit; Start-Sleep -Seconds 2 }
    Get-Process Kin, fake_phone -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    if ([IO.Directory]::Exists($work)) { try { [IO.Directory]::Delete($work, $true) } catch { } }
}
if (-not $ok) { Write-Host 'FAILED'; exit 1 } else { Write-Host 'all checks passed' }
