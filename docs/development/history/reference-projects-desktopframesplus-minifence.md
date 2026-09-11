# DesktopFramesPlus 与 MiniFences 技术路线调研及 LucidPane 借鉴建议

> 历史资料：本文记录当时的方案、实验与测量，不代表当前主线能力。旧路径、启动参数和已删除探针仅用于追溯；当前实现见[架构说明](../architecture.md)，可执行命令见[构建与验证](../build.md)。

> **2026-09-07 更正：本文的“保留接管与稀疏图标表面路线”建议已撤回。**
> 用户明确要求保留原生桌面外观和交互，只增加分组框。隐藏 Explorer 后重绘全部图标，
> 即使使用系统图标资源，也不满足该要求。下文保留为历史调研，不能继续作为默认实现依据。
> 当前默认使用无图标绘制的 `NativeFrame`，原生位置操作通过公开的桌面 Shell `IFolderView2`
> 接口进行，不读写 Explorer 进程内存。接口来源：[Microsoft 原生桌面图标位置示例](https://devblogs.microsoft.com/oldnewthing/20130318-00/?p=4933)。
> 框中心从窗口 region 中扣除，保留原生鼠标输入；整块背景填充仍需另行实现，不能用覆盖层染暗图标。

> 调研日期：2026-09-05
> DesktopFramesPlus 基线：`ef0edc14ecd7f32323a7a9708bb38596963c16bb`
> MiniFences 基线：`22128adb3ed29d04626ba903f7b077b61794baa5`

## 1. 结论摘要

这两个项目对 LucidPane 的价值不同：

- **DesktopFramesPlus 更适合做产品能力参考**：它已经覆盖 Frame 类型、Folder Portal、Tabs、Workspace Profile、进程触发的 Profile Automation、自动分类、搜索、备份、导入导出和插件等完整产品面。
- **MiniFences 更适合做 Windows 交互与可靠性参考**：它对原生 Shell 右键菜单、OLE 拖放、跨盘文件操作恢复、显示器拓扑、桌面层兼容、崩溃恢复、自动更新校验及测试矩阵投入较多。
- **LucidPane 不应替换现有底层路线**：当前以完整 Desktop Shell Namespace、稳定 `ShellIdentity`、Shell-owned 顶层 HWND、每屏稀疏桌面表面和 SQLite 事务快照为核心的设计，比两者基于 WPF、`Progman`/`WorkerW`、`SysListView32` 的方案更适合作为长期基础。

因此，推荐策略是：

1. 保留 LucidPane 的 Shell 身份、窗口宿主、稀疏命中区域和持久化模型。
2. 优先吸收 MiniFences 的 Shell 交互闭环、操作日志、显示拓扑恢复和测试方法。
3. 再吸收 DesktopFramesPlus 的 Tabs、Profiles、Portal 导航、备份导入导出等产品能力。
4. 只参考行为和模块边界，不照搬两个项目的窗口挂载、巨型 Manager 或动态 JSON 数据模型。

## 2. 三个项目的路线对照

| 维度 | DesktopFramesPlus | MiniFences | LucidPane 当前路线 | 判断 |
| --- | --- | --- | --- | --- |
| 技术栈 | .NET 8，WPF + WinForms | .NET 8，WPF + WinForms 互操作 | Rust + Win32 + Windows Composition + SQLite | LucidPane 保持原路线 |
| 桌面容器 | 每个 Frame 一个透明、无激活顶层 WPF Window | 一个覆盖虚拟桌面的 WPF Window，内部 Canvas 承载 Fence 和散落图标 | 每显示器一个稀疏 `DesktopItemSurface`，每 Pane 一个独立 HWND | LucidPane 的隔离性和多屏模型更好 |
| 桌面层集成 | 不嵌入 Explorer；普通顶层窗口，按需 Topmost | 默认 `SetParent` 到 `Progman`/`WorkerW`；新系统构建使用顶层 owner + Z-order 修复 | `GetShellWindow` 作为 popup owner，不转为 child window | 不采用 WorkerW reparent |
| 原生图标处理 | 查找 `SHELLDLL_DefView/SysListView32` 后 `ShowWindow` 隐藏 | 同样隐藏 Explorer ListView，并由独立 watchdog 恢复 | `SHGetSetSettings + SSF_HIDEICONS`，退出守护进程和磁盘接管标记 | LucidPane 更稳，继续使用公开 Shell 状态接口 |
| 桌面数据源 | 主要是路径、快捷方式和 Portal 目录 | 用户桌面 + 公共桌面路径，并补充部分 Shell 系统项 | 从 `SHGetDesktopFolder` 枚举完整 Shell Namespace | LucidPane 更完整 |
| 项目标识 | 主要依赖路径和动态 JSON | 主要依赖路径、显示名和部分 Shell parsing name | 文件使用 Volume ID/File ID，虚拟对象使用 parsing name | LucidPane 更适合处理重命名和同名项 |
| Shell 右键菜单 | 以自定义菜单为主 | 已实现 `IContextMenu/IContextMenu2/IContextMenu3` 消息转发 | 尚未实现 | 优先借鉴 MiniFences |
| 拖放 | 自定义 WPF 拖放和预览 | WPF 拖放 + COM `IDataObject` 兼容层 + Shell drag helper | 当前是应用内拖动，完整 OLE 拖放未完成 | 优先借鉴 MiniFences 的协议边界 |
| 渲染 | WPF 控件树、图标缓存、后台懒加载 | WPF 控件树、异步图标、虚拟化 WrapPanel | 当前 GDI PoC，目标 D2D/DWrite/DirectComposition | 借鉴异步与虚拟化策略，不借 WPF 实现 |
| 存储 | Profile 目录中的 JSON 和快捷方式资源，强调 portable | `%APPDATA%` JSON、布局文件、操作历史 | SQLite schema v5，事务式全量快照 | 保持 SQLite，增加版本化子模型 |
| 多显示器 | 有屏幕边界与吸附处理 | 按显示组合、bounds、DPI 建 topology key 并保存布局 | 已有稳定显示器 ID、DPI、显示器消失迁移 | 借鉴 topology profile 与 remap |
| 文件操作安全 | 自动分类可直接移动真实文件，提供冲突策略 | 操作日志、跨盘安全移动、部分完成状态和崩溃恢复 | Managed Desktop 分组只改元数据；真实 Folder Portal 操作尚未完整实现 | 在引入真实文件操作前先做 journal |
| 测试 | 仓库中未见独立测试项目 | 约 3000 行 SmokeTests + VisualHarness | 已有 Rust 单元及 Win32 集成测试 | 扩充为 MiniFences 风格的场景矩阵 |

## 3. DesktopFramesPlus 技术路线

### 3.1 窗口与桌面交互

DesktopFramesPlus 为每个 Frame 创建一个独立的 WPF `Window`：

- `WindowStyle=None`、透明背景、不显示在任务栏。
- 使用 `WS_EX_NOACTIVATE` 和 `WM_MOUSEACTIVATE -> MA_NOACTIVATE`，避免点击 Frame 抢走前台应用焦点。
- Frame 可选 `Topmost`；默认并未作为 Explorer 的 child window 嵌入 `WorkerW`。
- 为适配 Win+D，另有全局热键、桌面鼠标 hook 和 DWM cloaking 检测。
- 当需要隐藏桌面原生图标时，直接定位 `Progman/WorkerW -> SHELLDLL_DefView -> SysListView32`，调用 `ShowWindow`。

这条路线实现快、每个 Frame 独立，但窗口层级和 Win+D 行为需要大量补丁式处理。LucidPane 已经通过 Shell owner 语义和独立 Pane HWND 达到相同产品形态，无需改成该方案。

### 3.2 内容模型

DesktopFramesPlus 的主要内容类型包括：

- Data Frame：保存自定义快捷方式和引用。
- Portal Frame：镜像真实目录，支持导航、过滤和刷新。
- Note Frame：轻量文本内容。
- Tabs：一个 Frame 中承载多个内容页。
- Plugin Frame：通过 `IFramePlugin` 扩展计算器、终端、系统性能等内容。

其价值在于产品分类清晰。但具体实现大量依赖 `dynamic`、`JObject/JArray` 和静态 Manager；核心 `FrameManager.cs` 超过 8000 行，不适合作为 LucidPane 的代码组织模板。

### 3.3 数据、Profiles 与自动化

DesktopFramesPlus 使用应用目录下的 Profile 数据，主要以 JSON 保存：

- 每个 Profile 有独立 `frames.json`、选项、快捷方式和备份目录。
- Profile 切换会关闭当前 Frame，重新装载目标 Profile。
- Automation 每秒读取前台进程，按规则切换 Profile，并记录自动切换来源，避免覆盖用户手动切换。
- 定时自动备份、单 Frame 导入导出和旧字段迁移已经形成完整用户流程。

这套产品语义值得采用，但 LucidPane 应将其建模为强类型实体并存入 SQLite，而不是复制目录和动态 JSON。

### 3.4 自动整理

DesktopFramesPlus 使用 `FileSystemWatcher` 监控桌面目录，按扩展名/规则匹配后把真实文件移动到目标目录；它实现了：

- 规则优先级。
- Rename / Overwrite / Skip 冲突策略。
- 最长 60 秒的文件解锁等待。
- 移动完成后自动创建或刷新 Portal。

这里可以借鉴规则匹配、冲突预览和 Portal 联动，但不能把“自动移动真实文件”作为 LucidPane 默认行为。默认应只修改 `DesktopPlacement`；真实移动必须显式启用，并进入可恢复的操作日志。

### 3.5 适合借鉴的产品能力

1. Tabs 的完整交互：排序、溢出、拆出和重新合并。
2. Workspace Profiles 及前台应用触发的自动切换。
3. Portal 的面包屑、过滤、内部导航和空白占位。
4. Snap to Dimension、边缘吸附和尺寸反馈。
5. SpotSearch：跨 Pane/Tab 的统一搜索和启动。
6. 单 Pane 导出、整 Profile 备份、导入预检和迁移提示。
7. Pane 级自动隐藏、roll-up、focus mode 和 Peek。

## 4. MiniFences 技术路线

### 4.1 单桌面窗口与稀疏命中区域

MiniFences 创建一个覆盖整个虚拟桌面的透明 WPF Window，所有 Fence 和散落桌面图标都放在同一个 `Canvas` 中。

桌面挂载有两套路径：

- 默认把 HWND 从 `WS_POPUP` 改为 `WS_CHILD`，通过 `SetParent` 挂到包含 `SHELLDLL_DefView` 的 `Progman` 或 `WorkerW`。
- Windows build 26200 及以后默认使用顶层兼容模式：保持 `WS_POPUP`，把 Explorer desktop host 设为 owner，并周期性修复 Z-order、DWM cloak 和 Win+D 状态。

为避免透明全屏窗口吃掉桌面输入，它用 `SetWindowRgn` 把原生窗口区域裁成所有可见 Fence 和散落图标矩形的并集；拖动期间临时恢复全屏 region。这一点与 LucidPane 当前每屏稀疏表面的设计目标一致，可以参考它的状态切换和测试用例，但不应合并为单个全屏窗口。

### 4.2 Explorer 图标替换与恢复

MiniFences 在启用 Desktop Group 后：

1. 枚举用户桌面、公共桌面及部分虚拟 Shell 项目。
2. 在 WPF 中重新绘制已分组与未分组图标。
3. 隐藏 Explorer 的 `SysListView32`。
4. 启动独立 `--desktop-icon-watchdog` 进程；主进程退出或崩溃后，watchdog 重试恢复 Explorer 图标。

它还包含一个 `DesktopIconLayoutService`，会打开 Explorer 进程、分配远程内存并发送 `LVM_GETITEMPOSITION/LVM_SETITEMPOSITION`，用于读取或移动原生桌面 ListView 图标。

LucidPane 可借鉴独立 watchdog 的恢复思路，但已有更完整的守护进程、接管 marker 和 `--restore-shell`。远程读写 Explorer 进程内存和 ListView 消息属于高脆弱实现，不应采用。

### 4.3 Shell 右键菜单

MiniFences 的 `ShellContextMenuService` 是本次调研中最直接的参考点：

- `SHParseDisplayName` 生成完整 PIDL。
- `SHBindToParent` 获得共同父 `IShellFolder` 和 child PIDL。
- `GetUIObjectOf` 获取 `IContextMenu`。
- `QueryContextMenu` 填充菜单。
- 对 `IContextMenu2/IContextMenu3` 转发 `WM_INITMENUPOPUP`、`WM_DRAWITEM`、`WM_MEASUREITEM` 和 `WM_MENUCHAR`。
- 读取 canonical verb，把 rename 留给宿主内联编辑，其余命令交给 Shell `InvokeCommand`。
- finally 中释放菜单、COM 对象和 PIDL。

LucidPane 应复用这套生命周期和消息路由设计，但以 Rust RAII 包装 COM/PIDL/HMENU 资源。

### 4.4 OLE 拖放与 Shell 反馈

MiniFences 对 WPF 拖放做了多层兼容：

- 用同时实现 WPF `IDataObject` 和 COM `IDataObject` 的适配器承载数据。
- 支持 `Preferred DropEffect`、Copy/Move/Link 修饰键和 Shell `DropDescription`。
- 用 `IDragSourceHelper` 生成原生 Shell drag image。
- 用 `IDropTargetHelper` 接收来自 Explorer 的拖动图像和状态。
- 将“应用内调整归属”与“Portal 中真实复制/移动/创建链接”分成不同路径。

LucidPane 应借鉴它对协议格式和操作意图的拆分，不应把 WPF 适配层照搬到 Rust。

### 4.5 文件操作日志与恢复

MiniFences 为真实文件操作保存 action journal：

- Transaction 和 entry 有 Pending、InProgress、Completed、PartiallyCompleted、Undone、Failed 等状态。
- Move、Copy、Link、Fence 删除、成员归属变化、布局恢复分别记录。
- 跨盘移动按复制、验证、删除源的方式处理。
- 启动时扫描中断的操作，恢复源或清理未完成副本。
- Undo 按相反顺序执行，并在冲突时停止，不覆盖现有内容。
- 日志通过临时文件后原子替换写入。

这是 LucidPane 实现 Folder Portal 真实文件操作前最应该补齐的能力。由于 LucidPane 已使用 SQLite，推荐把 journal 放进同一数据库事务体系，而不是追加 JSON 文件。

### 4.6 显示器拓扑和恢复

MiniFences 根据以下信息生成显示 topology key：

- 显示器设备名。
- 虚拟桌面 bounds。
- DPI X/Y。
- 主显示器标记。

每种拓扑可保存独立布局。找不到精确布局时，它会按旧/新工作区尺寸 remap 并 clamp；还支持交换显示器内容。

LucidPane 当前能识别显示器、使用 monitor-local DIP 并把丢失显示器上的散落图标迁到主屏，但 Pane 的 monitor ID 和多拓扑布局还未完整持久化。应直接把 topology profile 纳入下一次 schema 演进。

### 4.7 测试方法

MiniFences 的 SmokeTests 覆盖了：

- 配置 round-trip 和布局迁移。
- Shell 图标、虚拟桌面项、右键菜单路径选择。
- 拖放 effect、Shell drag image、负坐标副屏和 DPI 热点。
- Explorer child / 顶层兼容模式选择。
- Win+D 和 DWM cloak 恢复判断。
- 多显示器 topology、DPI bounds 和 remap。
- 跨盘文件操作、部分完成、冲突和崩溃恢复。
- 600 项 Folder Portal 的控件虚拟化。
- 更新包路径穿越防护和 SHA-256 解析。

LucidPane 已有良好的纯 Rust 单元测试基础，下一步应按这些“用户场景故障”补充状态机和 Win32 集成测试。

## 5. LucidPane 已有优势，应继续保持

### 5.1 完整 Shell Namespace

LucidPane 从 `SHGetDesktopFolder` 出发，通过 `IShellFolder::EnumObjects` 枚举完整 Desktop Shell Namespace，不只是拼接用户和公共桌面目录。它天然覆盖虚拟对象，并避免两个来源中同名项冲突。

### 5.2 稳定身份和重命名保持

文件系统项保存 Volume ID 和 128-bit File ID；虚拟项保存 parsing name。重新枚举时按稳定 identity reconcile，因此文件重命名后可以保留 Pane 归属和位置。

### 5.3 不移动真实文件

Managed Desktop 中从自由桌面拖到 Pane 只修改 `DesktopPlacement`。这个边界应作为默认安全承诺继续保持。Folder Portal 未来执行真实文件操作时，应在 UI 和 domain model 中明确区分 `Assign` 与 `Copy/Move/Link/Delete`。

### 5.4 窗口模型

LucidPane 的自由桌面图标按显示器拆分为独立稀疏 HWND，Pane 也是独立 HWND；它们是 Shell-owned、非置顶的顶层 popup。相比单个虚拟桌面 WPF 窗口，这种模型更容易做到：

- 每屏独立 DPI。
- Pane 独立移动、缩放、折叠和 Z-order。
- 空白桌面不参与命中。
- Explorer 重启后重新绑定 owner。
- 某个 Pane 的绘制或输入问题不污染整个桌面区域。

### 5.5 事务持久化

SQLite schema v5 已经事务化保存 Pane、Shell identity 和 placement。后续 Profiles、Tabs、display topology、operation journal 都应延续强类型、可迁移、事务式的方向。

## 6. 建议借鉴项目清单

### P0：先补 Shell 交互闭环

#### 6.1 原生 Shell 右键菜单

参考：MiniFences `ShellContextMenuService`。

建议落点：

- `desktop-shell::context_menu`
  - `ShellContextMenuSession`
  - `Pidl`、`MenuHandle`、COM interface RAII
  - canonical verb 和 host-handled verb
- `desktop-window`
  - Pane 和 Desktop surface 的 `WM_CONTEXTMENU`
  - `IContextMenu2/3` owner-draw 消息转发
- `app`
  - rename 等宿主命令转换为 domain action

验收条件：

- 文件、文件夹、快捷方式和虚拟项均能显示原生菜单。
- Send To、Open With 等包含子菜单和 owner-draw 的扩展正常。
- 多选仅在共同父 ShellFolder 时显示菜单；不满足时安全降级。
- 菜单关闭后无 PIDL、HMENU 或 COM 引用泄漏。

#### 6.2 完整 OLE 拖入拖出

参考：MiniFences `ShellCompatibleDataObject`、`ShellDragSourceImage`、`ShellDropTargetBridge` 和 `DesktopDragData`。

建议落点：

- `desktop-core`
  - `DragIntent::Assign | Copy | Move | Link`
  - `DropTarget::FreeDesktop | Pane | FolderPortal | External`
- `desktop-shell::drag_drop`
  - COM `IDataObject`、`IDropSource`、`IDropTarget`
  - `CF_HDROP`、Shell IDList、Preferred DropEffect、DropDescription
- `desktop-window`
  - drag session、命中、跨 Pane 路由和反馈视觉

验收条件：

- Explorer 到 Pane、Pane 到 Explorer、Pane 间、Pane 到自由桌面都可工作。
- Ctrl/Shift/Ctrl+Shift 的 Copy/Move/Link 语义与 Explorer 一致。
- 纯归属变化绝不触发真实文件操作。
- 多选、取消、跨显示器和负坐标副屏可测试。

#### 6.3 Pane 的显示器归属和 topology profile

参考：MiniFences `DisplayLayoutService`。

建议 schema：

- `display_topologies(id, fingerprint, created_at, updated_at)`
- `display_members(topology_id, monitor_id, bounds_px, work_area_px, dpi_x, dpi_y, primary)`
- `panel_layouts(topology_id, panel_id, monitor_id, x_dip, y_dip, width_dip, height_dip, state)`

恢复策略：

1. topology fingerprint 精确命中时恢复独立布局。
2. 显示器 ID 仍存在但尺寸/DPI 变化时按 monitor-local DIP clamp。
3. 显示器消失时迁到主屏并保留相对位置。
4. 新 topology 首次出现时从最近兼容布局确定性 remap。

### P0：在真实文件操作前建立安全层

#### 6.4 Operation journal 与崩溃恢复

参考：MiniFences `ActionHistoryService`。

建议状态机：

```text
Planned -> InProgress -> Completed
                    \-> PartiallyCompleted
                    \-> Failed
Completed/PartiallyCompleted -> Undoing -> Undone/PartiallyUndone
```

建议落点：

- `desktop-core::operations`：计划、entry、状态、不变量。
- `desktop-shell::file_operation`：优先使用 `IFileOperation`。
- `desktop-storage`：operation 和 operation_entry 表，状态更新与 workspace 更新进入事务。
- `app`：启动恢复、用户确认、冲突呈现和 Undo。

原则：

- 先记录计划，再执行外部副作用。
- 每完成一个 entry 就提交进度。
- 跨盘 Move 按 Copy -> 校验 -> 删除源处理。
- Undo 不覆盖冲突目标。
- 诊断信息保存路径脱敏版本。

### P0：建立场景化测试矩阵

参考：MiniFences SmokeTests 和 VisualHarness。

建议新增：

- 纯逻辑测试：topology fingerprint、remap、drag effect、tab merge、operation 状态机。
- Shell 集成测试：PIDL 生命周期、IContextMenu 消息路由、Explorer 重启后重新绑定。
- Win32 测试：稀疏 region、不同 DPI、负坐标副屏、Win+D、显示器热插拔。
- 故障注入：文件操作每个阶段中断、SQLite 提交失败、Explorer 不可用、Shell extension 超时。
- Visual harness：生成固定 Pane、图标密度、DPI 和主题的可复现窗口。

### P1：渲染和大目录性能

#### 6.5 异步图标管线与虚拟化

参考：DesktopFramesPlus `LazyIconLoader/IconManager` 和 MiniFences 的 Folder Portal 虚拟化测试。

建议：

- 图标缓存 key 使用 `ShellIdentity + size + DPI + theme/icon generation`。
- 后台 STA 线程获取 Shell 图标，UI 线程只接收不可变图像资源。
- 可见区优先、滚动方向预取、离屏取消。
- Pane grid 使用 viewport virtualization，不为 600 个项目同时创建完整 render state。
- 保留 GDI 作为诊断 fallback，正式路径迁移到 D2D/DWrite/DirectComposition。

### P1：Tabs、Pages 和 Profiles

#### 6.6 Tabs

参考：两个项目的 Tabs 产品行为，数据模型优先参考 MiniFences 的强类型 `TabGroupId` 思路。

建议不要嵌套 Pane HWND。增加逻辑 `PaneGroup`：

```text
PaneGroup
|- group_id
|- ordered_pane_ids
|- active_pane_id
|- presentation
`- shared_rect / optional synchronized layout
```

同一 group 只显示 active Pane 的内容，标题栏渲染 tab strip；拆出时恢复 Pane 自己的 rect。

#### 6.7 Pages 与 Workspace Profiles

两者语义应区分：

- Page：同一 workspace 内的可见视图切换，切换成本低。
- Profile：一整套独立工作区，包括 Pane、规则、主题和页面。

建议先做 Page，再做 Profile。Profile Automation 可参考 DesktopFramesPlus 的“只在仍处于自动切换目标时回退”规则，避免覆盖用户手工选择。

### P1：Folder Portal 体验

参考：DesktopFramesPlus Portal 导航与 MiniFences Shell 交互。

建议能力顺序：

1. 面包屑、返回/前进、上一级。
2. Shell display name、图标、原生右键菜单。
3. 名称/类型过滤和隐藏项策略。
4. OLE Copy/Move/Link 与明确 drop description。
5. 大目录虚拟化和后台缩略图。
6. Shell notification reconcile；纯文件系统目录可保留 `FileSystemWatcher` 作为补充。

### P1：备份、导入导出与迁移

参考：DesktopFramesPlus 的用户流程，不复制其 raw JSON/目录覆盖实现。

LucidPane 建议导出格式：

```text
lucidpane-export.zip
|- manifest.json       # format version、app version、created_at、hashes
|- workspace.sqlite    # 一致性快照或裁剪后的逻辑数据库
`- assets/             # 用户自定义 Pane 图标等可携带资源
```

导入必须先校验 schema、路径 containment、哈希、重复 Pane ID 和缺失资源，再以新 Profile 或 merge plan 的形式提交。

### P2：自动分类、搜索和发布能力

#### 6.8 自动分类

- 默认动作应为 metadata assignment。
- 真实 Move 作为显式规则动作，并显示 dry-run 预览。
- 规则支持优先级、扩展名、名称 glob、时间、来源目录和冲突策略。
- 所有真实操作接入 operation journal。

#### 6.9 全局搜索

参考 DesktopFramesPlus SpotSearch，索引 Shell display name、路径、Pane、Tab、Profile 和最近使用；启动仍通过 Shell。

#### 6.10 更新与诊断

参考 MiniFences：

- Release manifest 和 SHA-256 校验。
- 防 zip-slip/path traversal。
- 更新后健康标记和失败回滚。
- 诊断包默认路径脱敏。

这些能力应在核心 Shell 交互和布局可靠性稳定后再做。

## 7. 明确不建议采用的实现

### 7.1 不使用 `Progman/WorkerW + SetParent` 作为主宿主

原因：

- 依赖 Explorer 未公开的窗口树结构。
- WPF 透明窗口从 popup 切回 child 后可能停止绘制。
- Windows 新构建已经迫使 MiniFences 增加另一套顶层兼容模式和周期修复。
- LucidPane 已有更简单的 Shell-owned top-level popup 模型。

### 7.2 不远程读写 Explorer 的 `SysListView32`

不采用 `OpenProcess + VirtualAllocEx + LVM_GET/SETITEMPOSITION`。这会把布局正确性绑定到 Explorer 内部控件实现、进程权限和消息格式。LucidPane 应继续自己拥有 Desktop View 与 Layout。

### 7.3 不以 `ShowWindow(SysListView32)` 作为唯一恢复机制

保留 LucidPane 当前 `SHGetSetSettings`、独立守护进程、磁盘 takeover marker 和手动 `--restore-shell` 的组合。

### 7.4 不引入动态 JSON 领域模型和巨型 Manager

DesktopFramesPlus 的 `dynamic/JObject` 和超大静态 `FrameManager`，MiniFences 的超大 `MainWindow.xaml.cs` 都说明：功能增长后，如果 UI、Shell、持久化和业务规则集中在同一文件，回归成本会快速上升。

LucidPane 应继续保持：

- `desktop-core` 不依赖 Win32。
- `desktop-shell` 只处理 Shell/COM。
- `desktop-window` 只处理 HWND、渲染和输入。
- `desktop-storage` 只处理 schema、迁移和事务。
- `app` 负责编排，不承载长期业务规则。

### 7.5 不默认移动用户文件

DesktopFramesPlus 和 MiniFences 都有把桌面文件移动到分类目录的功能。LucidPane 应把“整理显示”和“整理文件系统”作为两个不同产品模式；前者默认安全，后者必须显式授权、预览、记账并可恢复。

## 8. 推荐实施顺序

### 里程碑 A：Shell Fidelity

1. `IContextMenu3` 原生右键菜单。
2. OLE `IDataObject/IDropTarget` 拖入。
3. OLE 拖出、多选和 Shell drag image。
4. 内联 rename 与 Shell command 路由。

### 里程碑 B：布局可靠性

1. Pane monitor ID 持久化。
2. display topology profile。
3. remap/clamp 和显示器热插拔测试。
4. Explorer 重启、Win+D、注销/恢复场景测试。

### 里程碑 C：真实文件操作安全

1. operation journal schema 和状态机。
2. `IFileOperation` Copy/Move/Link/Delete。
3. 冲突提示、Undo 和启动恢复。
4. Folder Portal 开放真实拖放。

### 里程碑 D：正式渲染

1. D2D/DWrite 图标和文本。
2. Composition 视觉树、阴影和动画。
3. 异步 Shell 图标缓存。
4. grid virtualization 和大目录基准。

### 里程碑 E：产品组织能力

1. Tabs。
2. Pages。
3. Profiles 和 Profile Automation。
4. 备份、导入导出。
5. 自动分类、搜索和诊断。

## 9. 源码索引

### DesktopFramesPlus

- [项目 README](https://github.com/limbo666/DesktopFramesPlus/tree/ef0edc14ecd7f32323a7a9708bb38596963c16bb)
- [项目文件与技术栈](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/Code/Desktop%20Frames/Desktop%20Frames.csproj)
- [非激活窗口](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/Code/Desktop%20Frames/NonActivatingWindow.cs)
- [Explorer 桌面图标显隐](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/Code/Desktop%20Frames/DesktopIconManager.cs)
- [Folder Portal](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/Code/Desktop%20Frames/PortalFrameManager.cs)
- [Tabs](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/Code/Desktop%20Frames/TabManager.cs)
- [Profiles](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/Code/Desktop%20Frames/ProfileManager.cs)
- [Profile Automation](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/Code/Desktop%20Frames/AutomationManager.cs)
- [自动分类真实文件操作](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/Code/Desktop%20Frames/AutoOrganizeManager.cs)
- [备份与导入导出](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/Code/Desktop%20Frames/BackupManager.cs)
- [MIT License](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/License.md)

### MiniFences

- [项目 README](https://github.com/dskiiii/minifence/tree/22128adb3ed29d04626ba903f7b077b61794baa5)
- [桌面窗口、挂载、稀疏 region 和 Win+D 兼容](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences/MainWindow.xaml.cs)
- [Explorer ListView 显隐和位置读写](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences/Services/DesktopIconLayoutService.cs)
- [原生 Shell 右键菜单](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences/Services/ShellContextMenuService.cs)
- [WPF/COM IDataObject 兼容层](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences/ShellCompatibleDataObject.cs)
- [Shell drag source image](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences/ShellDragSourceImage.cs)
- [Shell drop target helper](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences/ShellDropTargetBridge.cs)
- [操作历史与崩溃恢复](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences/Services/ActionHistoryService.cs)
- [显示器拓扑布局](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences/Services/DisplayLayoutService.cs)
- [Folder Portal 与 Shell icon](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences/Services/FolderItemService.cs)
- [SmokeTests](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences.SmokeTests/Program.cs)

## 10. 许可证注意事项

- DesktopFramesPlus 使用 MIT License。若复用其具体代码或 substantial portions，需要保留版权和许可证声明。
- MiniFences 在本次基线的仓库根目录未发现 LICENSE 文件。当前建议仅研究功能、行为和通用架构思想；在作者明确补充授权前，不直接复制其实现代码。

本节只是工程使用边界提示，不构成法律意见。
