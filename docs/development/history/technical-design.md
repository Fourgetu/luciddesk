# LucidPane 桌面图标管理程序技术方案

> 历史资料：本文记录当时的方案、实验与测量，不代表当前主线能力。旧路径、启动参数和已删除探针仅用于追溯；当前实现见[架构说明](../architecture.md)，可执行命令见[构建与验证](../build.md)。

> **最新决策：** 用户已否定下面的原生轮廓框路线，允许按 Fences 产品效果重新设计。
> 以 [新方案](redesign-fences.md) 为当前提案；本文其余内容作为历史设计保留。

> **2026-09-07 路线修正：原生桌面优先。** 用户要求保留 Windows 现有图标、文字和交互，
> 只增加分组框。下文关于“默认 Managed Desktop”和“自行绘制桌面”的设计属于历史方案，
> 不再代表默认产品方向。
>
> 当前默认走 `native_desktop` / `NativeFrame`：不隐藏 Explorer，不创建 `DesktopItemSurface`，
> 不绘制图标。分组框只有半透明标题和边框，内部从 HWND region 中实际扣除，使原生绘制和
> 跨进程输入都保留。拖动分组结束时，使用桌面 Shell 的 `IFolderView2` 移动框内原生图标；
> 不修改自动排列或网格设置。多个框的标题和位置保存在独立的 `native-frames.db`，旧布局不迁入。
>
> 这一步实现的是原生图标外的分组轮廓，**还没有图标下方的整块半透明背景**，也没有折叠隐藏
> 原生图标或独占成员关系。不能把覆盖图标的透明窗口、手工重画图标或禁用原生桌面当作替代。
> 后续背景层、Win+D、Explorer 重启和多 DPI 行为需要实机验证。原接管模式仅保留为显式
> `--managed-desktop` 实验入口。

> 文档版本：0.1
> 更新日期：2026-09-04
> 目标平台：Windows 11 22H2 及以上，后续兼容 Windows 10
> 开发语言：Rust

## 0. MVP 实施状态

截至 2026-09-04，Managed Desktop Mode 的第一条可运行纵向链路已经落地：

- [x] 以 `SHGetDesktopFolder → IShellFolder::EnumObjects → IEnumIDList` 枚举完整 Desktop Shell Namespace，不再手工拼接用户桌面与公共桌面。
- [x] 文件系统项目与虚拟 Shell 项目使用统一 `ShellIdentity`；文件系统身份包含 Volume ID / 128-bit File ID，虚拟项目保存 parsing name，重命名后可保留布局。
- [x] `DesktopPlacement` 明确区分 `FreeDesktop` 与 `Pane`；改变归属只更新 LucidPane 元数据，不移动真实文件。
- [x] 新建 Pane 默认名称为 `新建分组` 且内容为空；已有桌面项目先显示在自由桌面层，可拖入 Pane，也可拖回桌面。
- [x] 每台显示器创建一个 `DesktopItemSurface`；窗口区域只覆盖图标单元，空白桌面不占用输入。
- [x] Pane 和桌面图标表面都是 Shell 拥有的非置顶 `WS_POPUP | WS_EX_TOOLWINDOW`，普通应用窗口可覆盖它们。
- [x] 多显示器枚举、有效 DPI、显示器局部 DIP 坐标、显示器丢失后的主屏迁移与边界钳制。
- [x] 单击选择、双击或 Enter Shell 执行、自由桌面拖动、自由桌面与 Pane 间的逻辑分组。
- [x] SQLite schema 持久化 Shell 身份、显示名和桌面位置/Pane 网格位置。
- [x] 拖动/重排/移动缩放结束、标题与外观变更、刷新、显示器变化和会话结束时事务提交工作区，不再只依赖正常退出保存。
- [x] 正常退出恢复 Explorer 图标；独立守护进程处理进程崩溃；磁盘接管标记和 `--restore-shell` 处理跨重启残留。
- [x] `SHGetSetSettings` 为首选接管接口；当当前 Explorer 版本回滚该状态时，由 `desktop-shell` 内聚的 `FolderView` 可见性兼容后备完成隐藏/恢复。
- [x] 用户主动重新显示 Explorer 图标时停止 Managed Desktop；注销/关机消息到达时先释放接管再退出。
- [x] `--manual` 空手工集合与显式目录 Folder Portal 继续作为降级/开发路径。
- [x] 递归 `SHChangeNotifyRegister`、300ms 防抖全量对账，以及 Explorer `TaskbarCreated` owner/通知重挂载。
- [x] `WM_DISPLAYCHANGE` / `WM_DPICHANGED` 后重新枚举显示器并重建各屏稀疏表面；消失显示器上的自由图标迁移到主屏。
- [x] Workspace 全量测试、Clippy `-D warnings` 与 Win32 稀疏窗口区域集成测试。
- [ ] Pane 自身完整的 Per-Monitor DPI 尺寸换算与显示器 ID 持久化。
- [ ] `IContextMenu3` 原生右键菜单、`IFileOperation` 重命名/删除。
- [ ] 真正的 OLE `IDataObject` / `IDropTarget` 拖入拖出与多选。
- [ ] Direct2D/DirectWrite/DirectComposition 正式渲染与异步缩略图。
- [ ] 多 Pane 创建/删除/锁定、托盘、设置、排序和自动分类规则。

