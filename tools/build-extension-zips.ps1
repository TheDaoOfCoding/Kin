# Builds the zips of the browser extension: the store zips for Chrome / Edge (Chromium) and Firefox, and a Chromium side-load zip. ASCII only.
#
#   powershell -ExecutionPolicy Bypass -File tools\build-extension-zips.ps1 [-OutDir <folder>]
#
# What it does: `bun run package` in extension\ (a release build of both targets, then extension\scripts\package.ts), then CHECKS what went into the zips:
#   * the version of both manifests is the version of extension\package.json,
#   * the Chromium store zip has no manifest `key` (the stores give their own extension ID), the Chromium side-load zip has the key and it computes to the ID
#     cbioefpddjpaionhljhffgbedncfchmn that the native host's allowed_origins names (so "Load unpacked" works before a store listing), and the Firefox zip has the gecko ID
#     and `data_collection_permissions`,
#   * `downloads` is only in `optional_permissions` (never in `permissions`), there are no host permissions and no `<all_urls>`, and the dev / test pieces are not in it
#     (the dev host name app.kinrelay.dev, the test host permissions).
# Output: extension\dist\kin-chromium-<version>.zip, kin-chromium-<version>-sideload.zip and kin-firefox-<version>.zip plus SHA256SUMS.txt next to them. `dist\` is ignored by git,
# so nothing here is tracked; -OutDir copies the four files to a folder of your choice. The justification for the optional `downloads` permission (R47) that the stores ask for is in
# extension\store\downloads-permission-justification.md. Nothing here touches a browser, the registry or an installed Kin.
param([string]$OutDir = '')
$ErrorActionPreference = 'Stop'
$Repo = Split-Path -Parent $PSScriptRoot
$Ext = Join-Path $Repo 'extension'
$bun = Get-Command bun -ErrorAction SilentlyContinue
if (-not $bun) { throw 'bun is not on the PATH' }
$version = (Get-Content (Join-Path $Ext 'package.json') -Raw | ConvertFrom-Json).version

Push-Location $Ext
try {
    if (-not (Test-Path (Join-Path $Ext 'node_modules'))) { & bun install --frozen-lockfile; if ($LASTEXITCODE) { throw 'bun install failed' } }
    & bun run package
    if ($LASTEXITCODE) { throw 'bun run package failed' }
} finally { Pop-Location }

Add-Type -AssemblyName System.IO.Compression.FileSystem
function Read-ZipEntry([string]$Zip, [string]$Name) {
    $z = [IO.Compression.ZipFile]::OpenRead($Zip)
    try {
        $e = $z.Entries | Where-Object { $_.FullName -eq $Name } | Select-Object -First 1
        if (-not $e) { return $null }
        $r = New-Object IO.StreamReader($e.Open(), [Text.Encoding]::UTF8)
        try { return $r.ReadToEnd() } finally { $r.Dispose() }
    } finally { $z.Dispose() }
}
function Get-ZipNames([string]$Zip) {
    $z = [IO.Compression.ZipFile]::OpenRead($Zip)
    try { return @($z.Entries | ForEach-Object { $_.FullName }) } finally { $z.Dispose() }
}

$fails = 0
function Check([string]$Name, [bool]$Ok) { if ($Ok) { Write-Host "ok    $Name" } else { Write-Host "FAIL  $Name"; $script:fails++ } }

# The extension ID Chromium derives from a manifest `key`: SHA-256 of the DER public key, the first 32 hex chars with 0-f mapped to a-p.
function Get-ChromiumExtensionId([string]$KeyBase64) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $hash = $sha.ComputeHash([Convert]::FromBase64String($KeyBase64)) } finally { $sha.Dispose() }
    $hex = ($hash | ForEach-Object { $_.ToString('x2') }) -join ''
    return -join ($hex.Substring(0, 32).ToCharArray() | ForEach-Object { [char]([int][char]'a' + [Convert]::ToInt32([string]$_, 16)) })
}
$keyFile = Get-Content (Join-Path $Ext 'manifest-key.json') -Raw | ConvertFrom-Json
$expectedId = 'cbioefpddjpaionhljhffgbedncfchmn'

$zips = [ordered]@{}
foreach ($t in 'chromium', 'chromium-sideload', 'firefox') {
    $target = $t -replace '-sideload$', ''
    $zip = Join-Path $Ext ("dist\kin-$target-$version" + $(if ($t -eq 'chromium-sideload') { '-sideload' } else { '' }) + '.zip')
    if (-not (Test-Path $zip)) { throw "$zip was not made" }
    $zips[$t] = $zip
    $m = (Read-ZipEntry $zip 'manifest.json') | ConvertFrom-Json
    Check "$t zip: manifest version is $version" ($m.version -eq $version)
    Check "$t zip: downloads is optional and not in permissions" ((@($m.optional_permissions) -contains 'downloads') -and (@($m.permissions) -notcontains 'downloads'))
    Check "$t zip: no host permissions, no <all_urls>" (($null -eq $m.host_permissions) -and (-not ((Read-ZipEntry $zip 'manifest.json') -match 'all_urls')))
    $names = Get-ZipNames $zip
    Check "$t zip: has the manifest, the background script and the popup" (($names -contains 'manifest.json') -and ($names -contains 'background.js') -and ($names -contains 'popup/popup.html'))
    $allText = ($names | Where-Object { $_ -match '\.(js|json|html)$' } | ForEach-Object { Read-ZipEntry $zip $_ }) -join "`n"
    Check "$t zip: no dev host name and no local test server in it" (($allText -notmatch 'app\.kinrelay\.dev') -and ($allText -notmatch 'http://127\.0\.0\.1'))
    if ($t -eq 'chromium') {
        Check 'chromium zip: no manifest key (the stores assign the extension ID)' ($null -eq $m.key)
    } elseif ($t -eq 'chromium-sideload') {
        Check 'chromium-sideload zip: manifest key is the one of manifest-key.json' (($null -ne $m.key) -and ($m.key -eq $keyFile.key))
        $id = if ($m.key) { Get-ChromiumExtensionId $m.key } else { '' }
        Check "chromium-sideload zip: the key computes to the extension ID $expectedId (got $id)" (($id -eq $expectedId) -and ($keyFile.id -eq $expectedId))
    } else {
        Check 'firefox zip: gecko id kin@kinrelay.app and data_collection_permissions "none"' (($m.browser_specific_settings.gecko.id -eq 'kin@kinrelay.app') -and (@($m.browser_specific_settings.gecko.data_collection_permissions.required) -contains 'none'))
    }
}
if ($fails) { throw "$fails check(s) on the zips failed" }

$lines = foreach ($t in $zips.Keys) { $h = (Get-FileHash -Algorithm SHA256 -LiteralPath $zips[$t]).Hash.ToLowerInvariant(); "$h  $(Split-Path -Leaf $zips[$t])" }
$sums = Join-Path $Ext 'dist\SHA256SUMS.txt'
[IO.File]::WriteAllText($sums, (($lines -join "`n") + "`n"), (New-Object Text.UTF8Encoding($false)))
if ($OutDir) {
    New-Item -ItemType Directory -Force $OutDir | Out-Null
    foreach ($f in @($zips.Values) + $sums) { Copy-Item -LiteralPath $f -Destination $OutDir -Force }
    Write-Host "copied to $OutDir"
}
$lines | ForEach-Object { Write-Host $_ }
Write-Host "the zips are in $(Join-Path $Ext 'dist') (ignored by git); the permission justification is extension\store\downloads-permission-justification.md"
