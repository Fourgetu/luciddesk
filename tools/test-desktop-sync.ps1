param(
    [ValidateRange(1, 100)][int]$Repeat = 5,
    [switch]$LiveDesktop
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$logs = Join-Path $repo ('target/sync-validation/' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
New-Item -ItemType Directory -Path $logs -Force | Out-Null
$results = [System.Collections.Generic.List[object]]::new()
function Invoke-CargoCheck([string]$Name, [string[]]$CargoArgs) {
    $log = Join-Path $logs ($Name + '.log')
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    & cargo @CargoArgs *> $log
    $code = $LASTEXITCODE
    $results.Add([pscustomobject]@{
        Name = $Name; ExitCode = $code; Seconds = $timer.Elapsed.TotalSeconds
        Arguments = $CargoArgs; Log = $log
    })
    $results | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $logs 'results.json') -Encoding utf8
    Write-Host "$Name : exit=$code ($([math]::Round($timer.Elapsed.TotalSeconds, 2))s)"
    if ($code -ne 0) {
        Get-Content -LiteralPath $log -Tail 40
        throw "Validation failed: $Name. Logs: $logs"
    }
}
Push-Location $repo
try {
    Invoke-CargoCheck 'all-targets' @('check', '--workspace', '--all-targets', '--all-features', '--locked', '--offline')
    Invoke-CargoCheck 'workspace-tests' @('test', '--workspace', '--lib', '--bins', '--tests', '--locked', '--offline', '--', '--test-threads=1')
    Invoke-CargoCheck 'doc-tests' @('test', '--workspace', '--doc', '--locked', '--offline')
    for ($round = 1; $round -le $Repeat; $round++) {
        Invoke-CargoCheck "folder-watch-$round" @('test', '-p', 'luciddesk', '--bin', 'luciddesk', '--locked', '--offline', 'pane::folder::tests::folder_watch_tracks_children_and_recovers_after_missing_directory', '--', '--exact', '--test-threads=1')
    }
    if ($LiveDesktop) {
        # Read-only Explorer probe. Do not enable all ignored tests: some open
        # menus, change the clipboard or exercise the real desktop membership.
        Invoke-CargoCheck 'live-desktop' @('test', '-p', 'desktop-shell', '--lib', '--locked', '--offline', 'native_layout::tests::background_revision_matches_full_snapshot', '--', '--ignored', '--exact', '--test-threads=1')
        Invoke-CargoCheck 'live-hook' @('test', '-p', 'desktop-hook', '--lib', '--locked', '--offline', 'filter::items::tests::live_filter_snapshot_reads_without_mutating_desktop', '--', '--ignored', '--exact', '--test-threads=1')
    }
    Write-Host "All requested checks passed. Logs: $logs"
} finally {
    Pop-Location
}
