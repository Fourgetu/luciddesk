# LucidPane

A Rust-native Windows desktop icon organizer. The current vertical slice provides:

- A borderless, draggable and resizable portal window.
- An interactive non-topmost tool pane that ordinary application windows can cover.
- A persistent workspace backed by SQLite.
- A customizable pane title and header icon, editable from the window and persisted across launches.
- Managed Desktop Mode by default: enumerate Explorer's complete Desktop Shell Namespace, hide
  Explorer's native icon view, and render ungrouped items on sparse per-monitor desktop surfaces.
- A new Pane starts empty. Dragging an icon between the free desktop and a Pane changes only its
  LucidPane placement; it never moves, copies, or deletes the underlying Shell item.
- A `--manual` fallback that creates an empty pane and accepts dropped filesystem references.
- Pane items use native system icons and open through the Windows Shell on double-click.
- A responsive, row-major icon grid that redistributes columns to fill the pane width.
- Double-click Shell activation, debounced Desktop Shell notifications, and F5 recovery refresh.
- Runtime-selectable Mica, Mica Alt, Desktop Acrylic, and translucent materials.

The application targets Windows 11 22H2 (build 22621) or newer.

## Run

```powershell
cargo run -p lucidpane
```

With no arguments, LucidPane starts in Managed Desktop Mode. Explorer still owns the files and Shell
semantics, while LucidPane owns the Desktop View and Layout. The default Pane is named `新建分组`
and starts empty. Existing desktop items initially remain on LucidPane's free desktop surface; drag
one onto the Pane to group it, or drag it back outside the Pane to return it to the desktop.

Desktop enumeration starts at `SHGetDesktopFolder` and walks the complete Shell Namespace rather
than manually concatenating the current-user and public desktop directories. This keeps virtual
objects such as Recycle Bin in the same inventory and gives same-named items distinct identities.
Desktop/Panes changes are transactionally committed when a drag, reorder, move/resize, title edit,
appearance change, refresh, display reconfiguration, or session shutdown completes.

While Managed Desktop Mode is running, Explorer's native desktop icons are hidden to prevent two
copies from appearing. Closing LucidPane restores the previous Explorer setting. A separate hidden
restore helper covers process crashes, and an on-disk takeover marker covers stale state after an
abnormal restart. To force Explorer's icon view visible, run:

```powershell
cargo run -p lucidpane -- --restore-shell
```

The public `SHGetSetSettings/SSF_HIDEICONS` path remains the first choice. Some current Explorer
builds revert that state, so `desktop-shell` contains one isolated compatibility fallback that hides
and restores Explorer's desktop `FolderView`; this private window lookup is not used by the layout
or identity model.

To run the earlier empty-pane workflow instead, use:

```powershell
cargo run -p lucidpane -- --manual
```

The manual pane is named `新建分组`. Drag files, folders, or shortcuts into it; LucidPane stores
references only and does not move or delete the real files.

Passing a folder path explicitly opens the optional Folder Portal mode instead:

```powershell
cargo run -p lucidpane -- "E:\Project"
```

Customize the pane title and header icon. `.ico` files are drawn directly; other files use their
Windows system icon:

```powershell
cargo run -p lucidpane -- --title "Work" --icon "E:\Icons\work.ico" "E:\Project"
```

Icons automatically flow from left to right and evenly fill the pane width; resizing immediately
recomputes the number and width of columns. Double-click an item to open it, press `F5` to refresh,
and use the mouse wheel to scroll. Drag the header to move the panel, double-click the header to
collapse it, drag its border to resize it, right-click to switch material, and press `Esc` to close
it. Geometry, title, header icon, collapse state, source, and material are restored on the next
launch.

Drag an icon within the Pane to change its persisted grid order. In managed mode, recursive
`SHChangeNotifyRegister` events are debounced before a complete namespace reconciliation. Newly
discovered Shell items receive a free-desktop position; deleted items disappear while surviving
items keep their Pane membership and location—even after a filesystem rename when Volume ID and
File ID are available. File and shortcut labels omit their last filename extension. The title bar
includes a visible menu button and close button.

Single-click an icon to select it with a visible highlight. Double-click it, or press `Enter` while
it is selected, to open it through the Windows Shell. Labels use the Shell display name with a
filename-stem fallback so shortcut names remain visible without `.lnk`.

Press `F2` or choose **Rename Pane** from the right-click menu to edit the title. The same menu can
choose a custom pane icon or restore the folder icon.

Legacy panes positioned against the top/left edge or saved with an abnormally large size are reset
once to the visible 420×360 default; title, icon, items, and appearance are preserved.

For isolated development or smoke tests, set `LUCIDPANE_DATA_DIR` to keep the SQLite database in a
custom writable directory.

## Architecture

- `desktop-core`: Shell identity, desktop placement, workspace, Pane, geometry, and material model.
- `desktop-window`: Shell-owned HWND lifecycle, per-monitor sparse desktop surfaces, Pane windows,
  hit testing, provisional GDI rendering, and input.
- `desktop-compositor`: DWM system backdrops and the Windows Composition visual tree.
- `desktop-shell`: Desktop Shell Namespace enumeration, native icon visibility, guarded process-exit
  detection, and filesystem/namespace Shell activation.
- `desktop-storage`: SQLite schema v5 and transactional identity/placement snapshots.
- `app`: wires the window and material controller together.

The two system materials are applied with `DWMWA_SYSTEMBACKDROP_TYPE`. Ordinary transparency is a
solid Composition color plus per-window alpha, so it contains no blur and remains a predictable
fallback.
