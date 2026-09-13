# LucidPane

**把 Windows 桌面整理成顺手的工作区。**

将桌面图标拖入分组，把常用文件夹留在手边，用 Everything 随时查找文件。LucidPane 使用 Rust 开发，支持自由布局、主题材质、键盘操作与文件预览。

![LucidPane 功能示意：桌面项目拖入工作分组，旁边放置文件夹面板和 Everything 搜索框；分组收纳只保存引用，文件留在原处。](docs/images/overview.svg)

*功能示意图，非实际界面截图。*

当前预览版本：**0.9.1** · 主要验证平台：**Windows 11 x64** · [版本记录](CHANGELOG.md)

## 三种面板，一个工作区

| 面板 | 适合做什么 | 内容来自哪里 |
| --- | --- | --- |
| 桌面分组 | 按工作、学习或工具收纳桌面图标 | 拖入的桌面文件、文件夹和快捷方式 |
| 文件夹面板 | 浏览常用目录，排序并进入子文件夹 | 映射文件夹的内容，随文件变化刷新 |
| Everything 搜索 | 输入关键词或查询语法，快速打开与定位文件 | 本机运行的 Everything 索引 |

桌面分组只保存项目引用。收纳、移出或删除分组不会移动或删除原文件；未收纳图标继续由 Explorer 显示。**文件菜单中的删除、重命名、剪切等命令会操作真实文件。** 文件夹面板中的拖入和粘贴也属于文件操作。

## 让布局适合你的习惯

- **安排空间**：移动、缩放、折叠面板，使用边缘吸附、自动收起、锁定与置顶。
- **调整外观**：选择主题、纯色或系统材质，调整圆角、边框和文字明暗。
- **保持熟悉的操作**：多选、键盘导航、F2 重命名、Explorer 文件菜单，以及 Peek / QuickLook 预览。
- **延续工作区**：保存分组归属和显示器布局，通过设置导出配置、恢复备份。

## 从源码开始

项目处于开发阶段，尚未提供面向普通用户的稳定发行版。准备 Windows x64、Rust 1.95 或更新版本的 MSVC 工具链、Visual Studio C++ 构建工具和 Windows SDK，然后在仓库根目录打开 PowerShell：

```powershell
cargo build -p lucidpane -p desktop-hook --locked
$env:LUCIDPANE_DATA_DIR = Join-Path $PWD 'target\dev-data'
& .\target\debug\lucidpane.exe
```

该示例将配置放在独立的开发目录。`lucidpane.exe` 与 `desktop_hook.dll` 必须来自同次构建、放在同一目录；启动前退出旧实例。已下载预览包时，解压完整目录后启动主程序即可。

1. 将桌面项目拖入分组，拖回桌面即可取消收纳。
2. 通过托盘菜单创建分组或文件夹面板，在设置中启用 Everything 搜索。
3. 在设置中调整外观，使用托盘菜单退出程序。

Everything 搜索需要本机 Everything；文件预览需要单独安装 PowerToys Peek 或 QuickLook。它们不包含在 LucidPane 中。

## 兼容性与数据

桌面分组依赖经过校验的 Windows 系统组件。不兼容或 Explorer 暂不可用时，程序保留分组配置并尝试重连，文件夹与搜索面板仍可独立运行。在“设置 → 关于”可查看桌面连接状态。

未指定开发目录时，数据保存在 `%LOCALAPPDATA%\LucidPane`：`config.toml` 保存全局设置，`workspace.db` 保存面板与布局。备份包含配置和布局，不包含原文件。开发阶段不迁移旧数据库，也不再读取 `hook-desktop.db`。

## 继续阅读

| 想了解什么 | 文档 |
| --- | --- |
| 操作、快捷键、外观和故障处理 | [使用说明](docs/usage.md) |
| 构建、测试和制作预览包 | [构建指南](docs/development/build.md) |
| 源码放在哪里、如何划分模块 | [目录结构](docs/development/structure.md) · [crates 导航](crates/README.md) |
| 架构、存储和绘图实现 | [开发文档](docs/development/README.md) |
| 全部文档与历史资料 | [文档导航](docs/README.md) |
