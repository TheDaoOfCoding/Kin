# Release build, NSIS installer only. Output: src-tauri\target\release\Kin.exe and ...\bundle\nsis\Kin_0.1.0_x64-setup.exe
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
