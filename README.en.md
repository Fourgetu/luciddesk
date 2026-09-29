<div align="center">

<img src="docs/images/app-icon.png" width="128" height="128" alt="LucidDesk" />

# LucidDesk

[简体中文](README.md) · English

**Turn your Windows desktop into a workspace that works for you.**

Desktop panels · Folder panels · Everything search · Spacebar preview

[![License: MIT](https://img.shields.io/badge/license-MIT-green?style=flat-square)](LICENSE)
[![Build CI](https://github.com/Yuch3nE/luciddesk/actions/workflows/build.yml/badge.svg)](https://github.com/Yuch3nE/luciddesk/actions/workflows/build.yml)
[![Version](https://img.shields.io/badge/version-0.12.0-087EA4?style=flat-square)](CHANGELOG.en.md)
[![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4?style=flat-square)](#compatibility)

[![Architecture](https://img.shields.io/badge/arch-x64-475569?style=flat-square)](docs/portable.md)
[![Rust](https://img.shields.io/badge/Rust-1.95%2B-CE6F32?style=flat-square)](Cargo.toml)

[Downloads](https://github.com/Yuch3nE/luciddesk/releases) · [Getting started](#getting-started) · [Changelog](CHANGELOG.en.md) · [Contributing](#contributing)

</div>

![LucidDesk feature overview: organize desktop icons into tabbed panels, browse folders, and find local files with Everything.](docs/images/overview.en.svg)

> Feature illustration, not an actual screenshot.

LucidDesk is a Windows desktop organizer written in Rust. Organize desktop icons into panels, keep frequently used folders within reach, and find local files with Everything.

While icons load, panels show scalable file or folder outlines suited to light and dark themes, then replace them with the actual icons.

Folder panels support natural name sorting, with newest-first dates and largest-first sizes. Search refreshes restore selections within the previously loaded result range.

Find fonts by name in a continuously scrolling list filtered for the interface language. Candidates load in the background on first use, without font previews, and the list is released when settings closes.

## Features

- **Desktop panels:** organize files, folders and shortcuts without moving the original files.
- **Panel tabs:** switch between panels, reorder tabs, or detach them into separate panels.
- **Folder panels:** browse directories with sorting and automatic updates.
- **File search:** search through Everything and open files or their locations.
- **File preview:** press Space to preview selected files with PowerToys Peek or QuickLook.
- **Appearance:** customize fonts, rounded corners, colors, and solid, Acrylic, Mica or Mica Alt backgrounds.

Panels can move, resize, collapse, snap to edges, lock, or stay on top. Folder and search panels remain independent of panel tabs. File commands such as delete, rename, cut and paste operate on real files.

## Getting started

### Choose a package

Download a ZIP from [GitHub Releases](https://github.com/Yuch3nE/luciddesk/releases). **Both packages run without installation and provide the same features; they differ in where settings are stored.**

| Package | File name | Default settings location | Best suited for |
| --- | --- | --- | --- |
| Standard (non-portable) | Contains `windows-x64`, without `portable` | `%LOCALAPPDATA%\LucidDesk` | Keeping settings separate from app files under one Windows account |
| Portable | Contains `windows-x64-portable` | The `data` folder beside the executable | Carrying settings with the app folder |

See the [standard package guide](docs/package.md) or [portable guide](docs/portable.md) (Chinese). If no release is available, [build from source](#build-from-source).

### Extract and launch

1. Exit any running copy and extract the entire ZIP to a writable folder.
2. Run `luciddesk.exe`. Keep `luciddesk_desktop.dll` beside it; portable mode also requires `portable.marker`. Do not run the app inside the ZIP.
3. Drag desktop icons into a panel. Use the tray menu to create panels or folder panels and open Settings.

Search and preview are disabled by default. Install Everything, PowerToys Peek or QuickLook separately, then enable the corresponding integration in Settings. These tools are not bundled.

An optional **Show all panels** global shortcut is available in **Settings → Panel layout**. It is off by default, with `Ctrl + Shift + D` as the preset. It performs the same action as **Show panels** in the tray and preserves each panel’s always-on-top setting.

## Languages

The interface supports 简体中文, 繁體中文, English, 日本語, 한국어, Deutsch and Русский. It follows the Windows display language by default and falls back to English for unsupported languages. Choose a language in **Settings → Language**; it applies immediately.

Language changes update open windows in place while preserving panels, item selection and scroll positions. Font search text is retained, and candidates are filtered for the new language in the background.

Translations are embedded in the executable. Saved panel names and file names are unchanged. Native Windows Shell menus follow the system language. Translation contributions and corrections are welcome.

## Upgrading and backups

Exit the app before upgrading and replace the complete set of app files so the executable and DLL come from the same build.

- **Standard:** keep the settings directory, which is separate from the app folder and defaults to `%LOCALAPPDATA%\LucidDesk`.
- **Portable:** preserve the `data` folder and `portable.marker` beside the executable.

Upgrades may continue using an existing `LucidPane` data directory. To switch from standard to portable mode, export and restore your configuration.

Use **Settings → Backup & restore** to export or restore your configuration. Backups contain settings and layouts, not the original files referenced by panels. Moving a portable folder does not move those original files.

## Compatibility

Windows 11 x64 is the primary maintenance platform. Windows 10 has also been tested by a user and supports solid color and Acrylic backgrounds. Mica options are available on Windows 11. ARM64 and Remote Desktop have not been fully validated.

When Windows disables background effects, LucidDesk uses a theme-aware solid fallback and retains your material selection.

## Troubleshooting

**Panels are hidden:** click the tray icon or choose **Show panels**. Normal panels can still be covered when you switch to another app; panels set to stay on top remain above other windows.

**Desktop panels cannot connect:** check the connection status in **Settings → About** and try reconnecting. LucidDesk keeps the panel configuration and retries automatically. Folder and search panels can still work independently.

## Build from source

Requires Windows, Rust 1.95 or newer with the MSVC toolchain, Visual Studio C++ Build Tools and the Windows SDK.

```powershell
git clone https://github.com/Yuch3nE/luciddesk.git
cd luciddesk
cargo build -p luciddesk -p desktop-hook --locked
.\target\debug\luciddesk.exe
```

Build packages from the repository root:

```powershell
.\tools\package.ps1
.\tools\package.ps1 -Portable
```

Standard ZIPs are written to `target/packages/`; portable ZIPs are written to `target/portable/<timestamp>/`. Keep the executable and Hook DLL from the same build together. See the [build guide](docs/development/build.md) for checks and diagnostic builds.

## Contributing

The public [GitHub repository](https://github.com/Yuch3nE/luciddesk) is synchronized from the maintainer's local development repository. Issues and pull requests are welcome.

Include reproduction steps, Windows version, and the information from **Settings → About → Copy diagnostics** in bug reports. For rendering issues, include a screenshot, display scaling and the selected material. Review logs for personal file paths before sharing them.

For translations, see the [localization guide](docs/development/localization.md). Other documentation is currently maintained in Chinese: [user guide](docs/usage.md), [portable package](docs/portable.md), and [architecture](docs/development/architecture.md).

Author: **Yuchen95**.

## License

LucidDesk is licensed under the [MIT License](LICENSE). Third-party dependencies remain under their respective licenses.
