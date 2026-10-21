# Saves a PNG screenshot of the emulator screen and prints its path.
#
#   .\screenshot.ps1                       -> tools\emulator\screenshot-<timestamp>.png (gitignored)
#   .\screenshot.ps1 -Out C:\temp\a.png
#   .\screenshot.ps1 -Serial emulator-5554
param(
    [string]$Out = (Join-Path $PSScriptRoot ("screenshot-{0:yyyyMMdd-HHmmss}.png" -f (Get-Date))),
    [string]$Serial
)
. "$PSScriptRoot\_common.ps1"

Initialize-Adb
$Serial = Resolve-Serial $Serial
$dir = Split-Path -Parent ([IO.Path]::GetFullPath($Out))
New-Item -ItemType Directory -Force $dir | Out-Null
$Out = [IO.Path]::GetFullPath($Out)

# PowerShell pipes mangle binary data, so let cmd.exe redirect the raw bytes into the file.
cmd.exe /c "`"$Adb`" -s $Serial exec-out screencap -p > `"$Out`""
if ($LASTEXITCODE -ne 0 -or -not (Test-Path $Out) -or (Get-Item $Out).Length -lt 1000) { throw "Screenshot failed ($Out)" }
Write-Output $Out
