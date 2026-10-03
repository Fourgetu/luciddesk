//! Geometry-only saves avoid walking icons, folder preferences and TOML.
use super::*;
impl WorkspaceStore {
    /// Atomically saves panel rectangles and optional monitor layout, without other state.
    /// Returns an error for invalid geometry or missing panels; no partial writes survive.
    pub fn save_panel_geometry(
        &self,
        rectangles: &[(PanelId, RectDip)],
        layout: Option<(&str, &[(PanelId, RectDip)])>,
    ) -> Result<(), StoreError> {
        let previous = self.raw_change_count();
        let tx = self.connection.unchecked_transaction()?;
        let mut seen = std::collections::HashSet::new();
        for (id, r) in rectangles {
            if !seen.insert(*id)
                || ![r.x, r.y, r.width, r.height]
                    .into_iter()
                    .all(f32::is_finite)
                || r.width <= 0.0
                || r.height <= 0.0
            {
                return Err(StoreError::InvalidData("invalid panel geometry".into()));
            }
            if !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM panels WHERE id=?1)",
                [id.get()],
                |r| r.get::<_, bool>(0),
            )? {
                return Err(StoreError::InvalidData("panel does not exist".into()));
            }
            tx.prepare_cached("UPDATE panels SET x=?2,y=?3,width=?4,height=?5 WHERE id=?1 AND (x,y,width,height) IS NOT (?2,?3,?4,?5)")?.execute(params![id.get(),r.x,r.y,r.width,r.height])?;
        }
        if let Some((topology, entries)) = layout {
            monitor_layout::update(&tx, topology, entries)?;
        }
        tx.commit()?;
        self.notify_change(previous);
        Ok(())
    }
}
