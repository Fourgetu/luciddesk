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

## Hidden canonical marker anchor

A later physical drag reproduced a separate gap with three collected items,
including native item 80. The trace recorded 832 insertion calls, all matched
and translated: the final hit returned visible item 79, while the mark actually
stored by Shell was `80:0x80000000` (before hidden item 80). Its native rectangle
was `[560,1467,672,1474]`. Thus restoring the previous input-mapping code alone
could not restore the displayed marker: Shell canonicalized the boundary after
the hit returned, and hidden item 80 had no compact presentation position.

For a before-mark anchored to a hidden item in hybrid mode, rectangle translation
now counts the visible native slots preceding that item on its monitor and uses
the corresponding compact boundary. The stored insertion item, flags, OLE
coordinates, native selection and Drop call are left intact. This is a marker
geometry correction, not a reintroduction of the rejected drag optimizations.

The disposable native fixture now checks nine hidden anchors across leading,
middle, column-wrap and trailing cases against unhooked native marker rectangles,
and verifies that rendering preserves the native insertion item and before/after
flag. These checks do not replace the physical drag/drop confirmation.

## Shared visible boundary resolution

The follow-up uses `VisibleOrder` for both the rendered boundary and the native
commit target. A gap touching collected items resolves before the next visible
identity, or after the monitor's physical last item when there is no visible
successor. Using the physical end matters because `CListViewHost::GetInsertMark`
increments an after-mark's index before converting it back to a Shell identity.
An after-mark on the last visible item would otherwise resolve to its hidden
successor again.

The insertion hook normalizes a valid native hit result; the OLE proxy rechecks
the cached mark immediately before forwarding Drop. It does not redo DragOver,
change pointer coordinates, or replace native icon hit-testing and selection.
Hidden tail after-marks are rendered at the last visible item's compact position.
Native no-op marks and ordinary visible gaps pass through unchanged. The order
is read from the current scene/native positions, not retained across reorders.

Validation includes exhaustive five-item hidden/selected subsets with multi-item
drag order comparisons, monitor boundary isolation, and a disposable native view
whose real OLE proxy forwards Drop into a recording receiver. The receiver checks
the canonical insertion identity, unchanged pointer position, and matching native
marker geometry for nine leading/middle/wrap/tail cases. Eight unit tests and
the native fixture pass. Temporary per-hit tracing has been removed. Physical
Explorer drag/drop still requires interactive confirmation.
