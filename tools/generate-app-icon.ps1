# Requires ImageMagick 7. Run only when the selected design changes.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$sourceImage = Join-Path $projectRoot 'docs\design\luciddesk-green-simple-v17.png'
$iconOutput = Join-Path $projectRoot 'app\assets\luciddesk.ico'
# Measure the visible artwork, ignoring almost-transparent generation artifacts.
# Use this mask only for bounds; leave the selected design untouched.
$artBounds = & magick $sourceImage -alpha extract -threshold '25%' -format '%@' info:
if ($LASTEXITCODE -ne 0 -or $artBounds -notmatch '^(\d+)x(\d+)\+(\d+)\+(\d+)$') {
    throw 'Failed to measure the application icon artwork.'
}
$artWidth = [int]$Matches[1]
$artHeight = [int]$Matches[2]
if ($artWidth -le 0 -or $artHeight -le 0) {
    throw 'The application icon has no visible artwork.'
}
# Fill 87.5% of the square icon cell (14 of 16 pixels at tray size).
$canvasSize = [int][Math]::Ceiling([Math]::Max($artWidth, $artHeight) / 0.875)
# Normalize transparent padding before high-quality premultiplied resampling.
Add-Type -AssemblyName System.Drawing
$normalized = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName() + '.png')
$sourcePath = $sourceImage
$sizes = @(16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 128, 256)
$frames = [System.Collections.Generic.List[byte[]]]::new()
function ConvertTo-IconPng([int]$size, [double]$scale) {
    # Downsample in half-size steps so small frames average the artwork instead
    # of sampling isolated source pixels. Premultiplied alpha keeps RGB noise
    # in transparent pixels from bleeding into the visible silhouette.
    $current = $sourceImage
    try {
        while ($current.Width -gt $size * 2) {
            $nextSize = [int][Math]::Ceiling($current.Width / 2)
            $next = Resize-IconImage $current $nextSize 1.0
            if ($current -ne $sourceImage) { $current.Dispose() }
            $current = $next
        }
        # Smooth the final small shell frame without repeatedly softening fine details.
        $filter = if ($size -le 64) {
            [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBilinear
        } else {
            [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        }
        $bitmap = Resize-IconImage $current $size $scale $filter
    } finally {
        if ($current -ne $sourceImage) { $current.Dispose() }
    }
    try {
        $stream = [System.IO.MemoryStream]::new()
        try {
            $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
            return ,$stream.ToArray()
        } finally { $stream.Dispose() }
    } finally { $bitmap.Dispose() }
}
function Resize-IconImage([System.Drawing.Image]$image, [int]$size, [double]$scale,
    [System.Drawing.Drawing2D.InterpolationMode]$filter = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic) {
    $bitmap = [System.Drawing.Bitmap]::new($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppPArgb)
    try {
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
            $graphics.InterpolationMode = $filter
            $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $extent = [single]($size * $scale)
            $inset = [single](($size - $extent) / 2)
            $graphics.DrawImage($image, [System.Drawing.RectangleF]::new($inset, $inset, $extent, $extent),
                [System.Drawing.RectangleF]::new(0, 0, $image.Width, $image.Height), [System.Drawing.GraphicsUnit]::Pixel)
        } finally { $graphics.Dispose() }
        return ,$bitmap
    } catch { $bitmap.Dispose(); throw }
}

try {
    & magick $sourcePath -crop $artBounds +repage -background none -gravity center `
        -extent "${canvasSize}x${canvasSize}" $normalized
    if ($LASTEXITCODE -ne 0) { throw 'Failed to normalize application icon padding.' }
    $sourceImage = [System.Drawing.Image]::FromFile($normalized)
    try {
        foreach ($size in $sizes) {
            $png = ConvertTo-IconPng $size 1.0
            $frames.Add($png)
        }
        $uiFrame = $png # Keep the 256px PNG straight-alpha for application rendering.
    } finally { $sourceImage.Dispose() }
} finally {
    if (Test-Path -LiteralPath $normalized) { Remove-Item -LiteralPath $normalized }
}
# Store every size as straight-alpha PNG so Shell and window loaders decode
# transparency consistently, without DIB premultiplication ambiguity.
$iconStream = [System.IO.File]::Create($iconOutput)
try {
    $writer = [System.IO.BinaryWriter]::new($iconStream)
    try {
        $writer.Write([uint16]0)
        $writer.Write([uint16]1)
        $writer.Write([uint16]$sizes.Count)
        $offset = 6 + 16 * $sizes.Count
        for ($index = 0; $index -lt $sizes.Count; ++$index) {
            $dimension = if ($sizes[$index] -eq 256) { 0 } else { $sizes[$index] }
            $writer.Write([byte]$dimension)
            $writer.Write([byte]$dimension)
            $writer.Write([byte]0)
            $writer.Write([byte]0)
            $writer.Write([uint16]1)
            $writer.Write([uint16]32)
            $writer.Write([uint32]$frames[$index].Length)
            $writer.Write([uint32]$offset)
            $offset += $frames[$index].Length
        }
        foreach ($frame in $frames) { $writer.Write($frame) }
    } finally { $writer.Dispose() }
} finally { $iconStream.Dispose() }
[System.IO.File]::WriteAllBytes((Join-Path $projectRoot 'docs/images/app-icon.png'), $uiFrame)

$encoded = [Convert]::ToBase64String($uiFrame)
foreach ($name in @('overview.svg', 'overview.en.svg')) {
    $path = Join-Path $projectRoot "docs/images/$name"
    $svg = [IO.File]::ReadAllText($path)
    $svg = [regex]::Replace($svg, 'data:image/png;base64,[A-Za-z0-9+/=]+', "data:image/png;base64,$encoded")
    [IO.File]::WriteAllText($path, $svg, [Text.UTF8Encoding]::new($false))
}
