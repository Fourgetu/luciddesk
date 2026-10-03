[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$SourcePath,
    [Parameter(Mandatory)][string]$OutputPath,
    [Parameter(Mandatory)][string]$Version,
    [string]$ProductName = 'LucidDesk',
    [string]$UpgradeCode = '670DDAC1-DA49-4DA7-813E-FF96254FCFEA',
    [switch]$TestFixture
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$wixRoot = Join-Path $repo 'target/tooling/wix-5.0.2'
$wix = Join-Path $wixRoot 'wix.exe'
$extension = Join-Path $wixRoot '.wix/extensions/WixToolset.UI.wixext/5.0.2/wixext5/WixToolset.UI.wixext.dll'
if (-not (Test-Path -LiteralPath $wix) -or -not (Test-Path -LiteralPath $extension)) {
    throw 'Run ./tools/ensure-wix.ps1 first. WiX is kept inside the project.'
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'MSI requires a three-part numeric version.' }
$numericVersion = [version]$Version
if ($numericVersion.Major -gt 255 -or $numericVersion.Minor -gt 255 -or $numericVersion.Build -gt 65535) {
    throw 'MSI version limits are 255.255.65535.'
}
if ($ProductName -notmatch '^LucidDesk(?: Test [0-9a-f-]{36})?$') { throw 'Invalid product name.' }
if ($TestFixture -and ($ProductName -eq 'LucidDesk' -or $UpgradeCode -eq '670DDAC1-DA49-4DA7-813E-FF96254FCFEA')) {
    throw 'Fixtures require an isolated name and upgrade code.'
}
$null = [guid]::Parse($UpgradeCode)
$SourcePath = (Resolve-Path -LiteralPath $SourcePath).Path
foreach ($required in @('luciddesk.exe', 'luciddesk_desktop.dll', 'build.json', 'LICENSE')) {
    if (-not (Test-Path -LiteralPath (Join-Path $SourcePath $required) -PathType Leaf)) { throw "Missing payload: $required" }
}
$OutputPath = [IO.Path]::GetFullPath($OutputPath)
New-Item -ItemType Directory -Force -Path $OutputPath | Out-Null
$work = Join-Path $OutputPath 'build'
New-Item -ItemType Directory -Force -Path $work | Out-Null
& (Join-Path $PSScriptRoot 'use-windows-toolchain.ps1') | Out-Host
$cache = Join-Path $repo 'target/msi-cache'
$source = Join-Path $repo 'installer/msi-actions.cpp'
# A fixture DLL bypasses desktop checks; never share it with production builds.
# Include this script so compiler flag changes invalidate previously built actions.
$identity = @(
    (Get-FileHash -LiteralPath $source).Hash,
    (Get-FileHash -LiteralPath $PSCommandPath).Hash,
    $env:LUCIDDESK_WINDOWS_BUILD_ENVIRONMENT,
    "fixture=$([bool]$TestFixture)"
) -join "`n"
$sha = [Security.Cryptography.SHA256]::Create()
try { $key = [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($identity))).Replace('-', '').ToLowerInvariant() }
finally { $sha.Dispose() }
$actionWork = Join-Path $cache "actions/$key"
New-Item -ItemType Directory -Force -Path $actionWork | Out-Null
$actions = Join-Path $actionWork 'msi-actions.dll'
$stamp = "$actions.sha256"
$cached = (Test-Path -LiteralPath $actions) -and (Test-Path -LiteralPath $stamp) -and
    ((Get-Content -LiteralPath $stamp -Raw).Trim() -eq (Get-FileHash -LiteralPath $actions).Hash)
$compile = @('/nologo', '/LD', '/MT', '/std:c++17', '/EHsc', '/utf-8', '/O2', '/DUNICODE', '/D_UNICODE',
    $source, "/Fo$actionWork\msi-actions.obj", '/link', "/OUT:$actions", "/IMPLIB:$actionWork\msi-actions.lib", 'msi.lib', 'user32.lib', 'advapi32.lib', 'shell32.lib')
if ($TestFixture) { $compile = @('/DLUCIDDESK_INSTALLER_FIXTURE') + $compile }
if ($cached) {
    Write-Host "Reusing MSI actions (fixture=$([bool]$TestFixture))."
} else {
    & $env:CC_x86_64_pc_windows_msvc @compile | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'Native MSI action compilation failed.' }
    (Get-FileHash -LiteralPath $actions).Hash | Set-Content -LiteralPath $stamp -Encoding ASCII
}
$escape = { param($text) [Security.SecurityElement]::Escape($text) }
$components = foreach ($file in (Get-ChildItem -LiteralPath $SourcePath -File | Sort-Object Name)) {
    if ($file.Name -in @('portable', 'portable.marker', 'msix', 'README.md', 'installed')) { continue }
    $id = if ($file.Name -eq 'luciddesk.exe') { 'AppExe' } else { 'File_' + ($file.Name -replace '[^A-Za-z0-9_]', '_') }
    '<Component Guid="*"><File Id="' + $id + '" Source="' + (& $escape $file.FullName) + '" KeyPath="yes" /></Component>'
}
$components += '<Component Guid="*"><File Source="' + (& $escape (Join-Path $repo 'installer/installed')) + '" KeyPath="yes" /></Component>'
$components += '<Component Guid="*"><File Name="README.md" Source="' + (& $escape (Join-Path $repo 'docs/installer.md')) + '" KeyPath="yes" /></Component>'
$payload = Join-Path $work 'payload.wxs'
('<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs"><Fragment><ComponentGroup Id="Payload" Directory="INSTALLFOLDER">' + ($components -join "`n") + '</ComponentGroup></Fragment></Wix>') | Set-Content -LiteralPath $payload -Encoding UTF8
$output = Join-Path $OutputPath "LucidDesk-$Version-windows-x64.msi"
& $wix build (Join-Path $repo 'installer/LucidDesk.wxs') $payload -arch x64 -culture zh-CN -ext $extension `
    -cabcache (Join-Path $cache 'cabinets') -pdbtype none `
    -d "AppVersion=$Version" -d "ProductName=$ProductName" -d "UpgradeCode=$UpgradeCode" -d "RepoRoot=$repo" -d "ActionsDll=$actions" -o $output | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'MSI compilation failed.' }
Write-Output $output
