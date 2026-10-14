# Measures the release Kin.exe the same way as the .NET spike (docs/results/phase-0-dotnet.md), plus the WebView2 children.
#
#   -Mode Session   launch, idle for -IdleSeconds, sample (window never opened), open the window from the tray menu,
#                   sample, close the window, wait, sample again, then Quit from the tray menu.
#   -Mode SingleInstance   launch Kin twice: the second process must exit on its own and the first must show its window.
#   -Mode Quit      Quit an already running Kin from its tray menu (test helper).
#   -Mode Startup   launch -Runs times and report time from process start until WaitForInputIdle returns. Each run is
#                   stopped with Stop-Process (a startup loop does not need a graceful exit).
#
# The tray is driven with the keyboard (Win+B, Enter, Apps key, arrows, Enter), because the Win32 popup menu does not expose
# its items to UI Automation and synthetic right-clicks on the Windows 11 flyout turned out to be unreliable. UI Automation
# is only used to check the flyout state and the results. Needs an unlocked interactive desktop. Kin must be the first icon
# in the overflow flyout (a new icon lands there).
# Kin's WebView2 processes are found as the descendants of Kin.exe plus any msedgewebview2.exe whose command line mentions
# the app identifier (other apps' WebView2 processes are left out). Private working set comes from the perf counters.
param(
    [ValidateSet('Session', 'Startup', 'SingleInstance', 'Quit')] [string]$Mode = 'Session',
    [int]$IdleSeconds = 60,
    [int]$Runs = 5,
    [switch]$HideOnClose
)
$ErrorActionPreference = 'Stop'
$exe = Join-Path $PSScriptRoot 'src-tauri\target\release\Kin.exe'
$appId = 'app.kinrelay.desktop'

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class Native {
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  public static void Chord(byte mod, byte vk) { keybd_event(mod,0,0,UIntPtr.Zero); keybd_event(vk,0,0,UIntPtr.Zero); keybd_event(vk,0,2,UIntPtr.Zero); keybd_event(mod,0,2,UIntPtr.Zero); }
  public static void Key(byte vk) { keybd_event(vk,0,0,UIntPtr.Zero); keybd_event(vk,0,2,UIntPtr.Zero); }

  // True once the process owns a top-level window of this class. Kin's tray icon lives on a hidden "tray_icon_app" window
  // that the tray-icon crate creates right before it registers the icon with the shell.
  delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  public static bool HasWindowClass(uint pid, string cls) { bool found = false;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid) { var sb = new System.Text.StringBuilder(256);
      GetClassName(h, sb, 256); if (sb.ToString() == cls) found = true; } return !found; }, IntPtr.Zero); return found; }
}
"@
$AE = [Windows.Automation.AutomationElement]
function By-Class($c) { New-Object Windows.Automation.PropertyCondition($AE::ClassNameProperty, $c) }
function By-Name($n) { New-Object Windows.Automation.PropertyCondition($AE::NameProperty, $n) }

