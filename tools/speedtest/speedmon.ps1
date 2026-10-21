# Passive monitor for one transfer case. Start it BEFORE the transfer is triggered.
# Samples the PC's Ethernet byte counters (about every 50-250 ms) and polls the receiver until all files are final and full size.
# Every loop is bounded by -TimeoutSec. Never touches anything but files named kin-speedtest*.
param(
  [Parameter(Mandatory)][ValidateSet('in','out')][string]$Dir,   # in = phone -> PC, out = PC -> phone
  [Parameter(Mandatory)][long]$ExpectBytes,
  [Parameter(Mandatory)][int]$ExpectFiles,
  [string]$PcDir,                 # for Dir=in: where the PC receiver puts the files
  [string[]]$PhoneDirs,           # for Dir=out: where the phone receiver puts the files
  [string]$Adb = "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe",
  [string]$Serial = $env:ANDROID_SERIAL,
  [string]$NicName = 'Ethernet',
  [int]$TimeoutSec = 400,
  [double]$ThresholdMBps = 3.0,
  [int]$TopEverySec = 0,          # >0: sample the phone's `top -n 1` this often
  [Parameter(Mandatory)][string]$OutFile
)
$ErrorActionPreference = 'Stop'
function NowMs { [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }
$nic = [Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces() | Where-Object Name -eq $NicName | Select-Object -First 1
if (-not $nic) { throw "no interface $NicName" }

function Get-Received {
  if ($Dir -eq 'in') {
    $files = @(Get-ChildItem -LiteralPath $PcDir -File -Filter 'kin-speedtest*' -ErrorAction SilentlyContinue | Where-Object { $_.Name -notmatch '\.part$' })
    return [pscustomobject]@{ Count = $files.Count; Bytes = [long](($files | Measure-Object Length -Sum).Sum) }
  }
  $cmd = ($PhoneDirs | ForEach-Object { "stat -c '%s' '$_'/kin-speedtest* 2>/dev/null" }) -join '; '
  $lines = @(& $Adb -s $Serial shell $cmd)
  $sizes = @($lines | Where-Object { $_ -match '^\d+$' } | ForEach-Object { [long]$_ })
  return [pscustomobject]@{ Count = $sizes.Count; Bytes = [long](($sizes | Measure-Object -Sum).Sum) }
}

$init = Get-Received
if ($init.Count -ne 0) { throw "receiver already has $($init.Count) kin-speedtest files: clean first" }

$samples = New-Object System.Collections.Generic.List[object]
$tops = New-Object System.Collections.Generic.List[string]
$t0 = NowMs
$deadline = $t0 + $TimeoutSec * 1000
$nextPoll = 0; $nextTop = $t0 + 1000 * [Math]::Max($TopEverySec, 1)
$firstSeen = $null; $doneAt = $null; $last = $null
"READY" | Set-Content "$OutFile.ready"
while ((NowMs) -lt $deadline) {
  $st = $nic.GetIPv4Statistics()
  $now = NowMs
  $samples.Add([pscustomobject]@{ t = $now; tx = $st.BytesSent; rx = $st.BytesReceived })
  if ($now -ge $nextPoll) {
    $r = Get-Received
    $last = $r
    if ($r.Count -gt 0 -and -not $firstSeen) { $firstSeen = NowMs }
    if ($r.Count -ge $ExpectFiles -and $r.Bytes -ge $ExpectBytes) { $doneAt = NowMs; break }
    $nextPoll = (NowMs) + 150
  }
  if ($TopEverySec -gt 0 -and $now -ge $nextTop) {
    $tops.Add("--- t=$now") ; foreach ($l in (& $Adb -s $Serial shell top -b -n 1 -m 6)) { $tops.Add([string]$l) }
    $nextTop = (NowMs) + 1000 * $TopEverySec
  }
  Start-Sleep -Milliseconds 40
}
# a few more samples after completion so the window end is clean
for ($i = 0; $i -lt 6; $i++) { Start-Sleep -Milliseconds 100; $st = $nic.GetIPv4Statistics(); $samples.Add([pscustomobject]@{ t = (NowMs); tx = $st.BytesSent; rx = $st.BytesReceived }) }

# wire window: from the first interval at or above the threshold to the end of the last one
$thr = $ThresholdMBps * 1e6
$startT = $null; $endT = $null; $startIdx = $null; $endIdx = $null
for ($i = 1; $i -lt $samples.Count; $i++) {
  $a = $samples[$i - 1]; $b = $samples[$i]
  $dt = ($b.t - $a.t) / 1000.0; if ($dt -le 0) { continue }
  $d = if ($Dir -eq 'out') { $b.tx - $a.tx } else { $b.rx - $a.rx }
  if ($d / $dt -ge $thr) { if ($null -eq $startT) { $startT = $a.t; $startIdx = $i - 1 }; $endT = $b.t; $endIdx = $i }
}
$wireBytes = $null; $wireSec = $null; $completeSec = $null
if ($startT) {
  $wireSec = ($endT - $startT) / 1000.0
  $wireBytes = if ($Dir -eq 'out') { $samples[$endIdx].tx - $samples[$startIdx].tx } else { $samples[$endIdx].rx - $samples[$startIdx].rx }
  if ($doneAt) { $completeSec = ($doneAt - $startT) / 1000.0 }
}
$res = [ordered]@{
  dir = $Dir; ok = [bool]($doneAt -and $startT); expectBytes = $ExpectBytes; expectFiles = $ExpectFiles
  wireStartMs = $startT; wireEndMs = $endT; doneMs = $doneAt; firstFileSeenMs = $firstSeen
  wireSec = $wireSec; wireBytes = $wireBytes
  wireMBps = if ($wireSec) { [math]::Round($ExpectBytes / 1e6 / $wireSec, 1) } else { $null }
  completeSec = $completeSec
  completeMBps = if ($completeSec) { [math]::Round($ExpectBytes / 1e6 / $completeSec, 1) } else { $null }
  finalizeTailSec = if ($doneAt -and $endT) { [math]::Round(($doneAt - $endT) / 1000.0, 2) } else { $null }
  lastPoll = $last; samples = $samples.Count
}
$res | ConvertTo-Json | Set-Content $OutFile
if ($tops.Count) { $tops | Set-Content "$OutFile.top.txt" }
Remove-Item "$OutFile.ready" -ErrorAction SilentlyContinue


