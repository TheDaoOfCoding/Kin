# One LocalSend PC -> phone run with the SCRIPTED sender (ls-send.mjs): monitor + node sender, then phone clean-up of exact test names.
param(
  [Parameter(Mandatory)][string]$Label,
  [Parameter(Mandatory)][string[]]$Paths,
  [Parameter(Mandatory)][string[]]$PhoneDirs,
  [int]$TimeoutSec = 300,
  [int]$TopEverySec = 5
)
$ErrorActionPreference = 'Stop'
$root = Join-Path $env:LOCALAPPDATA 'Temp\kin-speedtest'
$adb = "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe"; $ser = $env:ANDROID_SERIAL
$out = "$root\results\$Label.json"
Get-ChildItem "$root\results" -Filter ((Split-Path $out -Leaf) + "*") -ErrorAction SilentlyContinue | Remove-Item -ErrorAction SilentlyContinue
$files = @($Paths | ForEach-Object { Get-Item -LiteralPath $_ })
$bytes = ($files | Measure-Object Length -Sum).Sum
$mon = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList @('-NoProfile','-File',"`"$PSScriptRoot\speedmon.ps1`"",'-Dir','out','-ExpectBytes',$bytes,'-ExpectFiles',$files.Count,
  '-PhoneDirs',($PhoneDirs -join ','),'-TimeoutSec',$TimeoutSec,'-TopEverySec',$TopEverySec,'-OutFile',"`"$out`"")
for ($i = 0; $i -lt 100 -and -not (Test-Path "$out.ready"); $i++) { Start-Sleep -Milliseconds 200 }
if (-not (Test-Path "$out.ready")) { throw 'monitor did not start' }
Start-Sleep -Milliseconds 500
$ip = (Get-Content "$root\phone.ip").Trim()
$sender = node "$PSScriptRoot\ls-send.mjs" $ip @($files.FullName) | Select-Object -Last 1
$null = $mon.WaitForExit(($TimeoutSec + 30) * 1000)
$res = Get-Content $out -Raw | ConvertFrom-Json
$s = $sender | ConvertFrom-Json
$summary = [ordered]@{ label = $Label; ok = $res.ok; bytes = $bytes; files = $files.Count; wireSec = $res.wireSec; wireMBps = $res.wireMBps; completeSec = $res.completeSec; completeMBps = $res.completeMBps
  finalizeTailSec = $res.finalizeTailSec; senderUploadSec = $s.uploadSec; senderUploadMBps = $s.uploadMBps; senderTotalSec = $s.totalSec }
$summary | ConvertTo-Json | Set-Content "$out.summary.json"
[pscustomobject]$summary | Format-List

