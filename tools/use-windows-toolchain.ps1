[CmdletBinding()]
param([switch]$GitHubActions)
$ErrorActionPreference = 'Stop'
$settings = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'windows-toolchain.json') -Raw | ConvertFrom-Json
if ($settings.msvc -notmatch '^\d+\.\d+\.\d+$' -or $settings.windowsSdk -notmatch '^\d+\.\d+\.\d+\.\d+$') {
    throw 'Invalid pinned Windows toolchain versions.'
}

$candidates = @()
if ($env:VSINSTALLDIR) { $candidates += $env:VSINSTALLDIR }
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path -LiteralPath $vswhere) {
    $candidates += & $vswhere -all -products '*' -property installationPath
}
$candidates += Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*',
    'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue |
    Where-Object { $_.DisplayName -match '^Visual Studio|^Microsoft Visual Studio Build Tools' } |
    ForEach-Object { $_.InstallLocation }
$installation = $candidates | Where-Object {
    $_ -and (Test-Path -LiteralPath (Join-Path $_ "VC\Tools\MSVC\$($settings.msvc)\bin\Hostx64\x64\link.exe"))
} | Select-Object -First 1
if (-not $installation) { throw "Install the pinned MSVC $($settings.msvc) toolset before building." }

# Import only build-related environment values, without displaying the full environment.
$vcvars = Join-Path $installation 'VC\Auxiliary\Build\vcvarsall.bat'
$start = [Diagnostics.ProcessStartInfo]::new($env:ComSpec)
$start.UseShellExecute = $false
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.CreateNoWindow = $true
# Quoted PATH entries can break the vendor batch script; normalize only this child process.
$start.Environment['PATH'] = (($env:PATH -split ';') | ForEach-Object { $_.Trim().Trim('"') }) -join ';'
if ($vcvars -match '["

&|<>]') { throw 'Invalid Visual Studio installation path.' }
$start.Arguments = '/d /s /c ""' + $vcvars + '" x64 ' + $settings.windowsSdk + ' -vcvars_ver=' + $settings.msvc + ' >nul && set"'
$process = [Diagnostics.Process]::Start($start)
$output = $process.StandardOutput.ReadToEnd()
$errors = $process.StandardError.ReadToEnd()
$process.WaitForExit()
if ($process.ExitCode -ne 0) { throw "Pinned Windows build environment failed: $errors" }
$buildVariables = @('PATH', 'INCLUDE', 'LIB', 'LIBPATH', 'VSINSTALLDIR', 'VCINSTALLDIR',
    'VCToolsInstallDir', 'VCToolsVersion', 'WindowsSdkDir', 'WindowsSDKVersion',
    'WindowsSDKLibVersion', 'WindowsSdkBinPath', 'WindowsSdkVerBinPath', 'UniversalCRTSdkDir', 'UCRTVersion')
foreach ($line in ($output -split "`r?`n")) {
    $pair = $line -split '=', 2
    if ($pair.Count -eq 2 -and $buildVariables -contains $pair[0]) {
        [Environment]::SetEnvironmentVariable($pair[0], $pair[1], 'Process')
    }
}
if ($env:VCToolsVersion.TrimEnd('\') -ne $settings.msvc -or $env:WindowsSDKVersion.TrimEnd('\') -ne $settings.windowsSdk) {
    throw 'The selected MSVC or Windows SDK does not match the pinned version.'
}
$bin = Join-Path $env:VCToolsInstallDir 'bin\Hostx64\x64'
$tools = [ordered]@{}
foreach ($name in @('cl.exe', 'link.exe', 'lib.exe', 'rc.exe')) {
    $path = if ($name -eq 'rc.exe') {
        Join-Path $env:WindowsSdkDir "bin\$($settings.windowsSdk)\x64\rc.exe"
    } else { Join-Path $bin $name }
    $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -ne $settings.tools.$name) { throw "Pinned $name SHA256 mismatch: $hash" }
    $tools[$name] = [ordered]@{ version = (Get-Item -LiteralPath $path).VersionInfo.FileVersion; sha256 = $hash }
}
if ($tools['link.exe'].version -ne $settings.linker) { throw 'Pinned linker version mismatch.' }
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $bin 'link.exe'
$env:CC_x86_64_pc_windows_msvc = Join-Path $bin 'cl.exe'
$env:CXX_x86_64_pc_windows_msvc = Join-Path $bin 'cl.exe'
$env:AR_x86_64_pc_windows_msvc = Join-Path $bin 'lib.exe'
$environment = [ordered]@{ msvc = $settings.msvc; windowsSdk = $settings.windowsSdk; tools = $tools }
$env:LUCIDDESK_WINDOWS_BUILD_ENVIRONMENT = $environment | ConvertTo-Json -Depth 4 -Compress
if ($GitHubActions) {
    if (-not $env:GITHUB_ENV) { throw 'GitHub Actions environment file is unavailable.' }
    foreach ($name in ($buildVariables + @('CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER',
        'CC_x86_64_pc_windows_msvc', 'CXX_x86_64_pc_windows_msvc', 'AR_x86_64_pc_windows_msvc',
        'LUCIDDESK_WINDOWS_BUILD_ENVIRONMENT'))) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if ($null -ne $value) { "$name=$value" | Add-Content -LiteralPath $env:GITHUB_ENV -Encoding UTF8 }
    }
}
Write-Output "Selected MSVC $($settings.msvc), linker $($settings.linker), SDK $($settings.windowsSdk); tool hashes verified."
