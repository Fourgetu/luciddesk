[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$SourceRoot,
    [Parameter(Mandatory)][string]$Destination
)
$ErrorActionPreference = 'Stop'
$SourceRoot = (Resolve-Path -LiteralPath $SourceRoot).Path
$Destination = [IO.Path]::GetFullPath($Destination)
$payload = [ordered]@{
    'docs/cli.md' = 'cli.md'
    'crates/luciddesk-api/protocol.schema.json' = 'protocol.schema.json'
    'skills/luciddesk-control/SKILL.md' = 'skills/luciddesk-control/SKILL.md'
}
foreach ($entry in $payload.GetEnumerator()) {
    $source = Join-Path $SourceRoot $entry.Key
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing Agent payload: $source" }
    $target = Join-Path $Destination $entry.Value
    New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
    Copy-Item -LiteralPath $source -Destination $target
    [ordered]@{ file = $entry.Value; sha256 = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant() }
}
