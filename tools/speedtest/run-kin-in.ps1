# One Kin phone -> PC run: monitor, then an ACTION_SEND / SEND_MULTIPLE intent to Kin's ShareActivity through adb (phone input only).
# Phone files must already be in MediaStore (adb push + MEDIA_SCANNER_SCAN_FILE). Uris are content://media/external/downloads/<id>.
param(
  [Parameter(Mandatory)][string]$Label,
  [Parameter(Mandatory)][long]$ExpectBytes,
  [Parameter(Mandatory)][int]$ExpectFiles,
  [string]$PcDir = "$env:USERPROFILE\Downloads\Kin",
  [int]$TimeoutSec = 400,
  [int]$TopEverySec = 5
)
$ErrorActionPreference = 'Stop'
$root = Join-Path $env:LOCALAPPDATA 'Temp\kin-speedtest'
$adb = "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe"; $ser = $env:ANDROID_SERIAL
$out = "$root\results\$Label.json"
Get-ChildItem "$root\results" -Filter ((Split-Path $out -Leaf) + "*") -ErrorAction SilentlyContinue | Remove-Item -ErrorAction SilentlyContinue
$mon = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList @('-NoProfile','-File',"`"$PSScriptRoot\speedmon.ps1`"",'-Dir','in','-ExpectBytes',$ExpectBytes,'-ExpectFiles',$ExpectFiles,
  '-PcDir',"`"$PcDir`"",'-TimeoutSec',$TimeoutSec,'-TopEverySec',$TopEverySec,'-OutFile',"`"$out`"")
for ($i = 0; $i -lt 100 -and -not (Test-Path "$out.ready"); $i++) { Start-Sleep -Milliseconds 200 }
if (-not (Test-Path "$out.ready")) { throw 'monitor did not start' }
Start-Sleep -Milliseconds 500
# Trigger: DocumentsUI already has the file(s) selected. Tap Share, wait for the share sheet, tap Kin. Phone input only.
. "$PSScriptRoot\phoneui.ps1"
Adb shell input tap 720 186
$kinNode = $null
for ($i = 0; $i -lt 12 -and -not $kinNode; $i++) { Start-Sleep -Milliseconds 400; $kinNode = Get-UiNodes | Where-Object { $_.text -eq 'Kin' -and $_.'resource-id' -eq 'android:id/text1' } | Select-Object -First 1 }
if (-not $kinNode) { throw 'Kin not in the share sheet' }
$trigMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$null = Tap-Node $kinNode
$null = $mon.WaitForExit(($TimeoutSec + 30) * 1000)
Start-Sleep -Seconds 4
$st = Get-Content "$env:APPDATA\Kin\status.json" -Raw | ConvertFrom-Json
$tr = @($st.transfers.recent | Where-Object { $_.direction -eq 'in' -and $_.startedMs -ge $trigMs - 2000 })
$res = Get-Content $out -Raw | ConvertFrom-Json
$kinSec = $null
if ($tr.Count) { $s0 = ($tr | Measure-Object startedMs -Minimum).Minimum; $s1 = ($tr | Measure-Object finishedMs -Maximum).Maximum; $kinSec = ($s1 - $s0) / 1000.0 }
$summary = [ordered]@{
  label = $Label; ok = $res.ok; bytes = $ExpectBytes; files = $ExpectFiles; wireSec = $res.wireSec; wireMBps = $res.wireMBps
  completeSec = $res.completeSec; completeMBps = $res.completeMBps; finalizeTailSec = $res.finalizeTailSec
  kinRecords = $tr.Count; kinSec = $kinSec; kinMBps = if ($kinSec) { [math]::Round($ExpectBytes / 1e6 / $kinSec, 1) } else { $null }
}
$summary | ConvertTo-Json | Set-Content "$out.summary.json"
[pscustomobject]$summary | Format-List




