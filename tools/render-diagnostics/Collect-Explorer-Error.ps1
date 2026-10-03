$ErrorActionPreference = 'Stop'
$reportPath = Join-Path $PSScriptRoot 'explorer-errors.txt'
$lines = [System.Collections.Generic.List[string]]::new()
$since = (Get-Date).AddDays(-7)
$appPattern = '(?i)\b(?:explorer|luciddesk|luciddesk)\.exe\b'
$lines.Add('Explorer and LucidDesk errors from the last 7 days. This script does not start or stop any application.')
$lines.Add("Collected: $((Get-Date).ToString('o'))")
foreach ($name in @('build.json', 'luciddesk.exe', 'luciddesk_explorer.dll')) {
    $path = Join-Path $PSScriptRoot $name
    if (Test-Path -LiteralPath $path -PathType Leaf) {
        if ($name -eq 'build.json') {
            $lines.Add((Get-Content -LiteralPath $path -Raw))
        } else {
            $lines.Add("SHA256 ${name}: $((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash)")
        }
    }
}
try {
    $events = @(Get-WinEvent -FilterHashtable @{
        LogName = 'Application'
        Id = @(1000, 1001, 1002)
        StartTime = $since
    } -ErrorAction Stop | Where-Object { $_.ToXml() -match $appPattern } | Sort-Object TimeCreated)
    if ($events.Count -eq 0) { $lines.Add('No matching application events found.') }
    foreach ($event in $events) {
        $lines.Add("Time: $($event.TimeCreated.ToString('o'))  Provider: $($event.ProviderName)  ID: $($event.Id)")
        $lines.Add($event.Message)
        $lines.Add($event.ToXml())
        $lines.Add('')
    }
} catch {
    $lines.Add("Event query result: $($_.Exception.Message)")
}
# Only read existing text reports; do not enable crash dumps or change WER settings.
foreach ($base in @($env:ProgramData, $env:LOCALAPPDATA)) {
    foreach ($queue in @('ReportArchive', 'ReportQueue')) {
        $root = Join-Path $base "Microsoft\Windows\WER\$queue"
        try {
            if (-not (Test-Path -LiteralPath $root)) { continue }
            $folders = Get-ChildItem -LiteralPath $root -Directory -ErrorAction Stop |
                Where-Object { $_.Name -match '(?i)(explorer|luciddesk|luciddesk)\.exe' }
            foreach ($folder in $folders) {
                $report = Join-Path $folder.FullName 'Report.wer'
                try {
                    if (-not (Test-Path -LiteralPath $report -PathType Leaf)) { continue }
                    if ((Get-Item -LiteralPath $report).LastWriteTime -lt $since) { continue }
                    $lines.Add("WER report: $report")
                    $lines.Add((Get-Content -LiteralPath $report -Raw -ErrorAction Stop))
                } catch {
                    $lines.Add("WER report unavailable: $report : $($_.Exception.Message)")
                }
            }
        } catch {
            $lines.Add("WER query result: $root : $($_.Exception.Message)")
        }
    }
}
$lines | Set-Content -LiteralPath $reportPath -Encoding UTF8
Write-Host "Saved: $reportPath"
