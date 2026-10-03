# CLI 使用

`luciddesk-cli.exe` 查询并整理正在运行的 LucidDesk。先启动同一版本主程序；不自动启动应用，不创建或直接打开数据库。

## 构建与运行

```powershell
cargo build -p luciddesk -p luciddesk-cli --locked --offline
.\target\debug\luciddesk.exe
.\target\debug\luciddesk-cli.exe status --json
.\target\debug\luciddesk-cli.exe capabilities --json
.\target\debug\luciddesk-cli.exe workspace get --json
.\target\debug\luciddesk-cli.exe pane list --json
.\target\debug\luciddesk-cli.exe pane get --id 1 --json
.\target\debug\luciddesk-cli.exe item list --pane 1 --json
.\target\debug\luciddesk-cli.exe item list --unassigned --json
.\target\debug\luciddesk-cli.exe schema --json
```

普通输出为格式化 JSON 数据，`--json` 输出单行 JSON 响应信封，适合脚本；错误码同时反映在退出码中。`schema` 是离线本地命令，直接输出协议信封 JSON Schema。`--help`、`--version` 也不需要主程序。

`--data-dir <现有目录>` 只核对主程序的数据目录，不切换或创建工作区。`--timeout-ms` 范围 1–60000，默认 10000；服务端查询等待 UI 最多 5 秒，整个连接 I/O 最多 10 秒。只有一个服务端连接同时执行，繁忙时客户端在自己的超时期限内等待。

## 响应与范围

响应包含 `protocol_version/request_id/ok/context/data/error`。退出码：0 成功，2 输入错误，3 应用未运行，4 面板不存在，5 版本冲突/锁定/过期计划/重复请求冲突/数据目录不匹配，6 响应过大/忙碌/桌面组件不可用，7 保存失败，8 超时或结果未知，9 权限拒绝，10 协议不兼容，1 其他错误。

`workspace get` 返回当前应用的面板、桌面项目、标签组快照，不强制重新扫描磁盘。面板明确区分 `manual_collapsed` 与 `effective_collapsed`；没有对应窗口时后者为 null。项目 ID 是当前应用实例内的不透明 token，不要解析或跨重启复用。项目清单不包含文件夹面板内文件或搜索结果。

当前支持计划式写入，`capabilities` 返回 `writes:true`、`plans:true`、`concurrency_tokens:true`。面板几何、标签修改、单项写入快捷命令及自动整理 Skills 尚未开放。

本地管道限制当前用户，核验对端用户与会话，并拒绝远程连接。独立 CLI 不链接桌面存储模块。查询队列只读访问 UI 状态，不调用保存、桌面刷新或备份维护；应用自己的既有后台任务仍可能独立运行。

当前通过 Cargo 构建运行，安装包与便携包的 CLI 分发属于后续交付。

## 整理计划：查询 → 预览 → 应用

所有写入通过 `plan preview` / `plan apply` 执行。计划包含协议版本、查询返回的完整 `context` 和有序操作列表。支持：

| op | 参数 | 行为 |
| --- | --- | --- |
| `pane.create` | `ref`, `title` | 创建普通面板，继承外观，使用 GUI 默认尺寸及避让位置 |
| `pane.update` | `pane_id`；可选 `title`, `locked`, `auto_hide`, `collapsed`, `always_on_top` | 至少提供一个修改字段；窗口选项作用于整个标签组 |
| `pane.remove` | `pane_id`, 可选 `release_items` | 删除指定内容面板，非空时必须显式释放项目 |
| `item.release` | `item_ids` | 将项目归还桌面，不移动或删除真实文件 |
| `item.assign` | `item_ids`，以及二选一的 `pane_id` / `pane_ref` | 收纳项目；保留目标现有顺序，新项目按输入追加 |
| `item.reorder` | `pane_id`, `item_ids` | 提供该面板全部项目 ID 的完整顺序；空面板允许空列表 |

`pane_ref` 引用同一计划中此前创建的面板。ID 均为字符串；项目 ID 必须取自当前实例的查询结果。源/目标面板锁定时拒绝修改；使用 `pane.update` 显式设置 `locked:false` 后可继续修改。不移动或删除真实文件。标题最多 256 个字符，计划最多 256 个操作，每个项目列表最多 10000 项，输入及 IPC 消息上限 4 MiB。

PowerShell 示例（创建空面板）：

```powershell
$cli = '.\target\debug\luciddesk-cli.exe'
$snapshot = & $cli workspace get --json | ConvertFrom-Json
if (-not $snapshot.ok) { throw '查询失败' }
$plan = @{
    protocol_version = 1
    base = $snapshot.context
    operations = @(@{ op = 'pane.create'; ref = 'documents'; title = '文档' })
}
$path = Join-Path $PWD 'plan.json'
[IO.File]::WriteAllText($path, ($plan | ConvertTo-Json -Depth 20), [Text.UTF8Encoding]::new($false))
$preview = & $cli plan preview --input $path --json | ConvertFrom-Json
if (-not $preview.ok) { throw ($preview.error | ConvertTo-Json) }
$preview.data.diff | ConvertTo-Json -Depth 20
$requestId = [guid]::NewGuid().ToString()
& $cli plan apply --token $preview.data.plan_token --request-id $requestId --json
& $cli request get --id $requestId --json
```

添加收纳操作时使用：

```json
{"op":"item.assign","pane_ref":"documents","item_ids":["从 workspace get 获取的项目 ID"]}
```

`--input -` 从标准输入读取 UTF-8 JSON；文件也必须为 UTF-8（不带 BOM）。未知字段和无效组合会被拒绝。

预览只在内存中执行，不保存数据库。返回完整前后标题/归属/排序快照、临时面板引用和 300 秒有效 token；最多保留 64 个计划，超过后淘汰最早的计划。预览中的新面板 ID 是临时结果，不构成预留。

应用会重新核对实例、工作区、清单与显示器状态。版本是保守的、不透明的进程内 token：当前各字段共用一个版本，任一相关变化均可能导致 `CONFLICT`，需重新查询和预览。临时自动收起不参与版本，也不触发保存。应用响应的 `context` 是本次校验的基础版本；下一次计划前重新 `workspace get`。

主程序在 UI 线程校验全部操作，以一次存储事务提交候选工作区，成功后替换内存状态并刷新界面。没有变化则不调用保存；失败预览和读取请求均不写库。项目收纳/排序发生变化时要求桌面组件已连接。数据库提交与桌面呈现分开：`commit_status:committed` 表示保存成功，`presentation_status:pending` 表示已请求刷新，但尚未确认 Explorer 完成呈现；呈现错误记录诊断日志，不重复提交数据库。

## 超时和重试

应用时建议显式提供唯一 `--request-id` 并保留 token。相同请求 ID 和相同内容重试，返回原结果，不重复写入；同 ID 不同内容返回 `REQUEST_ID_REUSED`。同一 token 使用另一请求 ID 再应用返回 `PLAN_ALREADY_APPLIED`。

成功结果只在当前主程序内存保留最多 10 分钟、1024 条；不为请求回执增加数据库 I/O。超时先 `request get --id ...`，或在保留期限内使用完全相同的应用请求重试。`RESULT_UNKNOWN` 不代表未执行：主程序重启或结果被淘汰后，应检查工作区再决定下一步，不自动生成新 ID 重做。
