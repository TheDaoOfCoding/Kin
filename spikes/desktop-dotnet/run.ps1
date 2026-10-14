# Build and launch the Debug build of Kin (tray-only: look for the "K" icon in the tray overflow).
# Quit it from the tray menu, or: Stop-Process -Name Kin
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'build.ps1')
if ($LASTEXITCODE) { exit $LASTEXITCODE }
Start-Process (Join-Path $PSScriptRoot 'bin\Debug\net10.0-windows\Kin.exe')
