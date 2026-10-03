//! Geometry persistence for individual display arrangements.
use super::{StoreError, WorkspaceStore};
use luciddesk_core::{PanelId, RectDip};
use rusqlite::params;

impl WorkspaceStore {
    /// Saves geometry for one display arrangement without changing other arrangements.
    /// # Errors
    /// Reports database failures.
    pub fn save_monitor_layout(
        &mut self,
        topology: &str,
        layout: &[(PanelId, RectDip)],
    ) -> Result<(), StoreError> {
        let previous = self.raw_change_count();
        let tx = self.connection.transaction()?;
        update(&tx, topology, layout)?;
        tx.commit()?;
        self.notify_change(previous);
        Ok(())
    }

    /// Loads saved geometry for a display arrangement.
    /// # Errors
    /// Reports invalid geometry or database failures.
    pub fn monitor_layout(&self, topology: &str) -> Result<Vec<(PanelId, RectDip)>, StoreError> {
        let mut stmt = self
            .connection
            .prepare("SELECT panel_id,x,y,width,height FROM monitor_layouts WHERE topology=?1")?;
        let rows = stmt.query_map([topology], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                RectDip {
                    x: r.get(1)?,
                    y: r.get(2)?,
                    width: r.get(3)?,
                    height: r.get(4)?,
                },
            ))
        })?;
        rows.map(|row| {
            let (id, r) = row?;
            if id == 0
                || ![r.x, r.y, r.width, r.height]
                    .into_iter()
                    .all(f32::is_finite)
                || r.width <= 0.0
                || r.height <= 0.0
            {
                return Err(StoreError::InvalidData("invalid saved layout".into()));
            }
            Ok((PanelId::new(id), r))
        })
        .collect()
    }
}

/// Shares the caller's transaction so geometry and workspace changes commit together.
pub(super) fn update(
    tx: &rusqlite::Transaction<'_>,
    topology: &str,
    layout: &[(PanelId, RectDip)],
) -> Result<(), StoreError> {
    let old: Vec<PanelId> = tx
        .prepare("SELECT panel_id FROM monitor_layouts WHERE topology=?1")?
        .query_map([topology], |row| Ok(PanelId::new(row.get(0)?)))?
        .collect::<Result<_, _>>()?;
    let live: std::collections::HashSet<_> = layout.iter().map(|(id, _)| *id).collect();
    if live.len() != layout.len() {
        return Err(StoreError::InvalidData("duplicate monitor panel id".into()));
    }
    for id in old {
        if !live.contains(&id) {
            tx.prepare_cached("DELETE FROM monitor_layouts WHERE topology=?1 AND panel_id=?2")?
                .execute(params![topology, id.get()])?;
        }
    }
    for (id, r) in layout {
        if ![r.x, r.y, r.width, r.height]
            .into_iter()
            .all(f32::is_finite)
            || r.width <= 0.0
            || r.height <= 0.0
        {
            return Err(StoreError::InvalidData("invalid monitor geometry".into()));
        }
        let id = i64::try_from(id.get())
            .map_err(|_| StoreError::InvalidData("invalid panel id".into()))?;
        tx.prepare_cached(
                "INSERT INTO monitor_layouts (topology,panel_id,x,y,width,height) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(topology,panel_id) DO UPDATE SET x=excluded.x,y=excluded.y,width=excluded.width,height=excluded.height WHERE (x,y,width,height) IS NOT (excluded.x,excluded.y,excluded.width,excluded.height)",
            )?.execute(
                params![topology, id, r.x, r.y, r.width, r.height],
            )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use luciddesk_core::{Panel, Workspace};
    #[test]
    fn display_profiles_do_not_overwrite_each_other() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let id = PanelId::new(1);
        let a = RectDip::new(10.0, 20.0, 200.0, 300.0);
        let b = RectDip::new(-900.0, 0.0, 200.0, 300.0);
        store
            .save_workspace(&Workspace::from_panels(vec![Panel::new(id, "Test", a)]).unwrap())
            .unwrap();
        store.save_monitor_layout("single", &[(id, a)]).unwrap();
        store.save_monitor_layout("dual", &[(id, b)]).unwrap();
        assert_eq!(store.monitor_layout("single").unwrap(), vec![(id, a)]);
        assert_eq!(store.monitor_layout("dual").unwrap(), vec![(id, b)]);
    }
}
