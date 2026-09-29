# 构建与验证

本文命令均从仓库根目录执行，适用于 Windows PowerShell。

## 环境

- 主要使用 Windows 11 x64 交互桌面验证，Explorer 正常运行；Windows 10 已由用户完成实机验证，平台记录见[验证记录](validation.md)。
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

默认 feature 集为空，应用使用 `FilterSession` 与独立 Shell 菜单。诊断功能按需启用，发布包使用默认功能集。

## 诊断构建

| Feature | 所属包 | 用途 |
| --- | --- | --- |
| `desktop-menu-diagnostics` | desktop-shell | 在真实桌面选择项目的菜单诊断入口，默认关闭 |
| `menu-diagnostics` | desktop-hook、desktop-shell；app 同时转发 | 在 Release 中收集菜单计时，默认关闭；Debug 自动收集 |

诊断构建使用独立目录，避免与发布产物混用。菜单探针会操作真实桌面选择，应在可中断的交互会话中运行。

```powershell
# 真实桌面菜单诊断入口
cargo build -p desktop-shell --example desktop_menu_service_probe --features desktop-menu-diagnostics --target-dir target\desktop-menu-diagnostics --locked --offline

# 同时启用主程序、Hook 和 Shell 的 Release 菜单计时
cargo build -p luciddesk -p desktop-hook --release --features luciddesk/menu-diagnostics --target-dir target\menu-diagnostics --locked --offline
```

`tools/package.ps1` 在 `target\production` 构建并取件，显式禁用默认 feature；不要用 `--all-features` 生成发布包，以免启用诊断入口和计时。

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

生成包含同次构建的 EXE、Hook DLL、使用说明、构建信息与校验值的免安装发布包：

```powershell
.\tools\package.ps1 -Offline
```

产物位于 `target\packages`。包内配置仍默认保存在 LocalAppData；未提交代码会在包名和 `build.json` 中标记为 dirty。GitHub Actions 的 Build CI 工作流执行全目标编译、核心和存储测试后，同时上传非便携版、便携版 ZIP 及 SHA256 校验文件；原生桌面 UI 测试仍在交互会话运行。

便携版使用以下命令，产物位于 `target\portable\时间戳`，含 `portable.marker`，配置保存在包旁的 `data` 中：

```powershell
.\tools\package.ps1 -Portable -Offline
# 排查问题时另行生成带诊断脚本的便携包
.\tools\package.ps1 -Portable -RenderDiagnostics -Offline
```

诊断包提供 A（当前渲染路径）、B（共享合成树）、C（禁用背景特效）三个启动入口，具体开关与日志见[渲染诊断说明](../../tools/render-diagnostics/RENDER-TEST.md)。崩溃采集、可选转储配置及恢复步骤见[崩溃转储说明](../../tools/render-diagnostics/CRASH-DUMPS.md)。这些脚本不随常规便携包分发。分析转储前保留同次构建的 EXE、DLL 和 PDB；之后重新构建会覆盖 `target\production` 中的符号文件。

```powershell
cargo check --workspace --all-targets --offline
cargo test -p desktop-core -p desktop-storage -p desktop-hook -p desktop-shell --lib --offline -- --test-threads=1
cargo test -p luciddesk --bin luciddesk --offline -- --test-threads=1
cargo test -p luciddesk --test canvas_compat --offline
```

UI 测试按单线程执行，降低原生窗口与 COM 消息的相互干扰。部分 Shell 测试需要实际桌面权限；受限会话中的失败应与代码回归区分，并记录具体错误。

搜索层级测试会激活真实窗口，需单独运行。退出回归测试会启动三个测试子进程，检查图形资源释放后整个进程能否正常退出：

```powershell
cargo test -p luciddesk editor_click_raises_search_among_panes_but_hotkey_stays_on_desktop --offline -- --ignored --test-threads=1
cargo test -p luciddesk graphics_caches_release_before_apartment_and_process_exit --offline -- --test-threads=1
```

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

使用下方的 `filter_backend_probe` 验证成员过滤及恢复。

自动测试不能代替实际拖入、拖出、排序、重命名、退出恢复及混合 DPI 检查。最近记录见[验证记录](validation.md)。

## 视图过滤后端回归

退出 LucidDesk 后运行。探针会临时移除两个原生桌面项目，验证刷新、菜单暂停/恢复、坐标恢复和测试控制进程退出后的恢复，不修改磁盘文件。

```powershell
cargo build -p desktop-hook
cargo build -p desktop-shell --example filter_backend_probe
.\target\debug\examples\filter_backend_probe.exe
```

结果及兼容边界见[验证记录](validation.md)。

品牌更名兼容：若新的数据目录不存在且旧目录 `%LOCALAPPDATA%\LucidPane` 已存在，继续使用旧目录。`LUCIDDESK_DATA_DIR` 优先，旧变量 `LUCIDPANE_DATA_DIR` 仍受支持；详见[品牌规范](../brand.md)。

## GitHub Release

公开仓库 `Yuch3nE/luciddesk` 收到 `v<版本>` 标签（例如 `v0.10.5`）后，Build CI 核对标签与应用 Cargo 版本，完成检查与双版本打包，再发布对应 GitHub Release。同步本地仓库时需要一并同步标签。普通分支推送和 PR 只生成 Actions 产物，不创建 Release；也可在已有版本标签上手动运行工作流补发。

Release 包含两个 ZIP 及各自的 SHA256 文件。发布任务先验证校验值；正文从标签对应源码中的 `CHANGELOG.md` 提取匹配版本章节，保留标题、日期及完整内容，并将相对链接转换为该标签下的 GitHub 链接。章节缺失、重复或为空时中止发布。重跑时同步更新正文与同名附件。发布权限仅授予独立的 Release 任务。
