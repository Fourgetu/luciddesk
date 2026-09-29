# 架构说明

更新：2026-09-29（0.10.5）。当前应用使用视图成员过滤后端，Pane 图标保持自绘。Windows 11 x64 为优先维护平台，Windows 10 已由用户完成实机验证。

## 系统分工

Explorer 负责未收纳图标的绘制、排列、命中和原生交互。LucidPane 使用独立窗口绘制分组内容。过滤 Hook 在 Explorer 桌面线程内调用 `IShellFolderView::RemoveObject`，将已收纳项目从视图集合移除；开启系统自动排列时，由 Explorer 补位。

收纳不移动文件，不切换系统自动排列设置。原生排列会更新实际视图位置；恢复时在用户没有改变布局的前提下，按原视图顺序恢复基线坐标。完整身份与分组状态由控制端维护，Hook 接收有大小边界的 Shell 解析名集合。

Win11 文件精简菜单由 Explorer 内的独立 Shell 宿主提供文件身份、菜单服务和普通命令执行。该宿主窗口区域为空，不显示图标，也不是完整 Explorer 文件窗口。Pane 保留自身绘制、选择和重命名输入；菜单 rename 动词通过动态编号识别后返回 Pane。命令转发、线程生命周期及实测边界见 [精简菜单技术文档](../win11-compact-menu-command-routing.md)。

## 模块职责

| 模块 | 职责 |
| --- | --- |
| `app/src/main.rs` | 参数解析、DPI/STA 初始化、配置路径与启动错误 |
| `app/src/pane/hybrid.rs` | Hook 生命周期、桌面输入、清单结果核验与同步顺序 |
| `app/src/pane/hybrid/audit.rs`、`icons.rs` | 后台审计通道与在途请求、图标批次加载和刷新 |
| `app/src/pane/runtime.rs`、`display_layout.rs` | 连接重试、窗口恢复、显示器布局切换与自动备份调度 |
| `app/src/pane/folder.rs`、`search/hotkey.rs` | 文件夹监听和导航排序、全局搜索快捷键生命周期 |
| `app/src/pane/search/` | 搜索窗口、Everything 查询与配置、快捷键和测试 |
| `app/src/pane/drag_drop/` | 拖放注册、拖动预览和临时描述 |
| `app/src/pane/recovery.rs` | 配置导出和恢复入口 |
| `app/src/pane/mod.rs`、`model.rs`、`events.rs` | 分组集合、布局、选择、操作与保存 |
| `app/src/pane/window.rs`、`render.rs`、`settings*.rs` | 分组窗口、绘制、设置与输入 |
| `app/src/tray.rs` | 通知区域入口及托盘资源生命周期 |
| `desktop-core` | Shell 身份、成员位置与工作区模型 |
| `desktop-storage` | 当前数据库格式、读取和事务式保存 |
| `desktop-shell` | Shell 快照、通知、菜单、重命名和 OLE 项目解析 |
| `desktop-hook` | DLL 引导、成员过滤与恢复、协议校验和控制端存活监测 |
| `desktop-graphics` | 工具生成的 DWM/DComp 绑定及合成内容层 |
| `desktop-window` | 显示器枚举与错误提示 |

### Rust 模块边界

`desktop-core/src/lib.rs` 只承担 crate 文档与公共类型重导出。领域实现分别放在
`identity.rs`（身份）、`geometry.rs`（坐标）、`appearance.rs`（外观）、
`item.rs`（桌面成员）、`panel.rs`（面板）和 `workspace.rs`（集合与默认值），
测试集中在 `tests.rs`。调用方仍使用 `desktop_core::Panel` 等根路径。

面板内容来源由内部枚举表示，桌面、文件夹、搜索三种来源互斥；独立的 UI 偏好保留布尔值。
`Panel::new` 与 `set_rect` 都应用最小尺寸限制。

`desktop-storage` 的公共错误位于 `src/error.rs`，存储实现及其测试位于 `src/store/`，
外观编解码和桌面成员持久化分别位于 `codec.rs` 和 `desktop_items.rs`。
`desktop-shell/src/apartment.rs` 独立管理 OLE 初始化；守卫不能直接构造或跨线程传递，
应在依赖 OLE 的资源释放之后析构。

