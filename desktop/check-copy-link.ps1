# 1.5: the pairing window's "Copy pairing link" button, in the real Kin.exe (release build, developer mode, loopback only, throwaway data dir).
# Opens the pairing window, clicks the button through UI Automation, and checks that the clipboard holds the link of the QR code (the same
# `kinrelay://pair?...` that --dev-print-pair-url writes to pair-url.txt) together with the three "keep this private" formats.
#
#   cd desktop; powershell -ExecutionPolicy Bypass -File check-copy-link.ps1 [-Exe <Kin.exe>] [-Screenshot <png>]
#
# The real clipboard is NOT used (2B.6): the test Kin runs with --dev-clipboard-sink, so the link and the three formats are read from the sink's files
# (tools\kin-dev.ps1; what only the real clipboard proves is in tools\check-real-clipboard.ps1).
# Exit code 0 only if every check passed. The link is a one-time token for a throwaway profile; it is not printed.
param(
    [string]$Exe = (Join-Path $PSScriptRoot 'src-tauri\target\release\Kin.exe'),
    [string]$Screenshot = ''
)
# Only one of the Kin scripts (e2e, smoke, capture, check) may run at a time: they share the real clipboard, the Kin.exe slot and the emulator (tools\kin-lock.ps1).
. (Join-Path $PSScriptRoot '..\tools\kin-lock.ps1'); Enter-KinLock 'check-copy-link'
. (Join-Path $PSScriptRoot '..\tools\kin-dev.ps1')   # Assert-NoGameRunning, Stop-OwnProcess (a Kin of our own beside the installed one: it has its own --data-dir in every command)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'clipboard-backup.ps1')
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing, System.Windows.Forms

$failures = 0
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    if ($ok) { Write-Host "PASS  $name" } else { Write-Host "FAIL  $name  $detail"; $script:failures++ }
}

