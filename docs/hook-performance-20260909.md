# Hook performance pass against 269831c

This pass retains the committed rectangle-based icon hit semantics, native
source/no-op suppression, visible insertion normalization, OLE coordinates and
5-physical-pixel pane gap. It does not restore the rejected native-grid hit
shortcut or suspend scene synchronization for the entire mouse press.

Implemented:

1. Cache the visible ordering and precomputed before/after boundaries using an
   immutable reference-counted snapshot. Native positions and monitor ownership
   are read once for a valid snapshot instead of once per hidden boundary query.
2. Index actual native icon and label rectangles in 128-pixel spatial buckets.
   Sort only matching candidates using the original distance/index order and
   return the same icon/label flags. A failed or invalidated build uses the old
   full scan. Long rectangles and negative monitor coordinates remain covered.
   The separate native-pane experiment retains its original scan: its mapping
   changes each frame, so building a cold index there adds unnecessary work.
3. Cache native damage bounds and index both native and displayed rectangles.
   Query only candidates intersecting the update region, then retain exact GDI
   region intersection checks. Failed construction retains the original scan.
4. Avoid per-item SaveDC/RestoreDC and extra postpaint requests when the hybrid
   desktop has no native pane clipping regions. Hidden custom-draw rejection and
   native-pane clipping remain active.
5. Coalesce duplicate controller timer synchronization checks at 100 ms. Explicit
   operations, dirty scenes, loaded images, reconciliation and button release
   remain immediate. Full Hook drop snapshots are disabled by default; build
   `desktop-hook` with `--features drag-trace` to enable them for diagnosis. This
   switch is compiled into the DLL because Explorer does not inherit the newly
   launched controller's environment.

Invalidation covers scene publication, membership updates, same-count identity
reconciliation, structural list changes, arrangement, icon spacing, image lists,
text, view/font/theme/DPI/display changes, scrolling, mouse release/capture change
and native Drop completion. Selection/focus/redraw invalidates text geometry;
unrelated cached bounds remain available. Native calls happen outside STATE
borrows; a revision check prevents publishing a cache built across a change.
The existing pre-paint/pre-input identity scan is deliberately retained.

Validation on the disposable native owner-data view:

- Nine Hook unit tests, including exhaustive hidden/multi-selection drop order
  and spatial coverage for overlapping, negative and long rectangles.
- Three hybrid controller tests including urgent-versus-duplicate sync timing.
- Native pixel equivalence, partial repaint, same-count reorder after cache
  warmup, selection invalidation, hidden isolation and OLE forwarding checks.
- 1,580 warm icon hits issue zero LVM_GETITEMRECT/LVM_GETITEMPOSITION queries.
  One run measured 79-hit batches at 234.6 microseconds median and 329.2
  microseconds P95; this is an isolated Debug fixture, not desktop frame rate.
- A repeated local WM_PAINT issues zero geometry queries after warmup.
- Repeated hidden marker rectangle queries do not rebuild the native order.
- Live read-only audit after launch: all 78 visible desktop icon hits match
  (81 total Shell items, three collected); the normal run has audit disabled.
- Cross-process geometry, invalid-transaction isolation and watchdog restoration
  pass. Clippy all targets/all features and the diagnostic-feature tests pass.

Real Explorer dragging still needs interactive confirmation. Baseline binaries
are saved under target/perf-269831c-* for diagnosis; the committed source baseline
is 269831c. No desktop paths or files are moved by the automated probes.
