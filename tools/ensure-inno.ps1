[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
# Official, pinned compiler installer. CI and local packaging use the same compiler.
$compilerRoot = Join-Path (Split-Path -Parent $PSScriptRoot) 'target\tooling\inno-7.1.0'
$compiler = Join-Path $compilerRoot 'ISCC.exe'
if (Test-Path -LiteralPath $compiler) { Write-Output $compiler; return }
New-Item -ItemType Directory -Force -Path $compilerRoot | Out-Null
$download = Join-Path $compilerRoot 'innosetup-7.1.0-x64.exe'
Invoke-WebRequest -Uri 'https://github.com/jrsoftware/issrc/releases/download/is-7_1_0/innosetup-7.1.0-x64.exe' -OutFile $download
$expected = '0362a383ed217d4c4239b5933866dd96d3eb2102737da92f80f6057a4b40df2f'
if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
    throw 'Inno Setup compiler download failed SHA256 verification.'
}
$process = Start-Process -FilePath $download -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/NOICONS', "/DIR=`"$compilerRoot`"") -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $compiler)) {
    throw "Inno Setup compiler installation failed: $($process.ExitCode)"
}
Write-Output $compiler
