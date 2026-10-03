//! Tab grouping is workspace metadata; existing panel/content IDs stay intact.
use super::StoreError;
use desktop_core::{PaneTabs, PanelId, Workspace};
use rusqlite::{Connection, OptionalExtension};

pub(super) fn load(connection: &Connection, workspace: &mut Workspace) -> Result<(), StoreError> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key='pane_tabs_v1'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(raw) = raw.filter(|raw| !raw.is_empty()) else {
        return Ok(());
    };
    let invalid = || StoreError::InvalidData("invalid pane tabs".into());
    let id = |raw: &str| {
        raw.parse::<u64>()
            .ok()
            .filter(|id| *id > 0)
            .map(PanelId::new)
            .ok_or_else(invalid)
    };
    let mut groups = raw
        .split(';')
        .map(|group| {
            let (active, members) = group.split_once(':').ok_or_else(invalid)?;
            Ok(PaneTabs {
                active: id(active)?,
                members: members.split(',').map(id).collect::<Result<_, _>>()?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    // Validate legacy data before migration so corrupt references still fail loudly.
    let mut seen = std::collections::HashSet::new();
    if groups.iter().any(|group| group.members.len() < 2
        || !group.members.contains(&group.active)
        || group.members.iter().any(|id| !seen.insert(*id)
            || workspace.panel(*id).is_none_or(|panel| panel.is_search()))) {
        return Err(invalid());
    }
    // Old folder tabs become independent panels, retaining their content and settings.
    for group in &mut groups {
        group.members.retain(|id| workspace.panel(*id).is_some_and(|panel| panel.supports_tabs()));
        if !group.members.contains(&group.active) {
            if let Some(first) = group.members.first() { group.active = *first; }
        }
    }
    groups.retain(|group| group.members.len() > 1);
    workspace
        .set_tab_groups(groups)
        .map_err(|error| StoreError::InvalidData(error.to_string()))
}

pub(super) fn save(connection: &Connection, workspace: &Workspace) -> Result<(), StoreError> {
    let value = workspace
        .tab_groups()
        .iter()
        .map(|group| {
            format!(
                "{}:{}",
                group.active.get(),
                group
                    .members
                    .iter()
                    .map(|id| id.get().to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    if value.is_empty() {
        connection.execute("DELETE FROM metadata WHERE key='pane_tabs_v1'", [])?;
    } else {
        connection.execute("INSERT INTO metadata(key,value) VALUES ('pane_tabs_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE value != excluded.value", [&value])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkspaceStore;
    use desktop_core::{Panel, RectDip};

    #[test]
    fn legacy_folder_tabs_restore_independently_without_losing_panels() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let mut workspace = Workspace::new();
        for id in 1..=4 {
            let mut panel = Panel::new(PanelId::new(id), format!("Panel {id}"), RectDip::default());
            if id >= 3 { panel.set_folder(Some(format!("C:/folder{id}").into())); }
            workspace.add_panel(panel).unwrap();
        }
        store.save_workspace(&workspace).unwrap();
        for raw in ["3:3,1,2,4", "1:1,3;4:4,2", "3:3,4"] {
            store.save_preference("pane_tabs_v1", raw).unwrap();
            let loaded = store.load_workspace().unwrap();
            assert_eq!(loaded.panels(), workspace.panels());
            for id in [3, 4] {
                assert!(loaded.tab_group(PanelId::new(id)).is_none());
                assert!(loaded.tab_visible(PanelId::new(id)));
            }
            if raw == "3:3,1,2,4" {
                assert_eq!(loaded.tab_groups(), &[PaneTabs {
                    members: vec![PanelId::new(1), PanelId::new(2)], active: PanelId::new(1),
                }]);
            } else { assert!(loaded.tab_groups().is_empty()); }
            store.save_workspace(&loaded).unwrap();
            assert_eq!(store.load_workspace().unwrap(), loaded);
        }
    }

    #[test]
    fn tabs_round_trip_without_rewriting_unchanged_groups_and_reject_bad_references() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let mut workspace = Workspace::new();
        for id in 1..=3 {
            workspace
                .add_panel(Panel::new(
                    PanelId::new(id),
                    format!("Tab {id}"),
                    RectDip::default(),
                ))
                .unwrap();
        }
        workspace
            .set_tab_groups(vec![PaneTabs {
                members: vec![PanelId::new(3), PanelId::new(1), PanelId::new(2)],
                active: PanelId::new(1),
            }])
            .unwrap();
        store.save_workspace(&workspace).unwrap();
        assert_eq!(store.load_workspace().unwrap(), workspace);
        let before = store.change_count();
        store.save_workspace(&workspace).unwrap();
        assert_eq!(store.change_count(), before);
        assert_eq!(
            store.backup_snapshot().unwrap().load_workspace().unwrap(),
            workspace
        );
        store.save_preference("pane_tabs_v1", "1:1,99").unwrap();
        assert!(store.load_workspace().is_err());
    }
}

impl super::WorkspaceStore {
    /// Persists only the active member of an existing tab group.
    /// # Errors
    /// Rejects stale selection or a target outside the saved group.
    pub fn save_active_tab(&self, from: PanelId, to: PanelId) -> Result<(), StoreError> {
        let previous = self.raw_change_count();
        let tx = self.connection.unchecked_transaction()?;
        let raw: String = tx.query_row("SELECT value FROM metadata WHERE key='pane_tabs_v1'", [], |r| r.get(0))?;
        let invalid = || StoreError::InvalidData("saved tab group changed; reload before selecting".into());
        let mut found = false;
        let mut groups = Vec::new();
        for group in raw.split(';') {
            let (active, members) = group.split_once(':').ok_or_else(invalid)?;
            let active = active.parse::<u64>().map_err(|_| invalid())?;
            let ids = members.split(',').map(str::parse::<u64>).collect::<Result<Vec<_>, _>>().map_err(|_| invalid())?;
            if ids.contains(&from.get()) {
                if found || active != from.get() || !ids.contains(&to.get()) {
                    return Err(invalid());
                }
                found = true;
                groups.push(format!("{}:{members}", to.get()));
            } else {
                groups.push(group.to_owned());
            }
        }
        if !found { return Err(invalid()); }
        let value = groups.join(";");
        tx.execute("UPDATE metadata SET value=?1 WHERE key='pane_tabs_v1' AND value IS NOT ?1", [&value])?;
        tx.commit()?;
        self.notify_change(previous);
        Ok(())
    }
}
