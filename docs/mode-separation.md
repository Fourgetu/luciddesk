# Application boundaries

- `app/src/main.rs`: hybrid-only CLI, STA/DPI initialization and data path.
- `app/src/pane/hybrid.rs`: Hook lifetime, Explorer identity synchronization,
  duplicate-icon handling, targeted notifications and batched icon extraction.
- `app/src/pane/mod.rs`: pane collection, persistence and view refresh.
- `app/src/pane/model.rs`: one pane grid, hit testing and scroll geometry.
- `app/src/pane/events.rs`: group commands and state changes.
- `app/src/pane/render.rs`, `window.rs`, `settings*.rs`: hybrid pane UI.
- `leagcy` branch: historical executable, UI implementation and exclusive dependencies.

The main branch has no legacy application package or mode-selection feature.
`legacy-app`, `desktop-compositor`, and old preview/redrawn-desktop run guides
are retained on `leagcy`. The optional Hook session in main represents startup,
teardown and isolated UI tests, not an alternate launch mode.

The archive was created from the complete migration snapshot, including common
libraries, so it remains independently buildable. Hybrid changes continue on
main without compiling or modifying the old application. Existing history is
preserved; this separation removes legacy files from main's current tree.

Validation commands on main:

```powershell
cargo check --workspace --all-targets --offline
cargo test -p lucidpane --bin lucidpane --offline -- --test-threads=1
```
