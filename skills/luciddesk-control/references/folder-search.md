## Folder panels and search

For window snapping or arrangement, also read [Desktop and layout](desktop-layout.md); basic folder/search queries do not need it.

Create a folder mapping with `folder.create` (`ref`, `title`, existing absolute directory `path`). `folder.update` changes mapping and view preferences. Columns are name/modified/type/size, with name always visible. `column_widths` uses that order and sums to 1; `sort_column` instead uses name=0, type=1, modified=2, size=3. Query `folder get --id ID`, checking loading/error and `runtime` preferences before relying on contents. Pane options, geometry, and removal apply; removing the mapping never deletes its directory.

Use `folder fit --id ID --icon-columns 4 --max-rows 5 --dry-run --json` for an icon-view folder. In list view, omit `--icon-columns`: width is preserved and the app fits list rows including the column header. With no max_rows, all rows must fit the monitor; max_rows limits visible rows while retaining all entries for scrolling. It never truncates the directory or changes icon scale. Check `folder get` → `content_layout.ready` before planning, and verify saved geometry plus actual bounds afterward.

Use `folder refresh --id ID --json` to request an asynchronous rescan of the current directory. It does not persist navigation or preferences. Boundedly poll folder get until available, not loading, and without error; check current_path still matches the intended directory. Exact receipt retry must not trigger another scan. Loaded item count, loading/error state, and navigation participate in plan conflicts, so re-preview if they change. Apply a mapping change or create a folder panel first, wait for its snapshot, then fit; do not fit an unloaded/new mapping in the same plan.

Submit `folder.navigate` (absolute `path`), `folder.back`, or `folder.home` individually with `pane_id`. These are transient and do not save navigation history.

Search must be enabled before `search.query`, `search.refresh`, or `search.more`; if disabled, change settings only when the user's request authorizes enabling it. Submit each individually with `pane_id`; query also takes `query`. These return `commit_status:not_persisted`. Retain `runtime_result.generation`, then boundedly poll `search get --id ID` until not busy, not replacing, and not failed, with the same generation. Older entries may remain while replacement is loading. Use `has_more` before paging; an empty query clears search. Do not launch a backend outside the user's authorization.
