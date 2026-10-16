# Checks desktop\clipboard-backup.ps1 on the REAL clipboard: Save-Clipboard / Restore-Clipboard with no earlier backup, a fresh earlier backup, a
# stale one (older than an hour) and a restore that fails. The user's own clipboard is saved first (every format, to a folder of this run) and put
# back at the end, also after a failure; every test text is quiet (Set-QuietClipboardText), so an installed Kin does not sync it.
#
#   powershell -ExecutionPolicy Bypass -File desktop\check-clipboard-backup.ps1
#
# Refuses to run while a clipboard backup of another run is in %LOCALAPPDATA%\Temp\kin-clipboard-backup (it would be this test's to clobber).
. (Join-Path $PSScriptRoot '..\tools\kin-lock.ps1'); Enter-KinLock 'check-clipboard-backup'
. (Join-Path $PSScriptRoot 'clipboard-backup.ps1')
$ErrorActionPreference = 'Stop'

$temp = Join-Path $env:LOCALAPPDATA 'Temp'
$fixed = Join-Path $temp 'kin-clipboard-backup'
if ([IO.Directory]::Exists($fixed)) { Write-Host "a clipboard backup is in $fixed (another run?): not running"; exit 2 }
$before = @([IO.Directory]::GetDirectories($temp, 'kin-clipboard-backup-*'))

$failures = 0
function Check([string]$name, [bool]$ok) { if ($ok) { Write-Host "  ok    $name" } else { Write-Host "  FAIL  $name"; $script:failures++ } }
function Get-ClipText { $t = [KinClip]::Read($true) | Where-Object { $_.Id -eq 13 } | Select-Object -First 1; if ($t) { [Text.Encoding]::Unicode.GetString($t.Data).TrimEnd([char]0) } else { $null } }
function New-Side { @([IO.Directory]::GetDirectories($temp, 'kin-clipboard-backup-*') | Where-Object { $before -notcontains $_ }) }
function Fake-Age($backup, [int]$minutes) { [IO.File]::SetLastWriteTime((Join-Path $backup.Dir 'manifest.json'), (Get-Date).AddMinutes(-$minutes)) }
function Text-Of-Folder([string]$dir) {
    $m = (Get-Content -LiteralPath (Join-Path $dir 'manifest.json') -Raw | ConvertFrom-Json | ForEach-Object { $_ }) | Where-Object { $_.Id -eq 13 } | Select-Object -First 1
    if ($m) { [Text.Encoding]::Unicode.GetString([IO.File]::ReadAllBytes((Join-Path $dir $m.File))).TrimEnd([char]0) } else { $null }
}
function Save-Capturing { $w = @(); $r = & { Save-Clipboard } 3>&1; $obj = $null; foreach ($x in @($r)) { if ($x -is [Management.Automation.WarningRecord]) { $w += $x.Message } else { $obj = $x } }; [pscustomobject]@{ Backup = $obj; Warnings = $w } }

