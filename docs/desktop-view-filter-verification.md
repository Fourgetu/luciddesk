# 桌面视图过滤验证

验证日期：2026-09-14。仅在当前 Windows 11 25H2 x64、26200.9168 上实测。

## 结论

通过 Explorer 桌面线程内的 `IShellFolderView::RemoveObject`，可以让 Shell 视图和原生 ListView 同时从 85 项变为 84 项，磁盘文件保留。当前桌面开启自动排列，移除中间一项后，其余 42 项由原生视图补位。补位后的图标命中和 ListView 选择与 Shell 项目身份一致。

一次移除不会持久过滤：调用 `IShellView::Refresh` 后，该项目重新进入视图，数量恢复为 85。

以下单项实验是集成前的初验；当前应用集成结果见下文。实验没有加载现有 `desktop_hook.dll`、安装五个几何 detour、查询固定 RVA 或下载 PDB。使用 Windows `WH_GETMESSAGE` 线程 Hook 加载诊断 DLL，再调用 COM 接口。

## 接口发现

| 查询位置 | IShellFolderView | IFolderFilterSite |
| --- | --- | --- |
| 进程外桌面视图 | E_NOINTERFACE | E_NOINTERFACE |
| Explorer 桌面线程内视图 | 成功 | E_NOINTERFACE |
| Explorer 桌面线程内浏览器 | 未查询 | E_NOINTERFACE |

最初尝试同步 `WH_CALLWNDPROC` 回调时，COM 返回 `RPC_E_CANTCALLOUT_ININPUTSYNCCALL`（0x8001010D）。改为 `WH_GETMESSAGE` 配合 `PostMessageW` 后成功。不能只根据进程外 QueryInterface 失败判断 Explorer 内部不支持。

## 实测结果

选取零基索引 42 的现有快捷方式；不删除、移动或改写文件。

| 检查 | 移除并恢复 | 移除、刷新并恢复 |
| --- | --- | --- |
| 移除返回索引 | 42 | 42 |
| 移除后 Shell / ListView 数量 | 84 / 84 | 84 / 84 |
| 指定 PIDL 不在视图内 | 通过 | 通过 |
| 其余 84 项身份集合完整 | 通过 | 通过 |
| 目标磁盘路径仍存在 | 通过 | 通过 |
| 原生补位项目数 | 42 | 42 |
| 补位后索引 42 的原生命中与 Shell 选择 | 通过 | 通过 |
| 刷新后项目重新出现 | 未执行刷新 | 是，85 / 85 |
| 恢复后身份、坐标、选择及焦点 | 全部一致 | 全部一致 |

`AddObject` 恢复时返回索引 84，因此探针按保存的 PIDL 和坐标调用 `IFolderView::SelectAndPositionItems` 恢复布局，不能假定重新加入自动恢复原位置。两次实验均未更改自动排列设置；最终数量为 85 / 85，文件夹标志仍为 `0x40200225`，Explorer PID 保持不变，诊断 DLL 已卸载。

## 复现

先退出 LucidPane 和其他会同步桌面项目/布局的工具，在桌面空闲时运行。探针选择中间项目；若不是可解析且存在的文件系统项目则拒绝修改。

```powershell
cargo build -p desktop-shell --example desktop_filter_probe --example desktop_filter_probe_hook
# 只读：进程外 / 进程内接口发现
.\target\debug\examples\desktop_filter_probe.exe
.\target\debug\examples\desktop_filter_probe.exe --in-process
# 可恢复的实际操作
.\target\debug\examples\desktop_filter_probe.exe --in-process --remove-restore
.\target\debug\examples\desktop_filter_probe.exe --in-process --remove-refresh
```

需要正常交互用户权限访问 Explorer COM；受限沙箱下可能返回 E_ACCESSDENIED，不代表接口不支持。日志位于 `target/desktop-filter-inprocess.log`，每次覆盖。修改前 PIDL/坐标/选择备份位于 `target/desktop-filter-baseline.txt`。异常时恢复守卫会重试并写入 `target/desktop-filter-recovery.log`。此守卫不能覆盖 Explorer 进程崩溃；探针没有执行 Explorer 重启实验。

## 单项初验时的待验证项（历史）

- 尚未测试 Win10、其他 Win11 构建、关闭自动排列、多屏/DPI、完整鼠标拖放、右键菜单、重命名及应用并行运行。
- 尚未验证在首次枚举/后续新增之前阻止项目进入集合。本次验证的是从已存在的视图集合移除项目。
- 持久方案需要处理刷新、文件通知、Explorer 重启后重新过滤，以及可能的短暂闪现。持续重新移除是否引起排序或选择扰动仍需测试。
- LucidPane 的完整项目来源应独立于已经过滤的视图枚举，否则会把面板内项目误判为消失。
- 可以据此研究替代当前几何 Hook 的后端，但不能直接认定五个 Hook 的全部职责都已被替代。

