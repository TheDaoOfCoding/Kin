# Makes the test files in %LOCALAPPDATA%\Temp\kin-speedtest\data (one 2 GiB random file) and \photos (50 random .jpg, about 3.4 MB each).
$r = "$env:LOCALAPPDATA\Temp\kin-speedtest"; New-Item -ItemType Directory -Force "$r\data","$r\photos","$r\results" | Out-Null
$rng = [Security.Cryptography.RandomNumberGenerator]::Create()
$buf = New-Object byte[] (16MB); $fs = [IO.File]::Create("$r\data\kin-speedtest-2g.bin")
for ($i = 0; $i -lt 128; $i++) { $rng.GetBytes($buf); $fs.Write($buf, 0, $buf.Length) }; $fs.Close()
for ($i = 1; $i -le 50; $i++) { $b = New-Object byte[] (3400000 + $i * 1000); $rng.GetBytes($b); $b[0] = 0xFF; $b[1] = 0xD8; $b[2] = 0xFF; [IO.File]::WriteAllBytes(("$r\photos\kin-speedtest-photo-{0:D2}.jpg" -f $i), $b) }
