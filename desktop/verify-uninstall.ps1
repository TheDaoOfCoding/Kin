# Checks that Kin Relay is gone after the uninstaller ran (milestone 1.5). Run it as the person who is signed in, without administrator rights
# (it only reads):
#
#   powershell -ExecutionPolicy Bypass -File verify-uninstall.ps1                      # you kept your data (the default)
#   powershell -ExecutionPolicy Bypass -File verify-uninstall.ps1 -ExpectDataDeleted   # you ticked "Delete the application data"
#
# It checks that these are gone: the two Windows Firewall rules, the Start Menu shortcut, the Explorer "Send to" shortcut, the "Start with Windows"
# value, the installed files and the entry in the list of installed programs, and that Kin is not running. Your data folder (%APPDATA%\Kin: the
# identity, the pairing, the history) is expected to be KEPT unless you ticked the box, and to be gone if you did.
#
# Exit code 0: nothing failed. 1: something that should be gone is still there. It changes nothing: what it finds, it says how to remove by hand.
param(
    [string]$Exe,
    [string]$SendToFolder = [Environment]::GetFolderPath('SendTo'),
    [string[]]$StartMenuFolder = @((Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs'), (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs')),
    [string]$DataDir = (Join-Path $env:APPDATA 'Kin'),
    [switch]$ExpectDataDeleted
)
$ErrorActionPreference = 'Stop'
$script:fail = 0; $script:warn = 0; $script:pass = 0

function Report([string]$status, [string]$what, [string]$detail = '') {
    switch ($status) { 'PASS' { $script:pass++; $color = 'Green' } 'FAIL' { $script:fail++; $color = 'Red' } 'WARN' { $script:warn++; $color = 'Yellow' } default { $color = 'Gray' } }
    $line = '{0,-5} {1}' -f $status, $what
    if ($detail) { $line += "  ($detail)" }
    Write-Host $line -ForegroundColor $color
}
function Gone([string]$what, [bool]$isThere, [string]$howToRemove) {
    if ($isThere) { Report 'FAIL' "$what is still there" $howToRemove } else { Report 'PASS' "$what is gone" }
}

Write-Host 'Kin Relay: checking the uninstall' -ForegroundColor Cyan
if (-not $Exe) { $Exe = Join-Path $env:ProgramFiles 'Kin\Kin.exe' }
$installDir = Split-Path -Parent $Exe

try {
    foreach ($name in 'Kin Relay', 'Kin Relay mDNS') {
        Gone "Firewall rule `"$name`"" (@(Get-NetFirewallRule -DisplayName $name -ErrorAction SilentlyContinue).Count -gt 0) "remove it in an administrator window: netsh advfirewall firewall delete rule name=`"$name`""
    }
} catch { Report 'WARN' 'The firewall rules could not be read' $_.Exception.Message }

foreach ($folder in $StartMenuFolder) {
    $p = Join-Path $folder 'Kin.lnk'
    Gone "Start Menu shortcut $p" (Test-Path -LiteralPath $p) 'delete the file'
}
$sendTo = Join-Path $SendToFolder 'Kin.lnk'
Gone 'Explorer "Send to" shortcut "Kin"' (Test-Path -LiteralPath $sendTo) "delete $sendTo"
$run = (Get-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'Kin' -ErrorAction SilentlyContinue).Kin
Gone 'The "Start with Windows" value "Kin"' ([bool]$run) 'Settings > Apps > Startup, or: reg delete HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v Kin /f'

Gone 'The entry in the list of installed programs' (Test-Path 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Kin') 'Settings > Apps > Installed apps'
Gone 'Kin.exe' (Test-Path -LiteralPath $Exe) "delete $installDir in an administrator window"
Gone 'The uninstaller' (Test-Path -LiteralPath (Join-Path $installDir 'uninstall.exe')) "delete $installDir in an administrator window"
if (Test-Path -LiteralPath $installDir) { Report 'WARN' 'The install folder is still there' "$installDir holds: $((Get-ChildItem -LiteralPath $installDir -Force | ForEach-Object Name) -join ', ')" }
Gone 'Kin (the process)' (@(Get-Process -Name 'Kin' -ErrorAction SilentlyContinue).Count -gt 0) 'right-click the Kin tray icon > Quit'

# The installer keeps its own bookkeeping key (the install folder it offers next time) when the data is kept.
$manu = 'HKLM:\SOFTWARE\Kin Relay\Kin'
if ($ExpectDataDeleted) { Gone 'The installer''s key HKLM\SOFTWARE\Kin Relay\Kin' (Test-Path $manu) 'delete the key in regedit as an administrator' }
elseif (Test-Path $manu) { Report 'INFO' 'The installer kept HKLM\SOFTWARE\Kin Relay\Kin' 'normal: it is the folder to offer at the next install, and goes when the data is deleted' }

# Your data.
$data = Test-Path -LiteralPath $DataDir
if ($ExpectDataDeleted) {
    Gone "Your data folder $DataDir" $data 'delete the folder (this removes your pairing: the phone has to pair again)'
    foreach ($web in (Join-Path $env:APPDATA 'app.kinrelay.desktop'), (Join-Path $env:LOCALAPPDATA 'app.kinrelay.desktop')) {
        Gone "The window cache $web" (Test-Path -LiteralPath $web) 'delete the folder'
    }
} elseif ($data) {
    Report 'PASS' "Your data folder is kept, as it should be" "${DataDir}: identity, pairing and history, for a later install"
} else {
    Report 'INFO' 'There is no data folder' "${DataDir} was not there: Kin never ran as you, or it was deleted"
}

Write-Host ''
Write-Host ("{0} passed, {1} failed, {2} warnings" -f $script:pass, $script:fail, $script:warn) -ForegroundColor $(if ($script:fail) { 'Red' } elseif ($script:warn) { 'Yellow' } else { 'Green' })
if ($script:fail) { Write-Host 'Something was left behind: send this output with your bug report.' -ForegroundColor Red; exit 1 }
exit 0
