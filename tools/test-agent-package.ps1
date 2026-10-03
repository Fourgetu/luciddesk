[CmdletBinding()]
param([Parameter(Mandatory)][string]$Directory)
$ErrorActionPreference = 'Stop'
$Directory = (Resolve-Path -LiteralPath $Directory).Path
$build = Get-Content -LiteralPath (Join-Path $Directory 'build.json') -Raw | ConvertFrom-Json
$required = @('luciddesk-cli.exe', 'cli.md', 'protocol.schema.json', 'skills/luciddesk-control/SKILL.md')
foreach ($file in $required) {
    $entry = @($build.files | Where-Object file -eq $file)
    if ($entry.Count -ne 1) { throw "Missing or duplicate manifest entry: $file" }
    $hash = (Get-FileHash -LiteralPath (Join-Path $Directory $file) -Algorithm SHA256).Hash
    if ($hash -ne $entry[0].sha256) { throw "Payload hash mismatch: $file" }
}
$cli = Join-Path $Directory 'luciddesk-cli.exe'
$version = & $cli --version
if ($LASTEXITCODE -ne 0 -or $version -ne "luciddesk-cli $($build.version) protocol 1") { throw 'Packaged CLI version mismatch' }
$embedded = (& $cli skill show --json) | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or -not $embedded.ok) { throw 'Offline skill discovery failed' }
$skill = [IO.File]::ReadAllText((Join-Path $Directory 'skills/luciddesk-control/SKILL.md'))
if ($embedded.data.content -cne $skill) { throw 'Embedded and distributed Skills differ' }
$schema = (& $cli schema --json) -join "`n"
if ($LASTEXITCODE -ne 0) { throw 'Offline schema discovery failed' }
$packagedSchema = [IO.File]::ReadAllText((Join-Path $Directory 'protocol.schema.json'))
if (-not [System.Text.Json.Nodes.JsonNode]::DeepEquals([System.Text.Json.Nodes.JsonNode]::Parse($schema),[System.Text.Json.Nodes.JsonNode]::Parse($packagedSchema))) { throw 'Embedded and distributed schema differ' }
[ordered]@{ result='passed'; directory=$Directory; version=$build.version; files=$required } | ConvertTo-Json
