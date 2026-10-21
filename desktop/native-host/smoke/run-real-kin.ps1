# Runs the extension in headless Chromium against the REAL Kin (a dev instance beside the installed Kin: its own throwaway data directory, loopback only,
# no windows, --dev-no-open so nothing is ever opened) and the REAL dev host, with a fake phone on the other side. See smoke/run-real-kin.ts.
#
#   powershell -ExecutionPolicy Bypass -File desktop\native-host\smoke\run-real-kin.ps1
#
# Needs the extension's test build (cd extension; bun scripts/build.ts --test) and the Playwright Chromium. Builds the debug Kin.exe, the fake phone
# and the dev host (cargo, no admin). It takes the scripts' lock (tools\kin-lock.ps1), refuses to run while a game or an anti-cheat runs, writes only
# HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev (the dev host name; removed in a finally) and never touches the installed Kin, its data,
# its Run key, SendTo, Downloads or firewall rules, or the release native messaging keys. Only processes it started are stopped.
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path (Join-Path $PSScriptRoot '..\..\..')
. (Join-Path $repo 'tools\kin-lock.ps1'); Enter-KinLock 'run-real-kin' -WaitSeconds 900
. (Join-Path $repo 'tools\kin-dev.ps1')
Assert-NoGameRunning
Push-Location (Join-Path $repo 'desktop\src-tauri')
try {
    cargo build --bin kin --example fake_phone
    if ($LASTEXITCODE) { throw 'cargo build (Kin, fake_phone) failed' }
} finally { Pop-Location }
Push-Location (Join-Path $repo 'desktop\native-host')
try {
    cargo build --features dev --bin kin-native-host --target-dir target/dev
    if ($LASTEXITCODE) { throw 'cargo build (dev host) failed' }
    bun smoke/run-real-kin.ts
    exit $LASTEXITCODE
} finally { Pop-Location }
