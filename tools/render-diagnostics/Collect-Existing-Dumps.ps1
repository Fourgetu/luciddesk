$ErrorActionPreference = 'Stop'
$output = Join-Path $PSScriptRoot ('crash-dumps-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
New-Item -ItemType Directory -Path $output | Out-Null
$notes = [System.Collections.Generic.List[string]]::new()
$files = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
$since = (Get-Date).AddDays(-7)
$report = Join-Path $PSScriptRoot 'explorer-errors.txt'
$text = ''
if (Test-Path -LiteralPath $report -PathType Leaf) {
    $text = Get-Content -LiteralPath $report -Raw
    Copy-Item -LiteralPath $report -Destination $output
}

# Only inspect these applications' WER folders. Never collect unrelated dumps.
foreach ($base in @($env:ProgramData, $env:LOCALAPPDATA)) {
    foreach ($queue in @('ReportArchive', 'ReportQueue')) {
        $root = Join-Path $base "Microsoft\Windows\WER\$queue"
        try {
            if (-not (Test-Path -LiteralPath $root)) { continue }
            foreach ($folder in Get-ChildItem -LiteralPath $root -Directory) {
                if ($folder.Name -notmatch '(?i)^AppCrash_(explorer|luciddesk|lucidpane)\.exe_') { continue }
                if ($folder.LastWriteTime -lt $since) { continue }
                foreach ($file in Get-ChildItem -LiteralPath $folder.FullName -File -Recurse -ErrorAction Stop) {
                    if ($file.Extension -in @('.dmp', '.mdmp', '.hdmp', '.cab')) {
                        [void]$files.Add($file.FullName)
                    }
                }
            }
        } catch { $notes.Add("Cannot read ${root}: $($_.Exception.Message)") }
    }
    # Event 1001 lists temporary minidumps which may survive report archival.
    $tempRoot = [IO.Path]::GetFullPath((Join-Path $base 'Microsoft\Windows\WER\Temp'))
    foreach ($rawLine in ($text -split "`n")) {
        $candidate = $rawLine.Trim()
        if ($candidate.StartsWith('\\?\')) { $candidate = $candidate.Substring(4) }
        # Event XML can end a path line with </Data> and other markup.
        # Validate the whole line before passing untrusted log text to Path APIs.
        if ($candidate -notmatch '^[A-Za-z]:\\[^<>:"|?*\x00-\x1f]+\.(?:mdmp|dmp|hdmp)\z') { continue }
        try {
            $path = [IO.Path]::GetFullPath($candidate)
            if ([IO.Path]::GetDirectoryName($path) -ieq $tempRoot) {
                [void]$files.Add($path)
            }
        } catch {
            $notes.Add('Skipped an invalid dump path in the text report.')
        }
    }
}
$localDumps = Join-Path $env:LOCALAPPDATA 'CrashDumps'
try {
    if (Test-Path -LiteralPath $localDumps) {
        Get-ChildItem -LiteralPath $localDumps -File | Where-Object {
            $_.Name -match '(?i)^(explorer|luciddesk|lucidpane)\.exe.*\.dmp$' -and $_.LastWriteTime -ge $since
        } | ForEach-Object { [void]$files.Add($_.FullName) }
    }
} catch { $notes.Add("Cannot read ${localDumps}: $($_.Exception.Message)") }

$count = 0
foreach ($file in $files) {
    try {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
            $notes.Add("No longer present: $file")
            continue
        }
        $name = '{0:D3}-{1}' -f ($count + 1), [IO.Path]::GetFileName($file)
        Copy-Item -LiteralPath $file -Destination (Join-Path $output $name)
        $count++
        $notes.Add("${name} <- $file")
    } catch { $notes.Add("Cannot copy ${file}: $($_.Exception.Message)") }
}
$notes.Insert(0, "Existing dump/cab files collected: $count. No applications were started or stopped; no system settings were changed.")
$notes | Set-Content -LiteralPath (Join-Path $output 'collection-status.txt') -Encoding UTF8
if ($count -gt 0) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::CreateFromDirectory($output, "$output.zip")
    Write-Host "Saved: $output.zip"
    Write-Host 'Dumps can contain process memory and private data. Review before sharing. Nothing is uploaded automatically.'
} else {
    Write-Host "No existing dumps found. Send only: $output\collection-status.txt"
}
