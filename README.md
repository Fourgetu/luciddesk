# LucidPane

## Native Hook migration status (2026-09-08)

The requested Hook backend now supports experimental native grouping on the **verified x64
comctl32 image only** (6.0.26100.8972; full image hash and target prologues must match). Build
`cargo build -p desktop-hook -p lucidpane`, then run `target/debug/lucidpane.exe --hook-desktop`.
It redirects the existing Explorer item's geometry and hit testing: icons, labels, selection and
input remain Explorer's, files remain in place, and automatic arrangement stays enabled.
There is one native icon per item, not a copied pane icon plus a hidden original.

Native desktop drag-in/drag-out was confirmed by the user. Move/resize proposals now update icon
geometry during the native move loop. The controller caches Shell identities and baseline positions;
moving a pane and dropping a known item no longer enumerate the desktop. A transaction publishes
the layout and completes Explorer's repaint before accepting the frame proposal. Drop detection
still uses a 25 ms input timer. Configuration is saved at the end of pane movement.
Whole layouts now use a single bounded IPC batch, and repainting is limited to changed icons'
old/new/native bounds. Identical layouts do not request another repaint.

This is still an experimental migration. The Hook frame does not yet have the former renderer's
materials, collapse animation or snapping. Native-menu visual behavior and real desktop drag
smoothness require interactive verification. The earlier work-area experiment crashed Explorer
once; that backend remains blocked on owner-data views and is not used by geometry mode.

See [Hook validation and remaining work](docs/hook-backend-validation.md). The renderer described
below remains the existing default; use the explicit Hook switch for this migration.

The default now uses a **replacement desktop with Fences-style panes**. Run:

```powershell
cargo run -p lucidpane --release
```

Explorer's icon view is hidden only after the replacement windows and images are ready. Unassigned
icons stay on transparent desktop surfaces at their original positions. Drag them into a pane,
between panes, or back onto an empty desktop area. File paths and Explorer's automatic-arrangement
setting are unchanged. Membership and positions are saved in `redrawn-desktop.db`.

- Reads Explorer's icon size, grid spacing, display names and initial item positions, with DPI
  awareness initialized before enumeration. Labels use the system desktop LOGFONT and GDI metrics;
  neutral translucent highlights fit the icon and measured text rather than the full grid row.
- Uses Shell images/thumbnails and base system-image-list icons for shortcuts, with shortcut arrows hidden in LucidPane. Right-click an
  icon to open its Shell context menu; right-click a pane's header for pane controls.
- Main windows draw directly into a Direct2D/DXGI swap chain, with cached icons and labels
  retained across resizing. Dismissing an item menu no longer clears the image inventory;
  refresh keeps current images visible while replacements load and skips unchanged panes.
- Retains Acrylic/Mica, snapping, animated collapse, and optional hover expansion.
- Refreshes desktop inventory every three seconds; F5 reloads images. New icons remain reachable;
  if the desktop grid is full, overflow goes into the first scrollable pane.
- Exiting restores the previous native visibility. A separate watchdog restores it after an
  unexpected process exit. Display-topology changes or Explorer restarts end the takeover and
  restore Explorer; restart LucidPane to capture the new desktop configuration.
- Emergency recovery: `lucidpane.exe --restore-shell`.

This is **not certified pixel-for-pixel Explorer parity**. Text rasterization, some overlay and
thumbnail details, high-contrast styling, mixed-DPI behavior, marquee/multiple selection, inline
rename, external OLE drag-and-drop and full accessibility still need work. The current Shell
context menu is the classic extension menu, not the Windows 11 compact menu.

See [desktop implementation and validation](docs/redrawn-desktop-implementation.md).

## Independent preview

Use `cargo run -p lucidpane --release -- --preview` to keep native desktop icons visible.

It opens a Desktop-items container and an empty group. It leaves Explorer visible and does not
change automatic arrangement or move real files. The desktop items shown here are references.

- Native Acrylic and Mica backgrounds, opaque Shell icons/thumbnails, and DirectWrite labels.
- Direct2D/WIC content presented through a premultiplied DirectComposition swap chain over the DWM material.
- Right-click → Background: Acrylic / Mica. Each group's choice is saved; new groups default to Acrylic.
  Native materials require Windows 11 build 22621 or later; unsupported systems use a solid fallback.
  Acrylic uses an independent host-backdrop composition layer and remains visible when unfocused.
  Windows still controls transparency-preference and power-saving behavior; Mica retains the system policy.
- Asynchronous desktop enumeration and images; no synchronous legacy icon extraction on startup.
- Drag references between preview groups or reorder them within a group. Dropping outside the
  preview windows cancels the move. Source and destination automatically fill their grids.
- Drag the title to move the complete container; resize its border for automatic column reflow.
- Moving near another pane snaps its visible edges with a 12-DIP gap and a 14-DIP attraction range.
- Toggle “自动收起” in each pane's menu (off by default). Hover for about 120ms to expand;
  leaving for about 600ms folds it with animation. The setting persists independently per pane.
