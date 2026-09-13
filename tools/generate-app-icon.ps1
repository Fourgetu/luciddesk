# Requires ImageMagick 7. Run only when the selected design changes.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$sourceImage = Join-Path $projectRoot 'docs\design\lucidpane-icon-aligned-cyan-v16.png'
$iconOutput = Join-Path $projectRoot 'app\assets\lucidpane.ico'
# Measure the visible artwork, ignoring almost-transparent generation artifacts.
# Use this mask only for bounds; keep the source pixels and alpha unchanged.
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
& magick $sourceImage -crop $artBounds +repage -background none -gravity center `
    -extent "${canvasSize}x${canvasSize}" `
    -define 'icon:auto-resize=256,128,96,64,48,40,32,24,20,16' $iconOutput
if ($LASTEXITCODE -ne 0) {
    throw 'Failed to generate the application ICO.'
}
