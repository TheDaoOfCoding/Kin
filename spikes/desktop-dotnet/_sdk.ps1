# Dot-sourced by build.ps1 / run.ps1. Resolves the per-user .NET SDK and exports $Dotnet.
#
# Why: C:\Program Files\dotnet is on the machine PATH and holds runtimes only (no SDK).
# Machine PATH comes before user PATH, so a bare `dotnet` hits that muxer and reports
# "No .NET SDKs were found". DOTNET_ROOT does not change which SDKs that muxer sees,
# so we always call the per-user dotnet.exe by full path.

# The caller must call Restore-SdkEnv in a `finally` block. This file sets process environment
# variables, and in an open PowerShell window (or when dot-sourced) they would stay set after the
# script ends and leak into anything started from that window later.
$SdkEnvNames = 'DOTNET_ROOT', 'DOTNET_CLI_TELEMETRY_OPTOUT', 'DOTNET_NOLOGO'
$SdkEnvSaved = @{}
foreach ($name in $SdkEnvNames) { $SdkEnvSaved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
function Restore-SdkEnv {
    # A saved value of $null removes the variable.
    foreach ($name in $SdkEnvNames) { [Environment]::SetEnvironmentVariable($name, $SdkEnvSaved[$name], 'Process') }
}

# DOTNET_ROOT is never set at user scope (see below), so there is no user-scope candidate.
$candidates = @(
    $env:DOTNET_ROOT,
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
    Restore-SdkEnv
    throw "Found .NET SDK $sdkVersion at $Dotnet, but Kin needs SDK 10 or later. Install it: download https://dot.net/v1/dotnet-install.ps1 and run it with -Channel LTS -InstallDir `"$env:LOCALAPPDATA\Microsoft\dotnet`" -NoPath"
}
