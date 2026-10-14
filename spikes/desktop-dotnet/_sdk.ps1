# Dot-sourced by build.ps1 / run.ps1. Resolves the per-user .NET SDK and exports $Dotnet.
#
# Why: C:\Program Files\dotnet is on the machine PATH and holds runtimes only (no SDK).
# Machine PATH comes before user PATH, so a bare `dotnet` hits that muxer and reports
# "No .NET SDKs were found". DOTNET_ROOT does not change which SDKs that muxer sees,
# so we always call the per-user dotnet.exe by full path.

$candidates = @(
    $env:DOTNET_ROOT,
    [Environment]::GetEnvironmentVariable('DOTNET_ROOT', 'User'),
    (Join-Path $env:LOCALAPPDATA 'Microsoft\dotnet')
) | Where-Object { $_ }

$Dotnet = $null
foreach ($root in $candidates) {
    $exe = Join-Path $root 'dotnet.exe'
    if ((Test-Path $exe) -and (Test-Path (Join-Path $root 'sdk'))) { $Dotnet = $exe; break }
}
if (-not $Dotnet) {
    throw "No per-user .NET SDK found. Install it: download https://dot.net/v1/dotnet-install.ps1 and run it with -Channel LTS -InstallDir `"$env:LOCALAPPDATA\Microsoft\dotnet`" -NoPath"
}

# Set per process only. Do NOT set DOTNET_ROOT at user or machine scope: apphosts read it before
# C:\Program Files\dotnet, so other apps would stop finding their runtimes.
$env:DOTNET_ROOT = Split-Path $Dotnet
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_NOLOGO = '1'

# Kin.csproj targets net10.0-windows, so the SDK must be 10 or later.
$sdkVersion = (& $Dotnet --version).Trim()
if ([version]($sdkVersion -replace '[-+].*$', '') -lt [version]'10.0') {
    throw "Found .NET SDK $sdkVersion at $Dotnet, but Kin needs SDK 10 or later. Install it: download https://dot.net/v1/dotnet-install.ps1 and run it with -Channel LTS -InstallDir `"$env:LOCALAPPDATA\Microsoft\dotnet`" -NoPath"
}
