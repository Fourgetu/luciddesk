# 架构说明

更新：2026-09-12。本文描述兼容层收敛后的主线。

## 系统分工

Explorer 保留桌面 Shell 项目、未收纳图标及原生交互。LucidPane 使用独立窗口绘制分组内容。几何 Hook 在 Explorer 的视图线程中隐藏已收纳项目的桌面呈现，并将剩余项目映射到紧凑网格。

收纳不移动文件，不写入 Explorer 永久图标坐标，不切换系统自动排列设置。完整身份与分组状态由控制端维护，Hook 接收有边界的布局数据。

## 模块职责

| 模块 | 职责 |
| --- | --- |
| `app/src/main.rs` | 参数解析、DPI/STA 初始化、配置路径与启动错误 |
| `app/src/pane/hybrid.rs` | Hook 生命周期、Shell 清单同步、图标通知与后台加载 |
| `app/src/pane/runtime.rs`、`display_layout.rs` | 连接重试、窗口恢复、显示器布局切换与自动备份调度 |
| `app/src/pane/folder.rs`、`search_hotkey.rs` | 文件夹监听和导航排序、全局搜索快捷键生命周期 |
| `app/src/pane/recovery.rs` | 配置导出和恢复入口 |
| `app/src/pane/mod.rs`、`model.rs`、`events.rs` | 分组集合、布局、选择、操作与保存 |
| `app/src/pane/window.rs`、`render.rs`、`settings*.rs` | 分组窗口、绘制、设置与输入 |
| `app/src/tray.rs` | 通知区域入口及托盘资源生命周期 |
| `desktop-core` | Shell 身份、成员位置与工作区模型 |
| `desktop-storage` | 当前数据库格式、读取和事务式保存 |
| `desktop-shell` | Shell 快照、通知、菜单、重命名和 OLE 项目解析 |
| `desktop-hook` | DLL 引导、协议校验、几何映射、选择隔离和控制端存活监测 |
| `desktop-graphics` | 工具生成的 DWM/DComp 绑定及合成内容层 |
| `desktop-window` | 显示器枚举与错误提示 |

## 启动与退出

1. 初始化 DPI 与 COM，解析唯一可选参数 `--title`。
2. 获取当前会话的单实例互斥量；重复启动广播唤起消息。
3. 打开配置库，必要时备份并从 v8/v9 升级，载入工作区、设置和显示器布局。
4. 按 DLL 内容哈希建立运行副本，尝试连接经过版本校验的几何后端；成功后同步原生清单。
5. 创建可用的分组、独立文件夹和搜索窗口、托盘及运行时监控窗口；消息循环调度同步、重连、快捷键与备份。
6. 正常退出释放窗口、托盘和 Hook 会话；控制端消失时由存活监测触发清理。

DLL 运行副本解决已加载文件无法覆盖的问题。分离会撤销回调、定时器和几何映射；已固定到目标进程的 DLL 代码不立即卸载，以免留下悬空回调。

## 当前兼容边界

- Hook 仅提供几何后端，协议为 v2；连接成功的引擎必有几何会话。
- 具体系统映像的哈希、大小和入口指令由 `geometry_profile.rs` 校验。它不是跨版本稳定 ABI，校验失败必须拒绝安装。
- Hook 会话可在故障时缺失；文件夹与搜索 pane 独立运行，桌面分组保留归属并等待重连。
- 全局偏好保存在 `config.toml`，工作区数据库为 `workspace.db`，开发阶段不迁移旧库。成员仅通过 Shell 身份与 `DesktopPlacement` 表示。
- `native_graphics.rs` 集中转换两版绑定的 COM 引用与 HRESULT；普通绘图直接使用 Canvas 类型。
- 窗口嵌套消息、绘制错误清理、Shell 通知解析和系统能力回退仍服务当前功能，不属于已删除的旧模式包装。

旧 `GroupWindow`、`DesktopItemSurface`、`NativeFrame`、宿主抽象、Portal 扫描、桌面隐藏写入及工作区 Hook 后端已经移除。当前原生桌面快照仍用于刷新，不能随旧模式一起删除。

历史应用和专属依赖保存在本地 `leagcy` 分支；主线不保留旧入口，也不承诺新旧库接口兼容。历史背景见[资料索引](history/README.md)。
