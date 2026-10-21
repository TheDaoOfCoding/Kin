# Builds the dev host + the mock Kin and runs the extension's smoke test against them in headless Chromium (throwaway profile).
# Registers only HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev (refuses if it exists, removes it at the end).
# Needs the extension's dist\chromium-test (bun scripts/build.ts --test, in extension\). Does not touch the user's Kin or browsers.
$ErrorActionPreference = 'Stop'
$host_dir = Split-Path -Parent $PSScriptRoot
Push-Location $host_dir
try {
    cargo build --features dev,mock --bins --target-dir target/dev
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    $ext = Join-Path $host_dir '..\..\extension\dist\chromium-test\manifest.json'
    if (-not (Test-Path $ext)) { throw "Build the extension's test flavor first: cd extension; bun scripts/build.ts --test" }
    python -I smoke/make-real-host-smoke.py   # regenerates smoke/run-real-host.ts from the extension's own smoke test
    if ($LASTEXITCODE -ne 0) { throw 'generating the smoke test failed' }
    bun smoke/run-real-host.ts
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
