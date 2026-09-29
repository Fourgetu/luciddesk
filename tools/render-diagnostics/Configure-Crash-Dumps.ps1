param([ValidateSet('Enable', 'Restore')][string]$Mode = 'Enable')
$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Right-click the CMD file and choose Run as administrator.'
}
$backup = Join-Path $PSScriptRoot 'crash-dump-settings-backup.json'
$root = 'SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps'
$apps = @('explorer.exe', 'luciddesk.exe')
$names = @('DumpFolder', 'DumpType', 'DumpCount')
$hive = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
    [Microsoft.Win32.RegistryHive]::LocalMachine, [Microsoft.Win32.RegistryView]::Registry64)
try {
    if ($Mode -eq 'Enable') {
        if (Test-Path -LiteralPath $backup) {
            throw 'A settings backup already exists. Run Restore-Crash-Dumps.cmd before enabling again.'
        }
        $entries = @()
        foreach ($app in $apps) {
            $key = $hive.OpenSubKey("$root\$app")
            try {
                $values = @()
                foreach ($name in $names) {
                    $exists = $null -ne $key -and $key.GetValueNames() -contains $name
                    $value = $null
                    $kind = $null
                    if ($exists) {
                        $value = $key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                        $kind = $key.GetValueKind($name).ToString()
                    }
                    $values += @{ Name = $name; Exists = $exists; Value = $value; Kind = $kind }
                }
                $entries += @{ App = $app; KeyExisted = ($null -ne $key); Values = $values }
            } finally { if ($null -ne $key) { $key.Dispose() } }
        }
        # Save recovery data before the first registry change.
        @{ Entries = $entries } | ConvertTo-Json -Depth 6 |
            Set-Content -LiteralPath $backup -Encoding UTF8
        foreach ($app in $apps) {
            $key = $hive.CreateSubKey("$root\$app")
            try {
                $key.SetValue('DumpFolder', '%LOCALAPPDATA%\CrashDumps', [Microsoft.Win32.RegistryValueKind]::ExpandString)
                $key.SetValue('DumpType', 1, [Microsoft.Win32.RegistryValueKind]::DWord)
                $key.SetValue('DumpCount', 5, [Microsoft.Win32.RegistryValueKind]::DWord)
            } finally { $key.Dispose() }
        }
        Write-Host 'Enabled minidumps for explorer.exe and luciddesk.exe only.'
        Write-Host 'Future crash dumps: %LOCALAPPDATA%\CrashDumps'
        Write-Host 'No application was started, stopped, or restarted.'
        Write-Host 'Keep this folder and backup JSON until you run Restore-Crash-Dumps.cmd.'
    } else {
        if (-not (Test-Path -LiteralPath $backup -PathType Leaf)) { throw 'Settings backup not found.' }
        $state = Get-Content -LiteralPath $backup -Raw | ConvertFrom-Json
        foreach ($entry in $state.Entries) {
            if ($entry.App -notin $apps) { throw 'Invalid application in settings backup.' }
            foreach ($value in $entry.Values) {
                if ($value.Name -notin $names) { throw 'Invalid value name in settings backup.' }
            }
        }
        foreach ($entry in $state.Entries) {
            $key = $hive.CreateSubKey("$root\$($entry.App)")
            $removeEmptyKey = $false
            try {
                foreach ($value in $entry.Values) {
                    if ($value.Exists) {
                        $kind = [Microsoft.Win32.RegistryValueKind][Enum]::Parse([Microsoft.Win32.RegistryValueKind], $value.Kind)
                        $data = $value.Value
                        if ($kind -eq [Microsoft.Win32.RegistryValueKind]::DWord) { $data = [int]$data }
                        if ($kind -eq [Microsoft.Win32.RegistryValueKind]::QWord) { $data = [long]$data }
                        if ($kind -eq [Microsoft.Win32.RegistryValueKind]::Binary) { $data = [byte[]]$data }
                        if ($kind -eq [Microsoft.Win32.RegistryValueKind]::MultiString) { $data = [string[]]$data }
                        $key.SetValue($value.Name, $data, $kind)
                    } else { $key.DeleteValue($value.Name, $false) }
                }
                $removeEmptyKey = -not $entry.KeyExisted -and $key.ValueCount -eq 0 -and $key.SubKeyCount -eq 0
            } finally { $key.Dispose() }
            if ($removeEmptyKey) { $hive.DeleteSubKey("$root\$($entry.App)", $false) }
        }
        Move-Item -LiteralPath $backup -Destination ($backup + '.restored-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
        Write-Host 'Previous per-application dump settings restored. Existing dump files were kept.'
    }
} finally { $hive.Dispose() }
