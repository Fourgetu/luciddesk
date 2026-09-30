# Contributing

[简体中文](CONTRIBUTING.md) · English

Bug reports, features, translations and code contributions are welcome. Check existing [issues](https://github.com/Yuch3nE/luciddesk/issues) and [pull requests](https://github.com/Yuch3nE/luciddesk/pulls) before starting.

## Reports and proposals

Include reproduction steps, expected and actual behavior, Windows and LucidDesk versions, and the package type. For visual issues, include display scaling, monitor layout, material settings and a relevant screenshot. **Settings → About → Copy diagnostics** provides environment details.

Describe the use case and expected interaction for feature requests. Discuss larger changes in an issue first. Review and redact screenshots, logs, paths and configuration before sharing. Do not post private workspace databases, credentials or personal files in public issues. See the [privacy policy](PRIVACY.en.md).

## Development

Use Windows x64, rustup, Visual Studio C++ Build Tools and the Windows SDK. `rust-toolchain.toml` selects Rust; the repository script selects the Windows build environment. See the [build guide](docs/development/build.md) and [architecture](docs/development/architecture.md), currently in Chinese.

```powershell
git clone https://github.com/Yuch3nE/luciddesk.git
cd luciddesk
.\tools\use-windows-toolchain.ps1
cargo build -p luciddesk -p desktop-hook --locked
$env:LUCIDDESK_DATA_DIR = Join-Path $PWD 'target\dev-data'
.\target\debug\luciddesk.exe
```

Keep the EXE and desktop DLL from the same build together. Exit an existing instance before testing desktop integration. Native desktop tests affect the real Explorer session; run them in an environment where interruption is acceptable. Use a separate data directory for development.

## Pull requests

Keep each PR focused. Explain its purpose, behavior changes and validation; include screenshots for UI changes. Follow surrounding code conventions and avoid unrelated formatting, temporary files, certificates or private keys. Ensure you have permission to contribute code and assets under the project's [MIT License](LICENSE) and applicable third-party licenses.

Choose checks relevant to the change; documentation edits do not require a full application build:

```powershell
cargo check --workspace --all-targets --locked
cargo test -p desktop-core -p desktop-storage --lib --locked
python tools/check-locales.py
cargo test -p luciddesk --bin luciddesk i18n::tests --locked
```

UI, Explorer integration and installer changes also need the corresponding Windows tests described in the [validation guide](docs/development/validation.md). Lifecycle changes should cover normal exit, reconnection and file release.

For translations, follow the [localization guide](docs/development/localization.md), preserve resource keys and placeholders, and update all language resources for new strings. Update user documentation for visible behavior changes and follow the existing bilingual Changelog structure for release changes.

Use Conventional Commit types such as `feat`, `fix`, `docs`, `test`, `build` and `ci`. Keep discussions respectful, focus on evidence, and respond to reproduction and review feedback.