- Double-click the title (or click the chevron) to fold/unfold. Wheel scrolling, arrow selection,
  and Enter/double-click Shell opening are implemented.
- Right-click for a new group, collapse/expand, name sorting, or exit. Esc exits all preview windows.
- The menu is now a WinUI 3-inspired acrylic flyout with a material submenu and a short fade-in.
  Esc dismisses the submenu/menu before exiting the preview. This is custom Rust rendering, not a WinUI 3 control;
  UI Automation semantics for individual flyout items are still pending.
- Group membership, order, geometry and collapse state are stored separately in `preview.db`.
- An explicit folder path previews that folder, using the separate `folder-preview.db`.

This implements the first visual/layout prototype, **not the complete desktop integration**.
External OLE drag-and-drop, multi-selection, rename UI, accessibility,
desktop-layer attachment and automatic classification are not yet implemented. Current icon layout
uses a 48-DIP baseline with 96px Shell images; matching every desktop icon-size/overlay preference
and asynchronous refresh are still pending. Do not use `--managed-desktop` as a new-renderer mode:
it is the old implementation.

See [implementation and validation notes](docs/preview-implementation.md).

## Rejected outline-frame prototype (historical)

The following describes the earlier `--native-desktop` mode; it is no longer the default.

A Rust-native Windows desktop icon organizer. **The earlier outline prototype preserves Explorer's native
desktop and adds group frames only.** It does not hide, redraw, rename, resize, or rearrange
desktop icons on startup.

```powershell
cargo run -p lucidpane -- --native-desktop
```

- Explorer continues to draw all icons and labels and handle selection, double-click, native
  context menus, and drag-and-drop.
- Frames have a neutral translucent title and border. The center is entirely absent from the
  window region: it neither paints over the icons nor intercepts their mouse input.
- Drag native icons into or out of the outlined area using the ordinary desktop interaction.
  Drag the title to move the frame and the native icons whose origins lie inside it; resize the
  border to change the grouping area. Icons move when the title drag ends.
- Right-click the title for **新建分组**, **重命名分组**, **移动框内图标**, **移除分组框**, and
  **退出 LucidPane**. Uncheck **移动框内图标** to position the frame around existing icons.
  Renaming commits when the edit box loses focus. Removing a frame leaves its icons in place.
- Native positioning uses `IFolderView2` via the desktop Shell service. Auto-arrange and
  snap-to-grid settings are preserved; a group move blocked by auto-arrange reports the reason.
- Frame titles and geometry are saved in `native-frames.db`. The old `lucidpane.db` is retained
  separately, so old replacement-desktop placements are never replayed into Explorer.

This is a native **outline grouping** implementation. A filled translucent background behind
native icons, roll-up that hides grouped icons, automatic grouping, and full Fences parity are
not implemented. Overlapping frames use spatial membership, not exclusive membership. Geometry
currently follows the window host's screen-pixel convention; complete monitor/DPI layout recovery
remains future work.

Validation (run from a normal interactive Windows desktop; sandboxed processes may be denied
access to Explorer's COM service):

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The native integration checks read Explorer with an empty selection and verify that the frame's
center remains outside its window region after resizing. They do not exercise real icon moves.

## Legacy experimental modes

The old replacement desktop is available only through an explicit opt-in:

```powershell
cargo run -p lucidpane -- --managed-desktop
```

It hides and redraws the desktop and **does not preserve the native appearance**. The following
section documents that earlier implementation, not the current default:

- A borderless, draggable and resizable portal window.
- An interactive non-topmost tool pane that ordinary application windows can cover.
- A persistent workspace backed by SQLite.
- A customizable pane title and header icon, editable from the window and persisted across launches.
- Experimental Managed Desktop Mode: enumerate Explorer's complete Desktop Shell Namespace, hide
  Explorer's native icon view, and render ungrouped items on sparse per-monitor desktop surfaces.
- A new Pane starts empty. Dragging an icon between the free desktop and a Pane changes only its
  LucidPane placement; it never moves, copies, or deletes the underlying Shell item.
- A `--manual` fallback that creates an empty pane and accepts dropped filesystem references.
- Pane items use native system icons and open through the Windows Shell on double-click.
- A responsive, row-major icon grid that redistributes columns to fill the pane width.
- Double-click Shell activation, debounced Desktop Shell notifications, and F5 recovery refresh.
- Runtime-selectable Mica, Mica Alt, Desktop Acrylic, and translucent materials.

The application targets Windows 11 22H2 (build 22621) or newer.

## Run the legacy managed desktop

```powershell
cargo run -p lucidpane -- --managed-desktop
```

With `--managed-desktop`, LucidPane starts in Managed Desktop Mode. Explorer still owns the files and Shell
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
