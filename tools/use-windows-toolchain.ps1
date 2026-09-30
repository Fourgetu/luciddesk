[CmdletBinding()]
param([switch]$GitHubActions)
$ErrorActionPreference = 'Stop'
$settings = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'windows-toolchain.json') -Raw | ConvertFrom-Json
if ($settings.minimumMsvc -notmatch '^\d+\.\d+\.\d+$' -or $settings.minimumWindowsSdk -notmatch '^\d+\.\d+\.\d+\.\d+$') {
    throw 'Invalid minimum Windows toolchain versions.'
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
# Compare numeric toolset versions across installations, not discovery/PATH order.
$toolsets = foreach ($candidate in ($candidates | Where-Object { $_ } | Select-Object -Unique)) {
    if (-not (Test-Path -LiteralPath (Join-Path $candidate 'VC\Auxiliary\Build\vcvarsall.bat'))) { continue }
    Get-ChildItem -LiteralPath (Join-Path $candidate 'VC\Tools\MSVC') -Directory -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Name -match '^\d+\.\d+\.\d+$' -and
            [version]$_.Name -ge [version]$settings.minimumMsvc -and
            (Test-Path -LiteralPath (Join-Path $_.FullName 'bin\Hostx64\x64\cl.exe')) -and
            (Test-Path -LiteralPath (Join-Path $_.FullName 'bin\Hostx64\x64\link.exe')) -and
            (Test-Path -LiteralPath (Join-Path $_.FullName 'bin\Hostx64\x64\lib.exe'))
        } | ForEach-Object { [pscustomobject]@{ installation = $candidate; version = [version]$_.Name } }
}
$selected = $toolsets | Sort-Object version -Descending | Select-Object -First 1
if (-not $selected) { throw "Install an x64 MSVC toolset >= $($settings.minimumMsvc) before building." }
$installation = $selected.installation

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
# Omitting the SDK version lets vcvarsall select the latest installed SDK.
$start.Arguments = '/d /s /c ""' + $vcvars + '" x64 -vcvars_ver=' + $selected.version + ' >nul && set"'
$process = [Diagnostics.Process]::Start($start)
$output = $process.StandardOutput.ReadToEnd()
$errors = $process.StandardError.ReadToEnd()
$process.WaitForExit()
if ($process.ExitCode -ne 0) { throw "Windows build environment failed: $errors" }
$buildVariables = @('PATH', 'INCLUDE', 'LIB', 'LIBPATH', 'VSINSTALLDIR', 'VCINSTALLDIR',
    'VCToolsInstallDir', 'VCToolsVersion', 'WindowsSdkDir', 'WindowsSDKVersion',
    'WindowsSDKLibVersion', 'WindowsSdkBinPath', 'WindowsSdkVerBinPath', 'UniversalCRTSdkDir', 'UCRTVersion',
    'VSCMD_ARG_HOST_ARCH', 'VSCMD_ARG_TGT_ARCH')
foreach ($line in ($output -split "`r?`n")) {
    $pair = $line -split '=', 2
    if ($pair.Count -eq 2 -and $buildVariables -contains $pair[0]) {
        [Environment]::SetEnvironmentVariable($pair[0], $pair[1], 'Process')
    }
}
$msvcVersion = $env:VCToolsVersion.TrimEnd('\')
$sdkVersion = $env:WindowsSDKVersion.TrimEnd('\')
if ($env:VSCMD_ARG_HOST_ARCH -ne 'x64' -or $env:VSCMD_ARG_TGT_ARCH -ne 'x64') {
    throw 'The Windows build environment must use an x64 host and target.'
}
if ([version]$msvcVersion -ne $selected.version -or [version]$sdkVersion -lt [version]$settings.minimumWindowsSdk) {
    throw 'The selected MSVC toolset does not match discovery, or the Windows SDK is below the minimum version.'
}
$bin = Join-Path $env:VCToolsInstallDir 'bin\Hostx64\x64'
$tools = [ordered]@{}
foreach ($name in @('cl.exe', 'link.exe', 'lib.exe', 'rc.exe')) {
    $path = if ($name -eq 'rc.exe') {
        Join-Path $env:WindowsSdkDir "bin\$sdkVersion\x64\rc.exe"
    } else { Join-Path $bin $name }
    $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    $tools[$name] = [ordered]@{ version = (Get-Item -LiteralPath $path).VersionInfo.FileVersion; sha256 = $hash }
    Write-Output "$name version=$($tools[$name].version) sha256=$hash path=$path"
}
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $bin 'link.exe'
$env:CC_x86_64_pc_windows_msvc = Join-Path $bin 'cl.exe'
$env:CXX_x86_64_pc_windows_msvc = Join-Path $bin 'cl.exe'
$env:AR_x86_64_pc_windows_msvc = Join-Path $bin 'lib.exe'
$environment = [ordered]@{ msvc = $msvcVersion; windowsSdk = $sdkVersion; tools = $tools }
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
Write-Output "Selected MSVC $msvcVersion, linker $($tools['link.exe'].version), SDK $sdkVersion; tool hashes recorded."
