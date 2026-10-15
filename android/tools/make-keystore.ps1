# Creates the release signing key for Kin: %USERPROFILE%\.kinrelay\release.jks and signing.properties next to it.
#   android\tools\make-keystore.ps1
# Nothing is written inside the repo, the password is never printed, and an existing keystore is never touched.
#
# LOSING THIS KEY MEANS NO UPDATE CAN EVER INSTALL OVER AN EXISTING INSTALL (Android refuses an APK signed by a different key;
# the only way out is to uninstall, which deletes the pairing and the queue). Back up the whole .kinrelay folder.
$ErrorActionPreference = 'Stop'
if (-not $env:JAVA_HOME) { $env:JAVA_HOME = [Environment]::GetEnvironmentVariable('JAVA_HOME', 'User') }
$keytool = Join-Path $env:JAVA_HOME 'bin\keytool.exe'
if (-not (Test-Path $keytool)) { throw "keytool not found (JAVA_HOME=$env:JAVA_HOME)" }

$dir = Join-Path $env:USERPROFILE '.kinrelay'
$jks = Join-Path $dir 'release.jks'
$props = Join-Path $dir 'signing.properties'
if ((Test-Path $jks) -or (Test-Path $props)) { throw "$dir already has a keystore or signing.properties. Not touching it." }
New-Item -ItemType Directory -Force $dir | Out-Null

# 32 random characters from a cryptographic generator: letters and digits only, so no file or shell quoting can break it.
$alphabet = [char[]]'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789'
$bytes = New-Object byte[] 64
[System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
$password = -join (0..31 | ForEach-Object { $alphabet[$bytes[$_] % $alphabet.Length] })
$alias = 'kinrelay'

# The password goes to keytool through an environment variable (-storepass:env), not the command line.
$env:KIN_TMP_PASSWORD = $password
try {
    & $keytool -genkeypair -keystore $jks -storetype PKCS12 -alias $alias -keyalg RSA -keysize 4096 -sigalg SHA256withRSA `
        -validity 10000 -dname 'CN=Kin Relay, O=Kin Relay' -storepass:env KIN_TMP_PASSWORD -keypass:env KIN_TMP_PASSWORD
    if ($LASTEXITCODE -ne 0) { throw "keytool failed ($LASTEXITCODE)" }
} finally {
    Remove-Item Env:\KIN_TMP_PASSWORD -ErrorAction SilentlyContinue
}

@(
    '# Kin Relay release signing. Read by android/app/build.gradle.kts. NEVER commit or share this file.',
    '# BACK UP THIS WHOLE FOLDER (release.jks + this file). Lose the key and no update installs over existing installs.',
    'storeFile=release.jks',
    "storePassword=$password",
    "keyAlias=$alias",
    "keyPassword=$password"
) | Set-Content -Path $props -Encoding ascii

# Only the current user may read the folder.
& icacls $dir /inheritance:r /grant:r "$($env:USERNAME):(OI)(CI)F" | Out-Null
"Created $jks and $props (RSA 4096, valid 10000 days = 27 years). Back up $dir now."
