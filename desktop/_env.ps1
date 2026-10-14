# Dot-source this to get cargo/rustc on PATH in a shell that predates the rustup install.
# rustup adds %USERPROFILE%\.cargo\bin to the USER PATH itself. This only prepends it when the session lacks it, and keeps the rest of the session PATH.
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $cargoBin })) { $env:Path = "$cargoBin;$env:Path" }
