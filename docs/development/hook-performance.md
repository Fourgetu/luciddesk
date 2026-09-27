# Hook 与面板后台消耗测量

## 菜单兼容路径与 IPC 精简

- 普通 Release 不再写入菜单 Presenter 阶段属性，也不再追加 `menu-presenter.log`；
  Debug 或显式 `menu-diagnostics` 构建仍支持诊断。最终产物已检查相关日志路径和阶段属性字符串不再存在。
- 编码端在生成请求时校验头、UTF-16 长度及大小限制，删除编码后再次完整解码的字符串分配。
  接收端仍独立验证全部 IPC 数据，共用头部约束；补充 UTF-16 边界和总大小回归覆盖。
- `legacy` 重命名为 `object_view`；经典菜单、IContextMenu2 兼容和回调/事务保护保留。
- 两个 desktop_filter_probe 实验入口需要显式 `desktop-menu-diagnostics` 功能，默认构建不包含。
- 普通和诊断模式各 24 项 hook 测试通过；真实 Shell 的经典菜单回退验证通过；
  诊断主程序及两个实验入口编译检查通过。实验入口仍有复用模块中未使用 main 的既有警告。

## 2026-09-28 代码审计与构建清理

范围为近期重连、进程监听、巡检调度、设置预览缓存和相关测量代码。

- 移除临时重连日志、每次 IPC 的环境变量查询和阶段属性写入，合并纯转发的 IPC 包装。
- Explorer 退出回调直接传递窗口句柄，移除只包装一个 HWND 的堆分配；仍在关闭进程句柄前注销并等待回调结束。
- 保留进程/窗口退出、失效目标、事务保护、通知与退避测试；删除一次性 WARP 性能采样测试。
- 修复设置页原有测试对旧坐标的依赖，按当前控件布局定位并滚动，验证拖动延迟保存、材质独立强度、键盘调整和纯色设置保存。
- 保留按需启用的 hook 测量入口；采样中连接丢失时报告无效样本，避免误报零开销。
- 清理 20 个未使用的 Cargo debug/release 构建目录，文件逻辑大小合计 55.30 GiB。
  已排除运行路径并检查目录边界及重解析点；保留主 Release、测试数据、工具源码和测量记录。

验证：Release 编译成功，215 项面板测试与 24 项 hook 测试通过；分别有 12/4 项交互或专项测试默认忽略。
当前代码未重新执行破坏性的 Explorer/应用崩溃测试；之前边界结果见后文。
清理清单与测试日志保存在 `target/audit-cleanup-manifest.json`、`target/audit-pane-tests.log`。

### 本轮 Release 采样

两轮各约 60 秒，采样期间没有编译或压力测试；结束已恢复 5 个面板窗口。
CPU 按本机 32 个逻辑处理器归一化；括号为单核等效占用。该短时采样不能证明长期无泄漏。

| 状态 | 应用平均 CPU | 私有内存起止 | GDI 起止 | 句柄起止 | hook 扫描次数 / 累计墙钟耗时 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 面板显示 | 0.091%（2.899%） | 104.73 → 104.55 MiB | 57 → 57 | 1051 → 1036 | 17 / 113.59 ms |
| 面板隐藏 | 0.042%（1.346%） | 104.55 → 104.61 MiB | 57 → 57 | 1036 → 1032 | 6 / 17.07 ms |

未发现本轮采样内持续的内存或 GDI 增长。hook 扫描仍受系统/应用通知影响；不能以这两组
次数推导固定空闲频率，也没有同环境的前后对照来宣称百分比性能提升。
计时仅覆盖后台成员扫描，不覆盖所有 hook 路径，也不是 Explorer 总 CPU。

记录：`target/perf-audit-visible.csv/json`、`target/perf-audit-hidden.csv/json`、
`target/audit-hook-visible.json`、`target/audit-hook-hidden.json`。
最终只读桌面快照测试通过；运行版本为 `target/release/lucidpane.exe`，本轮启动 PID 4448。