微软已将 [IShellFolderView](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nn-shlobj_core-ishellfolderview) 标记为 Windows 7 起不再提供使用；本机仍实际实现它。[RemoveObject 文档](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-ishellfolderview-removeobject) 明确提醒数据源可随时重新加入项目。因此应通过能力探测决定是否启用，不能据本机成功承诺所有系统兼容。

## 从 main 基线接入应用后的验证

按用户要求先恢复到 `main` 的 `f37656d`，确认工作区干净后重新接入。应用现在使用 `FilterSession`，自绘桌面文件不在该基线中；分组窗口和原有菜单路径保留。旧几何实现仅供既有探针对照，应用不调用它。

新后端使用 `WH_GETMESSAGE`、有边界的异步 IPC、`IShellFolderView::RemoveObject/AddObject` 和控制进程监测。列表变化后重滤，一秒定时器补查。主程序独立读取来源清单，避免过滤成员被当作删除；请求过期时丢弃其后台清单结果。OLE 收纳提交排队执行，避免在 Explorer 等待 Drop 返回时同步等候 Explorer。

生产后端回归 `filter_backend_probe` 已通过：

- 两个文件系统项目：85 → 83，剩余身份集合正确，原文件仍存在。
- 独立桌面来源仍能枚举两个已过滤项目。
- 菜单暂停恢复到 85，退出暂停后重新变为 83。
- Explorer 刷新后仍保持 83。
- 逐项释放、全部释放，以及全部原坐标恢复。
- 虚拟桌面项目移除和恢复。
- 测试控制进程直接退出、不运行 Rust 析构后，定时监测恢复为 85。

坐标恢复必须按原视图顺序批量提交；按 AddObject 产生的新顺序提交会在自动排列模式下交换相邻项目。已用真实桌面回归确认修正。

正式 Debug 应用也使用 `target/filter-smoke-clean/workspace.db` 独立配置测试：启动和刷新后都是 83 个原生项目，持久化清单始终为 85，分组成员为 2；正常退出后恢复到 85，测试实例已关闭，应用 stderr 为空。未改写实际用户配置。

回退基线后发现旧构建产物与源码不一致，清理本地工作区包缓存后重新构建，最终结果为：

- 应用单元测试：159 通过、14 默认跳过。
- Hook 库测试：13 通过。
- Shell 库测试：6 通过、2 默认跳过。
- `cargo check --workspace --all-targets` 通过；示例仍有原有未使用成员警告。
- `cargo build --release --locked -p lucidpane -p desktop-hook` 通过。

应用日志及测试输出位于 `target/filter-smoke-clean/`、`target/filter-app-tests-clean.log` 和 `target/filter-all-targets.log`。

上述结果限当前 Windows 11 x64 26200.9168。Win10、其他 Win11 构建、多屏/DPI、完整人工鼠标拖放、实际菜单/Peek 弹出，以及 Explorer 进程重启尚未完成本轮覆盖。刷新重滤允许短暂显示原生项目；不宣称已经实现首次枚举前过滤或绝对无闪现。

### Pane 右键期间原生图标重新出现的修正

原实现只发送一次 `WM_SETREDRAW(FALSE)`，随后为原生菜单恢复视图成员；没有保护 Explorer 在加入项目、选择或激活菜单时重新启用重绘的路径。数量回归可以通过，但不能证明菜单期间未绘制这些成员。

现在用独立于 COM 状态借用的线程局部重绘保护，在同步重入时也拦截 `WM_SETREDRAW(TRUE)`，并阻止绘制、擦除及 ListView 的 `NM_CUSTOMDRAW/CDDS_PREPAINT` 默认绘制。结束菜单时先重新过滤，再解除保护；退出和失联恢复继续解除保护。原生菜单路径保持不变。

新增真实 Win32 窗口测试验证：状态被可变借用时、菜单消息循环期间均不能重新启用绘制，自定义绘制返回跳过标志，解除保护后恢复绘制。Hook 测试为 14 项通过，真实桌面过滤/菜单暂停恢复/刷新/坐标恢复/控制进程退出回归再次通过，Release 构建通过。实际 Pane 弹出菜单的视觉效果仍需人工复核，不能用数量回归替代。

参考：[WM_SETREDRAW 的启停和窗口状态语义](https://learn.microsoft.com/en-us/windows/win32/gdi/wm-setredraw)。公共控件可自行处理该消息，不能假定所有控件都提供 `SysSetRedraw` 属性；测试使用直接转发到 `DefWindowProc` 的可见窗口作观测。
