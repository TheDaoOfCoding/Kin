# Builds the throwaway "Demo Bank" test app (test.bank.demo) into <Out>\bankdemo.apk, signed with a throwaway key made here. ASCII only.
#   tools\e2e-2b-2\bankdemo\build.ps1 -Out C:\some\empty\dir
# Needs the Android SDK (build-tools 37, platform android-37.0) and the JDK of JAVA_HOME. Never uses Kin's release key.
param([Parameter(Mandatory = $true)][string]$Out)
$ErrorActionPreference = 'Stop'
$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
$bt = Get-ChildItem (Join-Path $sdk 'build-tools') -Directory | Sort-Object Name -Descending | Select-Object -First 1
$jar = Join-Path $sdk 'platforms\android-37.0\android.jar'
$java = if ($env:JAVA_HOME) { $env:JAVA_HOME } else { [Environment]::GetEnvironmentVariable('JAVA_HOME', 'User') }
$javac = Join-Path $java 'bin\javac.exe'; $keytool = Join-Path $java 'bin\keytool.exe'
New-Item -ItemType Directory -Force $Out | Out-Null
$classes = Join-Path $Out 'classes'; New-Item -ItemType Directory -Force $classes | Out-Null
$srcs = @(Get-ChildItem (Join-Path $PSScriptRoot 'src') -Recurse -Filter *.java | ForEach-Object { $_.FullName })
& $javac --release 11 -classpath $jar -d $classes @srcs
if ($LASTEXITCODE -ne 0) { throw 'javac failed' }
$cls = @(Get-ChildItem $classes -Recurse -Filter *.class | ForEach-Object { $_.FullName })
& (Join-Path $bt.FullName 'd8.bat') --lib $jar --min-api 29 --output $Out @cls
if ($LASTEXITCODE -ne 0) { throw 'd8 failed' }
$base = Join-Path $Out 'base.apk'
& (Join-Path $bt.FullName 'aapt2.exe') link -o $base --manifest (Join-Path $PSScriptRoot 'AndroidManifest.xml') -I $jar --min-sdk-version 29 --target-sdk-version 37
if ($LASTEXITCODE -ne 0) { throw 'aapt2 failed' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::Open($base, 'Update')
try { [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, (Join-Path $Out 'classes.dex'), 'classes.dex') } finally { $zip.Dispose() }
$aligned = Join-Path $Out 'aligned.apk'
& (Join-Path $bt.FullName 'zipalign.exe') -f -p 4 $base $aligned
if ($LASTEXITCODE -ne 0) { throw 'zipalign failed' }
$ks = Join-Path $Out 'throwaway.jks'
if (-not (Test-Path $ks)) { $ErrorActionPreference = 'Continue'; & $keytool -genkeypair -keystore $ks -storepass throwaway -keypass throwaway -alias t -keyalg RSA -keysize 2048 -validity 30 -dname 'CN=throwaway' 2>$null | Out-Null; $ErrorActionPreference = 'Stop' }
$apk = Join-Path $Out 'bankdemo.apk'
& (Join-Path $bt.FullName 'apksigner.bat') sign --ks $ks --ks-pass pass:throwaway --key-pass pass:throwaway --out $apk $aligned
if ($LASTEXITCODE -ne 0) { throw 'apksigner failed' }
Write-Host "built $apk"

