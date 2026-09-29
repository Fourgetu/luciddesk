[CmdletBinding()]
param([string]$Executable = (Join-Path $PSScriptRoot 'luciddesk.exe'))
$ErrorActionPreference = 'Stop'
$resolved = (Resolve-Path -LiteralPath $Executable).ProviderPath
if (-not (Test-Path -LiteralPath $resolved -PathType Leaf) -or [IO.Path]::GetExtension($resolved) -ine '.exe') {
    throw 'Provide the path of the LucidDesk executable to refresh.'
}
if (-not ('LucidDesk.IconRefresh' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
namespace LucidDesk {
    public static class IconRefresh {
        [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
        public static extern void SHChangeNotify(uint change, uint flags, string item, IntPtr other);
    }
}
'@
}
# Notify only this file and folder. Never delete global caches or restart Explorer.
[LucidDesk.IconRefresh]::SHChangeNotify(0x00002000, 0x00001005, $resolved, [IntPtr]::Zero)
[LucidDesk.IconRefresh]::SHChangeNotify(0x00001000, 0x00001005, [IO.Path]::GetDirectoryName($resolved), [IntPtr]::Zero)
Write-Output "Requested Shell icon refresh: $resolved"
