# 原生 Hook 后端验证与限制（2026-09-08）

> 历史记录：以下结果对应原记录日期和当时的代码、系统映像。旧探针与启动入口可能已删除；当前命令见[构建与验证](../build.md)，当前边界见[架构说明](../architecture.md)。

## 当时的方案

当时验证的是“Explorer 绘制全部图标、LucidPane 只管理分组框”的实验几何后端。它在经过校验的系统映像上通过 MinHook 修改内存入口，不修改磁盘上的 Explorer 二进制，不移动文件，不关闭自动排列，也不注册永久 Hook。

这一方案随后被当前混合桌面替代。旧工作区后端、`app/src/hook_desktop.rs`、`--hook-desktop` 入口及相关探针均不是当前主线接口。

## 映像与几何证据

| 项目 | 当时记录 |
| --- | --- |
| 公共控件映像 | WinSxS `6.0.26100.8972` x64 |
| 大小 | 2,688,512 字节 |
| FNV-1a 64 | `b4fc8de893e82e37` |
| 图标矩形入口 | `CLVIconView::GetRectsOwnerData`，RVA `0x16eb0` |
| 命中入口 | `CLVIconView::v_ItemHitTest`，RVA `0x16260` |
| 位置入口 | `CLVView::OnGetItemPosition`，RVA `0x24524` |

当时安装前校验这三个入口的指令。它们属于特定版本的私有实现，不是微软承诺稳定的扩展接口；后续插入标记入口以当前源码配置为准。

独立 v6 虚拟 ListView 验证了平移前后的图标、文字、选择像素一致，命中位置一致，自动排列不变，其他视图不受影响，以及分阶段发布、分离与重新连接。

旧跨进程几何探针验证了 DLL 与有边界的 IPC，连续提交 120 次几何更新并核对命中。隐藏夹具单次约 0.15 ms，不代表真实桌面帧率。后续改成一次 `WM_COPYDATA` 批次传送最多 512 个映射项目，并验证过期标签会拒绝整批发布。

## 性能与交互演进

- 原生变更使用独立代数，枚举前后核验，应用自身提交不会吞掉并发清单变化。
- 当时的原生分组框通过 `WM_MOVING`/`WM_SIZING` 更新映射，接受位置前提交并绘制，移动结束后保存配置。
- 原生绘制仅标记旧位置、新位置及基线中受影响的图标、文字、选择与阴影区域；无变化映射不产生新的脏区。
- 可见夹具局部 `WM_PRINTCLIENT` 输出与完整参考图一致，更新区域不包含无关桌面点。
- 当时可用 `LUCIDPANE_TRACE_LAYOUT=1` 每 60 次更新记录控制端中位数、P95、最大耗时与 IPC/绘制均值；不记录名称或路径。
- 第二轮优化后，用户确认更流畅。真实桌面批次中位数约 9–24 ms，部分区间 P95 为 35–62 ms，主要耗在原生 IPC/绘制；不宣称恒定刷新率。
- 实际桌面几何探针检查 81 个项目，命中通过并恢复 81 项原位置，Explorer 保持运行。拖入和拖回曾得到确认，后续移动延迟仍继续排查。

## 生命周期

当时已具备线程级 `WH_CALLWNDPROC` 引导、无远程指针的请求协议、控制进程存活监测和分离清理。控制端崩溃后 DLL 仍保持映射，分离撤销回调和定时器，避免卸载正在执行的代码。按内容版本保存 DLL 副本，避免覆盖已加载文件。

独立存活监测夹具验证控制窗口销毁后撤销映射。这是特定夹具结果，不等于覆盖所有 Explorer 退出与重启路径。

## 早期工作区实验故障

2026-09-08 约 21:20，真实桌面工作区实验导致 Explorer 在 `comctl32.dll` 中访问冲突退出。系统报告版本 `6.10.26100.8972`、故障偏移 `0x106c6`，随后 Windows 重启 Explorer。未分析崩溃转储，因此不能将其写成指令级根因结论。

最初夹具是普通 ListView，遗漏了真实桌面的 `LVS_OWNERDATA` 条件。[ListView 兼容表](https://learn.microsoft.com/en-us/windows/win32/controls/list-view-controls-overview#compatibility-issues)将 `LVM_GETWORKAREAS`、`LVM_SETWORKAREAS` 和 `LVM_SETITEMPOSITION` 列为该模式不支持的消息。普通列表测试成功不能证明虚拟桌面可用，也不应动态移除 `LVS_OWNERDATA`。

恢复检查发现项目数仍为 81，通过 `IFolderView2::SetCurrentFolderFlags` 恢复并读回自动排列。快照记录 HWND 198012、样式 `0x56003b40`、扩展样式 `0x14c14c30`，虚拟列表与自动排列均启用。Explorer 重启前的精确位置没有完整恢复。

本地证据为 `target/debug/hook-layout-before.txt` 和 `hook-layout-recovered.txt`。它们是未纳入版本控制的桌面元数据，不应作为公共文档附件。

## 位置回调研究

对活动 `IShellView` 查询 `IOwnerDataCallback` 返回 `E_NOINTERFACE`。回调属于列表宿主，不必由公开 Shell 视图直接暴露。

`tools/explorer_symbols.py` 下载匹配的公共 PDB，核对 PE RSDS GUID 和 DBI age，只读取文件，不注入 Explorer。当时 shell32 符号记录：

- `CListViewHost::GetItemPosition(int, POINT*)`：RVA `0x4d290`。
- `CListViewHost::SetItemPosition(int, POINT)`：RVA `0x35ed0`。
- `IOwnerDataCallback` 虚表：RVA `0x60d418`。
- PDB GUID：`4907816C76ABD6288BBE01D3E8033EE9`，DBI age 1。

这些偏移仅是该映像的研究证据，没有据此安装新的运行期补丁。

独立虚拟列表可以注册回调并增加引用计数，但自动与手动夹具都未在当时配置下调用位置回调。离线检查发现自动排列会绕过手动位置路径，并存在其他激活条件，不能把注册成功当作布局后端完成。[早期回调 ABI 研究](https://www.geoffchappell.com/studies/windows/shell/comctl32/controls/listview/interfaces/iownerdatacallback.htm)来自 Vista 时期，不保证 Windows 11 兼容。

## 后续使用限制

当时的 `hook_probe`、`desktop_probe` 和 `hook_layout_probe` 已在兼容层收敛时删除。保留本记录用于追溯故障与决策，不提供绕过校验的复现命令。

当时跨进程夹具在受限会话中出现“Hook 注册成功但回调未投递”，在实际桌面权限下才完成验证。这与夹具是否指向 Explorer 是两个独立条件。

真实拖动、多选、取消、键盘导航、混合 DPI 和菜单仍需按当前实现持续回归。系统映像变化后必须重新审查配置，不宣称完整 Fences 等价或普遍 Windows 兼容。
