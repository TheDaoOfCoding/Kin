# Checks a Kin Relay installation after the installer ran (milestone 1.5). Run it as the person who is signed in, without administrator
# rights (it only reads), after the installer has finished and Kin has been started once:
#
#   powershell -ExecutionPolicy Bypass -File verify-install.ps1
#   powershell -ExecutionPolicy Bypass -File verify-install.ps1 -Toast     # also shows a test notification, to look at its name and icon
#
# It checks: the installed program, the Windows Firewall rules, the Start Menu shortcut and its app user model ID (what makes a notification
# say "Kin" with the app's icon instead of "Windows PowerShell"), the Explorer "Send to" shortcut, the "Start with Windows" value, and that Kin
# runs from the installed folder. The "Send to" shortcut and the "Start with Windows" value are made by the app the first time it runs (as you,
# not as the administrator who ran the installer), so they show as WARN until Kin has been started once.
#
# Exit code 0: nothing failed (warnings are allowed). 1: something failed. It changes nothing.
# The folder parameters are there so the script itself can be tried on a test folder.
param(
    [string]$Exe,
    [string]$SendToFolder = [Environment]::GetFolderPath('SendTo'),
    [string[]]$StartMenuFolder = @((Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs'), (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs')),
    [string]$DataDir = (Join-Path $env:APPDATA 'Kin'),
    [switch]$Toast
)
$ErrorActionPreference = 'Stop'
$AppId = 'app.kinrelay.desktop'
$script:fail = 0; $script:warn = 0; $script:pass = 0

function Report([string]$status, [string]$what, [string]$detail = '') {
    switch ($status) { 'PASS' { $script:pass++; $color = 'Green' } 'FAIL' { $script:fail++; $color = 'Red' } 'WARN' { $script:warn++; $color = 'Yellow' } default { $color = 'Gray' } }
    $line = '{0,-5} {1}' -f $status, $what
    if ($detail) { $line += "  ($detail)" }
    Write-Host $line -ForegroundColor $color
}

