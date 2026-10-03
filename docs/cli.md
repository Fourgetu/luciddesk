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

当前支持计划式写入，`capabilities` 返回 `writes:true`、`plans:true`、`concurrency_tokens:true`。单项写入快捷命令尚未开放；仓库提供配套 Skill，安装与随包分发仍待完善。

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

## 设置查询

`luciddesk-cli settings get --json` 返回 `scope: "application_config"` 与 `values`。
`values` 使用稳定的命名字段（如 `diagnostics.level`、`search.enabled`、`panel_defaults.grid_scale`），保留字符串、布尔和数值类型。返回应用已加载的有效配置，包含缺省值；不读取外部尚未重新加载的编辑，不写入文件。
此范围为 config.toml 中的全局设置，尚不包含开机启动注册、字体及数据库内其他设置。支持通过 `settings.update` 计划修改上述字段。

## 设置修改

操作格式：`{"op":"settings.update","values":{"diagnostics.level":"debug","panel_defaults.grid_scale":125}}`。
一个计划必须只包含这一个设置操作；多个字段放入同一个 `values`，不能混合面板/项目操作。设置保存使用 TOML 原子替换，工作区保存使用 SQLite 事务，两者不构成跨文件事务。预览会校验全部字段、类型和值，并返回修改前后的有效配置。同值提交不写配置文件。

沿用 `plan preview`、`plan apply`、`request get` 与相同 request ID 重试规则。设置回执 `scope` 为 `settings`。`commit_status:committed` 只确认保存；检查 `presentation_status:applied` 才能确认此次生效处理完成。若为 `failed`，读取 `presentation_error`，配置仍已保存，不应重新执行原提交；修复依赖后可使用新上下文提交同值设置重新尝试生效。快捷键冲突、缺少 Everything 或预览提供程序都会明确报告。界面重绘使用正常消息循环。

`settings get` 的 `runtime` 同时返回当前日志等级、图标缩放、圆角、搜索窗口可见性、预览实际启用状态与快捷键状态。窗口外观更新保留普通/文件夹面板实例及临时收起状态；改变图标缩放会重置网格滚动，和 GUI 操作一致。

## 显示器与面板几何

`monitor list --json` 返回显示器 ID、DPI、物理工作区和工作区 DIP 尺寸。`pane get` 返回持久布局 `geometry`，以及实际窗口像素边界 `window_bounds_px`（无窗口时为 null，标签成员返回共享活动窗口）。临时收起后的实际高度与保存的展开高度不同。

计划操作 `{"op":"pane.geometry","pane_id":"1","monitor_id":"查询返回的ID","x":40,"y":50,"width":480,"height":320}` 使用指定显示器**工作区相对 DIP**。完整矩形必须位于工作区内，最小为 260 × 160 DIP；拒绝无效显示器、锁定面板及越界矩形。可与普通面板/项目操作组成同一计划。标签组共享窗口，所以几何作用于全部成员；普通、文件夹及搜索面板均支持，搜索保存展开尺寸，实际窗口高度由搜索内容管理。

预览 `diff.geometry` 显示像素取整后的有效几何。提交保存工作区与当前显示器布局到同一个 SQLite 事务；不改变其他显示器拓扑的记录。查询得到的上下文包含拓扑版本，显示器变化会使旧计划冲突。提交后检查 `pane get` 中的布局及实际窗口边界；原生移动失败会返回 `presentation_status:failed`，保存状态仍保留。

## 标签组

普通桌面面板支持以下计划操作，可与面板/项目操作批量提交：

| 操作 | 参数 | 语义 |
| --- | --- | --- |
| `tab.merge` | `pane_id`, `into_pane_id` | 将源整个标签组合并到目标组末尾，保留目标活动标签及共享窗口选项 |
| `tab.select` | `pane_id` | 激活该标签；锁定面板仍允许切换内容 |
| `tab.reorder` | `pane_id`, `pane_ids` | 指定组内完整顺序，必须每个成员恰好一次 |
| `tab.detach` | `pane_id` | 拆分为独立窗口，保留内容及展开位置，可随后用 `pane.geometry` 移动 |

合并、排序、拆分需要解锁。文件夹和搜索面板不支持标签，与 GUI 一致。预览包含修改前后 `tabs`；读取 `workspace get` 的 `tabs` 验证成员、顺序与活动标签。切换复用原生窗口和临时收起状态，合并/拆分保留内容模型缓存。界面处理结果写入幂等回执；`applied` 表示更新已成功处理，重绘和收起动画由正常消息循环完成。

## 文件夹面板

- `folder.create`：`ref`、`title`、绝对目录 `path`。目录必须存在；继承 GUI 的文件夹视图默认值。预览不启动文件夹工作线程，也不写默认设置。
- `folder.update`：`pane_id`，以及至少一个 `path`、`list_view`、`sort_column`、`descending`、`column_widths`、`visible_columns`。排序列和可见列使用 `name/modified/type/size`；名称列必须可见。列宽按 `[name, modified, type, size]` 提供四个正比例，总和为 1。字段省略保持原值，null 无效。
- `folder get --id ID --json`：返回根目录、当前浏览路径、保存偏好、窗口实际视图偏好、加载状态及当前文件快照。`loading:true` 表示异步加载尚未完成，文件列表可能仍是上一批结果。查询不强制扫描，也不写入数据库。

以上写操作可以与普通面板操作组成同一计划，工作区、排序和列设置在同一个 SQLite 事务保存。文件夹面板的标题和窗口选项使用 `pane.update`，位置大小使用 `pane.geometry`，移除映射使用 `pane.remove`；移除只关闭面板，不删除目录内文件。桌面 `item.assign/release/reorder` 不适用于文件夹内容。

## 临时导航与搜索

以下操作各自作为单独计划，仍经过 preview/apply 和幂等回执，但返回 `scope:runtime`、`commit_status:not_persisted`，不会保存路径、搜索词或结果：

| 操作 | 参数 | 行为 |
| --- | --- | --- |
| `folder.navigate` | `pane_id`, 绝对目录 `path` | 浏览目录，保留映射根目录 |
| `folder.back` | `pane_id` | 返回上一目录，没有历史时不操作 |
| `folder.home` | `pane_id` | 回到映射根目录并清空导航历史 |
| `search.query` | `pane_id`, `query` | 设置输入；空字符串清空结果，同值不重发查询 |
| `search.refresh` | `pane_id` | 刷新当前查询并推进查询代次 |
| `search.more` | `pane_id` | 加载下一页；忙碌时报告失败，没有更多时不操作 |

`search get --id ID --json` 返回 `query`、`generation`、`busy`、`failed`、`replacing`、`total`、`loaded_count`、`has_more` 和已加载 `entries`。执行成功只说明输入/分页请求已经处理，不能据此认定异步查询完成。以返回的代次查询，等待 `busy:false`，再检查 `failed`；加载替换期间旧结果仍可能存在。Everything 必须可用，关闭的搜索面板需先通过设置启用。查询最长 32767 UTF-16 单元，不接受 NUL。

应用的并发上下文包含当前文件夹导航历史与搜索代次/加载状态；GUI 的并发操作可能使旧计划冲突，此时重新查询和预览。重试必须沿用原 request ID，避免返回/刷新/分页重复执行。`runtime_result` 是操作处理后的即时状态，最新结果仍通过查询取得。