当前代码使用分层 GDI 窗口完成交互 PoC，正式渲染仍应替换为 Direct2D/DirectWrite/DirectComposition。Managed Desktop 的数据源和变化触发器均已进入 Shell Namespace 路径；物理目录 watcher 仅保留给显式 Folder Portal。本节描述的是“已经实现什么”，后文描述目标架构和后续里程碑。

## 1. 项目概述

LucidPane 是一个面向 Windows 的桌面图标与文件入口管理程序。产品形态参考 Stardock Fences 6，但不以完整复制为第一阶段目标，而是优先建立稳定、可维护的原生桌面基础设施。

产品通过半透明面板组织文件、目录、快捷方式和应用入口，支持文件夹门户、自动分类、标签页、面板折叠、快速隐藏和全局呼出等能力。

Fences 6 的公开核心能力包括分组、自动整理、Folder Portal、标签页、Peek、图标染色及快速隐藏。参考：[Stardock Fences 官方介绍](https://www.stardock.com/products/fences/index)。

## 2. 产品目标

### 2.1 核心目标

- 在桌面上创建可移动、缩放、锁定和折叠的文件面板。
- 以文件夹门户形式直接展示任意目录内容。
- 使用自定义集合组织桌面项目，但默认不移动真实文件。
- 支持系统图标、缩略图、双击打开和原生右键菜单。
- 支持从 Explorer 拖入文件以及将项目拖出到其他程序。
- 根据文件类型、名称、时间等条件自动分类。
- 在多显示器和不同 DPI 环境下可靠恢复布局。
- Explorer 重启、显示器热插拔或应用异常退出后能够自动恢复。

### 2.2 非目标

以下能力不纳入首个 MVP：

- 向 `explorer.exe` 注入 DLL 或安装全局 Hook。
- 完整替代 Explorer 的全部 Shell 行为。
- 使用 Windows 私有虚拟桌面 COM API。
- 企业集中部署、云同步和账户系统。
- 自动移动或删除用户真实文件。

## 3. 总体技术路线

采用以下技术组合：

- Win32 HWND：窗口生命周期、消息循环、输入、DPI 和桌面层级。
- Windows Composition：面板视觉树、材质、阴影和动画。
- Direct2D/DirectWrite：图标、文本、选择框和自定义控件渲染。
- Windows Shell COM：文件枚举、系统图标、右键菜单、打开与拖放。
- SQLite：布局、规则、面板和项目关系的事务化存储。
- Rust 后台线程：目录扫描、缩略图加载和规则计算。

不建议将桌面主体改造成 Tauri 或 WebView。Web 技术可以快速制作设置页面，但无法降低桌面层级、Explorer 集成、OLE 拖放和 Shell 菜单的实现复杂度，反而会引入额外运行时与输入协调问题。

## 4. 产品模式

LucidPane 将内容来源抽象为三种模式。

### 4.1 文件夹门户

面板直接绑定到一个真实目录，并实时显示目录内容。

特点：

- 实现稳定，适合作为 MVP 主路径。
- 可以绑定下载、文档、项目目录、网络盘或云盘同步目录。
- 文件变化由 Shell 通知驱动。
- 拖入文件时，默认执行用户选择的复制、移动或创建快捷方式操作。

### 4.2 手工集合

面板保存一组项目引用，不改变项目的物理位置。

特点：

- 同一个项目可以出现在多个集合中。
- 删除集合成员只删除引用，不删除真实文件。
- 适合组织应用、常用文档和跨目录工作区。

### 4.3 桌面集合

读取当前用户桌面和公共桌面内容，并在 LucidPane 中进行逻辑分组。

首个版本由 LucidPane 自己渲染项目。后续可以提供高级兼容模式，通过 `IFolderView` 调整 Explorer 原生桌面项目的位置，但不能将该能力作为核心数据模型的基础。

### 4.4 Managed Desktop Mode（当前默认）

Managed Desktop Mode 的边界是“接管桌面图标的显示与启动”，不是注入或修改 Explorer：

1. 从 `SHGetDesktopFolder` 开始枚举完整 Desktop Shell Namespace，其中自然包含用户桌面、公共桌面以及回收站等命名空间对象。
2. 文件系统项目保存路径、Volume ID 与 128-bit File ID；非文件系统项目保存 parsing name。PIDL 只作为进程内临时身份。
3. 优先使用 `SHGetSetSettings` 和 `SSF_HIDEICONS` 保存并隐藏 Explorer 原生桌面图标视图；运行时验证失败时才启用封装在 `desktop-shell` 的 ListView 可见性兼容后备。
4. 未分组项目由每显示器 `DesktopItemSurface` 渲染；新建 Pane 为空，用户拖入后才建立 `Pane` 归属。
5. 双击仍由 Shell 执行；分组、排序和拖动只更新 `DesktopPlacement`，不改变文件系统位置。
6. 正常退出、进程崩溃和下次启动分别通过 RAII、独立守护进程及持久接管标记恢复 Explorer 原设置。

“当前用户桌面与公共桌面合并”只适用于物理文件夹扫描的简化实现，不应作为 Managed Desktop Mode 的最终枚举模型。正确边界是：Explorer/Shell 继续拥有项目语义，LucidPane 只替换 Desktop View 与 Layout。

`SHGetSetSettings` 是微软公开的 Shell 状态接口，但在当前 Windows 11 实测中设置可能被 Explorer 回滚，因此必须验证最终视图状态，不能把函数返回视为接管成功。兼容后备只能存在于 `desktop-shell`，Explorer 重启后重新解析，不得成为身份、布局或窗口宿主模型的依赖。仍保留 `--manual` 降级模式。参考：[SHGetSetSettings](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shgetsetsettings)、[SHELLSTATE](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/ns-shlobj_core-shellstatea)、[SSF 常量](https://learn.microsoft.com/en-us/windows/win32/shell/ssf-constants)。

## 5. 窗口与渲染架构

### 5.1 宿主模型

Managed Desktop Mode 使用两类 Shell-owned 顶层窗口：每台显示器一个自由桌面图标表面，以及每个 Pane 一个独立窗口。窗口均为非置顶工具窗口，由 `GetShellWindow()` 返回的 Shell 窗口作为 owner。

```text
Monitor
├── DesktopItemSurface HWND
│   ├── Free desktop icon A
│   ├── Free desktop icon B
│   └── sparse window region (icons only)
├── Pane A HWND
│   ├── Header / menu / close
│   └── Icon Grid
└── Pane B HWND
    ├── Header / menu / close
    └── Icon Grid
```

自由桌面表面不能是覆盖整屏的透明点击穿透窗口；其实际 Window Region 必须是图标单元的并集，使墙纸空白处从窗口命中范围中消失并交回 Explorer。每 Pane 一个 HWND 则能保持独立移动、缩放、折叠和 Z-order。跨窗口拖动由应用级 Drag Session 协调，后续替换为完整 OLE 拖放。

### 5.2 命中测试

- 面板、标题栏、图标和缩放边缘接受鼠标输入。
- 宿主空白区域返回穿透命中，使桌面仍可正常操作。
- 折叠面板只保留标题栏命中区域。
- Peek 模式临时切换为顶层交互窗口，退出后恢复桌面层级。

### 5.3 桌面层级

Windows 没有面向第三方程序公开一个完整、稳定的“嵌入桌面图标层”接口。常见的 Progman/WorkerW 挂载方式存在随 Explorer 或 Windows 更新变化的风险。

因此定义独立接口：

```rust
trait DesktopHost {
    fn attach(&mut self, hwnd: Hwnd) -> Result<AttachmentMode>;
    fn detach(&mut self, hwnd: Hwnd) -> Result<()>;
    fn refresh_shell(&mut self) -> Result<()>;
}
```

当前主路径和降级路径：

- `ShellOwnedDesktopHost`：使用 Shell 顶层 owner 语义，不使用 `HWND_TOPMOST`，是公开 API 主路径。
- `Standalone`：无法获得 Shell owner 时的安全降级模式。
- `WorkerWAttachment`：仅作为将来按系统版本验证的兼容层，不应成为业务模型依赖。

不得让 WorkerW 查找逻辑散落在窗口、渲染或业务模块中。

### 5.4 DPI 与多显示器

- 进程使用 Per-Monitor DPI Awareness V2。
- 面板布局以 DIP 存储，渲染阶段转换为物理像素。
- 每个显示器单独维护缩放比例、工作区和 Composition Target。
- 监听 `WM_DPICHANGED`、`WM_DISPLAYCHANGE` 和设备热插拔。
- 显示器消失时，将面板迁移到主显示器的可见区域，但保留原始布局快照。

## 6. Rust Workspace 规划

在现有 workspace 基础上逐步调整，不需要一次性重写。

| crate | 主要职责 |
|---|---|
| `desktop-core` | 纯 Rust 领域模型、命令、事件和接口定义 |
| `desktop-window` | HWND、消息循环、命中测试、显示器、DPI、桌面挂载 |
| `desktop-compositor` | Composition 视觉树、材质、动画和效果 |
| `desktop-renderer` | Direct2D、DirectWrite、图标网格和文本布局 |
| `desktop-shell` | Shell Item、文件夹枚举、打开、原生菜单和 Shell 通知 |
| `desktop-icons` | 图标/缩略图提取、转换、内存与磁盘缓存 |
| `desktop-dnd` | OLE 拖放、`IDataObject`、`IDropTarget` |
| `desktop-rules` | 自动分类规则、优先级和手工覆盖逻辑 |
| `desktop-storage` | SQLite、迁移、布局快照和撤销日志 |
| `app` | 生命周期、托盘、设置、热键和模块装配 |

### 6.1 依赖建议

- `windows` / `windows-sys`：继续作为 Windows API 绑定。
- `serde`、`serde_json`：配置导入导出和可读快照。
- `rusqlite`：本地数据库，建议启用 bundled SQLite。
- `uuid`：面板、标签、规则和集合 ID。
- `crossbeam-channel`：UI 与后台线程之间的消息通信。
- `tracing`、`tracing-subscriber`：结构化日志与问题诊断。
- `thiserror`：库级错误类型。
- `anyhow`：仅用于应用入口和最终错误汇总。

UI 主线程保持 STA，并由 Win32 消息循环驱动。目录扫描、缩略图加载和规则计算使用后台线程；不让 Tokio 接管窗口消息循环。

## 7. 核心数据模型

```rust
struct WorkspaceProfile {
    id: ProfileId,
    name: String,
    monitors: Vec<MonitorLayout>,
    panels: Vec<Panel>,
    rules: Vec<Rule>,
    preferences: Preferences,
}

struct Panel {
    id: PanelId,
    title: String,
    monitor_id: MonitorId,
    rect_dip: Rect,
    tabs: Vec<PanelTab>,
    active_tab: TabId,
    appearance: Appearance,
    collapsed: bool,
    locked: bool,
}

enum PanelSource {
    Folder { path: PathBuf },
    DesktopCollection,
    ManualCollection { collection_id: CollectionId },
}

struct ItemRef {
    shell_parsing_name: String,
    volume_id: Option<u64>,
    file_id: Option<u128>,
    fallback_path: Option<PathBuf>,
}

struct Rule {
    id: RuleId,
    priority: i32,
    enabled: bool,
    conditions: Vec<Condition>,
    target_panel: PanelId,
    manual_override: OverridePolicy,
}
```

PIDL 是运行时 Shell 标识，不应直接作为跨重启持久化主键。文件系统项目优先使用卷标识和 File ID，辅以路径；虚拟 Shell 项目保存 parsing name，并在运行时重新解析。

## 8. Shell 集成设计

### 8.1 项目枚举与身份

- 使用 `IShellFolder`、`IShellItem` 或 `IEnumShellItems` 枚举项目。
- Managed Desktop 从 Desktop Shell Namespace 根开始枚举；Known Folder 只用于 Folder Portal 或兼容性监听，不用于手工合并桌面清单。
- 同名的用户桌面、公共桌面与虚拟项目必须保持不同 Shell 身份。
- 路径变化后尝试通过 File ID 恢复引用。

### 8.2 图标与缩略图

使用 `IShellItemImageFactory::GetImage` 获取图标或缩略图。该 API 可以在缩略图不存在时回退到系统图标；非缓存提取可能访问磁盘，因此应在后台线程执行。参考：[Microsoft 文档](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitemimagefactory-getimage)。

缓存键至少包含：

```text
Item identity + requested size + DPI + theme + modified timestamp
```

缓存分为两级：

- GPU/内存缓存：当前可见项目，使用 LRU 淘汰。
- 磁盘缓存：可选，用于昂贵的文档和图片缩略图。

首屏先绘制缓存或占位图标，缩略图完成后只更新对应 Visual，不能阻塞整棵场景树。

### 8.3 Shell 变化通知

使用 `SHChangeNotifyRegister` 接收新建、删除、重命名、更新和目录刷新通知。Windows 可能合并大量项目通知为目录更新，因此收到目录级通知后要执行增量对账。参考：[Microsoft 文档](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shchangenotifyregister)。

事件处理：

```text
Shell notification
→ 200~500ms debounce
→ enumerate affected directory
→ diff with current snapshot
→ preserve selection and manual grouping
→ evaluate rules for new/changed items
→ commit model update
→ invalidate affected visuals
```

### 8.4 打开与右键菜单

- 双击使用 Shell 执行机制打开项目，不自行判断文件关联。
- 原生菜单通过 `IContextMenu`、`IContextMenu2` 或 `IContextMenu3` 承载。
- 菜单相关窗口消息需要转发给 Shell 菜单对象。
- LucidPane 自己的“移出分组”“固定”“更改标签”等命令应与系统菜单分区显示。

### 8.5 Explorer 原生图标位置

高级兼容模式可以使用 `IFolderView::SelectAndPositionItems` 定位当前文件夹视图中的项目。参考：[Microsoft 文档](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifolderview-selectandpositionitems)。

该模式只作为增强能力：

- 不能依赖它实现标签页和折叠。
- 不能通过把图标移动到屏幕外来模拟隐藏。
- Explorer 重启后必须重新获取 Folder View。
- 失败时不得破坏 LucidPane 自己的数据和布局。

## 9. 拖放设计

### 9.1 外部拖入

宿主 HWND 通过 `RegisterDragDrop` 注册 `IDropTarget`。进行 Shell 拖放的线程必须调用 `OleInitialize` 并持续处理消息。参考：[Microsoft Shell 拖放文档](https://learn.microsoft.com/en-us/windows/win32/shell/dragdrop)。

不同面板源的默认行为：

- 文件夹门户：遵循 Shell 的复制、移动、链接语义。
- 手工集合：只新增引用，不移动文件。
- 桌面集合：新增引用；需要创建快捷方式时明确提示。

### 9.2 内部拖动

- 在同一面板内拖动：改变手工顺序或网格位置。
- 在集合之间拖动：改变分组关系。
- 拖到文件夹门户：属于真实文件操作，必须显示复制/移动反馈。
- 按住修饰键时遵循 Windows 常见语义，并在光标附近显示最终动作。

### 9.3 撤销

逻辑分组操作必须可立即撤销。真实文件操作优先交由 Shell 文件操作 API，以获得冲突处理、进度和回收站支持。

## 10. 自动分类规则

MVP 支持以下条件：

- 扩展名或文件类型。
- 文件名 glob。
- 所在目录。
- 文件、目录或快捷方式类型。
- 创建时间和修改时间范围。
- 常见截图命名模式。

规则按优先级从高到低计算。第一条排他规则命中后停止；标签型规则可以继续累加。

```text
New or changed item
→ resolve stable identity
→ check manual override
→ evaluate enabled rules by priority
→ build proposed assignment
→ persist assignment and reason
→ notify UI
```

手工移动项目后，默认产生手工覆盖记录，避免下一次目录刷新又被规则移回。如果用户选择“重新应用规则”，再清除该项目的覆盖记录。

自动规则默认只修改分组元数据，不移动或删除真实文件。

## 11. 标签页、折叠与 Peek

### 11.1 标签页

一个面板可以包含多个 `PanelTab`，每个标签绑定一个 `PanelSource`。标签切换只替换内容 Visual，保留面板位置和外观。

### 11.2 折叠

- 双击标题栏切换折叠状态。
- 可选悬停展开，并提供延迟防止误触。
- 折叠动画只改变裁剪区域和内容透明度，不销毁项目模型。

### 11.3 Peek

Peek 将所有面板临时提升到普通窗口之上，退出后恢复到桌面层。

默认热键建议为 `Ctrl+Alt+Space`，并允许用户修改。涉及 Windows 键的组合可能被操作系统保留；`RegisterHotKey` 失败时必须显示冲突状态，而不是静默失效。参考：[Microsoft RegisterHotKey 文档](https://learn.microsoft.com/zh-cn/windows/win32/api/winuser/nf-winuser-registerhotkey)。

## 12. 存储与恢复

### 12.1 SQLite 数据

数据库保存：

- Workspace/Profile。
- 显示器布局与面板几何。
- 面板、标签和数据源。
- 项目引用与手工分组关系。
- 自动规则和手工覆盖。
- 布局快照和撤销日志。
- 数据库结构校验。

### 12.2 配置文件

少量启动前配置可以使用 JSON，例如日志级别、数据库位置和故障恢复标记。配置写入采用临时文件加原子替换。

### 12.3 崩溃恢复

- 数据库操作使用事务。
- 拖动和缩放过程只更新内存，鼠标释放后提交。
- 定时保存未提交的布局草稿。
- 启动时检测上次异常退出，并恢复到最后一次完整事务。
- 注册 `TaskbarCreated` 消息，在 Explorer 重启后重新挂载宿主窗口和 Shell 通知。

## 13. 安全与隐私

- 不注入 Explorer，不安装内核驱动。
- 默认不上传文件名、路径或缩略图。
- 自动规则默认不执行删除或物理移动。
- 删除面板不删除真实文件。
- 对网络路径和不可访问目录使用超时及后台加载。
- 日志对用户名和完整路径提供脱敏选项。

## 14. 性能目标

以下指标作为工程验收目标，而不是平台保证：

- 宿主与面板骨架在启动后 500ms 内可见。
- 200 个项目的面板滚动和拖动保持流畅。
- 缩略图提取不阻塞 UI 消息循环。
- 静置且无文件变化时 CPU 使用率接近零。
- 常规工作集控制在 120MB 以内。
- Explorer 重启后 3 秒内尝试完成桌面层恢复。
- 大量文件事件只触发一次合并刷新，避免刷新风暴。

## 15. 测试策略

### 15.1 单元测试

- 规则优先级、排他和手工覆盖。
- DIP 与像素坐标转换。
- 面板布局约束和可见区域修正。
- ItemRef 重命名与身份恢复。
- 数据库迁移和事务回滚。

### 15.2 集成测试

- 创建、删除、重命名及批量复制文件。
- 用户桌面与公共桌面同名项目。
- 外部拖入、拖出和跨面板拖放。
- Explorer 重启后的重新连接。
- 网络目录断开与恢复。
- Shell 右键菜单消息转发。

### 15.3 手工兼容矩阵

- Windows 11 22H2、23H2、24H2 及后续受支持版本。
- 单显示器、双显示器、负坐标排列。
- 100%、125%、150%、200% 缩放组合。
- 显示器热插拔和远程桌面。
- 明暗主题、高对比度和减少动画设置。
- Explorer 崩溃重启、睡眠恢复和用户锁屏。

## 16. 实施里程碑

### M0：基础设施，约 1 周

- 确立领域模型与命令/事件边界。
- 建立日志、错误处理和存储迁移框架。
- 验证每显示器单宿主窗口模型。
- 隔离 `DesktopAttachment`。

交付结果：可以创建多个内存面板，并在 Explorer 重启后恢复宿主。

### M1：空 Pane 与手工桌面图标集合，约 2 周

- 新 Pane 默认建立空的手工集合。
- 支持从 Explorer 桌面拖入文件系统项目并保存逻辑引用。
- 完成系统图标、标签与响应式图标网格。
- 支持 Shell 双击打开、滚动和重启恢复。

交付结果：桌面上可使用的基础 Fences 风格手工分组 Pane。

### M2：编辑与持久化，约 2 周

- 完整实现 OLE 虚拟 Shell 项目拖入、项目拖出及跨面板拖动。
- 增加 Folder Portal 可选模式。
- 完成 SQLite 持久化和布局恢复。
- 增加原生 Shell 右键菜单。

交付结果：可日常使用的 Alpha 版本。

### M3：自动化与 Fences 风格体验，约 2～3 周

- 自动分类规则。
- 标签页、折叠和悬停展开。
- 快速隐藏与全局 Peek。
- 面板主题、透明度、图标大小和染色。

交付结果：功能完整的 MVP。

### M4：稳定性与发布，约 3～4 周

- 完成多显示器、DPI 和 Explorer 恢复测试。
- 增加设置界面、托盘和开机启动。
- 完成安装包、代码签名和升级策略。
- 性能分析、无障碍和崩溃诊断。

交付结果：可公开测试的 Beta 版本。

单人全职开发预计 6～8 周形成 MVP，12～16 周达到较可靠的公开 Beta。高级 Explorer 原生桌面兼容不包含在该估算内。

## 17. 主要风险与应对

| 风险 | 级别 | 应对方式 |
|---|---:|---|
| WorkerW/Progman 行为随系统变化 | 高 | 独立兼容层、降级模式、版本回归测试 |
| 复刻全部 Explorer Shell 行为工作量过大 | 高 | 空手工集合优先，真实文件不移动；Folder Portal 保持可选 |
| 缩略图阻塞 UI | 中 | 后台线程、缓存优先、可见区域虚拟化 |
| OLE 拖放与 COM 生命周期复杂 | 中 | STA UI、集中封装、集成测试覆盖 |
| 多显示器/DPI 布局漂移 | 中 | DIP 持久化、每显示器 HWND、热插拔恢复 |
| 自动分类误操作文件 | 高 | 默认只改元数据，物理操作需明确确认与撤销 |

## 18. MVP 验收标准

MVP 满足以下条件后可以进入发布稳定化阶段：

- 能创建、重命名、移动、缩放、折叠和删除面板。
- 能创建文件夹门户和手工集合。
- 能显示系统图标/缩略图、文件名和基本状态。
- 能双击打开项目并调用系统右键菜单。
- 能从 Explorer 拖入并向 Explorer 拖出。
- 能建立至少五类自动分类规则。
- 重启应用后恢复面板、标签、项目关系和外观。
- 双显示器、混合 DPI 和 Explorer 重启场景无数据丢失。
- 删除面板或集合不会删除用户真实文件。
- 无文件事件时不持续轮询目录或占用明显 CPU。

## 19. 当前仓库落地顺序

当前代码已经完成 Shell Namespace 枚举/通知、稳定文件身份、Shell-owned 稀疏桌面表面、空 Pane、双向逻辑分组、Shell 执行、SQLite、Explorer 重挂载与三层接管恢复。后续按以下顺序演进：

1. 为 Pane 增加显示器 ID，并完成其 `WM_DPICHANGED` DIP/像素换算与建议矩形应用。
2. 将当前 Shell 通知后的全量对账优化为基于受影响 PIDL 的增量对账，并完善通知风暴测试。
3. 实现多 Pane 创建/删除/锁定以及统一 App Controller，去除单 Pane 消息循环假设。
4. 实现 `IContextMenu3`、`IFileOperation` 与完整 OLE `IDataObject` / `IDropTarget`。
5. 新增 `desktop-renderer`，用 Direct2D/DirectWrite/DirectComposition 替换临时 GDI；缩略图在后台提取并缓存。
6. 完善不可访问卷、云占位符、网络项目和命名空间 parsing name 失效时的恢复策略。
7. 增加托盘设置、排序、自动规则、标签页、折叠和 Peek。
8. 最后按受支持 Windows 版本评估 WorkerW 兼容层，不让它影响默认公开 API 路径。

已完成的首个开发切片是“完整桌面清单显示在自由桌面层，一个默认为空的 Pane 接收图标引用并可持久化、执行和拖回”。Folder Portal 仍是用户显式选择目录时的独立模式。
