# LucidPane

用分组面板整理 Windows 桌面，让文件、文件夹和快捷方式各归其位。

LucidPane 是使用 Rust 开发的桌面图标分组工具。将桌面项目拖入面板，按工作、学习或常用工具分类；再通过移动、折叠和外观设置，安排适合自己的桌面布局。

项目目前处于开发阶段，主要在 **Windows 11 x64** 上开发和验证，尚未提供面向普通用户的稳定发行版。

当前预览版本为 **0.8.0**，完整提交脉络与编号依据见[版本记录](CHANGELOG.md)。

## 主要功能

- **拖放收纳**：将桌面项目拖入分组，在分组内排序、跨分组移动，或拖回桌面取消收纳。
- **自由布局**：移动、缩放、折叠面板，支持边缘吸附、自动收起、锁定和置顶。
- **个性外观**：在设置中调整主题、背景材质和面板布局。
- **熟悉的操作**：打开项目、调用 Explorer 图标菜单，使用 F2 重命名。
- **托盘管理**：新建或显示分组、打开设置、退出程序；分组配置与成员归属会保存到本地。
- **Everything 搜索面板**：与分组风格一致的紧凑搜索框，输入后展开文件列表，支持滚动、多选、文件操作及所选程序预览。
- **文件预览**：在设置中选择 Peek 或 QuickLook，所有面板只使用所选程序；支持独立程序路径、自动检测和自定义快捷键。
- **文件夹映射**：列表或图标视图，按名称、类型、修改时间排序，在面板内进入子目录。
- **恢复与备份**：桌面连接自动重试，保存不同显示器组合的布局，支持自动备份和配置导入导出。

未收纳的图标继续由 Explorer 显示，已收纳的项目显示在 LucidPane 面板中。分组保存项目引用，收纳、移出或删除分组不会移动或删除原文件；图标菜单中的重命名、删除等命令则会操作真实项目。

## 快速开始

目前可从源码构建。准备 Windows x64 桌面环境、Rust 1.95 或更新版本的 MSVC 工具链，以及 Visual Studio C++ 构建工具和 Windows SDK。

在仓库根目录打开 PowerShell：

```powershell
cargo build -p lucidpane -p desktop-hook --locked
$env:LUCIDPANE_DATA_DIR = Join-Path $PWD 'target\dev-data-v9'
& .\target\debug\lucidpane.exe
```

上述命令使用独立的开发数据目录。`lucidpane.exe` 与 `desktop_hook.dll` 需要来自同一次构建并放在同一目录；启动前请退出已有的 LucidPane 实例。

启动后，将桌面图标拖入分组即可开始整理。通过托盘菜单新建分组、打开设置或退出。详细操作见[使用说明](docs/usage.md)，完整构建与测试步骤见[构建指南](docs/development/build.md)。

## 开发状态

桌面分组依赖经过校验的 Windows 系统组件版本；不兼容或 Explorer 暂不可用时，文件夹和搜索面板仍可运行，桌面分组保留配置并等待重连。全局设置保存到可编辑的 `config.toml`，布局保存到 `workspace.db`。开发阶段不迁移旧数据库，也不再读取 `hook-desktop.db`。

## 文档

详细文档统一存放在 [`docs`](docs/README.md)，开发资料单独放在 `docs/development`。

| 文档 | 内容 |
| --- | --- |
| [项目介绍](docs/README.md) | 功能概览、运行方式与适用范围 |
| [使用说明](docs/usage.md) | 分组操作、设置、数据位置与常见问题 |
| [开发文档](docs/development/README.md) | 构建、架构、绘图、存储与验证 |
| [历史资料](docs/development/history/README.md) | 早期方案、调研与实验记录 |