Assert-NoGameRunning   # this script opens the pairing window of its own Kin
$dataDir = Join-Path $env:LOCALAPPDATA ('Temp\kin-copylink-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($dataDir) | Out-Null
Set-KinDev -Exe $Exe -DataDir $dataDir      # the clipboard sink of this run (kin-dev.ps1): the real clipboard is never used
$proc = $null
try {
    # The same name in every picture, whatever this PC is called.
    $env:COMPUTERNAME = 'DESK-PC'
    $proc = Start-DevKin @('--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-print-pair-url', '--data-dir', ('"' + $dataDir + '"'))
    $urlFile = Join-Path $dataDir 'pair-url.txt'
    for ($i = 0; $i -lt 100 -and -not (Test-Path $urlFile); $i++) { Start-Sleep -Milliseconds 100 }
    # Opens the pairing window of the running instance (it keeps the code that is already there).
    Invoke-Kin --pair --dev-bind 127.0.0.1 --dev-qr-address 10.0.2.2 | Out-Null

    $root = [Windows.Automation.AutomationElement]::RootElement
    $window = $null
    for ($i = 0; $i -lt 100 -and $null -eq $window; $i++) {
        $window = $root.FindFirst('Children', (New-Object Windows.Automation.AndCondition(
            (New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ProcessIdProperty, $proc.Id)),
            (New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, 'Pair new phone')))))
        if ($null -eq $window) { Start-Sleep -Milliseconds 100 }
    }
    Check 'the pairing window opened' ($null -ne $window)
    if ($null -eq $window) { throw 'no pairing window' }

    $buttonCondition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, 'Copy pairing link')
    $button = $null
    for ($i = 0; $i -lt 100 -and $null -eq $button; $i++) {
        $button = $window.FindFirst('Descendants', $buttonCondition)
        if ($null -eq $button) { Start-Sleep -Milliseconds 100 }
    }
    Check 'the "Copy pairing link" button is on the page' ($null -ne $button)
    if ($null -eq $button) { throw 'no button' }

    # Something else on the clipboard first, so a pass can't be what was already there.
    # (Quietly: text on the clipboard that an installed Kin with its automatic sync on would send to the user's phone must be marked. clipboard-backup.ps1)
    Set-QuietClipboardText 'not the link'
    $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()

    $expected = (Get-Content $urlFile -Raw).Trim()
    $text = $null
    $entries = $null
    for ($i = 0; $i -lt 50; $i++) {
        $text = Get-PcClipboardText
        if ($text -ne 'not the link') { break }
        Start-Sleep -Milliseconds 100
    }
    $entries = @((Get-KinSinkCurrent).formats)     # the formats Kin put with the link (name, bytes, dword)
    Check 'the clipboard holds the link of the QR code (same as pair-url.txt)' ($text -eq $expected) "(clipboard text: $(if ($text) { $text.Length.ToString() + ' chars' } else { 'none' }))"
    Check 'the link is a kinrelay://pair URL with a token' ($expected -match '^kinrelay://pair\?.*&tk=[A-Za-z0-9_-]{20,}')

    $exclude = $entries | Where-Object { $_.name -eq 'ExcludeClipboardContentFromMonitorProcessing' }
    $history = $entries | Where-Object { $_.name -eq 'CanIncludeInClipboardHistory' }
    $cloud = $entries | Where-Object { $_.name -eq 'CanUploadToCloudClipboard' }
    Check 'ExcludeClipboardContentFromMonitorProcessing is present' ($null -ne $exclude)
    Check 'CanIncludeInClipboardHistory is present and 0' ($null -ne $history -and $history.dword -eq 0)
    Check 'CanUploadToCloudClipboard is present and 0' ($null -ne $cloud -and $cloud.dword -eq 0)
    $last = @(Get-KinSinkWrites) | Select-Object -Last 1
    Check 'the sink recorded it as a sensitive text write' ($last -and $last.kind -eq 'text' -and $last.flags.sensitive -eq $true -and $last.flags.excludedFromHistory -eq $true -and $last.flags.excludedFromCloud -eq $true)

    $note = $null
    $noteCondition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, "Copied $([char]0x2013) paste it in Kin on your phone (e.g. send it to yourself).")
    for ($i = 0; $i -lt 30 -and $null -eq $note; $i++) {
        $note = $window.FindFirst('Descendants', $noteCondition)
        if ($null -eq $note) { Start-Sleep -Milliseconds 100 }
    }
    Check 'the window says "Copied - paste it in Kin on your phone"' ($null -ne $note)
    # The picture is of the window after the click: it shows the "Copied" note.
    if ($Screenshot) {
        # PrintWindow draws this window alone, even when another program is in front of it (a screen grab would picture that program).
        Add-Type -Namespace Kin -Name Shot -MemberDefinition '[DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);'
        $r = $window.Current.BoundingRectangle
        $bmp = New-Object Drawing.Bitmap([int]$r.Width, [int]$r.Height)
        $g = [Drawing.Graphics]::FromImage($bmp)
        $dc = $g.GetHdc()
        [void][Kin.Shot]::PrintWindow([IntPtr]$window.Current.NativeWindowHandle, $dc, 2)
        $g.ReleaseHdc($dc); $g.Dispose()
        $bmp.Save($Screenshot, [Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
    }
    # The log says a link was copied, and does not hold the link or the token.
    $token = ($expected -split '&' | Where-Object { $_ -like 'tk=*' }) -replace '^tk=', ''
    $logs = Get-ChildItem -Path $dataDir -Recurse -Include '*.log', '*.txt', '*.json' -File | Where-Object { $_.Name -ne 'pair-url.txt' }
    $logText = ($logs | ForEach-Object { Get-Content $_.FullName -Raw -ErrorAction SilentlyContinue }) -join "`n"
    Check 'the log notes the copy' ($logText -match 'pairing link copied')
    Check 'no log or data file (but pair-url.txt, a developer hook) holds the token' (-not $logText.Contains($token))
} catch {
    Check 'the check ran to the end' $false $_.Exception.Message
} finally {
    if ($proc) {
        Stop-DevKin 15      # --quit to our data directory, then (if it hangs) our own process by ID
        if (-not $proc.WaitForExit(5000)) { Stop-OwnProcess $proc; $proc.WaitForExit() }
    }
    Remove-DevDataDir $dataDir   # (the web view of the pairing window holds files in it for a moment after Kin has quit)
}
if ($failures) { Write-Host "$failures check(s) failed"; exit 1 }
Write-Host 'all checks passed'
