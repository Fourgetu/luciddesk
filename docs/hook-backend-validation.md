# Native Hook backend: validation and limits

Status on 2026-09-08: **experimental native geometry backend working on the verified image**.
The user authorized live testing after exiting Fences. Geometry mode modifies reviewed in-memory
function entry points through MinHook; it does not modify Explorer binaries on disk, move files,
change native automatic arrangement, or register a permanent Hook.

## Intended behavior

Keep Explorer's one native icon view, text, selection, native input and context menus. LucidPane
owns only group metadata and frames. Keep automatic arrangement enabled and files at original
paths. Unassigned items remain in the same native desktop view. Each grouped item is the same
Explorer item with redirected rectangles and hit testing. No second icon view or per-item hiding
is used. Menu invocation remains in Explorer's native input path.

## Geometry validation and current interaction changes

- Exact supported common-controls image: WinSxS `6.0.26100.8972` x64, 2,688,512 bytes,
  FNV-1a 64 `b4fc8de893e82e37`. All three entry prologues are checked before installation.
- Verified private targets: `CLVIconView::GetRectsOwnerData` (`0x16eb0`),
  `CLVIconView::v_ItemHitTest` (`0x16260`), `CLVView::OnGetItemPosition` (`0x24524`).
  These are version-specific internal methods, not supported Microsoft extension APIs.
- `geometry_probe`: v6 owner-data fixture, identical native icon/label/selection pixel crops
  before and after translation, matching hit testing, automatic arrangement, another view's
  isolation, atomic staged publication, detach and reattach.
- `hook_probe --geometry`: actual cross-process DLL and bounded IPC. Added 120 synchronous
  geometry commits with hit-test verification and no false Shell cache invalidation. Measured
  0.15 ms per iteration in the hidden fixture; this is not a real-desktop FPS measurement.
- The controller cache uses a separate native-change generation, with before/after checks during
  enumeration. Our commits cannot acknowledge or erase a concurrent native inventory change.
- Move/resize proposals update the cached layout through `WM_MOVING`/`WM_SIZING`, then commit
  all mappings and repaint Explorer before accepting the frame proposal. Invalid proposals keep
  the last accepted rectangle. Shell enumeration and baseline coordinate IPC run only at startup
  and on native inventory/layout changes. Configuration writes occur at the end of movement.
- Known-item drag/drop uses cached identities; its input detector still polls every 25 ms.
  Ordinary clicks and right-click menus do not invalidate the inventory cache.
- Follow-up after the user confirmed live movement but reported dropped frames: layouts now use
  one bounded `WM_COPYDATA` batch (up to 512 mapped items), instead of per-item IPC. Every label
  is checked before atomic publication. A stale second item was verified to reject the entire
  batch while preserving the previously displayed layout.
- Painting invalidates only changed icons' native baseline, previous and next bounds, including
  label/selection/shadow margins. Native baseline damage ensures virtual-grid culling visits the
  translated item. Ordinary paints expand only intersecting mapped regions; unchanged mappings
  cause no damage. The visible native fixture's clipped `WM_PRINTCLIENT` output matches its full
  reference image, and the update region excludes an unrelated desktop point. Real desktop
  compositor smoothness still requires interactive verification.
- Optional `LUCIDPANE_TRACE_LAYOUT=1` logs median/p95/max controller layout duration and mean
  native IPC/paint duration every 60 updates. It records no item names or file paths.
- User verification after the second optimization: **noticeably smoother**. Real desktop traces
  show batch median durations around 9–24 ms, with occasional 35–62 ms p95 intervals dominated
  by native IPC/painting. Improvement is confirmed, but constant refresh-rate presentation is
  not claimed; the hidden fixture's sub-millisecond timing must not be used as desktop FPS.
- Real desktop geometry probe: 81 items, automatic arrangement enabled, mapped icon hit testing
  passed, then all 81 original positions restored. Explorer remained running. The user confirmed
  dragging an icon into a pane and back. They then reported latency and end-only pane movement;
  the changes above address those paths, with updated visual smoothness awaiting user feedback.
- Watchdog fixture removes mapping after controller destruction. The DLL remains pinned but its
  detours are disabled on detach; versioned runtime copies allow subsequent builds to coexist.

## Implemented infrastructure

- `desktop-hook`: a per-thread `WH_CALLWNDPROC` DLL bootstrap, bounded pointer-free `WM_COPYDATA`
  protocol, owner-process watchdog, detach handling and a standard-list work-area experiment.
- DLL code remains mapped until the target process exits to avoid dangling callbacks after the
  controller crashes. Detach removes the subclass and timer; it does not unload pinned code.
- Versioned runtime DLL copies avoid overwriting a DLL already loaded into another process.
- `app/src/hook_desktop.rs`: experimental group-frame/controller integration using a separate
  `hook-desktop.db`. It is not the launch default; use `--hook-desktop`.
- The old work-area backend rejects `LVS_OWNERDATA` before sending any work-area messages.
  Geometry mode supports this style and never sends unsupported work-area/position mutations.
  There is no fallback that disables automatic arrangement.
