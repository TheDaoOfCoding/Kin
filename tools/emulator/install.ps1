# Installs the Kin debug APK on the running emulator and launches it.
#
#   .\install.ps1                          installs android\app\build\outputs\apk\debug\app-debug.apk
#   .\install.ps1 -Apk C:\path\to.apk
#   .\install.ps1 -Serial emulator-5554    when more than one device is connected
#
# Build the APK first if needed:  cd android; gradlew.bat assembleDebug
param(
    [string]$Apk = (Join-Path $PSScriptRoot '..\..\android\app\build\outputs\apk\debug\app-debug.apk'),
    [string]$Serial,
    [string]$Activity = 'app.kinrelay.android/.MainActivity'
)
. "$PSScriptRoot\_common.ps1"

if (-not (Test-Path $Apk)) { throw "APK not found: $Apk. Build it: cd android; gradlew.bat assembleDebug" }
$Apk = (Resolve-Path $Apk).Path
Initialize-Adb
$Serial = Resolve-Serial $Serial

Write-Host "Installing $Apk on $Serial..."
$out = & $Adb -s $Serial install -r $Apk 2>&1 | Out-String
if ($out -notmatch 'Success') { throw "adb install failed:`n$out" }

& $Adb -s $Serial shell am force-stop ($Activity -split '/')[0] | Out-Null
$out = & $Adb -s $Serial shell am start -W -n $Activity 2>&1 | Out-String
if ($out -match 'Error|Exception') { throw "Launch failed:`n$out" }
Write-Host "Launched $Activity"
