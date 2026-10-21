# One Kin PC -> phone run: monitor + `Kin.exe --send`, then Kin's own startedMs/finishedMs from status.json, then phone clean-up of exact test names.
#
# *** THIS DRIVES THE USER'S INSTALLED KIN (C:\Program Files\Kin\Kin.exe) and its real data (%APPDATA%\Kin\status.json). *** Regression runs must never run it
# (tests use their own dev Kin with its own data directory, see tools\kin-dev.ps1). Only a person runs it, by hand, for speed tests with a real phone, and says so
# with -UseInstalledKin; without that switch it refuses.
param(
  [Parameter(Mandatory)][string]$Label,
  [Parameter(Mandatory)][string[]]$Paths,
  [Parameter(Mandatory)][string[]]$PhoneDirs,
  [int]$TimeoutSec = 400,
  [int]$TopEverySec = 0,
  [switch]$UseInstalledKin
)
$ErrorActionPreference = 'Stop'
if (-not $UseInstalledKin) { throw 'run-kin-out.ps1 drives the INSTALLED Kin and the user''s real data, so it only runs when the user starts it by hand for a real-phone speed test, with -UseInstalledKin. Tests must use a dev Kin (tools\kin-dev.ps1) instead.' }
$root = Join-Path $env:LOCALAPPDATA 'Temp\kin-speedtest'
$adb = "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe"; $ser = $env:ANDROID_SERIAL
$kin = 'C:\Program Files\Kin\Kin.exe'
$out = "$root\results\$Label.json"
Get-ChildItem "$root\results" -Filter ((Split-Path $out -Leaf) + "*") -ErrorAction SilentlyContinue | Remove-Item -ErrorAction SilentlyContinue
$files = @($Paths | ForEach-Object { Get-Item -LiteralPath $_ })
$bytes = ($files | Measure-Object Length -Sum).Sum
$mon = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList @('-NoProfile','-File',"`"$PSScriptRoot\speedmon.ps1`"",'-Dir','out','-ExpectBytes',$bytes,'-ExpectFiles',$files.Count,
  '-PhoneDirs',($PhoneDirs -join ','),'-TimeoutSec',$TimeoutSec,'-TopEverySec',$TopEverySec,'-OutFile',"`"$out`"")
for ($i = 0; $i -lt 100 -and -not (Test-Path "$out.ready"); $i++) { Start-Sleep -Milliseconds 200 }
if (-not (Test-Path "$out.ready")) { throw 'monitor did not start' }
Start-Sleep -Milliseconds 500
$before = (Get-Content "$env:APPDATA\Kin\status.json" -Raw | ConvertFrom-Json).transfers.recent | ForEach-Object { $_.id }
$trigMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
& $kin --send @($files.FullName)
$null = $mon.WaitForExit(($TimeoutSec + 30) * 1000)
Start-Sleep -Seconds 2
$st = Get-Content "$env:APPDATA\Kin\status.json" -Raw | ConvertFrom-Json
$tr = $st.transfers.recent | Where-Object { $_.direction -eq 'out' -and $_.startedMs -ge $trigMs - 2000 } | Select-Object -First 1
$res = Get-Content $out -Raw | ConvertFrom-Json
$kinSec = $null
if ($tr -and $tr.finishedMs) { $kinSec = ($tr.finishedMs - $tr.startedMs) / 1000.0 }
$summary = [ordered]@{
  label = $Label; ok = $res.ok; bytes = $bytes; files = $files.Count
  wireSec = $res.wireSec; wireMBps = $res.wireMBps; completeSec = $res.completeSec; completeMBps = $res.completeMBps; finalizeTailSec = $res.finalizeTailSec
  kinStatus = if ($tr) { $tr.status } else { 'not found' }; kinSec = $kinSec
  kinMBps = if ($kinSec) { [math]::Round($bytes / 1e6 / $kinSec, 1) } else { $null }
  kinStartLagMs = if ($tr) { $tr.startedMs - $trigMs } else { $null }
  kinFinishLagFromDoneMs = if ($tr -and $res.doneMs) { $tr.finishedMs - $res.doneMs } else { $null }
}
$summary | ConvertTo-Json | Set-Content "$out.summary.json"
[pscustomobject]$summary | Format-List
# clean the phone: exact test names only
foreach ($d in $PhoneDirs) { & $adb -s $ser shell "rm -f '$d'/kin-speedtest*" | Out-Null }





