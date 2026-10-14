# Release build, NSIS installer only. Output: src-tauri\target\release\Kin.exe and src-tauri\target\release\bundle\nsis\Kin_0.1.0_x64-setup.exe
# NSIS has to be staged in %LOCALAPPDATA%\tauri\NSIS first on a machine where the bundler's own download times out (see docs/results/phase-0-tauri.md).
. "$PSScriptRoot\_env.ps1"
Push-Location $PSScriptRoot
try {
    if (-not (Test-Path node_modules)) {
        bun install
        if ($LASTEXITCODE) { exit $LASTEXITCODE }
    }
    bun tauri build --bundles nsis
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
} finally { Pop-Location }
