# 构建与验证

本文命令均从仓库根目录执行，适用于 Windows PowerShell。

## 环境

- Windows x64 交互桌面，Explorer 正常运行。
- Rust MSVC 工具链；仓库声明的最低 Rust 版本为 1.95，使用 edition 2024。
- Visual Studio C++ 构建工具与 Windows SDK，用于链接 Win32 库。

首次获取依赖时省略 `--offline`。已有锁文件和依赖缓存后，可按以下方式离线构建。

## 构建与启动

```powershell
cargo build -p luciddesk -p desktop-hook --locked --offline
$env:LUCIDDESK_DATA_DIR = Join-Path $PWD 'target\dev-data'
& .\target\debug\luciddesk.exe
```

主程序与 Hook DLL 必须来自同次构建，位于同一目录。当前应用使用视图过滤协议 v1，数据库为 `workspace.db`，全局设置为 `config.toml`。不使用旧运行模式参数，也不提供旧数据库迁移。

默认 feature 集为空，应用使用 `FilterSession` 与独立 Shell 菜单。`main` 已移除旧几何后端、MinHook、专用探针和 `legacy-geometry`／`drag-trace`／`input-trace` 开关；完整旧方案及主程序接入保留在 `hook` 分支（清理时指向 `403463c`）。复现旧方案应使用该分支的独立工作目录及构建说明。

## 显式启用实验与诊断

| Feature | 所属包 | 用途 |
| --- | --- | --- |
| `desktop-menu-diagnostics` | desktop-shell | 在真实桌面选择项目的旧菜单对照入口，默认关闭 |
| `menu-diagnostics` | desktop-hook、desktop-shell；app 同时转发 | 在 Release 中收集菜单计时，默认关闭；Debug 自动收集 |

旧实验使用独立目录，避免与默认产物混用。调用探针前仍需阅读其交互范围。

```powershell
# 旧桌面菜单入口仅供对照实验
cargo build -p desktop-shell --example desktop_menu_service_probe --features desktop-menu-diagnostics --target-dir target\desktop-menu-diagnostics --locked --offline

# 同时启用主程序、Hook 和 Shell 的 Release 菜单计时
cargo build -p luciddesk -p desktop-hook --release --features luciddesk/menu-diagnostics --target-dir target\menu-diagnostics --locked --offline
```

`tools/package-preview.ps1` 在 `target\production` 构建并取件，显式禁用默认 feature；不要用 `--all-features` 生成发布包，以免启用诊断入口和计时。

`LUCIDDESK_DATA_DIR` 仅影响该环境下启动的程序。无需自定义目录时，在启动前移除该环境变量，程序会使用 LocalAppData。

可选标题参数示例（会设置首个面板的标题，包括已有工作区中的首个面板）：

```powershell
& .\target\debug\luciddesk.exe --title '工作'
```

如果正在运行的程序占用了原构建产物，可以先编译到独立目录：

```powershell
cargo build -p luciddesk -p desktop-hook --locked --offline --target-dir target\convergence-check
```

退出旧实例后，再从 `target\convergence-check\debug` 启动新程序。不要同时运行新旧实例以测试桌面 Hook。

## 自动检查

生成包含同次构建的 EXE、Hook DLL、使用说明、构建信息与校验值的免安装预览包：

```powershell
.\tools\package-preview.ps1 -Offline
```

产物位于 `target\preview`。包内配置仍默认保存在 LocalAppData；未提交代码会在包名和 `build.json` 中标记为 dirty。GitHub Actions 的 Windows preview 工作流执行全目标编译、核心和存储测试后上传 ZIP；原生桌面 UI 测试仍在交互会话运行。

```powershell
cargo check --workspace --all-targets --offline
cargo test -p desktop-core -p desktop-storage -p desktop-hook -p desktop-shell --lib --offline -- --test-threads=1
cargo test -p luciddesk --bin luciddesk --offline -- --test-threads=1
cargo test -p luciddesk --test canvas_compat --offline
```

UI 测试按单线程执行，降低原生窗口与 COM 消息的相互干扰。部分 Shell 测试需要实际桌面权限；受限会话中的失败应与代码回归区分，并记录具体错误。

设置页渲染测试默认不写图片。需要视觉检查时，设置环境变量 `LUCIDPANE_TEST_EXPORT_SNAPSHOTS=1` 后运行 `settings_layout_and_rendering_at_multiple_scales`，图片输出至 `target/settings-*.bmp`；检查后移除该环境变量即可恢复无图片写入的常规测试。

Canvas 集成测试可能连带构建主程序；若可执行文件正被占用，在命令末尾添加 `--target-dir target\convergence-check`。

托盘交互测试默认跳过，需要在实际 Windows 通知区域中单独执行：

```powershell
cargo test -p luciddesk tray::tests --bin luciddesk --offline -- --ignored --test-threads=1
```

## 生成绑定

生成器是独立工具，不是应用构建依赖：

```powershell
cargo run --locked --offline --manifest-path tools/windows-bindings/Cargo.toml
cargo run --locked --offline --manifest-path tools/windows-bindings/Cargo.toml -- --check
```

第一条更新生成源码，第二条生成到临时目录并核对一致性。修改 API 筛选清单时，同时提交筛选文件、工具锁文件和生成结果，不手工编辑生成文件。边界说明见[绘图与绑定](rendering.md)。

## 探针与真实桌面验证

当前主线使用下方的 `filter_backend_probe` 验证成员过滤。旧几何探针、`native_backdrop_probe` 和 Hook／绘图联合消融脚本已移至历史方案范围，仅在 `hook` 分支复现；历史文档中的对应命令不适用于当前主线。

自动测试不能代替实际拖入、拖出、排序、重命名、退出恢复及混合 DPI 检查。最近记录见[验证记录](validation.md)。

## 视图过滤后端回归

退出 LucidDesk 后运行。探针会临时移除两个原生桌面项目，验证刷新、菜单暂停/恢复、坐标恢复和测试控制进程退出后的恢复，不修改磁盘文件。

```powershell
cargo build -p desktop-hook
cargo build -p desktop-shell --example filter_backend_probe
.\target\debug\examples\filter_backend_probe.exe
```

结果及兼容边界见[视图过滤验证](../desktop-view-filter-verification.md)。

品牌更名兼容：若新的数据目录不存在且旧目录 `%LOCALAPPDATA%\LucidPane` 已存在，继续使用旧目录。`LUCIDDESK_DATA_DIR` 优先，旧变量 `LUCIDPANE_DATA_DIR` 仍受支持；详见[品牌规范](../brand.md)。

便携包构建：运行 `./tools/package-preview.ps1 -Portable`（可加 `-Offline`）。产物写入 `target/portable/时间戳/`，包含 `portable.marker`，不包含个人数据。
