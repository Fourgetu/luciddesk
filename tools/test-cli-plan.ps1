param([string]$BuildDirectory = "$PSScriptRoot/../target/cli-plan-build/debug")
$ErrorActionPreference = 'Stop'
$build = (Resolve-Path -LiteralPath $BuildDirectory).Path
if ((Split-Path $build -Leaf) -ne 'debug') { throw 'Only Debug builds are allowed' }
if (Get-Process luciddesk -ErrorAction SilentlyContinue) { throw 'LucidDesk is already running; stop the authorized Debug instance first' }
$cli = Join-Path $build 'luciddesk-cli.exe'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../target/cli-e2e-$([guid]::NewGuid().ToString('N'))"))
[IO.Directory]::CreateDirectory($root) | Out-Null
function Get-SharedHash([string]$Path) {
    $stream = [IO.FileStream]::new($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
    try { return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)) } finally { $stream.Dispose() }
}
function Invoke-Cli([string[]]$Command) {
    $raw = & $cli @Command --data-dir $root --json
    $code = $LASTEXITCODE
    $result = $raw | ConvertFrom-Json
    if ($code -ne 0 -or -not $result.ok) { throw ($raw -join "`n") }
    return $result
}
function Invoke-Plan([object[]]$Operations) {
    $snapshot = Invoke-Cli @('workspace','get')
    $inputPath = Join-Path $root 'plan.json'
    [IO.File]::WriteAllText($inputPath, (@{protocol_version=1;base=$snapshot.context;operations=$Operations} | ConvertTo-Json -Depth 30), [Text.UTF8Encoding]::new($false))
    $preview = Invoke-Cli @('plan','preview','--input',$inputPath)
    $requestId = [guid]::NewGuid().ToString()
    $command = @('plan','apply','--token',$preview.data.plan_token,'--request-id',$requestId)
    $applied = Invoke-Cli $command
    $retry = Invoke-Cli $command
    if (($applied | ConvertTo-Json -Depth 30 -Compress) -ne ($retry | ConvertTo-Json -Depth 30 -Compress)) { throw 'Retry changed the receipt' }
    $receipt = Invoke-Cli @('request','get','--id',$requestId)
    if ($receipt.data.result.request_id -ne $requestId) { throw 'Receipt mismatch' }
    return $applied
}
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class CliTestShutdown { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, IntPtr pid); [DllImport("user32.dll")] public static extern bool PostThreadMessage(uint thread, uint message, UIntPtr w, IntPtr l); }'
$app = $null
try {
    $app = Start-Process -FilePath (Join-Path $build 'luciddesk.exe') -Environment @{ LUCIDDESK_DATA_DIR=$root } -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $root "stderr.log") -RedirectStandardOutput (Join-Path $root "stdout.log")
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    do {
        if ($app.HasExited) { throw "App exited: $($app.ExitCode); $([IO.File]::ReadAllText((Join-Path $root 'stderr.log')))" }
        try { $status=Invoke-Cli @('status'); break } catch {
            if ([DateTime]::UtcNow -gt $deadline) { throw }
            Start-Sleep -Milliseconds 250
        }
    } while ($true)
    $capabilities = Invoke-Cli @('capabilities')
    if ('pane.remove' -notin $capabilities.data.plan_operations) { throw 'Wrong CLI/App version' }
    $settings = Invoke-Cli @('settings','get')
    if ($settings.data.scope -ne 'application_config' -or $settings.data.values.'diagnostics.level' -ne 'error') { throw 'Settings query returned unexpected defaults' }
    if ($settings.data.values.'search.enabled' -isnot [bool]) { throw 'Settings query lost boolean types' }
    $changedSettings = Invoke-Plan @(@{op='settings.update';values=@{'diagnostics.level'='debug';'panel_defaults.grid_scale'=125}})
    if ($changedSettings.data.presentation_status -ne 'applied') { throw ($changedSettings | ConvertTo-Json -Depth 20) }
    $active = Invoke-Cli @('settings','get')
    if ($active.data.runtime.diagnostics_level -ne 'debug' -or $active.data.runtime.grid_scale -ne 125) { throw 'Settings were saved but not applied' }
    $configPath = Join-Path $root 'config.toml'
    $modified = (Get-Item -LiteralPath $configPath).LastWriteTimeUtc
    $unchanged = Invoke-Plan @(@{op='settings.update';values=@{'diagnostics.level'='debug';'panel_defaults.grid_scale'=125}})
    if ($unchanged.data.commit_status -ne 'unchanged' -or (Get-Item -LiteralPath $configPath).LastWriteTimeUtc -ne $modified) { throw 'No-op settings rewrote configuration' }
    $null = Invoke-Plan @(@{op='settings.update';values=@{'diagnostics.level'='error';'panel_defaults.grid_scale'=100}})
    $unavailable = Invoke-Plan @(@{op='settings.update';values=@{'preview.enabled'=$true;'preview.provider'='peek';'preview.peek_path'=(Join-Path $root 'missing-peek.exe')}})
    if ($unavailable.data.commit_status -ne 'committed' -or $unavailable.data.presentation_status -ne 'failed' -or -not $unavailable.data.presentation_error) { throw 'Missing provider did not report saved-but-unavailable state' }
    $failedSettings = Invoke-Cli @('settings','get')
    if (-not $failedSettings.data.values.'preview.enabled' -or $failedSettings.data.runtime.preview_enabled) { throw 'Configured and effective preview states were conflated' }
    $recovered = Invoke-Plan @(@{op='settings.update';values=@{'preview.enabled'=$false;'preview.peek_path'=''}})
    if ($recovered.data.presentation_status -ne 'applied') { throw 'Settings recovery failed' }
    $fixture = Join-Path $root 'folder-fixture'
    [IO.Directory]::CreateDirectory($fixture) | Out-Null
    [IO.File]::WriteAllText((Join-Path $fixture 'keep.txt'),'retained fixture')
    $mapped = Invoke-Plan @(@{op='folder.create';ref='folder';title='CLI 文件夹验证';path=$fixture})
    $folderId = [string]$mapped.data.refs.folder
    if ($mapped.data.presentation_status -ne 'applied') { throw ($mapped | ConvertTo-Json -Depth 20) }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $listing = Invoke-Cli @('folder','get','--id',$folderId)
        if (-not $listing.data.loading -and 'keep.txt' -in $listing.data.items.display_name) { break }
        if ($listing.data.error -or [DateTime]::UtcNow -gt $deadline) { throw ($listing | ConvertTo-Json -Depth 20) }
        Start-Sleep -Milliseconds 100
    } while ($true)
    $child = Join-Path $fixture 'nested'
    [IO.Directory]::CreateDirectory($child) | Out-Null
    $dbPath = Join-Path $root 'workspace.db'
    $savedBefore = (Get-SharedHash $dbPath)
    $configBefore = (Get-SharedHash (Join-Path $root 'config.toml'))
    $navigated = Invoke-Plan @(@{op='folder.navigate';pane_id=$folderId;path=$child})
    if ($navigated.data.commit_status -ne 'not_persisted' -or $navigated.data.presentation_status -ne 'applied') { throw 'Folder navigation was not transient' }
    $browsing = Invoke-Cli @('folder','get','--id',$folderId)
    if ((Split-Path $browsing.data.current_path -Leaf) -ne 'nested' -or (Split-Path $browsing.data.root_path -Leaf) -ne 'folder-fixture') { throw 'Navigation changed root mapping' }
    $grandchild = Join-Path $child 'deeper'
    [IO.Directory]::CreateDirectory($grandchild) | Out-Null
    $null = Invoke-Plan @(@{op='folder.navigate';pane_id=$folderId;path=$grandchild})
    $null = Invoke-Plan @(@{op='folder.back';pane_id=$folderId})
    $backOnce = Invoke-Cli @('folder','get','--id',$folderId)
    if ((Split-Path $backOnce.data.current_path -Leaf) -ne 'nested') { throw 'Idempotent back request navigated more than once' }

    $null = Invoke-Plan @(@{op='folder.navigate';pane_id=$folderId;path=$child})
    $null = Invoke-Plan @(@{op='folder.home';pane_id=$folderId})
    if ((Get-SharedHash $dbPath) -ne $savedBefore -or (Get-SharedHash (Join-Path $root 'config.toml')) -ne $configBefore) { throw 'Transient folder navigation wrote configuration' }
    $folderUpdated = Invoke-Plan @(@{op='folder.update';pane_id=$folderId;list_view=$false;sort_column='size';descending=$true;column_widths=@(0.4,0.2,0.2,0.2);visible_columns=@('name','size')})
    if ($folderUpdated.data.presentation_status -ne 'applied') { throw 'Folder settings presentation failed' }
    $listing = Invoke-Cli @('folder','get','--id',$folderId)
    if ($listing.data.list_view -or $listing.data.runtime.list_view -or $listing.data.runtime.visible_columns_mask -ne 9 -or $listing.data.runtime.sort_column_index -ne 3 -or -not $listing.data.runtime.descending) { throw 'Folder runtime preferences differ from saved settings' }
    $secondFixture = Join-Path $root 'folder-remap-fixture'
    [IO.Directory]::CreateDirectory($secondFixture) | Out-Null
    [IO.File]::WriteAllText((Join-Path $secondFixture 'remapped.txt'),'new root')
    $remapped = Invoke-Plan @(@{op='folder.update';pane_id=$folderId;path=$secondFixture})
    if ($remapped.data.presentation_status -ne 'applied') { throw 'Folder remap failed' }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $listing = Invoke-Cli @('folder','get','--id',$folderId)
        if (-not $listing.data.loading -and 'remapped.txt' -in $listing.data.items.display_name -and 'keep.txt' -notin $listing.data.items.display_name) { break }
        if ($listing.data.error -or [DateTime]::UtcNow -gt $deadline) { throw 'Folder source did not switch to new mapping' }
        Start-Sleep -Milliseconds 100
    } while ($true)
    $null = Invoke-Plan @(@{op='pane.remove';pane_id=$folderId})
    if ([IO.File]::ReadAllText((Join-Path $fixture 'keep.txt')) -ne 'retained fixture') { throw 'Removing a mapping changed real files' }
    $everythingPath = Join-Path $env:ProgramFiles 'Everything/Everything.exe'
    if (-not (Test-Path -LiteralPath $everythingPath)) { throw 'Search test requires an existing Everything installation' }
    $enabledSearch = Invoke-Plan @(@{op='settings.update';values=@{'search.enabled'=$true;'search.everything_path'=$everythingPath}})
    if ($enabledSearch.data.presentation_status -ne 'applied') { throw 'Search window failed to open' }
    $searchWorkspace = Invoke-Cli @('workspace','get')
    $searchId = [string]($searchWorkspace.data.panes | Where-Object kind -eq 'search' | Select-Object -First 1).id
    $savedBefore = (Get-SharedHash $dbPath)
    $configBefore = (Get-SharedHash (Join-Path $root 'config.toml'))
    $searched = Invoke-Plan @(@{op='search.query';pane_id=$searchId;query='exe'})
    if ($searched.data.commit_status -ne 'not_persisted' -or $searched.data.presentation_status -ne 'applied') { throw 'Search query execution failed' }
    $generation = $searched.data.runtime_result.generation
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $results = Invoke-Cli @('search','get','--id',$searchId)
        if ($results.data.generation -ne $generation) { throw 'Search generation unexpectedly changed' }
        if (-not $results.data.busy) { break }
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Search did not complete' }
        Start-Sleep -Milliseconds 100
    } while ($true)
    if ($results.data.failed -or $results.data.total -lt 1 -or $results.data.loaded_count -lt 1) { throw ($results | ConvertTo-Json -Depth 20) }
    if (-not $results.data.has_more) { throw 'Search fixture did not produce enough results to test pagination' }
    $firstCount = $results.data.loaded_count
    $more = Invoke-Plan @(@{op='search.more';pane_id=$searchId})
    if ($more.data.presentation_status -ne 'applied') { throw 'Search pagination request failed' }
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $results = Invoke-Cli @('search','get','--id',$searchId)
        if (-not $results.data.busy) { break }
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Search next page timed out' }
        Start-Sleep -Milliseconds 100
    } while ($true)
    if ($results.data.failed -or $results.data.loaded_count -le $firstCount) { throw 'Search next page did not append results' }
    $refreshed = Invoke-Plan @(@{op='search.refresh';pane_id=$searchId})
    if ($refreshed.data.presentation_status -ne 'applied' -or $refreshed.data.runtime_result.generation -eq $generation) { throw 'Refresh did not advance search generation' }
    $null = Invoke-Plan @(@{op='search.query';pane_id=$searchId;query=''})
    $cleared = Invoke-Cli @('search','get','--id',$searchId)
    if ($cleared.data.busy -or $cleared.data.total -ne 0) { throw 'Search clear did not apply' }
    if ((Get-SharedHash $dbPath) -ne $savedBefore -or (Get-SharedHash (Join-Path $root 'config.toml')) -ne $configBefore) { throw 'Search interaction wrote persisted settings' }
    $null = Invoke-Plan @(@{op='settings.update';values=@{'search.enabled'=$false}})
    $created=Invoke-Plan @(@{op='pane.create';ref='owned';title='CLI 验证面板'})
    $id=[string]$created.data.refs.owned
    $monitors = Invoke-Cli @('monitor','list')
    $monitor = $monitors.data.monitors | Where-Object { $_.work_area_dip.width -ge 560 -and $_.work_area_dip.height -ge 400 } | Select-Object -First 1
    if (-not $monitor) { throw 'No monitor large enough for geometry test' }
    $moved = Invoke-Plan @(@{op='pane.geometry';pane_id=$id;monitor_id=$monitor.id;x=40;y=50;width=480;height=320})
    if ($moved.data.presentation_status -eq 'failed') { throw ($moved | ConvertTo-Json -Depth 20) }
    $placed = Invoke-Cli @('pane','get','--id',$id)
    $scale = $monitor.dpi / 96.0
    if ([Math]::Abs($placed.data.geometry.x - 40) -gt (1 / $scale) -or [Math]::Abs($placed.data.geometry.width - 480) -gt (1 / $scale)) { throw 'Stored geometry did not match plan' }
    if ($placed.data.window_bounds_px.x -ne ($monitor.work_area_px.x + [Math]::Round(40*$scale)) -or $placed.data.window_bounds_px.y -ne ($monitor.work_area_px.y + [Math]::Round(50*$scale)) -or $placed.data.window_bounds_px.width -ne [Math]::Round(480*$scale) -or $placed.data.window_bounds_px.height -ne [Math]::Round(320*$scale)) { throw 'Native window geometry did not match committed geometry' }

    $tabCreated = Invoke-Plan @(@{op='pane.create';ref='tab';title='CLI 验证标签'})
    $tabId = [string]$tabCreated.data.refs.tab
    $merged = Invoke-Plan @(@{op='tab.merge';pane_id=$tabId;into_pane_id=$id})
    if ($merged.data.presentation_status -ne 'applied') { throw 'Tab merge did not apply to windows' }
    $grouped = Invoke-Cli @('workspace','get')
    $group = $grouped.data.tabs | Where-Object { $id -in $_.members }
    if ($group.active -ne $id -or $tabId -notin $group.members) { throw 'Tab merge mismatch' }
    $selected = Invoke-Plan @(@{op='tab.reorder';pane_id=$id;pane_ids=@($tabId,$id)},@{op='tab.select';pane_id=$tabId})
    if ($selected.data.presentation_status -ne 'applied') { throw 'Tab selection did not apply' }
    $grouped = Invoke-Cli @('workspace','get')
    $group = $grouped.data.tabs | Where-Object { $id -in $_.members }
    if ($group.active -ne $tabId -or $group.members[0] -ne $tabId) { throw 'Tab order/selection mismatch' }
    $detached = Invoke-Plan @(@{op='tab.detach';pane_id=$tabId})
    if ($detached.data.presentation_status -ne 'applied') { throw 'Tab detach did not apply' }
    $grouped = Invoke-Cli @('workspace','get')
    if ($grouped.data.tabs | Where-Object { $tabId -in $_.members }) { throw 'Detached tab remains grouped' }
    $null = Invoke-Plan @(@{op='pane.remove';pane_id=$tabId})
    $null=Invoke-Plan @(@{op='pane.update';pane_id=$id;locked=$true;auto_hide=$true;always_on_top=$true})
    $panel=Invoke-Cli @('pane','get','--id',$id)
    if (-not ($panel.data.locked -and $panel.data.auto_hide -and $panel.data.always_on_top)) { throw 'Options did not apply' }
    $null=Invoke-Plan @(@{op='pane.update';pane_id=$id;locked=$false},@{op='pane.remove';pane_id=$id})
    $workspace=Invoke-Cli @('workspace','get')
    if ($id -in $workspace.data.panes.id) { throw 'Removed panel remains' }
    @{result='passed';data_dir=$root;instance=$status.context.instance_id;created_and_removed_id=$id;desktop_connected=$status.data.desktop_connected} | ConvertTo-Json
} finally {
    if ($app -and -not $app.HasExited) {
        $app.Refresh()
        $thread = [CliTestShutdown]::GetWindowThreadProcessId($app.MainWindowHandle,[IntPtr]::Zero)
        if ($thread -eq 0) { $thread=($app.Threads | Sort-Object StartTime | Select-Object -First 1).Id }
        $null=[CliTestShutdown]::PostThreadMessage($thread,0x12,[UIntPtr]::Zero,[IntPtr]::Zero)
        if (-not $app.WaitForExit(10000)) { Stop-Process -Id $app.Id -Force }
    }
}