# The app user model ID of a shortcut, read from its property store (System.AppUserModel.ID).
if (-not ('KinVerify.ShortcutProps' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace KinVerify {
  public static class ShortcutProps {
    [StructLayout(LayoutKind.Sequential, Pack = 4)] struct PROPERTYKEY { public Guid fmtid; public uint pid; }
    [StructLayout(LayoutKind.Explicit, Size = 24)] struct PROPVARIANT { [FieldOffset(0)] public ushort vt; [FieldOffset(8)] public IntPtr p; }
    [ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IPropertyStore {
      [PreserveSig] int GetCount(out uint count);
      [PreserveSig] int GetAt(uint index, out PROPERTYKEY key);
      [PreserveSig] int GetValue(ref PROPERTYKEY key, out PROPVARIANT value);
      [PreserveSig] int SetValue(ref PROPERTYKEY key, ref PROPVARIANT value);
      [PreserveSig] int Commit();
    }
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
    static extern void SHGetPropertyStoreFromParsingName(string path, IntPtr bindContext, int flags, ref Guid riid, out IPropertyStore store);
    [DllImport("ole32.dll")] static extern int PropVariantClear(ref PROPVARIANT value);
    static PROPERTYKEY AppId() { PROPERTYKEY k; k.fmtid = new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"); k.pid = 5; return k; }
    public static string GetAppUserModelId(string path) {
      Guid iid = new Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99");
      IPropertyStore store; SHGetPropertyStoreFromParsingName(path, IntPtr.Zero, 0, ref iid, out store);
      try {
        PROPERTYKEY key = AppId(); PROPVARIANT v;
        int hr = store.GetValue(ref key, out v);
        if (hr != 0) throw new COMException("GetValue failed", hr);
        string text = (v.vt == 31 && v.p != IntPtr.Zero) ? Marshal.PtrToStringUni(v.p) : null; // VT_LPWSTR
        PropVariantClear(ref v);
        return text;
      } finally { Marshal.ReleaseComObject(store); }
    }
    // Only for trying this script on a test shortcut.
    public static void SetAppUserModelId(string path, string id) {
      Guid iid = new Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99");
      IPropertyStore store; SHGetPropertyStoreFromParsingName(path, IntPtr.Zero, 2, ref iid, out store); // GPS_READWRITE
      try {
        PROPERTYKEY key = AppId(); PROPVARIANT v = new PROPVARIANT(); v.vt = 31; v.p = Marshal.StringToCoTaskMemUni(id);
        int hr = store.SetValue(ref key, ref v); if (hr != 0) throw new COMException("SetValue failed", hr);
        hr = store.Commit(); if (hr != 0) throw new COMException("Commit failed", hr);
        Marshal.FreeCoTaskMem(v.p);
      } finally { Marshal.ReleaseComObject(store); }
    }
  }
}
'@
}

function Read-Shortcut([string]$path) {
    $link = (New-Object -ComObject WScript.Shell).CreateShortcut($path)
    [pscustomobject]@{ Target = $link.TargetPath; Arguments = $link.Arguments }
}
function Same-Path([string]$a, [string]$b) { return ([string]$a).TrimEnd('\') -ieq ([string]$b).TrimEnd('\') }

Write-Host 'Kin Relay: checking the installation' -ForegroundColor Cyan
if (-not [Environment]::Is64BitProcess) { Report 'WARN' 'This is a 32-bit PowerShell' 'the app user model ID check needs the 64-bit one: start powershell.exe from the Start menu' }
if ([Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Report 'INFO' 'This window is elevated' 'fine, but the shortcuts and the Run value are checked for the account that runs it: use a normal window to check your own'
}

# ---- 1. the installed program
$uninstallKey = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Kin'
$entry = Get-ItemProperty -Path $uninstallKey -ErrorAction SilentlyContinue
if ($entry) {
    Report 'PASS' 'Kin is in the list of installed programs (per machine)' "$($entry.DisplayName) $($entry.DisplayVersion), publisher $($entry.Publisher)"
    if (-not $Exe) { $Exe = Join-Path (([string]$entry.InstallLocation).Trim('"')) 'Kin.exe' }
} else {
    Report 'FAIL' 'Kin is not in the list of installed programs for all users' "$uninstallKey is missing: was the installer finished, and run as administrator?"
    if (-not $Exe) { $Exe = Join-Path $env:ProgramFiles 'Kin\Kin.exe' }
}
if (Test-Path -LiteralPath $Exe) {
    $item = Get-Item -LiteralPath $Exe
    $sig = (Get-AuthenticodeSignature -LiteralPath $Exe).Status
    Report 'PASS' 'Kin.exe is there' ("{0}, {1:N1} MB, file version {2}" -f $Exe, ($item.Length / 1MB), $item.VersionInfo.FileVersion)
    Report 'INFO' "Signature: $sig" 'unsigned is expected in the MVP: this is why Windows SmartScreen warned at the installer'
    if ($Exe -notlike "$env:ProgramFiles*") { Report 'WARN' 'Kin.exe is not under Program Files' $Exe }
} else {
    Report 'FAIL' 'Kin.exe is not there' $Exe
}

# ---- 2. the firewall rules
function Check-Rule([string]$name, [string]$protocol, [string]$localPort) {
    $rules = @(Get-NetFirewallRule -DisplayName $name -ErrorAction SilentlyContinue)
    if ($rules.Count -eq 0) { Report 'FAIL' "Firewall rule `"$name`" is missing" 'the installer adds it; a phone can not connect without it'; return }
    if ($rules.Count -gt 1) { Report 'WARN' "Firewall rule `"$name`" exists $($rules.Count) times" 'harmless, but the installer should leave one' }
    $rule = $rules[0]
    $app = ($rule | Get-NetFirewallApplicationFilter).Program
    $port = $rule | Get-NetFirewallPortFilter
    $profiles = $rule.Profile.ToString()
    $problems = @()
    if ($rule.Enabled.ToString() -ne 'True') { $problems += 'it is not enabled' }
    if ($rule.Direction.ToString() -ne 'Inbound') { $problems += "direction is $($rule.Direction)" }
    if ($rule.Action.ToString() -ne 'Allow') { $problems += "action is $($rule.Action)" }
    if ($profiles -ne 'Any' -and ($profiles -notmatch 'Private' -or $profiles -notmatch 'Public')) { $problems += "profiles are $profiles (needs Private and Public)" }
    if ($port.Protocol -ne $protocol) { $problems += "protocol is $($port.Protocol)" }
    if ($localPort -and $port.LocalPort -ne $localPort) { $problems += "local port is $($port.LocalPort)" }
    if (-not (Same-Path $app $Exe)) { $problems += "it is for $app and not for $Exe" }
    # Since v0.1.1 the rules only answer the PC's own subnet (D12 addendum). A rule that says Any is from an older installer: it works, so WARN.
    $remote = (($rule | Get-NetFirewallAddressFilter).RemoteAddress | ForEach-Object { "$_" }) -join ','
    if ($problems.Count) { Report 'FAIL' "Firewall rule `"$name`" is wrong" ($problems -join '; ') }
    else {
        Report 'PASS' "Firewall rule `"$name`"" "inbound, allow, $protocol$(if ($localPort) { " $localPort" }), profiles $profiles, remote $remote, for $app"
        if ($remote -ne 'LocalSubnet') { Report 'WARN' "Firewall rule `"$name`" answers $remote, not only the local subnet" 'rules from before v0.1.1 do; reinstall to get remoteip=LocalSubnet (a phone on another subnet than the PC needs this rule widened by hand)' }
    }
}
try {
    Check-Rule 'Kin Relay' 'TCP' ''
    Check-Rule 'Kin Relay mDNS' 'UDP' '5353'
} catch {
    Report 'WARN' 'The firewall rules could not be read' $_.Exception.Message
}

# ---- 3. the Start Menu shortcut and its app user model ID
$start = $null
foreach ($folder in $StartMenuFolder) {
    $candidate = Join-Path $folder 'Kin.lnk'
    if (Test-Path -LiteralPath $candidate) { $start = $candidate; break }
}
if ($start) {
    $s = Read-Shortcut $start
    if (Same-Path $s.Target $Exe) { Report 'PASS' 'Start Menu shortcut "Kin"' "$start -> $($s.Target)" } else { Report 'FAIL' 'Start Menu shortcut "Kin" points at the wrong file' "$($s.Target), expected $Exe" }
    try {
        $id = [KinVerify.ShortcutProps]::GetAppUserModelId($start)
        if ($id -eq $AppId) { Report 'PASS' 'The Start Menu shortcut has the app user model ID' $id }
        elseif ($id) { Report 'FAIL' 'The Start Menu shortcut has another app user model ID' "$id, expected $AppId" }
        else { Report 'FAIL' 'The Start Menu shortcut has no app user model ID' "toasts would say Windows PowerShell; expected $AppId" }
    } catch {
        Report 'WARN' 'The app user model ID of the shortcut could not be read' $_.Exception.Message
    }
} else {
    Report 'FAIL' 'There is no Start Menu shortcut "Kin"' ($StartMenuFolder -join ' or ')
}

# ---- 4. what the app makes for you the first time it runs
$sendTo = Join-Path $SendToFolder 'Kin.lnk'
$firstRunHint = 'made when Kin runs for the first time as you: start Kin from the Start menu, wait a few seconds, and run this again'
if (Test-Path -LiteralPath $sendTo) {
    $s = Read-Shortcut $sendTo
    if ((Same-Path $s.Target $Exe) -and $s.Arguments -eq '--send') { Report 'PASS' 'Explorer "Send to" shortcut "Kin"' "$sendTo -> $($s.Target) $($s.Arguments)" }
    else { Report 'FAIL' 'The "Send to" shortcut is wrong' "$($s.Target) $($s.Arguments), expected $Exe --send" }
} else {
    Report 'WARN' 'There is no "Send to" shortcut yet' $firstRunHint
}
$intentFile = Join-Path $DataDir 'integration.json'
$intent = $null
if (Test-Path -LiteralPath $intentFile) { try { $intent = Get-Content -LiteralPath $intentFile -Raw | ConvertFrom-Json } catch { } }
$run = (Get-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'Kin' -ErrorAction SilentlyContinue).Kin
if ($run) {
    if ($run -ieq ('"' + $Exe + '"')) { Report 'PASS' 'Start with Windows (Run value "Kin", current user)' $run }
    else { Report 'FAIL' 'The Run value "Kin" is not the quoted path of the installed Kin.exe' "$run, expected `"$Exe`"" }
} elseif ($intent -and $intent.startWithWindows -eq $false) {
    Report 'INFO' 'Start with Windows is off' 'you turned it off in Settings: that is respected'
} else {
    Report 'WARN' 'There is no Run value "Kin"' $firstRunHint
}

# ---- 5. the running app
$running = @(Get-Process -Name 'Kin' -ErrorAction SilentlyContinue)
if ($running.Count -eq 0) { Report 'WARN' 'Kin is not running' 'start it from the Start menu: it lives in the tray, near the clock' }
else {
    $paths = $running | ForEach-Object { try { $_.Path } catch { '' } }
    if ($paths | Where-Object { $_ -and -not (Same-Path $_ $Exe) }) { Report 'WARN' 'A Kin is running from somewhere else' ($paths -join ', ') }
    else { Report 'PASS' 'Kin is running from the installed folder' "pid $($running[0].Id), $([math]::Round($running[0].PrivateMemorySize64 / 1MB, 1)) MB private" }
}

# ---- 6. a notification, to look at
if ($Toast) {
    try {
        [void][Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
        [void][Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime]
        $xml = New-Object Windows.Data.Xml.Dom.XmlDocument
        $xml.LoadXml('<toast><visual><binding template="ToastGeneric"><text>Kin: notification check</text><text>The name above this text should be Kin, with the Kin icon. If it says Windows PowerShell, the Start Menu shortcut has no app user model ID.</text></binding></visual></toast>')
        [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($AppId).Show([Windows.UI.Notifications.ToastNotification]::new($xml))
        Report 'INFO' 'A test notification was shown' 'look at its name and icon (it is also in the notification centre): "Kin" with the app icon is right'
    } catch {
        Report 'WARN' 'The test notification could not be shown' $_.Exception.Message
    }
} else {
    Report 'INFO' 'Notification name and icon not checked' 'run with -Toast to show one and look at it'
}

Write-Host ''
Write-Host ("{0} passed, {1} failed, {2} warnings" -f $script:pass, $script:fail, $script:warn) -ForegroundColor $(if ($script:fail) { 'Red' } elseif ($script:warn) { 'Yellow' } else { 'Green' })
if ($script:fail) { Write-Host 'Something is wrong: send this output with your bug report.' -ForegroundColor Red; exit 1 }
exit 0
