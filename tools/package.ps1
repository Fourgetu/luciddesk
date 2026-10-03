[CmdletBinding()]
param([switch]$Offline, [switch]$Portable, [switch]$RenderDiagnostics, [switch]$Installer, [string]$InnoCompiler)
$ErrorActionPreference = 'Stop'
if ($RenderDiagnostics -and -not $Portable) { throw 'Rendering comparison launchers require -Portable.' }
if ($Installer -and $Portable) { throw 'Installer and portable packages are separate channels.' }
$repoRoot = Split-Path -Parent $PSScriptRoot
$previousRevision = $env:LUCIDDESK_BUILD_REVISION
Push-Location -LiteralPath $repoRoot
try {
    & (Join-Path $PSScriptRoot 'use-windows-toolchain.ps1')
    $revision = & git rev-parse --short HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Could not read Git revision.' }
    $dirty = [bool](& git status --porcelain --untracked-files=no)
    $revisionLabel = if ($dirty) { "$revision-dirty" } else { $revision }
    $env:LUCIDDESK_BUILD_REVISION = $revisionLabel
    $toolchain = Get-Content -LiteralPath (Join-Path $repoRoot 'rust-toolchain.toml') -Raw
    $pinnedVersion = [regex]::Match($toolchain, 'channel\s*=\s*"([^"]+)"').Groups[1].Value
    $hostInfo = & rustc -vV
    if ($LASTEXITCODE -ne 0 -or $hostInfo -notcontains 'host: x86_64-pc-windows-msvc') {
        throw 'Packaging requires the Windows x64 MSVC Rust toolchain.'
    }
    if (-not $pinnedVersion -or $hostInfo -notcontains "release: $pinnedVersion") {
        throw "Packaging requires the pinned Rust $pinnedVersion toolchain."
    }
    $cargoVersion = & cargo -V
    if ($LASTEXITCODE -ne 0) { throw 'Could not read Cargo version.' }
    # Keep release artifacts separate from explicitly enabled diagnostic backends.
    $productionTarget = Join-Path $repoRoot 'target\production'
    $buildArgs = @('build', '--release', '--locked', '--no-default-features', '--target-dir', $productionTarget, '-p', 'luciddesk', '-p', 'desktop-hook')
    if ($Offline) { $buildArgs += '--offline' }
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    & (Join-Path $PSScriptRoot 'verify-app-icon.ps1') -Executable (Join-Path $productionTarget 'release/luciddesk.exe')
    $metadata = & cargo metadata --no-deps --format-version 1 --offline --locked | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Could not read package metadata.' }
    $version = ($metadata.packages | Where-Object name -eq 'luciddesk').version
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
    $name = "LucidDesk-$version-$revisionLabel-windows-x64-$stamp"
    if ($Portable) { $name = "LucidDesk-$version-windows-x64-portable" }
    if ($RenderDiagnostics) { $name += '-render-test' }
    $outRoot = Join-Path $repoRoot $(if ($Portable) { "target\portable\$stamp" } else { 'target\packages' })
    $stage = Join-Path $outRoot $name
    New-Item -ItemType Directory -Path $stage | Out-Null
    foreach ($file in @('luciddesk.exe', 'luciddesk_desktop.dll')) {
        Copy-Item -LiteralPath (Join-Path $productionTarget "release\$file") -Destination $stage
    }
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Refresh-App-Icon.ps1') -Destination $stage
    $readme = if ($Portable) { 'docs\portable.md' } else { 'docs\package.md' }
    Copy-Item -LiteralPath (Join-Path $repoRoot $readme) -Destination (Join-Path $stage 'README.md')
    if ($Portable) {
        'LucidDesk portable mode: store configuration in ./data.' | Set-Content -LiteralPath (Join-Path $stage 'portable') -Encoding ASCII
    }
    if ($RenderDiagnostics) {
        Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'render-diagnostics') -File | ForEach-Object {
            Copy-Item -LiteralPath $_.FullName -Destination $stage
        }
    }
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\usage.md') -Destination (Join-Path $stage 'usage.md')
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\brand.md') -Destination (Join-Path $stage 'brand.md')
    foreach ($guide in @('installer.md', 'portable.md')) {
        Copy-Item -LiteralPath (Join-Path $repoRoot "docs\$guide") -Destination (Join-Path $stage $guide)
    }
    Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE') -Destination (Join-Path $stage 'LICENSE')
    foreach ($policy in @('PRIVACY.md', 'PRIVACY.en.md')) {
        Copy-Item -LiteralPath (Join-Path $repoRoot $policy) -Destination $stage
    }
    foreach ($changelog in @('CHANGELOG.md', 'CHANGELOG.en.md')) {
        Copy-Item -LiteralPath (Join-Path $repoRoot $changelog) -Destination (Join-Path $stage $changelog)
    }
    $files = @('luciddesk.exe', 'luciddesk_desktop.dll') | ForEach-Object {
        $fileHash = Get-FileHash -LiteralPath (Join-Path $stage $_) -Algorithm SHA256
        [ordered]@{ file = $_; sha256 = $fileHash.Hash.ToLowerInvariant() }
    }
    [ordered]@{
        version = $version; channel = 'release'; revision = $revision; uncommittedChanges = $dirty
        portable = [bool]$Portable
        renderingDiagnostics = [bool]$RenderDiagnostics
        builtAt = (Get-Date).ToUniversalTime().ToString('o'); architecture = 'windows-x64'
        buildEnvironment = [ordered]@{
            rustc = ($hostInfo -join "`n"); cargo = $cargoVersion
            runnerImage = $env:ImageOS; runnerVersion = $env:ImageVersion
            windows = ($env:LUCIDDESK_WINDOWS_BUILD_ENVIRONMENT | ConvertFrom-Json)
        }
        files = $files
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $stage 'build.json') -Encoding UTF8
    $archive = Join-Path $outRoot "$name.zip"
    Compress-Archive -LiteralPath $stage -DestinationPath $archive -CompressionLevel Optimal
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $name.zip" | Set-Content -LiteralPath "$archive.sha256" -Encoding ASCII
    Write-Output $archive
    if ($Installer) {
        if (-not $InnoCompiler) {
            $InnoCompiler = Join-Path $repoRoot 'target\tooling\inno-7.1.0\ISCC.exe'
            if (-not (Test-Path -LiteralPath $InnoCompiler)) {
                throw 'Install the compiler with ./tools/ensure-inno.ps1, or pass -InnoCompiler <ISCC.exe>.'
            }
        }
        $setupRoot = Join-Path $repoRoot "target\installers\$version-$revisionLabel-$stamp"
        & $InnoCompiler /Q "/DAppVersion=$version" "/DSourcePath=$stage" "/DOutputPath=$setupRoot" (Join-Path $repoRoot 'installer\LucidDesk.iss')
        if ($LASTEXITCODE -ne 0) { throw 'Inno Setup compilation failed.' }
        $setup = Join-Path $setupRoot "LucidDesk-$version-windows-x64-setup.exe"
        $setupHash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
        "$setupHash  $(Split-Path -Leaf $setup)" | Set-Content -LiteralPath "$setup.sha256" -Encoding ASCII
        Write-Output $setup
    }
} finally {
    $env:LUCIDDESK_BUILD_REVISION = $previousRevision
    Pop-Location
}
