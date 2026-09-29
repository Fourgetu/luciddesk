<div align="center">

# LucidDesk

[简体中文](README.md) · English

**Turn your Windows desktop into a workspace that works for you.**

Desktop groups · Folder panels · Everything search · Spacebar preview

[![Build CI](https://github.com/Yuch3nE/luciddesk/actions/workflows/build.yml/badge.svg)](https://github.com/Yuch3nE/luciddesk/actions/workflows/build.yml)
[![Version](https://img.shields.io/badge/version-0.10.6-087EA4?style=flat-square)](CHANGELOG.en.md)
[![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4?style=flat-square)](#compatibility)

[![Architecture](https://img.shields.io/badge/arch-x64-475569?style=flat-square)](docs/portable.md)
[![Rust](https://img.shields.io/badge/Rust-1.95%2B-CE6F32?style=flat-square)](Cargo.toml)

[Downloads](https://github.com/Yuch3nE/luciddesk/releases) · [Getting started](#getting-started) · [Changelog](CHANGELOG.en.md) · [Contributing](#contributing)

</div>

![LucidDesk feature overview: organize desktop icons into tabbed groups, browse folders, and find local files with Everything.](docs/images/overview.en.svg)

> Feature illustration, not an actual screenshot.

LucidDesk is a Windows desktop organizer written in Rust. Group desktop icons, keep frequently used folders within reach, and find local files with Everything.

## Features

- **Desktop groups:** organize files, folders and shortcuts without moving the original files.
- **Group tabs:** switch between groups, reorder tabs, or detach them into separate panels.
- **Folder panels:** browse directories with sorting and automatic updates.
- **File search:** search through Everything and open files or their locations.
- **File preview:** press Space to preview selected files with PowerToys Peek or QuickLook.
- **Appearance:** customize fonts, rounded corners, colors, and solid, Acrylic, Mica or Mica Alt backgrounds.

Panels can move, resize, collapse, snap to edges, lock, or stay on top. Folder and search panels remain independent of group tabs. File commands such as delete, rename, cut and paste operate on real files.

## Getting started

1. Download a ZIP from [GitHub Releases](https://github.com/Yuch3nE/luciddesk/releases). Choose `windows-x64-portable` to keep settings beside the app.
2. Exit any running copy and extract the entire ZIP to a writable folder.
3. Run `luciddesk.exe`. Keep `desktop_hook.dll` beside it; portable mode also requires `portable.marker`.
4. Drag desktop icons into a group. Use the tray menu to create groups or folder panels and open Settings.

Search and preview are disabled by default. Install Everything, PowerToys Peek or QuickLook separately, then enable the corresponding integration in Settings. These tools are not bundled.

## Languages

The interface supports 简体中文, 繁體中文, English, 日本語, 한국어, Deutsch and Русский. It follows the Windows display language by default and falls back to English for unsupported languages. Choose a language in **Settings → Language**, then restart LucidDesk.

Translations are embedded in the executable. Saved group names and file names are unchanged. Native Windows Shell menus follow the system language. Translation contributions and corrections are welcome.

## Upgrading and backups

Exit the app before upgrading. **Preserve the `data` folder when replacing portable app files.** The standard package stores settings in `%LOCALAPPDATA%\LucidDesk`; portable mode stores them in `data` beside the executable. Upgrades may continue using an existing `LucidPane` data directory. To switch from standard to portable mode, export and restore your configuration.

Use **Settings → Backup & restore** to export or restore your configuration. Backups contain settings and layouts, not the original files referenced by groups. Moving a portable folder does not move those original files.

## Compatibility

Windows 11 x64 is the primary maintenance platform. Windows 10 has also been tested by a user and supports solid color and Acrylic backgrounds. Mica options are available on Windows 11. ARM64 and Remote Desktop have not been fully validated.

When Windows disables background effects, LucidDesk uses a theme-aware solid fallback and retains your material selection.

## Troubleshooting

**Panels are hidden:** click the tray icon or choose **Show panels**. Normal panels can still be covered when you switch to another app; panels set to stay on top remain above other windows.

**Desktop groups cannot connect:** check the connection status in **Settings → About** and try reconnecting. LucidDesk keeps the group configuration and retries automatically. Folder and search panels can still work independently.

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

Keep the executable and Hook DLL from the same build together. See the [build guide](docs/development/build.md) for checks and diagnostic builds.

## Contributing

The public [GitHub repository](https://github.com/Yuch3nE/luciddesk) is synchronized from the maintainer's local development repository. Issues and pull requests are welcome.

Include reproduction steps, Windows version, and the information from **Settings → About → Copy diagnostics** in bug reports. For rendering issues, include a screenshot, display scaling and the selected material. Review logs for personal file paths before sharing them.

For translations, see the [localization guide](docs/development/localization.md). Other documentation is currently maintained in Chinese: [user guide](docs/usage.md), [portable package](docs/portable.md), and [architecture](docs/development/architecture.md).

Author: **Yuchen95**.
