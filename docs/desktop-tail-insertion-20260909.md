# Desktop tail insertion correction (2026-09-09)

The reported case has 81 Shell items, two collected identities at native indices
57 and 58, 112 × 147 cells and 14 rows per column. The final visible item is
index 80 (新手盒子). Some interior gaps have no insertion mark and earlier items
cannot reliably be moved after the final item.

Two paths previously disagreed:

1. `CLVView::v_OnInsertMarkHitTest` ran on an unfiltered native grid point and the
   Hook then rewrote the returned index. On the reviewed comctl32 image, the
   function suppresses source/adjacent no-op insertion positions before it
   returns (source comparison around RVA 0xac4d0). Once it returns -1, changing
   the output index cannot recover a legitimate visible gap.
2. OLE `Drop` synthesized another `DragOver` at translated coordinates with all
   geometry hooks bypassed, then tried to force a tail mark. This could replace
   the preview's cached drop target/state at release. Logs showed tail marks
   before release and a different input point passed to the commit path.

The hybrid insertion hook now maps the visible cell to its native cell **before**
calling the original function. Native code still determines source/no-op rules
and before/after flags. Collected identities are excluded, monitor boundaries
are respected, and trailing empty space maps inside the final visible identity's
native cell. The older native-pane experiment retains its previous fallback.

OLE DragEnter/DragOver/Drop now retain their original screen points. Drop uses
the existing native insertion target without an extra synthetic DragOver or a
forced mark. Full drag diagnostics remain enabled as in the rolled-back version;
the previously rejected performance changes were not reapplied.

Validation:

- Six Hook unit tests pass, including all visible cell before/after mappings,
  recorded tail positions, hidden tail identities, and COM forwarding that
  asserts Drop forwards the original point without synthesizing DragOver.
- Clippy all targets passes.
- Native geometry fixture: all 79 icon/label hits, partial repaint, selection
  isolation, same-count reorder, insertion rectangles and detach pass.
- Cross-process fixture: geometry updates, invalid transaction isolation and
  watchdog restoration pass.
- Live read-only audit: all 79 visible icon hits and 158 before/after insertion
  queries match, including native indices 77–80. It does not simulate an active
  drag source or prove physical drag/drop behavior.

The gap audit is behind the existing `LUCIDPANE_HIT_AUDIT` switch, disabled in
normal use. Real dragging of the two reported icons remains the final visual
and behavioral check; do not equate read-only geometry checks with that result.
