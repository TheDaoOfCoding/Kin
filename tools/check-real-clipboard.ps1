# THE ONE check that uses the user's REAL Windows clipboard (milestone 2B.6). ASCII only.
#
#   powershell -ExecutionPolicy Bypass -File tools\check-real-clipboard.ps1 -RealClipboard
#
# *** RUN IT ONLY DELIBERATELY: it overwrites the clipboard of the person at the keyboard for about a minute (saved in every format first and put back at the end, only if
# *** they have not copied something meanwhile), it must not run while that person is working, and it is not part of any regression run. It refuses without -RealClipboard.
#
# Every other script runs its test Kin with --dev-clipboard-sink (Kin never opens the real clipboard; tools\kin-dev.ps1, desktop\src-tauri\src\clipboard_sink.rs). The sink proves
# what Kin WOULD put on the clipboard. This script proves what only the real clipboard can show, the Win32 side:
#   A. the real-clipboard unit tests (cargo, #[ignore]d): a text round trip with the opt-out formats, and clear_if_text
#   B. a dev Kin on the real clipboard (own throwaway data directory, loopback, --dev-no-windows, no browser, settings with the automatic sync off) and the fake phone:
#      1. --dev-set-clipboard puts CF_UNICODETEXT plus ExcludeClipboardContentFromMonitorProcessing and KinRelayDevTestContent on the clipboard, and --dev-get-clipboard reads it back
#      2. --dev-clipboard-mark no-history puts CanIncludeInClipboardHistory = 0 on it
#      3. a camera photo NEVER goes to the clipboard (every format and size as it was)
#      4. a screenshot goes on it as CF_DIBV5 (+ PNG + CF_HDROP) with the three sensitive formats: ExcludeClipboardContentFromMonitorProcessing, CanIncludeInClipboardHistory = 0,
#         CanUploadToCloudClipboard = 0, and no text
#      5. the user's installed Kin (if one runs; its log is only READ) skips the marked content and sends nothing of it
# It never touches the installed Kin (process, data, AUMID, Run key, SendTo, firewall), kills only processes it started, makes no sound, injects no input.
param([switch]$RealClipboard, [switch]$SkipUnitTests, [switch]$KeepEvidence)
$ErrorActionPreference = 'Stop'
if (-not $RealClipboard) {
    Write-Host 'This check uses the user''s REAL clipboard (it overwrites it for about a minute and puts it back). Run it only deliberately, with -RealClipboard.'
    exit 2
}
$Repo = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'kin-lock.ps1'); Enter-KinLock 'check-real-clipboard'
. (Join-Path $PSScriptRoot 'kin-dev.ps1')
. (Join-Path $Repo 'desktop\clipboard-backup.ps1')
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $cargoBin })) { $env:Path = "$cargoBin;$env:Path" }

$Target = Join-Path $Repo 'desktop\src-tauri\target\debug'
$KinExe = Join-Path $Target 'Kin.exe'
$Fake = Join-Path $Target 'examples\fake_phone.exe'
foreach ($f in $KinExe, $Fake) { if (-not (Test-Path $f)) { throw "$f is missing: cd desktop\src-tauri; cargo build; cargo build --examples" } }
Assert-NoGameRunning

