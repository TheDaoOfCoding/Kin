# Generates Assets\kin.ico: a white "K" on a rounded indigo square. Sizes 16/32/48/256, PNG-compressed.
# Only needed to regenerate the icon; the .ico is committed.
Add-Type -AssemblyName System.Drawing

$out = Join-Path $PSScriptRoot 'Assets\kin.ico'
New-Item -ItemType Directory -Force (Split-Path $out) | Out-Null

function New-KinPng([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.TextRenderingHint = 'AntiAliasGridFit'
    $g.Clear([System.Drawing.Color]::Transparent)

    $r = [int]($size * 0.22)
    $d = $r * 2
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.AddArc(0, 0, $d, $d, 180, 90)
    $path.AddArc($size - $d - 1, 0, $d, $d, 270, 90)
    $path.AddArc($size - $d - 1, $size - $d - 1, $d, $d, 0, 90)
    $path.AddArc(0, $size - $d - 1, $d, $d, 90, 90)
    $path.CloseFigure()
    $g.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 79, 70, 229))), $path)

    $font = New-Object System.Drawing.Font 'Segoe UI', ($size * 0.58), ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Pixel)
    $fmt = New-Object System.Drawing.StringFormat
    $fmt.Alignment = 'Center'; $fmt.LineAlignment = 'Center'
    $rect = New-Object System.Drawing.RectangleF 0, ($size * 0.02), $size, $size
    $g.DrawString('K', $font, [System.Drawing.Brushes]::White, $rect, $fmt)
    $g.Dispose()

    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    , $ms.ToArray()
}

$sizes = 16, 32, 48, 256
$pngs = $sizes | ForEach-Object { , (New-KinPng $_) }

$fs = [System.IO.File]::Create($out)
$w = New-Object System.IO.BinaryWriter $fs
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)   # ICONDIR
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]
    $w.Write([byte]($s % 256)); $w.Write([byte]($s % 256))                  # 256 is stored as 0
    $w.Write([byte]0); $w.Write([byte]0)                                    # palette, reserved
    $w.Write([uint16]1); $w.Write([uint16]32)                               # planes, bpp
    $w.Write([uint32]$pngs[$i].Length); $w.Write([uint32]$offset)
    $offset += $pngs[$i].Length
}
foreach ($p in $pngs) { $w.Write($p) }
$w.Dispose(); $fs.Dispose()
"Wrote $out ($((Get-Item $out).Length) bytes)"
