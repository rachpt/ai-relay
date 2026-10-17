# Windows PowerShell 5.1; uses only the Windows .NET Framework and System.Drawing.
[CmdletBinding()]
param([switch]$ValidateOnly)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$assetDirectory = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\assets'))
$svgPath = Join-Path $assetDirectory 'ai-relay.svg'
$iconPath = Join-Path $assetDirectory 'ai-relay.ico'
$sizes = @(16, 24, 32, 48, 64, 128, 256)
$culture = [Globalization.CultureInfo]::InvariantCulture

function Get-Number([string]$Value) {
    return [single]::Parse($Value, $culture)
}

function Get-Points([string]$Value) {
    foreach ($pair in ($Value.Trim() -split '\s+')) {
        $xy = $pair.Split(',')
        if ($xy.Count -ne 2) { throw "Invalid SVG point: $pair" }
        [Drawing.PointF]::new((Get-Number $xy[0]), (Get-Number $xy[1]))
    }
}

function New-IconFrame([xml]$Svg, [int]$Size) {
    $scale = 4
    $large = [Drawing.Bitmap]::new($Size * $scale, $Size * $scale)
    $small = [Drawing.Bitmap]::new($Size, $Size)
    $graphics = $null
    $outputGraphics = $null
    $stream = [IO.MemoryStream]::new()
    try {
        $graphics = [Drawing.Graphics]::FromImage($large)
        $graphics.Clear([Drawing.Color]::Transparent)
        $graphics.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $graphics.ScaleTransform(($Size * $scale / 64.0), ($Size * $scale / 64.0))
        foreach ($node in $Svg.DocumentElement.ChildNodes) {
            if ($node.LocalName -in @('title', 'desc')) { continue }
            switch ($node.LocalName) {
                'rect' {
                    $x = Get-Number $node.GetAttribute('x')
                    $y = Get-Number $node.GetAttribute('y')
                    $w = Get-Number $node.GetAttribute('width')
                    $h = Get-Number $node.GetAttribute('height')
                    $d = 2 * (Get-Number $node.GetAttribute('rx'))
                    $path = [Drawing.Drawing2D.GraphicsPath]::new()
                    $brush = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml($node.GetAttribute('fill')))
                    try {
                        $path.AddArc($x, $y, $d, $d, 180, 90)
                        $path.AddArc(($x + $w - $d), $y, $d, $d, 270, 90)
                        $path.AddArc(($x + $w - $d), ($y + $h - $d), $d, $d, 0, 90)
                        $path.AddArc($x, ($y + $h - $d), $d, $d, 90, 90)
                        $path.CloseFigure()
                        $graphics.FillPath($brush, $path)
                    }
                    finally { $brush.Dispose(); $path.Dispose() }
                }
                'polyline' {
                    if ($node.GetAttribute('stroke-linecap') -ne 'round' -or
                        $node.GetAttribute('stroke-linejoin') -ne 'round') {
                        throw 'Only round SVG polyline caps and joins are supported.'
                    }
                    $points = [Drawing.PointF[]]@(Get-Points $node.GetAttribute('points'))
                    $pen = [Drawing.Pen]::new(
                        [Drawing.ColorTranslator]::FromHtml($node.GetAttribute('stroke')),
                        (Get-Number $node.GetAttribute('stroke-width')))
                    try {
                        $pen.StartCap = [Drawing.Drawing2D.LineCap]::Round
                        $pen.EndCap = [Drawing.Drawing2D.LineCap]::Round
                        $pen.LineJoin = [Drawing.Drawing2D.LineJoin]::Round
                        $graphics.DrawLines($pen, $points)
                    }
                    finally { $pen.Dispose() }
                }
                'polygon' {
                    $points = [Drawing.PointF[]]@(Get-Points $node.GetAttribute('points'))
                    $brush = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml($node.GetAttribute('fill')))
                    try { $graphics.FillPolygon($brush, $points) }
                    finally { $brush.Dispose() }
                }
                default { throw "Unsupported SVG element: $($node.LocalName)" }
            }
        }
        $outputGraphics = [Drawing.Graphics]::FromImage($small)
        $outputGraphics.CompositingMode = [Drawing.Drawing2D.CompositingMode]::SourceCopy
        $outputGraphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $outputGraphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $outputGraphics.DrawImage($large, [Drawing.Rectangle]::new(0, 0, $Size, $Size),
            0, 0, $large.Width, $large.Height, [Drawing.GraphicsUnit]::Pixel)
        $small.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
        return ,$stream.ToArray()
    }
    finally {
        if ($null -ne $outputGraphics) { $outputGraphics.Dispose() }
        if ($null -ne $graphics) { $graphics.Dispose() }
        $stream.Dispose(); $small.Dispose(); $large.Dispose()
    }
}

