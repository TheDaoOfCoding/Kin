# Runs the one real-clipboard unit test of v0.1.1 (clipboard::tests::real_clipboard_clear_if_text_clears_only_the_exact_text) with every format of the
# user's clipboard saved first and put back at the end (also after a failure). It touches the clipboard and nothing else: no Kin.exe is started.
#
#   powershell -ExecutionPolicy Bypass -File desktop\run-real-clipboard-test.ps1
. (Join-Path $PSScriptRoot '..\tools\kin-lock.ps1'); Enter-KinLock 'run-real-clipboard-test'
. (Join-Path $PSScriptRoot 'clipboard-backup.ps1')
$ErrorActionPreference = 'Stop'
$backup = Save-Clipboard | Select-Object -Last 1
$code = 1
try {
    Push-Location (Join-Path $PSScriptRoot 'src-tauri')
    try {
        & cargo test --lib real_clipboard_clear -- --ignored --test-threads=1 --nocapture
        $code = $LASTEXITCODE
    } finally { Pop-Location }
} finally {
    $ok = Restore-Clipboard $backup
    Write-Host ("the clipboard is back as it was (every format): {0}" -f [bool]$ok)
}
exit $code