## 最新优化：握手失效检测与 Explorer 退出唤醒

本轮没有通过缩短所有超时来强行追求更低数字，新增以下行为：

- 握手检查 HWND、所属进程/线程、父窗口；每 50 ms 重新核对当前桌面视图，旧目标失效立即退出。
- 握手期间处理 UI 消息，保留 WM_QUIT 语义和已有重连重入保护。
- 将原本就需要的完整桌面快照读取前移到安装 hook 之前，先从客户端确认 Shell 可读。
  订阅仍先于快照读取，避免漏掉初始化期间的变更；复用该快照，不增加一次枚举。
- 普通 IPC 等待确认时若连接消失立即返回；DETACH 已释放所有权仍算成功。
- 客户端监听 Explorer 进程退出，直接唤醒恢复流程，不再主要依赖较晚的 TaskbarCreated。
  等待是一次性的系统事件，无固定高频轮询；注销等待后才释放上下文和句柄。
- 原始定位使用过临时分阶段日志与挂载阶段 HWND 属性；本轮审计已移除这些诊断。
  下方重连时间为移除前的实测记录，保留以说明优化依据。

### 验证结论

仅检查失效 HWND 没有消除本机的超时：当时窗口仍有效，Explorer 已进入挂载但尚未取得 Shell
视图。前移快照后，实测握手降到 13～20 ms，后续运行没有再出现原来的 3 秒握手超时。

最后一次真实重启记录（`target/exitwake-boundary.json`）：

| 观察节点 | 自开始重启计时 |
| --- | ---: |
| 桌面窗口出现 | 0.66 s |
| Hook 连接被观察到 | 4.38 s |
| 过滤至正确的 80 项 | 同一轮采样，约 4.38 s |

不能把同轮观察到连接/过滤的约 0.15 ms 差值解释成真实过滤耗时；外部采样间隔约 200 ms。
内部日志显示最后成功握手 20 ms，完整快照及首次发布的这次成功尝试约 937 ms。

退出通知日志提供了剩余等待的证据：应用在通知后约 56 ms 开始首次尝试；约 0.85 s 和 1.92 s
时桌面快照仍不可读；约 3.12 s 后收到 TaskbarCreated，随后完成快照与挂载。因此本轮端到端
恢复仍约 4 秒，不能声称已缩短为亚秒级，也不能只拿握手时间代替完整恢复时间。

24 项 hook、24 项 hybrid、5 项 runtime 测试通过，另有真实只读快照验证通过。新增测试覆盖
失效/替换/重新挂父窗口的目标、确认前连接消失、退出信号只唤醒一次。5 个面板恢复且标题一致，
原生桌面回到 80 项。每次重启均按该轮开始时的文件夹窗口快照恢复，后面几轮用户只有 1 个窗口。

原始记录：`target/exitwake-stderr.log`、`target/exitwake-filter-timeline.json`、
`target/exitwake-hook-tests.log`、`target/exitwake-hybrid-tests.log`、`target/exitwake-runtime-tests.log`。
中间阶段记录分别使用 `handshake-`、`handshake-pump-`、`preflight-`、`handshake-final-` 前缀；
它们用于定位原因，不作为最终版本的性能承诺。

## 当前实现：事件通知与统一巡检

2026-09-28 后续修改已替代下文第一轮的双重定时扫描：

- 主程序统一安排一致性巡检，稳定时按 2→4→8→16→30 秒退避；Shell 通知立即使结果失效。
  有变化时正常发布成员集合，无变化时向 hook 发送一次修复请求，避免两套独立扫描时钟。
- hook 不再每秒唤醒或独立每 8 秒扫描。进程退出使用一次性线程池等待，控制窗口销毁使用
  WinEvent 通知；回调只发消息，桌面恢复仍在 Explorer 的桌面 STA 上执行。
