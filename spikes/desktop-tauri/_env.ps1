# Dot-source this to get cargo/rustc on PATH in a shell that predates the rustup install.
# rustup adds %USERPROFILE%\.cargo\bin to the USER PATH itself; this only re-reads the registry values.
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
