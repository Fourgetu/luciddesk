# 贡献指南

简体中文 · [English](CONTRIBUTING.en.md)

欢迎为 LucidDesk 反馈问题、改进功能、完善翻译或提交代码。先查看已有 [Issues](https://github.com/Yuch3nE/luciddesk/issues) 和 [Pull Requests](https://github.com/Yuch3nE/luciddesk/pulls)，避免重复工作。

## 反馈问题与建议

问题报告请包含复现步骤、预期与实际结果、Windows 版本、LucidDesk 版本及安装包类型。界面问题还请注明屏幕缩放、显示器布局和所选材质，附上必要的截图。可以使用“设置 → 关于 → 复制诊断”获取环境信息。

功能建议请描述实际使用场景及期望的操作方式。较大的功能或架构调整，建议先通过 Issue 讨论范围。不要在公开 Issue 中提交完整工作区数据库、含个人路径的配置、敏感截图或密钥；分享前请检查并脱敏，数据范围见[隐私政策](PRIVACY.md)。

## 准备开发环境

使用 Windows x64、rustup、Visual Studio C++ 构建工具和 Windows SDK。Rust 版本由 `rust-toolchain.toml` 指定，Windows 构建环境由仓库脚本选择。完整说明见[构建指南](docs/development/build.md)及[架构说明](docs/development/architecture.md)。

```powershell
git clone https://github.com/Yuch3nE/luciddesk.git
cd luciddesk
.\tools\use-windows-toolchain.ps1
cargo build -p luciddesk -p desktop-hook --locked
$env:LUCIDDESK_DATA_DIR = Join-Path $PWD 'target\dev-data'
.\target\debug\luciddesk.exe
```

EXE 与 `luciddesk_desktop.dll` 必须来自同次构建，并放在同一目录。测试桌面集成前退出已有实例；原生桌面测试会操作真实 Explorer，应在可中断的环境中执行。开发时使用独立数据目录，避免影响日常配置。

## 提交 Pull Request

保持一个 PR 聚焦一个问题，说明改动目的、行为变化和验证结果；涉及界面时附截图。沿用附近代码的风格，不混入无关格式修改、临时产物、证书或私钥。确保你有权提交新增代码和素材，遵循项目 [MIT 许可证](LICENSE)及相关第三方许可。

根据改动选择验证，不必为纯文档变更编译整个应用：

```powershell
cargo check --workspace --all-targets --locked
cargo test -p desktop-core -p desktop-storage --lib --locked
python tools/check-locales.py
cargo test -p luciddesk --bin luciddesk i18n::tests --locked
```

UI、Explorer 集成和安装卸载改动还需对应的 Windows 实机验证，见[验证与兼容边界](docs/development/validation.md)。应用主程序与 DLL 的生命周期修改尤其应验证正常退出、重连及文件释放。

翻译请阅读[本地化指南](docs/development/localization.md)，保留资源键与占位参数；新增文案同步维护各语言资源。可见行为变化同步更新使用说明，发布相关改动遵循双语 Changelog 的现有结构。

提交信息建议使用 `feat`、`fix`、`docs`、`test`、`build` 或 `ci` 等 Conventional Commits 类型。讨论时关注问题与证据，尊重不同观点，并对复现与审核反馈作出回应。