- 保留每分钟一次的生命周期兜底，仅检查进程/窗口存活，不枚举条目。
- 读取失败按原来的 250/500/1000 ms 到期重试；仅活动事务每秒检查释放标记；工作消息投递
  失败时 25 ms 补偿。没有待处理任务就取消恢复定时器。
- 卸载先取消窗口事件和等待回调，再关闭进程句柄，避免回调访问已释放的资源。

验证：20 项 hook 测试和 5 项主程序巡检测试通过。新增覆盖退出信号仅唤醒一次、注销后不再
回调、进程仍存活但控制窗口被销毁，以及空闲时没有恢复定时器。
真实强制结束 LucidPane 测试中，约 136 ms 后 hook 完成清理；只读快照显示原生桌面可见条目
由 80 恢复至 85，随后已重新启动新版。后续真实 Explorer 重启验证见下节。

### 卡死与 Explorer 重启边界实测

后续已修正下述重连后过滤延迟，最新结果见“首次发布与重连重试修正”。

- **模拟卡死**：暂停 PID 83192 的整个进程约 71 秒，包含超过一次分钟级存活检查的时间。
  在 0、35、71 秒检查，应用均不响应，Explorer 桌面均响应，hook 一直保留；暂停期间只读
  桌面快照仍为 80 项。恢复执行后应用重新响应。结论是卡死不会误伤 Explorer，但目前不会
  自动释放被管理的 5 项图标；模拟暂停不覆盖所有真实死锁形态。
- **Explorer 崩溃/重启**：实际结束桌面 Explorer PID 81400，系统恢复为 PID 81944。
  桌面窗口约 0.9 秒恢复，hook 约 4.7 秒重新连接同一应用 PID 83192。5 个面板恢复，标题
  集合一致，应用响应正常，托盘图标重新注册成功。
- **连接与过滤完成分开验证**：连接后约 3 秒的第一次快照仍为 85 项，后续稳定复查为 80 项。
  因此 4.7 秒只表示连接完成，不能称为全部图标过滤完成；本轮没有连续记录完整同步的精确
  时延。重连期间原生桌面可能暂时显示已分组的图标。
- 已检查原先记录的 6 个不同文件夹路径全部恢复，并补回重复路径窗口，窗口数恢复为 7。
  未验证各 Explorer 窗口内部的多标签结构。测试结束应用未暂停，所有面板可见。

记录：`target/hang-boundary.json`、`target/hang-desktop-suspended.log`、
`target/explorer-boundary.json`、`target/explorer-reconnected-snapshot.log`、
`target/explorer-settled-snapshot.log`、`target/explorer-window-restoration.json`、
`target/explorer-final-health.json`。测试脚本位于忽略的 target 目录，正常启动不执行故障注入。

### 首次发布与重连重试修正

新 hook 的成员集合为空，但重连时应用的图标缓存可能完整，不会再产生图标加载完成事件。
连接路径原先只重建清单而没有立即发布，因此过滤依赖后续事件才能恢复。现在清单重建成功后
立即调用 `sync`；只发布已有图像的条目，缺失图像仍由异步完成事件补齐，发布失败沿用连接清理。

同时将固定 10 秒重连重试改成 250/500/1000/2000/4000/8000/10000 ms 退避，仅在断线时启用；
桌面重建事件重置退避。计时从失败完成时开始，新增进行中标记防止 COM 消息泵重入造成重叠连接。
稳定连接后的巡检间隔保持不变。

两轮真实重启的分阶段测量（每约 200 ms 采样，观察值不是精确内部耗时）：

| 版本 | 桌面恢复 | Hook 连接 | 过滤完成 | 连接到过滤 |
| --- | ---: | ---: | ---: | ---: |
| 仅补首次发布 | 0.68 s | 13.69 s | 13.90 s | 0.21 s |
| 首次发布 + 快速退避重试 | 0.68 s | 4.32 s | 4.54 s | 0.22 s |

