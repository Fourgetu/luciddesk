[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$SourcePath,
    [switch]$AllUsers,
    [string]$InnoCompiler = (Join-Path $PSScriptRoot '../target/tooling/inno-6.7.3/ISCC.exe')
)
$ErrorActionPreference = 'Stop'
$SourcePath = (Resolve-Path -LiteralPath $SourcePath).Path
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$testRoot = Join-Path $repo "target\installer-test\$([guid]::NewGuid())"
$installed = Join-Path $testRoot 'app'
if (-not ([IO.Path]::GetFullPath($installed).StartsWith($repo + '\target\', [StringComparison]::OrdinalIgnoreCase))) {
    throw 'Installer test path is outside the workspace target directory.'
}
New-Item -ItemType Directory -Path $testRoot | Out-Null
$fixtureId = [guid]::NewGuid().ToString().ToUpperInvariant()
$fixtureName = "LucidDesk Installer Test $fixtureId"
if ($AllUsers) {
    $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'The all-users Program Files test must run in an elevated PowerShell.'
    }
    $programFiles = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)
    $installed = Join-Path $programFiles $fixtureName
    if (-not ([IO.Path]::GetFullPath($installed).StartsWith($programFiles + '\', [StringComparison]::OrdinalIgnoreCase))) {
        throw 'All-users installer test path is outside Program Files.'
    }
}
$registryRoot = if ($AllUsers) { 'HKLM:' } else { 'HKCU:' }
$uninstallKey = "$registryRoot\Software\Microsoft\Windows\CurrentVersion\Uninstall\{$fixtureId}_is1"
$mutexName = "Local\LucidDesk.Setup.Test.$fixtureId"
$windowClass = "LucidDesk.Installer.Test.$fixtureId"
$windowName = "LucidDesk Installer Test Window $fixtureId"
$userDataName = "LucidDesk.Installer.Test.$fixtureId"
$localDataBase = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$userData = Join-Path $localDataBase $userDataName
$unrelatedData = Join-Path $testRoot 'unrelated-data'
$uninstaller = Join-Path $installed 'unins000.exe'
$mutex = $null
$probeProcess = $null
$programsBase = [Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)
$desktopBase = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
$shortcutFixture = Join-Path $programsBase $fixtureName
$desktopFixtureLink = Join-Path $desktopBase "$fixtureName renamed.lnk"
function Compile-Fixture([string]$Version) {
    $output = Join-Path $testRoot $Version
    & $InnoCompiler /Q "/DAppVersion=$Version" "/DSourcePath=$SourcePath" "/DOutputPath=$output" "/DProductId={{$fixtureId}" "/DProductName=$fixtureName" "/DAppMutexName=$mutexName" "/DAppWindowClass=$windowClass" "/DAppWindowName=$windowName" "/DUserDataFolderName=$userDataName" /DShutdownTimeout=1000 (Join-Path $repo 'installer/LucidDesk.iss')
    if ($LASTEXITCODE -ne 0) { throw "Fixture compilation failed for $Version" }
    Join-Path $output "LucidDesk-$Version-windows-x64-setup.exe"
}
function Run-Setup([string]$Path, [string[]]$ExtraArguments = @(), [switch]$UsePreviousDirectory, [switch]$CreateIcons) {
    $log = Join-Path $testRoot "$([IO.Path]::GetFileNameWithoutExtension($Path))-$([guid]::NewGuid()).log"
    $mode = if ($AllUsers) { '/ALLUSERS' } else { '/CURRENTUSER' }
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', $mode, "/LOG=`"$log`"")
    if (-not $CreateIcons) { $arguments += '/NOICONS' }
    if (-not $UsePreviousDirectory) { $arguments += "/DIR=`"$installed`"" }
    $arguments += $ExtraArguments
    $process = Start-Process -FilePath $Path -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru
    $process.ExitCode
}
function Run-Uninstall([string[]]$ExtraArguments = @()) {
    $log = Join-Path $testRoot "uninstall-$([guid]::NewGuid()).log"
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$log`"") + $ExtraArguments
    (Start-Process -FilePath $uninstaller -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru).ExitCode
}
try {
    foreach ($directory in @($userData)) {
        New-Item -ItemType Directory -Path $directory | Out-Null
        'retain configuration' | Set-Content -LiteralPath (Join-Path $directory 'config.toml') -Encoding ASCII
        New-Item -ItemType Directory -Path (Join-Path $directory 'backups') | Out-Null
        'retain backup' | Set-Content -LiteralPath (Join-Path $directory 'backups/keep.txt') -Encoding ASCII
    }
    New-Item -ItemType Directory -Path $unrelatedData | Out-Null
    'retain unrelated file' | Set-Content -LiteralPath (Join-Path $unrelatedData 'keep.txt') -Encoding ASCII
    New-Item -ItemType Junction -Path (Join-Path $userData 'external-link') -Target $unrelatedData | Out-Null
    $currentVersion = (Get-Content -LiteralPath (Join-Path $SourcePath 'build.json') -Raw | ConvertFrom-Json).version
    $older = Compile-Fixture '0.0.1'
    $current = Compile-Fixture $currentVersion
    # A versioned inert EXE makes upgrade/downgrade checks independent of the app's current version.
    $stub = Join-Path $testRoot 'version.cs'
    '[assembly: System.Reflection.AssemblyVersion("0.0.1.0")] class Stub { static void Main() {} }' | Set-Content -LiteralPath $stub -Encoding ASCII
    $csc = Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
    & $csc /nologo /target:winexe "/out:$testRoot\old.exe" $stub
    if ($LASTEXITCODE -ne 0) { throw 'Could not compile versioned fixture executable.' }
    if ((Run-Setup $older) -ne 0) { throw 'Initial installation failed.' }
    if ((Get-ItemProperty -LiteralPath $uninstallKey).DisplayVersion -ne '0.0.1') { throw 'Installation was registered in the wrong scope.' }
    Copy-Item -LiteralPath (Join-Path $testRoot 'old.exe') -Destination (Join-Path $installed 'luciddesk.exe') -Force
    $data = Join-Path $installed 'data'
    New-Item -ItemType Directory -Path $data | Out-Null
    'retain configuration' | Set-Content -LiteralPath (Join-Path $data 'keep.txt') -Encoding ASCII
    if ((Run-Setup $current -UsePreviousDirectory) -ne 0) { throw 'In-place upgrade failed.' }
    if ((Get-ItemProperty -LiteralPath $uninstallKey).DisplayVersion -ne $currentVersion) { throw 'Upgrade registry version is incorrect.' }
    if ((Get-FileHash -LiteralPath (Join-Path $installed 'luciddesk.exe')).Hash -ne (Get-FileHash -LiteralPath (Join-Path $SourcePath 'luciddesk.exe')).Hash) { throw 'Upgrade did not replace app.' }
    if (-not (Test-Path -LiteralPath (Join-Path $installed 'installed'))) { throw 'Installation marker missing.' }
    if ((Run-Setup $older) -eq 0) { throw 'Downgrade was incorrectly accepted.' }
    $mutex = [Threading.Mutex]::new($false, $mutexName)
    if ((Run-Setup $current) -eq 0) { throw 'Running-app mutex did not block update.' }
    $process = Start-Process -FilePath $uninstaller -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -eq 0) { throw 'Uninstall did not block while the app was running.' }
    if ((Run-Uninstall @('/DELETEUSERDATA')) -eq 0) { throw 'Deleting settings bypassed the running-app guard.' }
    if (-not (Test-Path -LiteralPath (Join-Path $userData 'config.toml'))) { throw 'Blocked uninstall removed settings.' }
    $mutex.Dispose(); $mutex = $null

    $probe = Join-Path $testRoot 'close-probe.exe'
    & $csc /nologo /target:winexe /platform:x64 "/out:$probe" (Join-Path $repo 'tools/installer-close-probe.cs')
    if ($LASTEXITCODE -ne 0) { throw 'Could not compile the shutdown probe.' }
    foreach ($mode in @('ignore', 'exit')) {
        Copy-Item -LiteralPath (Join-Path $testRoot 'old.exe') -Destination (Join-Path $installed 'luciddesk.exe') -Force
        $before = (Get-FileHash -LiteralPath (Join-Path $installed 'luciddesk.exe')).Hash
        $ready = Join-Path $testRoot "$mode-ready.txt"
        $finished = Join-Path $testRoot "$mode-finished.txt"
        $arguments = @($mutexName, $windowClass, $windowName, $ready, $finished, (Join-Path $installed 'luciddesk.exe'), $before, $mode) |
            ForEach-Object { '"' + $_ + '"' }
        $probeProcess = Start-Process -FilePath $probe -ArgumentList $arguments -WindowStyle Hidden -PassThru
        $deadline = [DateTime]::UtcNow.AddSeconds(5)
        while (-not (Test-Path -LiteralPath $ready) -and -not $probeProcess.HasExited -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 30 }
        if (-not (Test-Path -LiteralPath $ready)) { throw 'Shutdown probe did not become ready.' }
        if ($mode -eq 'ignore') {
            if ((Run-Setup $current @('/NOCLOSEAPPLICATIONS')) -eq 0) { throw 'No-close option was ignored.' }
            if (Test-Path -LiteralPath "$finished.requested") { throw 'No-close option still sent a close request.' }
            if ((Run-Setup $current) -eq 0) { throw 'Installer continued while a process ignored normal exit.' }
            if (-not (Test-Path -LiteralPath "$finished.requested")) { throw 'Installer did not request normal exit.' }
            if ($probeProcess.HasExited) { throw 'Installer forcibly terminated the process.' }
            if ((Get-FileHash -LiteralPath (Join-Path $installed 'luciddesk.exe')).Hash -ne $before) { throw 'Failed shutdown still replaced files.' }
            # End only this test-owned, deliberately unresponsive process.
            Stop-Process -Id $probeProcess.Id -Force
            $probeProcess.WaitForExit(); $probeProcess = $null
        } else {
            if ((Run-Setup $current) -ne 0) { throw 'Automatic normal shutdown failed.' }
            if (-not $probeProcess.WaitForExit(5000)) { throw 'Shutdown probe is still running.' }
            if ((Get-Content -LiteralPath $finished -Raw) -ne 'normal exit completed') { throw 'Installer did not wait for process teardown.' }
            if ((Get-FileHash -LiteralPath (Join-Path $installed 'luciddesk.exe')).Hash -eq $before) { throw 'Upgrade did not resume after normal exit.' }
            $probeProcess = $null
        }
    }
    # An image can stay mapped in Explorer after a legacy app exits. Reproduce
    # the file lock in this test process without injecting into or restarting Explorer.
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class InstallerImageLock {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern IntPtr LoadLibraryW(string path);
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern bool FreeLibrary(IntPtr module);
}
'@
    Copy-Item -LiteralPath (Join-Path $testRoot 'old.exe') -Destination (Join-Path $installed 'luciddesk.exe') -Force
    $before = (Get-FileHash -LiteralPath (Join-Path $installed 'luciddesk.exe')).Hash
    $mapped = [InstallerImageLock]::LoadLibraryW((Join-Path $installed 'luciddesk_desktop.dll'))
    if ($mapped -eq [IntPtr]::Zero) { throw 'Could not map the DLL image for the legacy-lock test.' }
    try {
        if ((Run-Setup $current) -eq 0) { throw 'A still-mapped DLL did not block replacement.' }
        if ((Get-FileHash -LiteralPath (Join-Path $installed 'luciddesk.exe')).Hash -ne $before) {
            throw 'The installer replaced application files before checking the DLL lock.'
        }
        if ((Run-Uninstall) -eq 0) { throw 'A still-mapped DLL did not block uninstall.' }
        if ((Get-FileHash -LiteralPath (Join-Path $installed 'luciddesk.exe')).Hash -ne $before) { throw 'Blocked uninstall removed application files.' }
    } finally { $null = [InstallerImageLock]::FreeLibrary($mapped) }
    if ((Run-Setup $current) -ne 0) { throw 'Installation did not resume after the DLL was released.' }
    # Simulate an Explorer module preflight that reports a retained legacy image.
    # The candidate EXE is extracted by Setup before any installed payload is changed.
    $busySource = Join-Path $testRoot 'busy-source'
    New-Item -ItemType Directory -Path $busySource | Out-Null
    Get-ChildItem -LiteralPath $SourcePath -File | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $busySource
    }
    $busyStub = Join-Path $testRoot 'component-busy.cs'
    'class Stub { static int Main() { return 1; } }' | Set-Content -LiteralPath $busyStub -Encoding ASCII
    & $csc /nologo /target:winexe "/out:$busySource\luciddesk.exe" $busyStub
    if ($LASTEXITCODE -ne 0) { throw 'Could not compile the retained-component probe.' }
    $busyOutput = Join-Path $testRoot 'busy-installer'
    & $InnoCompiler /Q "/DAppVersion=$currentVersion" "/DSourcePath=$busySource" "/DOutputPath=$busyOutput" "/DProductId={{$fixtureId}" "/DProductName=$fixtureName" "/DAppMutexName=$mutexName" (Join-Path $repo 'installer/LucidDesk.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Could not compile retained-component installer.' }
    $before = (Get-FileHash -LiteralPath (Join-Path $installed 'luciddesk.exe')).Hash
    if ((Run-Setup (Join-Path $busyOutput "LucidDesk-$currentVersion-windows-x64-setup.exe")) -eq 0) { throw 'Retained Explorer component did not block the installer.' }
    if ((Get-FileHash -LiteralPath (Join-Path $installed 'luciddesk.exe')).Hash -ne $before) { throw 'Retained component still allowed application replacement.' }
    foreach ($mode in @('ignore', 'exit')) {
        $before = (Get-FileHash -LiteralPath (Join-Path $installed 'luciddesk.exe')).Hash
        $ready = Join-Path $testRoot "uninstall-$mode-ready.txt"
        $finished = Join-Path $testRoot "uninstall-$mode-finished.txt"
        $arguments = @($mutexName, $windowClass, $windowName, $ready, $finished, (Join-Path $installed 'luciddesk.exe'), $before, $mode) |
            ForEach-Object { '"' + $_ + '"' }
        $probeProcess = Start-Process -FilePath $probe -ArgumentList $arguments -WindowStyle Hidden -PassThru
        $deadline = [DateTime]::UtcNow.AddSeconds(5)
        while (-not (Test-Path -LiteralPath $ready) -and -not $probeProcess.HasExited -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 30 }
        if (-not (Test-Path -LiteralPath $ready)) { throw 'Uninstall shutdown probe did not become ready.' }
        if ($mode -eq 'ignore') {
            if ((Run-Uninstall @('/NOCLOSEAPPLICATIONS')) -eq 0) { throw 'Uninstall ignored the no-close option.' }
            if (Test-Path -LiteralPath "$finished.requested") { throw 'Uninstall requested exit despite the no-close option.' }
            if ((Run-Uninstall) -eq 0) { throw 'Uninstall deleted files while the app ignored exit.' }
            if (-not (Test-Path -LiteralPath "$finished.requested")) { throw 'Uninstall did not request normal exit.' }
            if ($probeProcess.HasExited) { throw 'Uninstall forcibly killed the process.' }
            if ((Get-FileHash -LiteralPath (Join-Path $installed 'luciddesk.exe')).Hash -ne $before) { throw 'Failed uninstall removed application files.' }
            Stop-Process -Id $probeProcess.Id -Force
            $probeProcess.WaitForExit(); $probeProcess = $null
        } else {
            if ((Run-Uninstall) -ne 0) { throw 'Uninstall after automatic normal shutdown failed.' }
            if (-not $probeProcess.WaitForExit(5000)) { throw 'Uninstall did not wait for process exit.' }
            if ((Get-Content -LiteralPath $finished -Raw) -ne 'normal exit completed') { throw 'Uninstall removed files before process teardown.' }
            if (Test-Path -LiteralPath (Join-Path $installed 'luciddesk.exe')) { throw 'Uninstall left the application behind.' }
            if ((Get-Content -LiteralPath (Join-Path $data 'keep.txt') -Raw).Trim() -ne 'retain configuration') { throw 'Uninstall removed user data.' }
            $probeProcess = $null
            if ((Run-Setup $current) -ne 0) { throw 'Reinstall after the automatic uninstall test failed.' }
        }
    }
    'portable' | Set-Content -LiteralPath (Join-Path $installed 'portable') -Encoding ASCII
    if ((Run-Setup $current) -eq 0) { throw 'Portable directory was incorrectly converted.' }
    Remove-Item -LiteralPath (Join-Path $installed 'portable')
    if ((Run-Setup $current -CreateIcons -ExtraArguments @('/TASKS=desktopicon')) -ne 0) { throw 'Shortcut installation failed.' }
    $nativePrograms = if ($AllUsers) { [Environment]::GetFolderPath([Environment+SpecialFolder]::CommonPrograms) } else { $programsBase }
    $nativeDesktop = if ($AllUsers) { [Environment]::GetFolderPath([Environment+SpecialFolder]::CommonDesktopDirectory) } else { $desktopBase }
    $nativeLinks = @((Join-Path $nativePrograms "$fixtureName.lnk"), (Join-Path $nativeDesktop "$fixtureName.lnk"))
    $shellProperties = New-Object -ComObject Shell.Application
    foreach ($link in $nativeLinks) {
        if (-not (Test-Path -LiteralPath $link)) { throw 'Native installer shortcut missing.' }
        $folder = $shellProperties.Namespace([IO.Path]::GetDirectoryName($link))
        $item = $folder.ParseName([IO.Path]::GetFileName($link))
        if ($item.ExtendedProperty('System.AppUserModel.ID') -cne 'Yuchen95.LucidDesk') {
            throw "Installer shortcut has an incorrect AppUserModelID: $link"
        }
    }
    New-Item -ItemType Directory -Path (Join-Path $shortcutFixture 'nested') -Force | Out-Null
    $outsideShortcuts = Join-Path $testRoot 'outside-shortcuts'
    New-Item -ItemType Directory -Path $outsideShortcuts | Out-Null
    $shortcutShell = New-Object -ComObject WScript.Shell
    $ownedLinks = @($desktopFixtureLink)
    # User-created Start Menu links are not part of the installer's [Icons] log.
    $preservedLinks = @((Join-Path $shortcutFixture 'nested/renamed.lnk'), (Join-Path $shortcutFixture 'user-copy.lnk'), (Join-Path $shortcutFixture 'other-install.lnk'), (Join-Path $shortcutFixture 'unrelated.lnk'), (Join-Path $outsideShortcuts 'outside.lnk'))
    $linkTargets = @((Join-Path $installed 'luciddesk.exe'), (Join-Path $installed 'luciddesk.exe'), (Join-Path $installed 'luciddesk.exe'), (Join-Path $SourcePath 'luciddesk.exe'), (Join-Path $env:WINDIR 'notepad.exe'), (Join-Path $installed 'luciddesk.exe'))
    $fixtureLinks = $ownedLinks + $preservedLinks
    for ($index = 0; $index -lt $fixtureLinks.Count; $index++) {
        $shortcut = $shortcutShell.CreateShortcut($fixtureLinks[$index]); $shortcut.TargetPath = $linkTargets[$index]; $shortcut.Save()
    }
    $preservedHashes = @($preservedLinks | ForEach-Object { (Get-FileHash -LiteralPath $_).Hash })
    New-Item -ItemType Junction -Path (Join-Path $shortcutFixture 'external') -Target $outsideShortcuts | Out-Null
    if ((Run-Uninstall) -ne 0) { throw 'Uninstall failed.' }
    foreach ($link in ($nativeLinks + $ownedLinks)) { if (Test-Path -LiteralPath $link) { throw "Uninstall left owned shortcut: $link" } }
    for ($index = 0; $index -lt $preservedLinks.Count; $index++) {
        if ((Get-FileHash -LiteralPath $preservedLinks[$index]).Hash -ne $preservedHashes[$index]) { throw 'Shortcut cleanup changed unrelated or junction-target links.' }
    }
    $lastUninstallLog = Get-ChildItem -LiteralPath $testRoot -Filter 'uninstall-*.log' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ((Get-Content -LiteralPath $lastUninstallLog.FullName -Raw) -notmatch 'Shell shortcut and AppsFolder refresh notifications sent') { throw 'Shell refresh was not performed.' }
    if (Test-Path -LiteralPath (Join-Path $installed 'luciddesk.exe')) { throw 'Uninstall left the application behind.' }
    if (Test-Path -LiteralPath $uninstallKey) { throw 'Uninstall left its registry entry behind.' }
    if ((Get-Content -LiteralPath (Join-Path $data 'keep.txt') -Raw).Trim() -ne 'retain configuration') { throw 'Uninstall removed user data.' }
    foreach ($directory in @($userData)) {
        if ((Get-Content -LiteralPath (Join-Path $directory 'config.toml') -Raw).Trim() -ne 'retain configuration') { throw 'Default uninstall removed default settings.' }
    }
    if ((Run-Setup $current) -ne 0) { throw 'Reinstall for settings options failed.' }
    if ((Run-Uninstall @('/DELETEUSERDATA', '/KEEPUSERDATA')) -ne 0) { throw 'Explicit retain-settings uninstall failed.' }
    if (-not (Test-Path -LiteralPath (Join-Path $userData 'config.toml'))) { throw 'Keep-settings option did not take priority.' }
    if ((Run-Setup $current) -ne 0) { throw 'Reinstall for deleting settings failed.' }
    if ((Run-Uninstall @('/DELETEUSERDATA')) -ne 0) { throw 'Delete-settings uninstall failed.' }
    if (Test-Path -LiteralPath $userData) { throw 'Selected default settings folders were not removed.' }
    if ((Get-Content -LiteralPath (Join-Path $unrelatedData 'keep.txt') -Raw).Trim() -ne 'retain unrelated file') { throw 'Settings cleanup followed a junction into unrelated files.' }
    if ((Get-Content -LiteralPath (Join-Path $data 'keep.txt') -Raw).Trim() -ne 'retain configuration') { throw 'Settings cleanup removed a custom data directory.' }
    $scope = if ($AllUsers) { 'All users in Program Files (HKLM)' } else { 'Current user (HKCU)' }
    Write-Output "$scope verified: install/upgrade and uninstall with normal auto-close, running/mapped-DLL guards, settings options; native shortcuts and owned desktop copy removed, user-created Start Menu/other-install/junction shortcuts preserved, Shell and AppsFolder notified."
} finally {
    if ($probeProcess -and -not $probeProcess.HasExited) {
        Stop-Process -Id $probeProcess.Id -Force
        $probeProcess.WaitForExit()
    }
    if ($mutex) { $mutex.Dispose() }
    if (Test-Path -LiteralPath $uninstaller) {
        $null = Start-Process -FilePath $uninstaller -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') -WindowStyle Hidden -Wait -PassThru
    }
    if (Test-Path -LiteralPath $desktopFixtureLink) { Remove-Item -LiteralPath $desktopFixtureLink }
    if (Test-Path -LiteralPath $shortcutFixture) {
        if ([IO.Path]::GetFullPath($shortcutFixture) -ne [IO.Path]::GetFullPath((Join-Path $programsBase $fixtureName))) { throw 'Unsafe shortcut fixture cleanup path.' }
        $junction = Join-Path $shortcutFixture 'external'
        if (Test-Path -LiteralPath $junction) { Remove-Item -LiteralPath $junction }
        Remove-Item -LiteralPath $shortcutFixture -Recurse -Force
    }
}