$script:Fails = 0
function Check([string]$Name, [bool]$Ok, [string]$Detail = '') {
    if ($Ok) { Write-Host "PASS $Name" } else { Write-Host "FAIL $Name $Detail"; $script:Fails++ }
}
function Wait-Until([scriptblock]$Cond, [int]$Seconds, [string]$What) {
    $end = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $end) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds 300 }
    Write-Host "(timed out waiting for $What)"; return $false
}
function Read-Shared([string]$Path) {
    try {
        $fs = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
        try { $ms = New-Object IO.MemoryStream; $fs.CopyTo($ms); return [Text.Encoding]::GetEncoding(28591).GetString($ms.ToArray()) } finally { $fs.Dispose() }
    } catch { return '' }
}
function Start-Fake([string[]]$ArgList, [string]$OutName) {
    $out = Join-Path $Data $OutName
    $args2 = ($ArgList | ForEach-Object { if ($_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ } }) -join ' '
    return Start-Process $Fake -ArgumentList $args2 -PassThru -RedirectStandardOutput $out -RedirectStandardError (Join-Path $Data ($OutName + '.err')) -WindowStyle Hidden
}
function Fake-Text([string]$OutName) { return (Read-Shared (Join-Path $Data $OutName)) }
function Clip-Signature { return ((Get-ClipboardFormats | ForEach-Object { "$($_.Name):$($_.Bytes)" }) -join ',') }
function Get-RealDword([string]$Name) {
    $e = [KinClip]::Read($true) | Where-Object { $_.Name -eq $Name } | Select-Object -First 1
    if ($e -and $e.Data.Length -ge 4) { return [BitConverter]::ToUInt32($e.Data, 0) }
    return $null
}
$installedLog = Join-Path $env:APPDATA 'Kin\logs\kin.log'
function Read-InstalledLogSince([string]$UtcStamp) {
    # READ ONLY (the installed Kin keeps writing: share mode ReadWrite). Lines are 'yyyy-MM-ddTHH:mm:ss.fffZ LEVEL module: message'.
    if (-not (Test-Path -LiteralPath $installedLog)) { return @() }
    $fs = [IO.File]::Open($installedLog, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    try { $text = (New-Object IO.StreamReader($fs)).ReadToEnd() } finally { $fs.Dispose() }
    @($text -split "`n" | Where-Object { $_ -match '^\d{4}-' -and ($_.Substring(0, 24) -gt $UtcStamp) })
}

$installedBefore = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
$Data = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-real-clipboard-' + $PID)
New-Item -ItemType Directory -Force $Data | Out-Null
Set-KinDev -Exe $KinExe -DataDir $Data -RealClipboard      # loud, on purpose: no sink in this run
$kin = $null; $fakeProc = $null; $backup = $null; $startSignature = ''
$identity = Join-Path $Data 'fake-identity.json'
$png = Join-Path $Data 'Screenshot_real_check.png'
Copy-Item (Join-Path $Repo 'protocol\fixtures\media-art-16.png') $png
try {
    $backup = Save-Clipboard | Select-Object -Last 1
    $startSignature = Clip-Signature
    Write-Host 'INFO the clipboard was saved in all formats; it is put back at the end (unless the user copied something meanwhile)'

    # ----- A. the real-clipboard unit tests
    if (-not $SkipUnitTests) {
        Push-Location (Join-Path $Repo 'desktop\src-tauri')
        try {
            $ErrorActionPreference = 'Continue'
            & cargo test --lib real_clipboard_round_trip_and_password_manager_formats -- --ignored --test-threads=1 2>&1 | Out-Null
            Check 'A1 cargo test: real clipboard text round trip and the password-manager formats' ($LASTEXITCODE -eq 0)
            & cargo test --lib real_clipboard_clear_if_text_clears_only_the_exact_text -- --ignored --test-threads=1 2>&1 | Out-Null
            Check 'A2 cargo test: clear_if_text on the real clipboard' ($LASTEXITCODE -eq 0)
            $ErrorActionPreference = 'Stop'
        } finally { Pop-Location }
    }

    # ----- B. a dev Kin on the real clipboard
    '{ "hotkey": "", "autoClipboardSync": false }' | Set-Content (Join-Path $Data 'settings.json') -Encoding ASCII
    $kin = Start-DevKin @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-print-pair-url', '--dev-toast-quiet', '--dev-no-open', '--dev-no-windows')
    $log = Join-Path $Data 'logs\kin.log'
    Wait-Until { (Test-Path $log) -and ((Read-Shared $log) -match 'Kin .* started') } 30 'the dev Kin' | Out-Null
    Invoke-Kin --dev-start-pairing | Out-Null
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 'the pairing URL' | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    $tClip = [DateTime]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'")

    $text = 'kin-real-clipboard-check-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
    Invoke-Kin --dev-set-clipboard $text | Out-Null
    $got = Wait-Until { (@(Get-ClipboardFormats | ForEach-Object { $_.Name }) -contains 'CF_UNICODETEXT') } 10 'the text on the real clipboard'
    $names = @(Get-ClipboardFormats | ForEach-Object { $_.Name })
    Check 'B1 --dev-set-clipboard: CF_UNICODETEXT + ExcludeClipboardContentFromMonitorProcessing + KinRelayDevTestContent are on the REAL clipboard' ($got -and ($names -contains 'ExcludeClipboardContentFromMonitorProcessing') -and ($names -contains 'KinRelayDevTestContent')) ($names -join ',')
    Invoke-Kin --dev-get-clipboard | Out-Null
    $readBack = Wait-Until { (Test-Path (Join-Path $Data 'dev-clipboard.txt')) -and ((Get-Content (Join-Path $Data 'dev-clipboard.txt') -Raw -Encoding UTF8) -eq $text) } 10 'dev-clipboard.txt'
    Check 'B1 --dev-get-clipboard reads the same text back from the real clipboard' $readBack

    Invoke-Kin --dev-set-clipboard 'kin-real-clipboard-check-secret' --dev-clipboard-mark no-history | Out-Null
    $got = Wait-Until { (@(Get-ClipboardFormats | ForEach-Object { $_.Name }) -contains 'CanIncludeInClipboardHistory') } 10 'the no-history mark'
    Check 'B2 --dev-clipboard-mark no-history: CanIncludeInClipboardHistory is on the real clipboard with the value 0' ($got -and (Get-RealDword 'CanIncludeInClipboardHistory') -eq 0)

    # a camera photo never goes to the clipboard
    $before = Clip-Signature
    $fakeProc = Start-Fake @($url, '--host', '127.0.0.1', '--name', 'ZQ Phone', '--identity-file', $identity, '--seconds', '14', '--upload', $png, '--upload-origin', 'camera', '--upload-after', '2') 'fake1.txt'
    $got = Wait-Until { (Fake-Text 'fake1.txt') -match 'uploaded Screenshot_real_check.png' } 25 'the camera upload'
    Check 'B3 a camera photo arrived and was saved' $got (Fake-Text 'fake1.txt')
    Start-Sleep -Seconds 1
    Check 'B3 a camera photo NEVER goes to the real clipboard (every format and size is as it was)' ((Clip-Signature) -eq $before)
    Stop-OwnProcess $fakeProc $Fake

    # a screenshot goes on it, with the sensitive formats
    $fakeProc = Start-Fake @($url, '--host', '127.0.0.1', '--name', 'ZQ Phone', '--identity-file', $identity, '--skip-pairing', '--seconds', '14', '--upload', $png, '--upload-origin', 'screenshot', '--upload-after', '2') 'fake2.txt'
    $got = Wait-Until { @(Get-ClipboardFormats | ForEach-Object { $_.Name }) -contains 'CF_DIBV5' } 25 'the screenshot on the real clipboard'
    $names = @(Get-ClipboardFormats | ForEach-Object { $_.Name })
    Check 'B4 a screenshot is on the REAL clipboard as CF_DIBV5 (with PNG and CF_HDROP)' ($got -and ($names -contains 'PNG') -and ($names -contains 'CF_HDROP')) ($names -join ',')
    Check 'B4 with ExcludeClipboardContentFromMonitorProcessing, CanIncludeInClipboardHistory = 0 and CanUploadToCloudClipboard = 0' (($names -contains 'ExcludeClipboardContentFromMonitorProcessing') -and ((Get-RealDword 'CanIncludeInClipboardHistory') -eq 0) -and ((Get-RealDword 'CanUploadToCloudClipboard') -eq 0)) ($names -join ',')
    Check 'B4 and no text' ($names -notcontains 'CF_UNICODETEXT')
    Stop-OwnProcess $fakeProc $Fake

    # the installed Kin (read only)
    Start-Sleep -Milliseconds 1000
    $seen = @(Read-InstalledLogSince $tClip)
    $skipped = @($seen | Where-Object { $_ -match 'clipboard changed: not sent, the source app marked it' })
    $sent = @($seen | Where-Object { $_ -match 'clipboard changed' -and $_ -notmatch 'not sent' })
    Write-Host ("INFO the installed Kin's log since the start: {0} 'not sent, the source app marked it' line(s), {1} line(s) that say something was sent" -f $skipped.Count, $sent.Count)
    Check 'B5 the installed Kin (if it runs) skipped the marked content and sent none of it' (($sent.Count -eq 0) -and (($skipped.Count -ge 1) -or $installedBefore.Count -eq 0 -or $seen.Count -eq 0)) "$($skipped.Count) skipped, $($sent.Count) sent"
}
catch {
    Write-Host ('FAIL the run stopped with an error: ' + $_)
    $script:Fails++
}
finally {
    if ($fakeProc) { Stop-OwnProcess $fakeProc $Fake }
    Stop-DevKin
    if ($backup) { try { $null = Restore-ClipboardIfNotUsers $backup $startSignature; Write-Host 'the clipboard restore ran (it puts the backup back only if the clipboard holds nothing the user copied meanwhile)' } catch { Write-Host "restoring the clipboard failed: $_" } }
}
$installedAfter = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
Check 'the installed Kin was not started, stopped or restarted (same PIDs)' (($installedBefore -join ',') -eq ($installedAfter -join ',')) "$($installedBefore -join ',') -> $($installedAfter -join ',')"
Check 'no process of this run is left' (@(Get-DevKinProcesses).Count -eq 0)
if (-not $KeepEvidence) { Remove-DevDataDir $Data } else { Write-Host "evidence kept in $Data" }
if ($script:Fails -gt 0) { Write-Host "$script:Fails check(s) FAILED"; exit 1 }
Write-Host 'all checks passed'
