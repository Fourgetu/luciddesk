## Common workflows

Commands below use illustrative numeric pane IDs and opaque item IDs. Replace them with live query results. Mutation examples preview only; apply the reviewed token via `next_step.args`, then verify before continuing a dependent step.

### Inspect or organize desktop icons

Use `pane list --json` for panels, `item list --unassigned --json` for free desktop items, and `item list --pane 1 --json` for one panel. Do not query folder contents through `item list`.

For “group my icons into Work and Games”, query `workspace get --json`, classify only the intended items by their display names/paths, and build one plan using its fresh context. Example `operations` (not a complete plan):

```json
[
  {"op":"pane.create","ref":"work","title":"Work"},
  {"op":"item.assign","pane_ref":"work","item_ids":["WORK_ITEM_ID"]},
  {"op":"pane.create","ref":"games","title":"Games"},
  {"op":"item.assign","pane_ref":"games","item_ids":["GAME_ITEM_ID"]}
]
```

Reuse suitable existing panes instead of creating duplicates. Leave ambiguous items unchanged or clarify their destination. Read [Plans and recovery](plans-and-recovery.md) to submit the plan. After applying, resolve created pane IDs from `refs` and verify membership; fit/place those actual IDs in a subsequent plan if requested. Do not assume any preferred desktop side.

### Fit, snap, sort or rename a pane

| User intent | Preview command | Verify after apply |
| --- | --- | --- |
| Six icons per row, height fits content | `pane fit --id 1 --icon-columns 6 --dry-run --json` | `pane get --id 1 --json`: layout and actual bounds |
| Place pane 1 below pane 2, left-aligned | `pane snap --id 1 --target 2 --side bottom --align start --dry-run --json` | Both bounds, fixed snap gap, anchor unchanged |
| Sort icons by name | `pane sort --id 1 --dry-run --json` | Item placement row/column order |
| Rename a pane | `pane update --id 1 --title Work --dry-run --json` | Pane title, membership unchanged |
| Move desktop items to an existing pane | `item assign --pane 1 --ids ITEM_A,ITEM_B --dry-run --json` | Destination membership |
| Return selected icons to the desktop | `item release --ids ITEM_A,ITEM_B --dry-run --json` | Items have desktop placement; files remain intact |
| Merge pane 1 into pane 2 as tabs | `tab merge --id 1 --into 2 --dry-run --json` | Workspace tab membership and active pane |

For “fit then snap”, optional `--icon-columns 6` on `pane snap` does both in one operation. Check the restrictions below before planning.

## Desktop panels, icons, and tabs

Omitted update fields remain unchanged; `null` is not a reset. Explicitly unlock a locked pane before changing protected content or layout. Window options affect all members of a tab group; title changes affect the target pane only. Manual collapse is persistent, while effective auto-hide collapse is transient.

`item.assign`, `item.release`, and `item.reorder` operate only on desktop item IDs. Reorder requires the complete current membership exactly once. Verify order by `placement.row`, then `placement.column`, not response array order. Assignment/release require desktop integration. Their receipts may remain pending while images load or Explorer confirms visibility; inspect `status.desktop_sync_status` if delayed. An open native menu can defer synchronization.

`pane.remove` removes one content pane. Nonempty desktop panes require `release_items:true`; release and removal return icons without moving or deleting their real files.

Use `pane sort --id ID --dry-run --json` to sort one ordinary panel by display name using Windows natural ordering (for example, item2 before item10). Add `--descending true` to reverse it. This is a one-time order change, with deterministic ties, no file metadata scans and no persistent auto-sort rule. Only the specified pane changes, including within tabs; locked panes are rejected. For a user-defined sequence, use `item reorder --pane ID --input FILE_OR_DASH` with the complete current item-ID array. Verify by placement row/column. Repeating an already satisfied sort is a no-op with no database save. Folder ordering uses folder.update instead.

Ordinary desktop panes support `tab.merge` (`pane_id`, `into_pane_id`), `tab.select` (`pane_id`), `tab.reorder` (`pane_id`, complete `pane_ids`), and `tab.detach` (`pane_id`). Merge appends the entire source group and retains target active tab/window options. Selection is allowed while locked. Detach preserves bounds. Folder and search panes do not support tabs.

## Content fit and snapped layout

Use `pane get` / `pane list` to inspect `content_layout`: supported kind, item count, icon columns, cell size, required content height, and native snap gap. Use these public fields rather than source code or guessed pixel formulas. Choose placement from the user’s request and monitor work area; do not assume a right-side layout for general organization tasks.

- To fit one desktop panel: `pane fit --id ID --icon-columns 6 --dry-run --json`. The app uses its renderer's grid, current scale and all tab members' contents to size the shared window. It preserves position where possible, moving inward only when needed to fit the monitor.
- For an explicitly requested top-right column layout: query `monitor list`, then `pane arrange --input FILE_OR_DASH --dry-run --json`. Input is `{"monitor_id":"ID_FROM_QUERY","columns":[["LEFT_TOP_ID","LEFT_NEXT_ID"],["RIGHT_TOP_ID","RIGHT_NEXT_ID"]],"icon_columns":6}`. Columns are ordered left-to-right, panels top-to-bottom. The app fits each panel and uses the GUI's physical snap gap between panels and at the top/right edges. No manual coordinate calculations are required.
- To snap one panel beside another: `pane snap --id MOVING_ID --target ANCHOR_ID --side left|right|top|bottom --align start|center|end --dry-run --json`. Alignment is vertical (top/center/bottom) for left/right, horizontal (left/center/right) for top/bottom. Start is the default. The fixed gap is shared with GUI snapping; there is no per-operation gap parameter. Optional `--icon-columns N` fits icon-view content and snaps in the same transaction; omit it to preserve size, including for folder panels.
- Relative snapping moves the source window (including its tabs), leaving the anchor fixed even if locked. Expand both windows first. Search panels have dynamic height and are not supported as source or anchor. The destination must fit the anchor's monitor and avoid other panes; failure leaves layout unchanged. In a batch, later snaps see geometry from earlier operations. This positions windows once; it does not bind their future movement.
- Start with a column count suitable for the requested area and an icon-column count such as 6; inspect preview geometry. If space is insufficient, adjust the grouping or icon columns and preview again. Do not reduce icon scale or hide items without user intent.
- Locked/collapsed panels require explicit unlock/expand first. List each shared tab window once. Arrangement rejects overlap with unselected panes; include the intended peers or move them first. Loaded folder panels also support fitting and arrangement. List-view folders preserve width and fit rows; use folder.fit for a bounded viewport. Search content fitting is not supported; retain its dimensions and use explicit geometry instead.

Verify actual window bounds and unchanged membership. `pane arrange` sets a layout, not a permanent attachment between windows. `panel_defaults.snap` controls subsequent manual dragging; enable it through settings if requested.

For explicit geometry, use `pane.geometry` with the queried monitor ID and work-area-relative DIP. The minimum is 260 × 160 DIP; the full expanded rectangle must fit. Geometry affects all tab members and supports desktop/folder/search panes. Re-query after topology changes; live collapsed/search height may differ from saved expanded height.
