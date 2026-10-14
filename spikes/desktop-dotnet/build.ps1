# Build (default) or publish the Kin tray spike with the per-user .NET SDK. No admin needed.
#   .\build.ps1                   Debug build  -> bin\Debug\net10.0-windows\Kin.exe
#   .\build.ps1 -Publish          Release publishes, win-x64: framework-dependent and self-contained single-file
param([switch]$Publish)

$ErrorActionPreference = 'Stop'
$exitCode = 0
try {
    . (Join-Path $PSScriptRoot '_sdk.ps1')
    $proj = Join-Path $PSScriptRoot 'Kin.csproj'

    if (-not $Publish) {
        & $Dotnet build $proj -c Debug
        $exitCode = $LASTEXITCODE
    }
    else {
        # WPF does not support trimming, so no -p:PublishTrimmed.
        & $Dotnet publish $proj -c Release -r win-x64 --self-contained false -o (Join-Path $PSScriptRoot 'publish\fdd')
        $exitCode = $LASTEXITCODE
        if (-not $exitCode) {
            & $Dotnet publish $proj -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -o (Join-Path $PSScriptRoot 'publish\sc-single')
            $exitCode = $LASTEXITCODE
        }
        if (-not $exitCode) {
            Get-ChildItem (Join-Path $PSScriptRoot 'publish') | ForEach-Object {
                $bytes = (Get-ChildItem $_.FullName -Recurse -File | Measure-Object Length -Sum).Sum
                '{0,-10} {1,8:N1} MB' -f $_.Name, ($bytes / 1MB)
            }
        }
    }
}
finally {
    # Put DOTNET_ROOT and friends back, so they don't leak into the calling PowerShell window.
    if (Get-Command Restore-SdkEnv -ErrorAction SilentlyContinue) { Restore-SdkEnv }
}
if ($exitCode) { exit $exitCode }