- Native desktop snapshots now retain the actual Shell view indices as well as stable identities.

## Live test failure and recovery

The initial disposable host was an ordinary ListView. Its successful auto-arranged work-area test
did **not** establish compatibility with Explorer's virtual desktop. At approximately 21:20 on
2026-09-08, the live work-area experiment caused Explorer to terminate with an access violation in
`comctl32.dll` (6.10.26100.8972, reported fault offset `0x106c6`). Windows restarted Explorer.
No crash dump was analyzed, so that event is not an instruction-level root-cause proof.

The desktop has `LVS_OWNERDATA`; Microsoft's [ListView compatibility table](https://learn.microsoft.com/en-us/windows/win32/controls/list-view-controls-overview#compatibility-issues)
lists `LVM_GETWORKAREAS`, `LVM_SETWORKAREAS` and `LVM_SETITEMPOSITION` as unsupported for that mode.
The first test omitted this condition. The work-area backend must remain blocked on virtual views.
Do not attempt to remove `LVS_OWNERDATA` dynamically; that is also unsupported.

Recovery verification found 81 items, matching the pre-test count. Automatic arrangement was
restored through `IFolderView2::SetCurrentFolderFlags(FWF_AUTOARRANGE, FWF_AUTOARRANGE)` and read
back enabled. Latest read-only snapshot: HWND 198012, style `0x56003b40`, extended ListView style
`0x14c14c30`, owner-data true, auto-arrange true. The original exact icon positions were not fully
restored after the Explorer restart; do not claim that they were.

Local evidence is in `target/debug/hook-layout-before.txt` and `hook-layout-recovered.txt`.
These snapshots contain local desktop metadata and should not be committed or published.

## Position callback investigation

Direct `QueryInterface(IOwnerDataCallback)` on the active `IShellView` returned `E_NOINTERFACE`.
The callback belongs to the list host, not necessarily the public Shell view interface.

`tools/explorer_symbols.py` downloads matching Microsoft public PDBs into `target/symbols` and
checks the PE RSDS GUID and DBI age. Public PDB info-stream age may differ from DBI age; both are
recorded. It reads PE/PDB files only and does not load, patch or inject code into Explorer.

On this machine, matching shell32 symbols identify:

- `CListViewHost::GetItemPosition(int, POINT*)` (RVA `0x4d290`).
- `CListViewHost::SetItemPosition(int, POINT)` (RVA `0x35ed0`).
- `CListViewHost`'s `IOwnerDataCallback` vtable (RVA `0x60d418`).
- shell32 PDB GUID `4907816C76ABD6288BBE01D3E8033EE9`, linked DBI age 1.

These RVAs are **research evidence for this exact image**, not supported Hook entry points or
portable offsets. No new runtime patch was installed using them.

A disposable version-6 virtual-list fixture registers an `IOwnerDataCallback` object. Registration
adds a reference, but neither its automatic nor its manual fixture consumes the position callbacks
with the tested setup, including matched extended styles. Therefore registration is insufficient.
Offline inspection of the matching comctl32 image shows an automatic-arrangement branch bypassing
the manual-position path and additional internal conditions before callback invocation. The missing
conditions and supported recovery behavior must be understood before building the next backend.

The [original callback ABI research](https://www.geoffchappell.com/studies/windows/shell/comctl32/controls/listview/interfaces/iownerdatacallback.htm)
is useful background, but it dates from Vista and is not a Windows 11 compatibility guarantee.

## Reproducible checks

```powershell
cargo build -p desktop-hook --lib --examples
cargo test -p desktop-hook --lib
cargo clippy -p desktop-hook --all-targets -- -D warnings
target/debug/examples/hook_probe.exe
target/debug/examples/hook_probe.exe --owner-data
target/debug/examples/owner_data_probe.exe
target/debug/examples/owner_data_probe.exe --manual-fixture
```

The ordinary cross-process fixture requires execution outside the Codex process sandbox on this
machine: the sandbox allowed Hook registration but did not deliver its callback. It does not target
Explorer. `--owner-data` verifies rejection before DLL loading by passing a nonexistent DLL path.
The callback fixtures verify the observed missing activation, not successful grouping.

For real-desktop read-only verification:

```powershell
cargo build -p desktop-shell --example hook_layout_probe
target/debug/examples/hook_layout_probe.exe --snapshot-only
```

The old live work-area probe is retained for diagnosis; its connect path now rejects the real
virtual desktop. Do not rerun it by bypassing the guard.

## Remaining migration work

Validate updated drag/move smoothness on the real desktop, large grouped sets, mixed DPI,
native multi-selection, drag cancellation, keyboard navigation and modern menus. Complete
materials, collapse/scroll clipping, animation, snapping and auto-collapse on the Hook frame
before migrating the launch default. OS image changes fail closed and require a newly reviewed
profile. No full Fences parity or general Windows-version compatibility is claimed.