$outer = Write-ClipboardSnapshot (Join-Path $temp ("kin-clipboard-test-outer-{0}" -f $PID))
try {
    Write-Host 'A. no earlier backup'
    Set-QuietClipboardText 'A-current'
    $s = Save-Capturing
    Check 'saved with no warning' ($s.Warnings.Count -eq 0 -and $s.Backup.Formats -ge 1)
    Set-QuietClipboardText 'A-changed'
    Check 'restore returns true' ([bool](Restore-Clipboard $s.Backup | Select-Object -Last 1))
    Check 'the clipboard is back' ((Get-ClipText) -eq 'A-current')
    Check 'the backup folder is gone and nothing was set aside' (-not [IO.Directory]::Exists($fixed) -and (New-Side).Count -eq 0)

    Write-Host 'B. a fresh earlier backup (a run that did not restore), and the user copied something since'
    Set-QuietClipboardText 'B-old'
    $first = Save-Capturing                    # the "crashed" run: never restored
    Set-QuietClipboardText 'B-newer'           # what the user copied after that run died
    $s = Save-Capturing
    $side = @(New-Side)
    Check 'a warning that names the side folder' ($s.Warnings.Count -eq 1 -and $side.Count -eq 1 -and $s.Warnings[0].Contains($side[0]) -and $s.Warnings[0].Contains('not restored automatically'))
    Check 'the side folder holds the clipboard as it was (B-newer)' ($side.Count -eq 1 -and $side[0] -like '*-superseded-*' -and (Text-Of-Folder $side[0]) -eq 'B-newer')
    Check 'the earlier backup is the one this run returns, still B-old' ($s.Backup.Dir -eq $fixed -and (Text-Of-Folder $fixed) -eq 'B-old')
    Set-QuietClipboardText 'B-during-the-run'
    Check 'restore puts back the earlier backup' ([bool](Restore-Clipboard $s.Backup | Select-Object -Last 1) -and (Get-ClipText) -eq 'B-old')
    Check 'the side folder was not restored or deleted' ($side.Count -eq 1 -and [IO.Directory]::Exists($side[0]) -and (Text-Of-Folder $side[0]) -eq 'B-newer')
    Check 'the backup folder is gone' (-not [IO.Directory]::Exists($fixed))
    foreach ($d in $side) { [IO.Directory]::Delete($d, $true) }

    Write-Host 'C. a stale earlier backup (2 hours old)'
    Set-QuietClipboardText 'C-old'
    $first = Save-Capturing
    Fake-Age $first.Backup 120
    Set-QuietClipboardText 'C-user'
    $s = Save-Capturing
    $side = @(New-Side)
    Check 'a warning says it is stale and where it went' ($s.Warnings.Count -eq 1 -and $side.Count -eq 1 -and $side[0] -like '*-stale-*' -and $s.Warnings[0].Contains($side[0]) -and $s.Warnings[0].Contains('never restored'))
    Check 'the stale backup was moved aside whole (C-old)' ($side.Count -eq 1 -and (Text-Of-Folder $side[0]) -eq 'C-old')
    Check 'this run backed up the clipboard as it is (C-user)' ((Text-Of-Folder $fixed) -eq 'C-user')
    Set-QuietClipboardText 'C-changed'
    Check 'restore gives C-user, not the stale C-old' ([bool](Restore-Clipboard $s.Backup | Select-Object -Last 1) -and (Get-ClipText) -eq 'C-user')
    Check 'no side folder was made besides the stale one' ((New-Side).Count -eq 1)
    foreach ($d in $side) { [IO.Directory]::Delete($d, $true) }

    Write-Host 'D. a restore that fails'
    Set-QuietClipboardText 'D-current'
    $s = Save-Capturing
    $text = (Get-Content -LiteralPath (Join-Path $fixed 'manifest.json') -Raw | ConvertFrom-Json | ForEach-Object { $_ }) | Where-Object { $_.Id -eq 13 } | Select-Object -First 1
    [IO.File]::Delete((Join-Path $fixed $text.File))    # a file of the backup is gone
    Set-QuietClipboardText 'D-after'
    $ok = [bool](Restore-Clipboard $s.Backup | Select-Object -Last 1)
    Check 'restore reports the failure and does not throw' (-not $ok)
    Check 'the clipboard was left as it was (D-after)' ((Get-ClipText) -eq 'D-after')
    Check 'the backup was kept' ([IO.File]::Exists((Join-Path $fixed 'manifest.json')))
    Fake-Age $s.Backup 120                              # "days later"
    Set-QuietClipboardText 'D-user-days-later'
    $s2 = Save-Capturing
    $side = @(New-Side)
    Check 'days later the failed backup is moved aside, not restored' ($side.Count -eq 1 -and $side[0] -like '*-stale-*' -and $s2.Warnings.Count -eq 1)
    Check 'this run backed up the clipboard as it is' ((Text-Of-Folder $fixed) -eq 'D-user-days-later')
    Set-QuietClipboardText 'D-changed'
    Check 'restore gives D-user-days-later, not D-current' ([bool](Restore-Clipboard $s2.Backup | Select-Object -Last 1) -and (Get-ClipText) -eq 'D-user-days-later')
    foreach ($d in $side) { [IO.Directory]::Delete($d, $true) }
}
catch { Write-Host "ERROR: $_"; $failures++ }
finally {
    if ([IO.Directory]::Exists($fixed)) { [IO.Directory]::Delete($fixed, $true) }
    foreach ($d in @(New-Side)) { [IO.Directory]::Delete($d, $true) }
    $ok = Restore-Clipboard $outer
    Write-Host ("the user's clipboard is back as it was (every format): {0}" -f [bool]$ok)
}
if ($failures -eq 0) { Write-Host 'all checks passed'; exit 0 } else { Write-Host "$failures check(s) FAILED"; exit 1 }
