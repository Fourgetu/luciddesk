# LucidPane

The primary application supports hybrid desktop mode only. Explorer retains the
desktop; LucidPane draws pane icons and controls with DirectWrite and Windows
composition. No Windows App SDK or WinUI 3 dependency is required.

## Build and run

```powershell
cargo build -p lucidpane -p desktop-hook --offline
target/debug/lucidpane.exe
```

Keep `desktop_hook.dll` beside the executable. Existing `--hybrid-desktop`
shortcuts and `--title <name>` remain compatible. Membership and settings remain
in `hook-desktop.db` in the existing data directory (`LUCIDPANE_DATA_DIR` can
override it). HookSession owns Hook cleanup; this application never takes over
Explorer using the old full-desktop hide/restore mechanism.

## Archived modes

The `main` branch contains only Hook + self-rendered panes. Historical modes,
the `legacy-app` package and their exclusive compositor library are preserved
on the local `leagcy` branch (branch name intentionally follows the requested
spelling). They are not part of main's source tree or workspace.

To work on the old application in that branch:

```powershell
git switch leagcy
cargo build -p lucidpane-legacy -p desktop-hook --offline
```

See [code boundaries](docs/mode-separation.md).