`desktop-shell/src/lib.rs` 只声明模块和导出 API；枚举与身份解析、桌面查询、通知注册、
激活与拖放身份解码、错误定义分属独立文件。`desktop-graphics/src/layer.rs` 管理合成层，
生成绑定仍位于 `bindings/`。`desktop-hook/src/discovery.rs` 负责桌面发现和冲突检测，
过滤会话、IPC 和引擎位于 `filter/`。各库入口见 [crates 导航](../../crates/README.md)。

应用子模块按真实归属存放：`pane/hybrid/icon_changes.rs` 处理图标通知，
`pane/settings/layout.rs` 处理设置页布局，使用常规 `mod` 声明加载。
搜索功能归入 `pane/search/`，拖放功能归入 `pane/drag_drop/`；搜索和设置的单元测试
分别放在对应目录的 `tests.rs`，模块路径保持在所属功能之下。
完整目录和新增文件的归属规则见[目录结构](structure.md)。
接口边界与释放规则见 [Rust API 与资源约定](rust-api-review.md)。

## 启动与退出

1. 初始化 DPI 与 COM，解析唯一可选参数 `--title`。
2. 获取当前会话的单实例互斥量；重复启动广播唤起消息。
3. 读取 `config.toml` 与 `workspace.db`，载入全局设置、工作区和显示器布局；旧格式不自动迁移。
4. 按 DLL 内容哈希建立运行副本，在 Explorer 的异步桌面线程回调中探测 `IShellFolderView`；成功后同步原生视图与独立来源清单。
5. 创建可用的分组、独立文件夹和搜索窗口、托盘及运行时监控窗口；消息循环调度同步、重连、快捷键与备份。
6. 正常退出先停止监控和托盘，取出 Hook 会话后在不持有 `PaneApp` 可变借用的情况下分离 Explorer，再释放状态中的窗口与渲染资源；控制端消失时由存活监测触发清理。
7. `GraphicsLifetime` 在 OLE 守卫之后、应用状态之前声明，确保窗口释放后依次清空 WinRT 合成运行时、DirectComposition 设备和 GPU 设备缓存，最后才退出 OLE/COM。错误返回也沿用此析构顺序，避免图形缓存留到进程退出时的 TLS 清理阶段。

DLL 运行副本解决已加载文件无法覆盖的问题。分离会恢复视图成员并撤销回调和定时器；已固定到目标进程的 DLL 代码不立即卸载，以免留下悬空回调。

## 拖放与成员同步

桌面分组拖入、拖出先持久化归属并刷新面板，再提交 Explorer 成员同步。`submit_hidden` 等待请求被接收，后续通过 `poll_hidden` 检查确认，不在 UI 线程等待整批视图更新完成。同步仍在进行时，保护在途确认编号，并暂缓清单审计、清除桌面选择及文件菜单事务；请求失败会使发布缓存失效并安排重试。

成员变化会丢弃先前已派发的过时审计结果，避免旧快照覆盖新布局。OLE Drop 在 Shell 拖动辅助对象和临时描述清理结束后才提交应用操作，避免嵌套消息提前执行收纳。该流程用于桌面分组；文件夹面板的真实文件复制仍由 Shell 处理。

三种面板及搜索输入框的窗口顺序约束见[面板层级](pane-drag-order.md)。

## 当前兼容边界

- 应用使用 `FilterSession` 和成员协议 v1，不安装原有五个几何 detour，不依赖固定 RVA 或 PDB。
- `IShellFolderView` 是已被微软标记为不再提供使用的旧接口；连接时按运行期能力判断是否可用；已验证环境不代表所有 Windows 构建均兼容。
- 刷新/列表变化触发重新过滤，1 秒定时器补查遗漏变化并监视控制端；刷新期间仍可能短暂出现原生项目。
- Hook 会话可在故障时缺失；文件夹与搜索 pane 独立运行，桌面分组保留归属并等待重连。
- 全局偏好保存在 `config.toml`，工作区数据库为 `workspace.db`，开发阶段不迁移旧库。成员仅通过 Shell 身份与 `DesktopPlacement` 表示。
- `native_graphics.rs` 集中转换两版绑定的 COM 引用与 HRESULT；普通绘图直接使用 Canvas 类型。
- 窗口嵌套消息、绘制错误清理、Shell 通知解析和系统能力回退是当前运行期的必要保护。

原生桌面快照用于读取项目身份、位置和视图参数，为面板刷新与退出恢复提供依据。



普通面板的单窗口标签分组、状态归属与事件驱动后台策略见[标签页实现](pane-tabs.md)。
