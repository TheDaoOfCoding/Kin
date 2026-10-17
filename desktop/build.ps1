# Release build, NSIS installer only. Output: src-tauri\target\release\Kin.exe and src-tauri\target\release\bundle\nsis\Kin_0.1.1_x64-setup.exe
# Since 2B.1 the installer also carries kin-native-host.exe (the browser extension's native messaging host), installed next to Kin.exe. It is a crate of
# its own (native-host\): built here in release mode **with no features** (the release host has no developer overrides), checked, copied to
# src-tauri\binaries\ and added to the bundle with a config override, so that a plain "cargo build" or "cargo test" never needs it.
# NSIS has to be staged in %LOCALAPPDATA%\tauri\NSIS first on a machine where the bundler's own download times out (see docs/results/phase-0-tauri.md).
. "$PSScriptRoot\_env.ps1"
Push-Location $PSScriptRoot
try {
    if (-not (Test-Path node_modules)) {
        bun install
        if ($LASTEXITCODE) { exit $LASTEXITCODE }
    }
    cargo build --release --manifest-path native-host\Cargo.toml
    if ($LASTEXITCODE) { throw 'building kin-native-host.exe failed' }
    $host_exe = Join-Path $PSScriptRoot 'native-host\target\release\kin-native-host.exe'
    $bytes = [IO.File]::ReadAllBytes($host_exe)
    # The release host must not contain a developer override or the dev host name (it would let a test setting reach the user's Kin).
    $text = [Text.Encoding]::ASCII.GetString($bytes)
    foreach ($forbidden in 'KIN_HOST_DEV', 'app.kinrelay.dev', 'kin-host-test', 'mock-kin') {
        if ($text.Contains($forbidden)) { throw "kin-native-host.exe contains '$forbidden': it was built with developer features" }
    }
    if ($bytes.Length -lt 100KB) { throw "kin-native-host.exe is only $($bytes.Length) bytes: not the real host" }
    $binaries = Join-Path $PSScriptRoot 'src-tauri\binaries'
    New-Item -ItemType Directory -Force $binaries | Out-Null
    Copy-Item -Force $host_exe (Join-Path $binaries 'kin-native-host.exe')
    # The config override is a file: Windows PowerShell 5.1 strips the quotes of a JSON text that is passed to a program.
    $bundle = Join-Path $PSScriptRoot 'src-tauri\windows\bundle-native-host.json'
    bun tauri build --bundles nsis --config $bundle
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
} finally { Pop-Location }
