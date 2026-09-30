[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
# Official, pinned compiler installer. CI and local packaging use the same compiler.
$compilerRoot = Join-Path (Split-Path -Parent $PSScriptRoot) 'target\tooling\inno-6.7.3'
$compiler = Join-Path $compilerRoot 'ISCC.exe'
if (Test-Path -LiteralPath $compiler) { Write-Output $compiler; return }
New-Item -ItemType Directory -Force -Path $compilerRoot | Out-Null
$download = Join-Path $compilerRoot 'innosetup-6.7.3.exe'
Invoke-WebRequest -Uri 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe' -OutFile $download
$expected = '9c73c3bae7ed48d44112a0f48e66742c00090bdb5bef71d9d3c056c66e97b732'
if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
    throw 'Inno Setup compiler download failed SHA256 verification.'
}
$process = Start-Process -FilePath $download -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/NOICONS', "/DIR=`"$compilerRoot`"") -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $compiler)) {
    throw "Inno Setup compiler installation failed: $($process.ExitCode)"
}
Write-Output $compiler
