param([string]$BuildDirectory = "$PSScriptRoot/../target/cli-plan-build/debug")
$ErrorActionPreference = 'Stop'
$build = (Resolve-Path -LiteralPath $BuildDirectory).Path
if ((Split-Path $build -Leaf) -ne 'debug') { throw 'Only Debug builds are allowed' }
if (Get-Process luciddesk -ErrorAction SilentlyContinue) { throw 'LucidDesk is already running; stop the authorized Debug instance first' }
$cli = Join-Path $build 'luciddesk-cli.exe'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../target/cli-e2e-$([guid]::NewGuid().ToString('N'))"))
[IO.Directory]::CreateDirectory($root) | Out-Null
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
    $created=Invoke-Plan @(@{op='pane.create';ref='owned';title='CLI 验证面板'})
    $id=[string]$created.data.refs.owned
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
