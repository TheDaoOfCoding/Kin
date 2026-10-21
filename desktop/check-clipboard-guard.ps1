# Self-test of the decision inside Restore-ClipboardIfNotUsers (clipboard-backup.ps1): when may the backup be put back, when is the clipboard the user's.
# It tests two PURE functions over lists of format names. It never opens, reads or writes the clipboard, starts no Kin and needs no data directory.
#   powershell -File desktop\check-clipboard-guard.ps1
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'clipboard-backup.ps1')

$fails = 0
function Expect([string]$what, [string]$want, [string[]]$names, [string]$sig = 'x', [string]$start = 'y') {
    $got = Get-ClipboardRestoreDecision $names $sig $start
    if ($got -eq $want) { Write-Host "ok   $what" } else { Write-Host "FAIL $what (wanted $want, got $got)"; $script:fails++ }
}

$ex = 'ExcludeClipboardContentFromMonitorProcessing'
$test = 'KinRelayDevTestContent'
Expect 'empty clipboard -> restore'                                         'restore' @()
Expect 'the test text of the run (B1) -> restore'                          'restore' @('CF_UNICODETEXT', $ex, $test)
Expect 'the test text with the no-history mark (B2) -> restore'            'restore' @('CF_UNICODETEXT', $ex, $test, 'CanIncludeInClipboardHistory')
Expect 'a quiet screenshot (B4, with CF_HDROP and the test mark) -> restore' 'restore' @('CF_DIBV5', 'PNG', 'CF_HDROP', $ex, $test, 'CanIncludeInClipboardHistory', 'CanUploadToCloudClipboard')
Expect 'a screenshot of Kin without the test mark, with CF_HDROP -> restore' 'restore' @('CF_DIBV5', 'PNG', 'CF_HDROP', $ex, 'CanIncludeInClipboardHistory', 'CanUploadToCloudClipboard')
Expect 'the same signature as at the start -> restore'                      'restore' @('CF_UNICODETEXT') 'CF_UNICODETEXT:10' 'CF_UNICODETEXT:10'
Expect "the user's text -> keep"                                           'keep'    @('CF_UNICODETEXT', 'CF_TEXT', 'CF_OEMTEXT', 'CF_LOCALE')
Expect "the user's text with HTML -> keep"                                 'keep'    @('CF_UNICODETEXT', 'HTML Format')
Expect "an image copied by the user (no PNG/CF_HDROP/mark) -> keep"        'keep'    @('CF_DIB', 'CF_DIBV5')
Expect "files copied in Explorer -> keep"                                  'keep'    @('CF_HDROP', 'Shell IDList Array', 'Preferred DropEffect')
Expect "an image with PNG and CF_HDROP but no opt-out mark -> keep"        'keep'    @('CF_DIBV5', 'PNG', 'CF_HDROP')
Expect 'Kin-shaped image that also carries text -> keep'                    'keep'    @('CF_DIBV5', 'PNG', 'CF_HDROP', $ex, 'CF_UNICODETEXT')

if ($fails -gt 0) { Write-Host "$fails check(s) FAILED"; exit 1 }
Write-Host 'all checks passed'
