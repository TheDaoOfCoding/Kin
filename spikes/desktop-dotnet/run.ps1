# Build and launch the Debug build of Kin (tray-only: look for the "K" icon in the tray overflow).
# Quit it from the tray menu. Don't use Stop-Process: it skips OnExit and leaves a ghost tray icon.
$ErrorActionPreference = 'Stop'
$exitCode = 0
try {
    # Set DOTNET_ROOT here too, so Kin (a framework-dependent build) finds the per-user runtime.
    . (Join-Path $PSScriptRoot '_sdk.ps1')
    & (Join-Path $PSScriptRoot 'build.ps1')
    $exitCode = $LASTEXITCODE
    if (-not $exitCode) {
        # The new process gets a copy of the current environment, so restoring ours afterwards is safe.
        Start-Process (Join-Path $PSScriptRoot 'bin\Debug\net10.0-windows\Kin.exe')
    }
}
finally {
    # Don't leave DOTNET_ROOT set in the calling PowerShell window.
    if (Get-Command Restore-SdkEnv -ErrorAction SilentlyContinue) { Restore-SdkEnv }
}
if ($exitCode) { exit $exitCode }
