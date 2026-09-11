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

## Separate legacy application

```powershell
cargo build -p lucidpane-legacy -p desktop-hook --offline
target/debug/lucidpane-legacy.exe --preview
```

`legacy-app` is an independent package with its own entry, UI, state, tests and
recovery helper. It supports `--desktop`, `--managed-desktop`, `--native-desktop`,
`--hook-desktop`, `--manual` and folder preview, but no hybrid mode. Its native
Hook mode uses `legacy-hook-desktop.db`. Old full-desktop recovery is available
through `lucidpane-legacy.exe --restore-shell`.

Default workspace builds exclude this package. `--workspace` explicitly builds
both applications. Neither application imports source files from the other.
Only the reusable libraries under `crates/` are shared.

See [code boundaries](docs/mode-separation.md) and [historical mode notes](docs/legacy-modes.md).
