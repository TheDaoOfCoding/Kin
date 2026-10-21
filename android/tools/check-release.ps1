# Release APK checks for milestone 1.5: no Google telemetry parts, what the manifest holds, size, and (if signed) the certificate.
#   android\tools\check-release.ps1 [-Apk path\to\app-release.apk]
# Exit code 1 when a forbidden component or string is found.
param(
    [string]$Apk = (Join-Path $PSScriptRoot '..\app\build\outputs\apk\release\app-release.apk')
)
$ErrorActionPreference = 'Continue'
if (-not $env:JAVA_HOME) { $env:JAVA_HOME = [Environment]::GetEnvironmentVariable('JAVA_HOME', 'User') }
$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
$bt = Get-ChildItem (Join-Path $sdk 'build-tools') | Sort-Object { [version]($_.Name -replace '[^\d.].*$','') } | Select-Object -Last 1
$aapt2 = Join-Path $bt.FullName 'aapt2.exe'
$apksigner = Join-Path $bt.FullName 'apksigner.bat'
if (-not (Test-Path $Apk)) { throw "No APK at $Apk" }
$Apk = (Resolve-Path $Apk).Path
$failed = $false

"APK: $Apk"
"Size: {0:N0} bytes ({1:N2} MiB)" -f (Get-Item $Apk).Length, ((Get-Item $Apk).Length / 1MB)

# 1. Manifest (the merged one inside the APK): every component and permission name.
$tree = & $aapt2 dump xmltree --file AndroidManifest.xml $Apk
$names = $tree | Select-String 'android:name\(0x01010003\)="([^"]+)"' | ForEach-Object { $_.Matches[0].Groups[1].Value }
"`nManifest names ($($names.Count)):"
$names | ForEach-Object { "  $_" }
$badManifest = @($names | Where-Object { $_ -match 'gms|firebase|datatransport|mlkit|google\.android' })
if ($badManifest.Count) { $failed = $true; "FORBIDDEN in manifest: $($badManifest -join ', ')" } else { 'Manifest: no gms / firebase / datatransport / mlkit names.' }
"Camera features (must say required=false):"
$tree | Select-String 'android.hardware.camera' -Context 0,1 | ForEach-Object { "  " + ($_.Line.Trim() -replace '^A: http://schemas.android.com/apk/res/android:','') + "  " + ($_.Context.PostContext -join ' ').Trim() }

# 2. Dex strings: class descriptors and any text with the forbidden words.
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::OpenRead($Apk)
$patterns = 'datatransport', 'firebase', 'play-services-mlkit', 'com/google/android/gms', 'com.google.android.gms', 'mlkit', 'com/google/mlkit', 'vision/codescanner', 'googleapis.com', 'app-measurement', 'crashlytics'
$enc = [System.Text.Encoding]::GetEncoding(28591)
"`nDex scan:"
try {
    foreach ($e in $zip.Entries | Where-Object { $_.FullName -match '^classes\d*\.dex$' }) {
        $ms = New-Object System.IO.MemoryStream
        $s = $e.Open(); $s.CopyTo($ms); $s.Dispose()
        $text = $enc.GetString($ms.ToArray())
        # OkHttp compares the security provider name with this string to detect Conscrypt. It is only a name, not a Google component.
        $conscrypt = ([regex]::Matches($text, 'com\.google\.android\.gms\.org\.conscrypt')).Count
        if ($conscrypt) { "  (allowed) $conscrypt x 'com.google.android.gms.org.conscrypt': OkHttp's provider-name check" }
        $text = $text.Replace('com.google.android.gms.org.conscrypt', '')
        foreach ($p in $patterns) {
            $n = ([regex]::Matches($text, [regex]::Escape($p))).Count
            if ($n -gt 0) { $failed = $true; "  FOUND '$p' x$n in $($e.FullName)" }
        }
        "  $($e.FullName): $([math]::Round($e.Length/1KB)) KiB scanned"
    }
    # Other files that can carry a Google component (assets, META-INF service files, resources).
    foreach ($e in $zip.Entries | Where-Object { $_.FullName -match 'gms|firebase|datatransport|mlkit|google' -and $_.FullName -notmatch '\.dex$' -and $_.FullName -ne 'META-INF/com.google.dagger_dagger.version' }) {
        $failed = $true; "  FORBIDDEN entry: $($e.FullName)"
    }
    $native = @($zip.Entries | Where-Object { $_.FullName -like 'lib/*' })
    "Native libraries in the APK: $($native.Count)"
} finally { $zip.Dispose() }
if (-not $failed) { 'Dex and entries: none of ' + ($patterns -join ', ') }

# 3. Signature.
"`nSignature:"
& $apksigner verify --verbose --print-certs $Apk 2>&1 | Where-Object { $_ -notmatch '^WARNING' } | Select-Object -First 12

if ($failed) { "`nRESULT: FAILED (see FOUND / FORBIDDEN lines)"; exit 1 } else { "`nRESULT: clean" }
