# Regenerates src-tauri\icons\* from the placeholder icon of the .NET spike, so both spikes show the same "K".
# kin.ico holds PNG-compressed frames. This copies out the largest one as icon-source.png and runs `tauri icon`.
# The original kin.ico is only read, never modified. Only needed to regenerate; the icons are committed.
. "$PSScriptRoot\_env.ps1"
$ico = Join-Path $PSScriptRoot '..\desktop-dotnet\Assets\kin.ico'
$bytes = [System.IO.File]::ReadAllBytes($ico)
$count = [BitConverter]::ToUInt16($bytes, 4)
$best = $null
for ($i = 0; $i -lt $count; $i++) {
    $e = 6 + 16 * $i
    $w = if ($bytes[$e] -eq 0) { 256 } else { $bytes[$e] }
    $len = [BitConverter]::ToUInt32($bytes, $e + 8); $off = [BitConverter]::ToUInt32($bytes, $e + 12)
    if (-not $best -or $w -gt $best.w) { $best = @{ w = $w; len = $len; off = $off } }
}
$src = Join-Path $PSScriptRoot 'icon-source.png'
[System.IO.File]::WriteAllBytes($src, $bytes[$best.off..($best.off + $best.len - 1)])
Push-Location $PSScriptRoot
try { bun tauri icon $src } finally { Pop-Location }
