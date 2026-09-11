# Application boundaries

- `app/src/main.rs`: hybrid-only CLI, STA/DPI initialization and data path.
- `app/src/pane/hybrid.rs`: Hook lifetime, Explorer identity synchronization,
  duplicate-icon handling, targeted notifications and batched icon extraction.
- `app/src/pane/mod.rs`: pane collection, persistence and view refresh.
- `app/src/pane/model.rs`: one pane grid, hit testing and scroll geometry.
- `app/src/pane/events.rs`: group commands and state changes.
- `app/src/pane/render.rs`, `window.rs`, `settings*.rs`: hybrid pane UI.
- `legacy-app/src/`: independent historical executable and UI implementation.

The primary package contains no launch-mode selector, managed/unmanaged rendering
flag, desktop-surface session, alternate grid, full-desktop scanner or restore
guard. Its optional Hook session represents initialization/teardown and isolated
UI fixtures, not another launch mode. Both applications depend on the common
Windows libraries under `crates/`; neither imports the other's application code.

The legacy UI is retained as an independent historical implementation rather than
an alternate branch in the actively maintained hybrid UI. Future hybrid UI fixes
belong in `app`; legacy behavior changes must be deliberately made in `legacy-app`.
Old mode regression tests remain there; hybrid behavior tests remain in `app`.

Validation commands:

```powershell
cargo check -p lucidpane --all-targets --offline
cargo test -p lucidpane --bin lucidpane --offline -- --test-threads=1
cargo test -p lucidpane-legacy --bin lucidpane-legacy --offline -- --test-threads=1
```
