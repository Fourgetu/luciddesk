# 构建与验证

本文命令均从仓库根目录执行，适用于 Windows PowerShell。

## 环境

- 主要使用 Windows 11 x64 交互桌面验证，Explorer 正常运行；Windows 10 已由用户完成实机验证，平台记录见[验证记录](validation.md)。
- Rust MSVC 工具链；`rust-toolchain.toml` 固定本地与 CI 使用 Rust 1.98.1，使用 edition 2024。
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

产物位于 `target\packages`。包内配置仍默认保存在 LocalAppData；未提交代码会在包名和 `build.json` 中标记为 dirty。GitHub Actions 的 Build CI 工作流执行全目标编译、核心和存储测试后，同时上传安装包、普通 ZIP、便携 ZIP 及 SHA256 校验文件；原生桌面 UI 测试仍在交互会话运行。

安装包使用固定版本的 Inno Setup 编译器。以下命令安装经 SHA256 校验的官方编译器到 `target\tooling`，生成普通 ZIP 和安装包；已安装其他版本的编译器时可传 `-InnoCompiler <ISCC.exe>`：

```powershell
.\tools\ensure-inno.ps1
.\tools\package.ps1 -Installer -Offline
```

安装包位于 `target\installers\版本-修订-时间戳`。安装器支持当前用户或所有用户安装，后者申请管理员权限并默认安装到 Program Files；快捷方式与卸载注册分别使用对应范围。固定 AppId，升级沿用原安装范围和目录，确认后请求应用正常退出并等待进程结束，阻止降级并保留用户数据。无响应时停止安装；`/NOCLOSEAPPLICATIONS` 禁用自动关闭。中文语言文件来自 Inno Setup 官方仓库 `is-6_7_3/Files/Languages/Unofficial/ChineseSimplified.isl`，保留文件中的译者署名。

关于页仅支持手动检查新版本与打开 Release 页面。检查使用后台 WinHTTP 请求 GitHub Releases API，支持 Windows 代理和显式 `HTTPS_PROXY` HTTP 代理，并比较稳定版本号（标签可有 `v` 前缀）。更新包由用户在浏览器中自行下载。

验证命令（安装器测试使用随机测试 AppId、快捷方式名称和工作区临时目录，安装与卸载自己的测试注册项）：

```powershell
cargo test -p luciddesk --bin luciddesk updates::tests --locked --offline
.\tools\test-installer.ps1 -SourcePath <普通ZIP解压目录>
# 在管理员 PowerShell 中验证 Program Files 安装、升级和卸载
.\tools\test-installer.ps1 -AllUsers -SourcePath <普通ZIP解压目录>
# 可选：检查实际 GitHub Release 元数据
cargo test -p luciddesk --bin luciddesk updates::tests::live_release_check --locked --offline -- --ignored --exact
```

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

Build CI 在推送标签或 `codex/ci-compare-*` 比较分支时运行，不限定 `v` 前缀；普通分支推送和 PR 不触发，手动启动用于比较构建产物，仅上传 Actions 附件，不发布 Release。发布标签支持 `<版本>` 和 `v<版本>`（例如 `0.14.0` 或 `v0.14.0`）。公开仓库 `Yuch3nE/luciddesk` 核对标签与应用 Cargo 版本，完成检查与双版本打包，再发布对应 GitHub Release；其他标签会在版本校验阶段报错。同步本地仓库时需要一并同步标签；已触发的运行可在 Actions 页面重新运行。

Release 包含一个安装包、两个 ZIP 及各自的 SHA256 文件，均附带中英文更新记录。发布任务先验证校验值；正文从标签对应源码中的 `CHANGELOG.md` 和 `CHANGELOG.en.md` 分别提取匹配版本章节，按中文、英文顺序合并，中间使用分隔线，不添加语言大标题，只显示一次版本标题与日期。新增功能和问题修复分别放在 `feat`、`fix` 分类下，文档和发布流程使用 `docs`、`ci` 分类；保留完整内容，并将相对链接转换为该标签下的 GitHub 链接。任一语言的章节缺失、重复或为空时中止发布；构建阶段会运行双语提取测试。重跑时同步更新正文与同名附件。发布权限仅授予独立的 Release 任务。

推送到 `main` 的更新记录或生成器改动会触发 `Sync release notes`，只同步当前应用版本已存在的 Release 正文，支持带或不带 `v` 的标签。这个任务不编译程序，不创建或移动标签，也不替换附件；修改旧版本说明时需同步对应版本的 Release 正文。


## 版本编号

应用版本以 `app/Cargo.toml` 为唯一构建来源；关于页和 Windows 文件版本自动读取它。Git 中最后一次明确的应用版本提交作为递增基准，标签用于标识发布点；标签落后时不能回退到旧版本。一个发布批次按影响最大的变更递增一次，不按提交数量累计。

项目在 `0.x` 阶段采用以下约定：新增功能递增次版本并将修订号归零；只有兼容修复时递增修订号。破坏兼容性的变更在 `0.x` 阶段递增次版本并明确记录迁移要求；稳定的兼容性承诺从 `1.0.0` 开始。进入 `1.x` 后，破坏兼容的变更递增主版本。参见 [Semantic Versioning 2.0.0](https://semver.org/lang/zh-CN/)。

已有版本记录和 Git 标签不追溯重编号。尚未发布的变更先记录在双语 Changelog 的“未发布 / Unreleased”章节，确定版本后移入对应版本章节。发布时同步 `Cargo.lock`、双语 README 徽章和双语 Changelog；CI 验证版本一致性，发布标签必须为 `<应用版本>` 或 `v<应用版本>`。

构建环境统一：CI 使用 `windows-2025-vs2026`，本地与 CI 共用 Rust 1.98.1、Inno Setup 6.7.3，以及 `tools/windows-toolchain.json` 指定的 MSVC 14.51.36231（链接器 14.51.36257.0）和 SDK 10.0.26100.0。`tools/use-windows-toolchain.ps1` 选择 x64 构建环境、校验 cl/link/lib/rc 的 SHA256 并显式指定 Cargo 链接器；工具缺失或哈希不符时停止，不回退。`build.json` 记录 Rust、Cargo、MSVC、SDK、工具校验值及 CI runner 版本。时间戳和构建路径仍可能造成二进制差异；校验值不同本身不代表文件被篡改。

直接执行 Cargo 构建前，在同一 PowerShell 会话执行 `.\tools\use-windows-toolchain.ps1`；发布打包脚本自动执行该步骤。安装或更新 Visual Studio 后若工具哈希变化，应先核对来源并更新固定配置。
