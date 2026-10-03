# CLI 使用（只读首版）

`luciddesk-cli.exe` 查询正在运行的 LucidDesk。先启动同一版本主程序；不自动启动应用，不创建或直接打开数据库。

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

响应包含 `protocol_version/request_id/ok/context/data/error`。退出码：0 成功，2 输入错误，3 应用未运行，4 面板不存在，5 数据目录不匹配，6 响应过大/忙碌，8 超时，9 权限拒绝，10 协议不兼容，1 其他错误。

`workspace get` 返回当前应用的面板、桌面项目、标签组快照，不强制重新扫描磁盘。面板明确区分 `manual_collapsed` 与 `effective_collapsed`；没有对应窗口时后者为 null。项目 ID 是当前应用实例内的不透明 token，不要解析或跨重启复用。项目清单不包含文件夹面板内文件或搜索结果。

当前 `capabilities` 返回 `writes:false`、`plans:false`、`concurrency_tokens:false`、`pane_geometry:false`；不要把 null 版本字段当作可用于写入的校验 token。尚不支持收纳、创建面板、计划应用、监视器查询或自动整理 Skills。

本地管道限制当前用户，核验对端用户与会话，并拒绝远程连接。独立 CLI 不链接桌面存储模块。查询队列只读访问 UI 状态，不调用保存、桌面刷新或备份维护；应用自己的既有后台任务仍可能独立运行。

当前通过 Cargo 构建运行，安装包与便携包的 CLI 分发属于后续交付。
