[CmdletBinding()]
param([switch]$Offline)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$previousRevision = $env:LUCIDPANE_BUILD_REVISION
Push-Location -LiteralPath $repoRoot
try {
    $revision = & git rev-parse --short HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Could not read Git revision.' }
    $dirty = [bool](& git status --porcelain --untracked-files=no)
    $revisionLabel = if ($dirty) { "$revision-dirty" } else { $revision }
    $env:LUCIDPANE_BUILD_REVISION = $revisionLabel
    $hostInfo = & rustc -vV
    if ($LASTEXITCODE -ne 0 -or $hostInfo -notcontains 'host: x86_64-pc-windows-msvc') {
        throw 'Preview packaging requires the Windows x64 MSVC Rust toolchain.'
    }
    # Keep release artifacts separate from explicitly enabled diagnostic backends.
    $productionTarget = Join-Path $repoRoot 'target\production'
    $buildArgs = @('build', '--release', '--locked', '--no-default-features', '--target-dir', $productionTarget, '-p', 'lucidpane', '-p', 'desktop-hook')
    if ($Offline) { $buildArgs += '--offline' }
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    $metadata = & cargo metadata --no-deps --format-version 1 --offline --locked | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Could not read package metadata.' }
    $version = ($metadata.packages | Where-Object name -eq 'lucidpane').version
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
    $name = "LucidPane-$version-preview-$revisionLabel-windows-x64-$stamp"
    $outRoot = Join-Path $repoRoot 'target\preview'
    $stage = Join-Path $outRoot $name
    New-Item -ItemType Directory -Path $stage | Out-Null
    foreach ($file in @('lucidpane.exe', 'desktop_hook.dll')) {
        Copy-Item -LiteralPath (Join-Path $productionTarget "release\$file") -Destination $stage
    }
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\preview.md') -Destination (Join-Path $stage 'README.md')
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\usage.md') -Destination (Join-Path $stage 'usage.md')
    Copy-Item -LiteralPath (Join-Path $repoRoot 'CHANGELOG.md') -Destination (Join-Path $stage 'CHANGELOG.md')
    $files = @('lucidpane.exe', 'desktop_hook.dll') | ForEach-Object {
        $fileHash = Get-FileHash -LiteralPath (Join-Path $stage $_) -Algorithm SHA256
        [ordered]@{ file = $_; sha256 = $fileHash.Hash.ToLowerInvariant() }
    }
    [ordered]@{
        version = $version; channel = 'preview'; revision = $revision; uncommittedChanges = $dirty
        builtAt = (Get-Date).ToUniversalTime().ToString('o'); architecture = 'windows-x64'
        files = $files
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $stage 'build.json') -Encoding UTF8
    $archive = Join-Path $outRoot "$name.zip"
    Compress-Archive -LiteralPath $stage -DestinationPath $archive -CompressionLevel Optimal
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $name.zip" | Set-Content -LiteralPath "$archive.sha256" -Encoding ASCII
    Write-Output $archive
} finally {
    $env:LUCIDPANE_BUILD_REVISION = $previousRevision
    Pop-Location
}
