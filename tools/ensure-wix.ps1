[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$root = Join-Path (Split-Path -Parent $PSScriptRoot) 'target/tooling/wix-5.0.2'
$wix = Join-Path $root 'wix.exe'
if (-not (Test-Path -LiteralPath $wix)) {
    & dotnet tool install wix --version 5.0.2 --tool-path $root --source https://api.nuget.org/v3/index.json
    if ($LASTEXITCODE -ne 0) { throw 'Project-local WiX download failed. A .NET SDK is required on the build machine.' }
}
$extension = Join-Path $root '.wix/extensions/WixToolset.UI.wixext/5.0.2/wixext5/WixToolset.UI.wixext.dll'
if (-not (Test-Path -LiteralPath $extension)) {
    Push-Location $root
    try {
        & $wix extension add WixToolset.UI.wixext/5.0.2
        if ($LASTEXITCODE -ne 0) { throw 'WiX UI extension download failed.' }
    } finally { Pop-Location }
}
Write-Output $wix