最终运行 `target/release/lucidpane.exe`，PID 36696。最终日志记录启动期间桌面视图暂不可用和
一次握手未确认，随后自动恢复；不能将两轮差值全部视为确定的性能加速比例。
采样记录可见 ACK 从 0 到 1，桌面条目从 85 到 80，持续稳定至少 2 秒；再用只读 Shell 快照
确认 80 项。5 个面板标题集合一致、应用响应正常。

测试：24 项 hybrid 测试、5 项 runtime 测试通过，另有 1 项实时只读桌面快照通过。新测试覆盖
完整/部分/空图标缓存的隐藏集合、已释放条目不因缓存被误隐藏、失败重试上限及重连重入保护。
实际重启测试覆盖新 hook 接收首次成员发布的整条路径。

原始数据：`target/reconnect-first-pass.json`、`target/reconnect-first-pass-timeline.json`、
`target/reconnect-final-boundary.json`、`target/reconnect-final-filter-timeline.json`、
`target/reconnect-final-stderr.log`、`target/reconnect-final-regression.log`、`target/reconnect-runtime-tests.log`。

漏通知后的兜底发现时间现在可能达到约 30 秒。已收到的变更通知、菜单事务和失败重试不等待
该间隔。30 秒是此次保守选择，不是对所有桌面环境的性能最优值承诺。
短于巡检间隔的计时可能没有样本；脚本此时返回次数 0、平均耗时 null，不能解释为所有 hook 路径均无消耗。

### 统一巡检后的复测

最终运行 `target/release/lucidpane.exe`（本轮 PID 83192），同一组 5 个面板，32 个逻辑处理器。
启动后等待退避进入稳定区间，再依次测量显示与隐藏状态，各约 60 秒。

| 场景 | 整机 CPU | 单核 CPU | 私有内存 MiB（起→止） | GDI | 句柄（起→止） | Hook 扫描次数 / 累计耗时 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 显示 | 0.011% | 0.361% | 105.04→104.94 | 57→57 | 1034→1027 | 2 / 6.923 ms |
| 隐藏 | 0.034% | 1.085% | 104.94→104.97 | 57→57 | 1027→1030 | 5 / 35.882 ms |

Hook 计数包含事件触发的扫描，未按触发来源细分，因此隐藏阶段的 5 次不能全部归为定时巡检。
显示/隐藏是顺序采样，有系统活动波动；不能据此判断隐藏导致更高消耗，也不能把前后 CPU 差值
全部归因于单项优化。此次未重新测量 GPU，第一轮 GPU 数据不作为本轮验证结果。
最终 10 次通知均在约 15～47 ms 内观测到扫描完成，所有面板已恢复显示。

原始数据：`target/perf-event-visible.csv/json`、`target/perf-event-hidden.csv/json`、
`target/perf-hook-event-visible.json`、`target/perf-hook-event-hidden.json`、
`target/event-crash-recovery.txt`、`target/event-desktop-before-crash.log`、
`target/event-desktop-after-crash.log`、`target/event-hook-tests.log`、`target/event-audit-tests.log`。

## 第一轮 Release 实测（下列为改成统一巡检之前的数据）

本机 32 个逻辑处理器。最终运行 `target/convergence-check/release/lucidpane.exe`，
同时构建并加载对应的 `desktop_hook.dll`。配置包含 4 个普通分组（其中两个在同一
面板的不同标签中）、1 个文件夹面板和 1 个搜索面板，使用 Mica Alt。
实际显示 5 个面板窗口；隐藏对照也隐藏了其独立输入窗口，共 6 个 HWND，结束后全部恢复。

| 场景 | 采样时间 | 主进程 CPU（整机） | 私有内存 MiB（起→止） | 工作集 MiB（结束） | GDI（起→止） | 句柄（起→止） |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 面板显示、静止 | 60.53 秒 | 0.083% | 106.04→105.86 | 119.16 | 57→57 | 1049→1034 |
| 全部面板隐藏、后台运行 | 60.51 秒 | 0.072% | 105.86→105.86 | 119.17 | 57→57 | 1034→1034 |

