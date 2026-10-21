# adb-only phone UI helpers (uiautomator dump + input tap). Phone input only; never touches the PC UI.
$script:Adb = "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe"; $script:Ser = $env:ANDROID_SERIAL
function Adb { & $script:Adb -s $script:Ser @args }
function Get-UiNodes {
  Adb shell uiautomator dump /sdcard/kin-speedtest-ui.xml | Out-Null
  $x = [xml](Adb exec-out cat /sdcard/kin-speedtest-ui.xml | Out-String -Width 99999 | ForEach-Object { $_.Substring($_.IndexOf('<?xml')) })
  Adb shell rm -f /sdcard/kin-speedtest-ui.xml | Out-Null
  $x.SelectNodes('//node')
}
function Find-Node([string]$Pattern, [string]$Attr = 'text') {
  Get-UiNodes | Where-Object { $_.GetAttribute($Attr) -match $Pattern -or $_.GetAttribute('content-desc') -match $Pattern } | Select-Object -First 1
}
function Tap-Node($n) {
  if (-not $n) { return $false }
  if ($n.bounds -match '\[(\d+),(\d+)\]\[(\d+),(\d+)\]') { Adb shell input tap ([int](([int]$Matches[1]+[int]$Matches[3])/2)) ([int](([int]$Matches[2]+[int]$Matches[4])/2)) | Out-Null; return $true }
  $false
}
function Show-Ui { Get-UiNodes | Where-Object { $_.text -or $_.'content-desc' } | ForEach-Object { "{0} | {1} | {2} | {3}" -f $_.text, $_.'content-desc', $_.'resource-id', $_.bounds } }