# ---- tray driving ---------------------------------------------------------------------------------------------------
function Get-Flyout { $AE::RootElement.FindFirst('Children', (By-Class 'TopLevelWindowForOverflowXamlIsland')) }
# Closes the overflow flyout if it is open, by toggling the chevron (Esc only works when the flyout has focus).
function Close-Flyout {
    if (-not (Get-Flyout)) { return }
    $tray = $AE::RootElement.FindFirst('Children', (By-Class 'Shell_TrayWnd'))
    # While the flyout is open the chevron's name gains a suffix ("Show Hidden Icons Hide ..."), so match on the prefix.
    $chev = $tray.FindAll('Descendants', (By-Class 'SystemTray.NormalButton')) | ? { $_.Current.Name -like 'Show Hidden Icons*' } | select -First 1
    $chev.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 700
}
# Opens the flyout from a closed state with Win+B, Enter. Returns Kin's button, which should hold keyboard focus.
function Open-Flyout {
    Close-Flyout
    [Native]::Chord(0x5B, 0x42); Start-Sleep -Milliseconds 700   # Win+B: focus the notification area
    [Native]::Key(0x0D); Start-Sleep -Milliseconds 900           # Enter: open the overflow flyout
    $ov = Get-Flyout
    if ($ov) { $ov.FindFirst('Descendants', (By-Name 'Kin')) }
}
function Test-KinFocused {
    foreach ($i in 1..10) {
        $f = $AE::FocusedElement
        if ($f -and $f.Current.Name -eq 'Kin' -and $f.Current.ClassName -eq 'SystemTray.NormalButton') { return $true }
        Start-Sleep -Milliseconds 200
    }
    $false
}
# $presses = number of Down presses before Enter: 1 = Open, 2 = Quit
function Invoke-TrayMenu([int]$presses) {
    foreach ($try in 1..3) {
        $k = Open-Flyout
        if ($k -and (Test-KinFocused)) {
            [Native]::Key(0x5D); Start-Sleep -Milliseconds 900                    # Apps key: context menu of the focused icon
            if ($AE::RootElement.FindFirst('Children', (By-Class '#32768'))) {
                1..$presses | % { [Native]::Key(0x28); Start-Sleep -Milliseconds 250 }   # Down
                [Native]::Key(0x0D)                                                       # Enter
                Start-Sleep -Milliseconds 500
                return
            }
        }
        [Native]::Key(0x1B); [Native]::Key(0x1B); Start-Sleep -Milliseconds 500   # dismiss whatever is open and retry
    }
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $bmp = New-Object Drawing.Bitmap 700, 300; $g = [Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen(1220, 900, 0, 0, $bmp.Size)
    $bmp.Save((Join-Path ([IO.Path]::GetFullPath($env:TEMP)) 'kin-tray-fail.png'))
    throw 'tray menu did not open after 3 tries (screenshot in the temp folder: kin-tray-fail.png)'
}
function Test-KinTrayIcon { $k = Open-Flyout; $found = [bool]$k; Close-Flyout; $found }

# ---- sampling -------------------------------------------------------------------------------------------------------
function Get-KinTree([int]$rootPid) {
    $all = Get-CimInstance Win32_Process
    $ids = [System.Collections.Generic.HashSet[int]]::new(); [void]$ids.Add($rootPid)
    do { $n = $ids.Count
         foreach ($p in $all) { if ($ids.Contains([int]$p.ParentProcessId)) { [void]$ids.Add([int]$p.ProcessId) } }
    } while ($ids.Count -ne $n)
    foreach ($p in $all) { if ($p.Name -eq 'msedgewebview2.exe' -and $p.CommandLine -like "*$appId*") { [void]$ids.Add([int]$p.ProcessId) } }
    $ids
}
function Sample([string]$label, [int]$rootPid) {
    $ids = Get-KinTree $rootPid
    $perf = @{}
    Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | % { $perf[[int]$_.IDProcess] = $_ }
    $cmd = @{}
    Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | % { $cmd[[int]$_.ProcessId] = $_.CommandLine }
    $rows = foreach ($id in $ids) {
        $p = Get-Process -Id $id -ErrorAction SilentlyContinue; if (-not $p) { continue }
        $role = if ($p.ProcessName -ne 'msedgewebview2') { 'app' } elseif ($cmd[$id] -match '--type=([\w-]+)') { $Matches[1] } else { 'browser' }
        [pscustomobject]@{ Pid = $id; Name = $p.ProcessName; Role = $role
            WorkingSetMB = [math]::Round($p.WorkingSet64 / 1MB, 1)
            PrivateWSMB = [math]::Round($perf[$id].WorkingSetPrivate / 1MB, 1)
            PrivateBytesMB = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
            Threads = $p.Threads.Count; Handles = $p.HandleCount }
    }
    $tot = [pscustomobject]@{ Processes = @($rows).Count
        WorkingSetMB = [math]::Round(($rows | Measure-Object WorkingSetMB -Sum).Sum, 1)
        PrivateWSMB = [math]::Round(($rows | Measure-Object PrivateWSMB -Sum).Sum, 1)
        PrivateBytesMB = [math]::Round(($rows | Measure-Object PrivateBytesMB -Sum).Sum, 1) }
    "=== $label"; $rows | Format-Table -AutoSize | Out-String -Width 200; "TOTAL: $($tot | ConvertTo-Json -Compress)"; ""
}

# Starts Kin and returns the process plus two times in ms: WaitForInputIdle (what the .NET spike reported) and the moment
# the tray icon window exists. For Tauri, WaitForInputIdle can return before the event loop is up, so the second is the real one.
function Start-Kin {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process $exe -PassThru
    $idleOk = $p.WaitForInputIdle(10000); $idle = $sw.Elapsed.TotalMilliseconds
    while (-not [Native]::HasWindowClass([uint32]$p.Id, 'tray_icon_app') -and $sw.Elapsed.TotalSeconds -lt 10) { Start-Sleep -Milliseconds 1 }
    [pscustomobject]@{ Process = $p; IdleMs = $idle; IdleOk = $idleOk; TrayMs = $sw.Elapsed.TotalMilliseconds }
}

# ---- run ------------------------------------------------------------------------------------------------------------
if ($Mode -ne 'Quit' -and (Get-Process Kin -ErrorAction SilentlyContinue)) { throw 'Kin is already running; quit it first' }
if ($HideOnClose) { $env:KIN_HIDE_ON_CLOSE = '1' } else { Remove-Item Env:KIN_HIDE_ON_CLOSE -ErrorAction SilentlyContinue }

if ($Mode -eq 'SingleInstance') {
    $k = Start-Kin; $first = $k.Process
    Start-Sleep -Milliseconds 500
    $second = Start-Process $exe -PassThru
    $exited = $second.WaitForExit(5000)
    "second instance exited on its own: $exited (exit code $(if ($exited) { $second.ExitCode }))"
    Start-Sleep -Seconds 3
    "Kin processes running: $(@(Get-Process Kin -ErrorAction SilentlyContinue).Count)"
    "first instance shows the window: $([bool]$AE::RootElement.FindFirst('Children', (By-Name 'Kin Relay')))"
    Stop-Process -Id $first.Id -Force; $first.WaitForExit()
    "(first instance stopped with Stop-Process; this mode does not test Quit)"
    return
}

if ($Mode -eq 'Quit') {
    $p = Get-Process Kin -ErrorAction Stop
    Invoke-TrayMenu 2
    [void]$p.WaitForExit(8000)
    "Quit: process exited = $($p.HasExited)"
    return
}

if ($Mode -eq 'Startup') {
    1..$Runs | % {
        $k = Start-Kin
        "run {0}: WaitForInputIdle {1:N0} ms (returned true: {2}); tray icon window ready {3:N0} ms" -f $_, $k.IdleMs, $k.IdleOk, $k.TrayMs
        Stop-Process -Id $k.Process.Id -Force; $k.Process.WaitForExit(); Start-Sleep -Milliseconds 500
    }
    return
}

$launch = [Diagnostics.Stopwatch]::StartNew()
$k = Start-Kin; $p = $k.Process
"startup: WaitForInputIdle {0:N0} ms; tray icon window ready {1:N0} ms; hide-on-close={2}" -f $k.IdleMs, $k.TrayMs, [bool]$HideOnClose
Start-Sleep -Seconds ([math]::Max(0, $IdleSeconds - $launch.Elapsed.TotalSeconds))
Sample "A: idle ~$IdleSeconds s after launch, window never opened" $p.Id
"tray icon present (tooltip button 'Kin'): $(Test-KinTrayIcon)"

Invoke-TrayMenu 1                                   # Open
Start-Sleep -Seconds 5
$w = $AE::RootElement.FindFirst('Children', (By-Name 'Kin Relay'))
"Open: window found = $([bool]$w)"
if ($w) {
    # The first UIA query makes WebView2 build its accessibility tree, so give it a few tries.
    foreach ($try in 1..5) { $text = $w.FindFirst('Descendants', (By-Name 'Hello from Kin Relay')); if ($text) { break }; Start-Sleep -Seconds 1 }
    "  text 'Hello from Kin Relay' found = $([bool]$text)"
}
Sample "B: window open (5 s after Open)" $p.Id

if ($w) { $w.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close() }   # the window's close button
Start-Sleep -Seconds 10
$p.Refresh()
"after closing the window: process alive = $(-not $p.HasExited); window still visible = $([bool]$AE::RootElement.FindFirst('Children', (By-Name 'Kin Relay')))"
Sample "C: 10 s after closing the window" $p.Id

Invoke-TrayMenu 2                                   # Quit
[void]$p.WaitForExit(8000)
"Quit: process exited = $($p.HasExited); exit code = $(if ($p.HasExited) { $p.ExitCode })"
Start-Sleep -Seconds 2
"leftover Kin WebView2 processes: $(@(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | ? { $_.CommandLine -like "*$appId*" }).Count)"
"tray icon after Quit: $(Test-KinTrayIcon)"