单核口径分别为 2.66%、2.30%。10 轮 GPU 采样，主进程的 15 个引擎实例共 150 条记录均为 0%；
此值不包括 DWM 合成和 Explorer 的 GPU 消耗。隐藏测试不是关闭/释放面板测试，资源保留符合预期。
这些短期采样没有出现持续资源增长，不能据此排除数小时运行、频繁操作或其他布局下的问题。

## 本次修正

`filter/engine.rs` 原来每秒在 Explorer 桌面 STA 上完整枚举并核对成员。
保留每秒检查宿主存活的 watchdog，仅把无事件时的兜底扫描间隔改为 8 秒。
客户端请求、Shell 列表变更、读取重试、事务释放和已排队任务均绕过间隔限制。
读取快照的一致性校验、写入失败处理和退出时恢复桌面逻辑没有删减。
如果某次 Shell 通知缺失，兜底发现的延迟可能达到约 8 秒；正常事件不等待兜底。

| Hook 后台成员扫描 | 时间 | 次数 | 扫描累计耗时 | 平均单次 | 耗时/测量时长 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 修改间隔前 | 45.00 秒 | 45 | 172.74 ms | 3.84 ms | 0.384% |
| 修改后，面板显示 | 60.64 秒 | 8 | 66.27 ms | 8.28 ms | 0.109% |
| 修改后，面板隐藏 | 60.57 秒 | 7 | 19.23 ms | 2.75 ms | 0.032% |

按时间归一化，空闲扫描频率降低约 87%。这里是 Explorer 内扫描函数的墙钟耗时，
不是整机 CPU，也不覆盖菜单、输入钩子、事务请求或绘制前恢复等其他路径。
前后单次耗时存在系统调度波动，不能声称单次扫描加速。
Explorer 整体 CPU 在脱离 hook 时也存在明显波动，因此不采用 Explorer 总 CPU 作为 hook 的归因指标。

前一轮设置材质预览缓存优化已包含在运行版本中。本轮未发现需要继续修改面板渲染循环的证据。

## 验证与复测

- `desktop-hook` Release 单元测试：19 通过，4 个需交互环境的测试默认忽略。
- 单独执行只读实时桌面快照测试：1 通过。
- 连续 10 次 `WM_SETTINGCHANGE` 通知，每次均触发扫描；通知到观测到扫描完成约 16～45 ms。
  测试使用有超时保护的 `SendMessageTimeout`，不改变真实系统设置。此延迟包括跨进程调用和采样等待。
- 最终计时脚本自检：10 秒捕获 1 次扫描，约 3.03 ms。

可重复采样：

```powershell
cargo build --release -p lucidpane -p desktop-hook --locked --offline
# 正常退出旧版，再启动以上构建的 exe，使 DLL 一并更新。
./tools/measure-hook.ps1 -Seconds 60 -OutputPath target/hook-measurement.json
cargo test --release -p desktop-hook --lib --locked --offline
cargo test --release -p desktop-hook --lib --locked --offline filter::items::tests::live_filter_snapshot_reads_without_mutating_desktop -- --ignored --exact --test-threads=1
```

脚本必须在同一个交互桌面、具备读取 Explorer 窗口属性的权限下运行。
计数使用 `LucidPane.Filter.Perf.*` 窗口属性，默认关闭；脚本在 `finally` 中关闭并清理，
hook 卸载时也清理。未启用时不计时、不收集文件名、不记录持久日志。
请勿同时运行多份计时脚本，也不要在空闲采样时编译或进行压力测试。

本机原始记录保存在忽略的 `target/` 下：`perf-hook-before.json`、
`perf-hook-final-visible.json`、`perf-hook-final-hidden.json`、
`perf-final-visible.csv/json`、`perf-final-hidden.csv/json`、`perf-final-gpu.csv`、
`perf-hook-events.json`、`perf-hook-tests.log`、`perf-hook-live-test.log`。