function Test-Icon([byte[]]$Bytes) {
    $stream = [IO.MemoryStream]::new($Bytes, $false)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0 -or $reader.ReadUInt16() -ne 1 -or
            $reader.ReadUInt16() -ne $sizes.Count) { throw 'Invalid ICO header.' }
        $expectedOffset = 6 + 16 * $sizes.Count
        foreach ($size in $sizes) {
            $widthByte = $reader.ReadByte()
            $heightByte = $reader.ReadByte()
            $width = if ($widthByte -eq 0) { 256 } else { [int]$widthByte }
            $height = if ($heightByte -eq 0) { 256 } else { [int]$heightByte }
            $colors = $reader.ReadByte()
            $reserved = $reader.ReadByte()
            $planes = $reader.ReadUInt16()
            $bits = $reader.ReadUInt16()
            $length = $reader.ReadUInt32()
            $offset = $reader.ReadUInt32()
            if ($width -ne $size -or $height -ne $size -or $colors -ne 0 -or
                $reserved -ne 0 -or $planes -ne 1 -or $bits -ne 32 -or
                $offset -ne $expectedOffset -or $length -lt 24 -or
                ([long]$offset + $length) -gt $Bytes.Length) {
                throw "Invalid ICO directory entry for ${size}px."
            }
            $position = $stream.Position
            $stream.Position = $offset
            $payload = $reader.ReadBytes([int]$length)
            if ([BitConverter]::ToString($payload, 0, 8) -ne '89-50-4E-47-0D-0A-1A-0A') {
                throw "Missing PNG signature for ${size}px."
            }
            $pngStream = [IO.MemoryStream]::new($payload, $false)
            $image = $null
            try {
                $image = [Drawing.Image]::FromStream($pngStream, $true, $true)
                if ($image.Width -ne $size -or $image.Height -ne $size -or
                    [Drawing.Image]::GetPixelFormatSize($image.PixelFormat) -ne 32) {
                    throw "Incorrect PNG dimensions or bit depth for ${size}px."
                }
            }
            finally {
                if ($null -ne $image) { $image.Dispose() }
                $pngStream.Dispose()
            }
            $stream.Position = $position
            $expectedOffset += $length
            Write-Host ("{0,3}x{0,-3}  32bpp  PNG  offset={1,6}  bytes={2,6}" -f $size, $offset, $length)
        }
        if ($expectedOffset -ne $Bytes.Length) { throw 'Unexpected trailing ICO data.' }
    }
    finally { $reader.Dispose(); $stream.Dispose() }
}

if (-not $ValidateOnly) {
    # This deliberately supports only the simple editable SVG vocabulary above,
    # not arbitrary SVG (no browser, SVG library, fonts or external references).
    [xml]$svg = [IO.File]::ReadAllText($svgPath)
    if ($svg.DocumentElement.GetAttribute('viewBox') -ne '0 0 64 64') {
        throw 'Expected SVG viewBox="0 0 64 64".'
    }
    $frames = @($sizes | ForEach-Object { New-IconFrame $svg $_ })
    $stream = [IO.MemoryStream]::new()
    $writer = [IO.BinaryWriter]::new($stream)
    try {
        $writer.Write([uint16]0)
        $writer.Write([uint16]1)
        $writer.Write([uint16]$sizes.Count)
        $offset = 6 + 16 * $sizes.Count
        for ($i = 0; $i -lt $sizes.Count; $i++) {
            $dimension = if ($sizes[$i] -eq 256) { 0 } else { $sizes[$i] }
            $writer.Write([byte]$dimension)
            $writer.Write([byte]$dimension)
            $writer.Write([byte]0)
            $writer.Write([byte]0)
            $writer.Write([uint16]1)
            $writer.Write([uint16]32)
            $writer.Write([uint32]$frames[$i].Length)
            $writer.Write([uint32]$offset)
            $offset += $frames[$i].Length
        }
        foreach ($frame in $frames) { $writer.Write([byte[]]$frame) }
        $writer.Flush()
        $bytes = $stream.ToArray()
        Test-Icon $bytes
        [IO.File]::WriteAllBytes($iconPath, $bytes)
    }
    finally { $writer.Dispose(); $stream.Dispose() }
    Write-Host "Generated: $iconPath"
}
else {
    Test-Icon ([IO.File]::ReadAllBytes($iconPath))
    Write-Host "Validated: $iconPath"
}
