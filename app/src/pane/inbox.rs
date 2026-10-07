//! Temporary inbox pane: newly created desktop entries are filed here instead of
//! being left loose on the desktop.
//!
//! The preference stores the designated pane id, not the switch, so the pane keeps
//! collecting after a restart. The stored pane id is `0` when the switch is off.
use crate::pane::PaneApp;
use luciddesk_core::{Panel, PanelId, ShellIdentity};
use luciddesk_storage::WorkspaceStore;

const ENABLED_KEY: &str = "pane_inbox_enabled";
const PANE_KEY: &str = "pane_inbox_pane";

/// Whether the switch is currently on.
pub(super) fn enabled() -> bool {
    ENABLED.with(std::cell::Cell::get)
}

/// The pane that collects new items, when the switch is on and the pane still exists.
pub(super) fn pane(s: &PaneApp) -> Option<PanelId> {
    if !enabled() {
        return None;
    }
    let id = PanelId::new(PANE.with(std::cell::Cell::get));
    s.workspace
        .panel(id)
        .filter(|panel| panel.supports_tabs())
        .map(Panel::id)
}

/// Loads the switch and the designated pane from the workspace database.
pub(super) fn load(store: &WorkspaceStore) -> Result<(), String> {
    let on = store
        .preference(ENABLED_KEY)
        .map_err(|e| e.to_string())?
        .as_deref()
        == Some("true");
    let id = store
        .preference(PANE_KEY)
        .map_err(|e| e.to_string())?
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    ENABLED.set(on);
    PANE.set(id);
    Ok(())
}

/// Remembers `pane` as the collector and turns the switch `on`.
pub(super) fn save(store: &WorkspaceStore, pane: Option<PanelId>, on: bool) -> Result<(), String> {
    store
        .save_preference(ENABLED_KEY, if on { "true" } else { "false" })
        .map_err(|e| e.to_string())?;
    let id = pane.map_or(0, |id| id.get());
    store
        .save_preference(PANE_KEY, &id.to_string())
        .map_err(|e| e.to_string())?;
    ENABLED.set(on);
    PANE.set(id);
    Ok(())
}

thread_local! {
    static ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PANE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Files `fresh` into the inbox pane and returns how many items moved.
///
/// Called right after a Shell inventory reconciliation: `fresh` are the identities
/// the workspace had never seen, so they are the ones the user just added. The caller
/// owns the database save, so a rejected save can restore the previous workspace.
pub(super) fn collect(s: &mut PaneApp, fresh: &[ShellIdentity]) -> usize {
    if fresh.is_empty() {
        return 0;
    }
    let Some(inbox) = pane(s) else {
        return 0;
    };
    s.workspace.append_to_pane(inbox, fresh)
}

/// The pane that should collect.
///
/// Prefers the remembered pane, then a pane the user named after the feature. Returns
/// `None` when neither exists, so the caller can create the temporary pane and persist
/// the result itself (a failed save must be able to restore the previous workspace).
pub(super) fn resolve(s: &PaneApp, title: &str) -> Option<PanelId> {
    let stored = PanelId::new(PANE.with(std::cell::Cell::get));
    if stored.get() > 0
        && s.workspace
            .panel(stored)
            .is_some_and(Panel::supports_tabs)
    {
        return Some(stored);
    }
    s.workspace
        .panels()
        .iter()
        .find(|panel| panel.supports_tabs() && panel.title() == title)
        .map(Panel::id)
}

/// Ensures the switch has a pane to collect into, creating the temporary pane when the
/// remembered pane is gone. Returns the pane id and whether it was just created, so the
/// caller can persist and open the new view.
pub(super) fn ensure_pane(s: &mut PaneApp, title: &str) -> Result<Option<(PanelId, bool)>, String> {
    if !enabled() {
        return Ok(None);
    }
    if let Some(existing) = resolve(s, title) {
        return Ok(Some((existing, false)));
    }
    let id = create_pane(s, title)?;
    save(&s.store, Some(id), true)?;
    Ok(Some((id, true)))
}

/// The pane id the designated pane was remembered as, for the caller to fall back on.
pub(super) fn remembered() -> PanelId {
    PanelId::new(PANE.with(std::cell::Cell::get))
}

/// Creates the temporary pane that collects new items.
///
/// Returns the new pane id. The caller persists the workspace and creates the view,
/// because a rejected save must be able to restore the previous workspace.
pub(super) fn create_pane(s: &mut PaneApp, title: &str) -> Result<PanelId, String> {
    let id = PanelId::new(
        s.workspace
            .panels()
            .iter()
            .map(|panel| panel.id().get())
            .max()
            .unwrap_or(0)
            + 1,
    );
    let mut panel = Panel::new(id, title, crate::pane::display_layout::corner_pane(&s.workspace));
    if let Some((theme, backdrop)) = s.workspace.appearance() {
        panel.set_theme(theme);
        panel.set_backdrop(backdrop);
    }
    s.workspace
        .add_panel(panel)
        .map_err(|error| error.to_string())?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use luciddesk_core::DesktopItem;

    fn store() -> WorkspaceStore {
        WorkspaceStore::open_in_memory().unwrap()
    }

    #[test]
    fn switch_and_pane_round_trip_through_the_database() {
        let store = store();
        load(&store).unwrap();
        assert!(!enabled());
        save(&store, Some(PanelId::new(7)), true).unwrap();
        assert!(enabled());
        assert_eq!(PANE.with(std::cell::Cell::get), 7);
        load(&store).unwrap();
        assert!(enabled());
        assert_eq!(PANE.with(std::cell::Cell::get), 7);
        save(&store, None, false).unwrap();
        assert!(!enabled());
        assert_eq!(PANE.with(std::cell::Cell::get), 0);
    }

    #[test]
    fn reconcile_reports_only_previously_unknown_identities() {
        let mut workspace = luciddesk_core::Workspace::new();
        let known = ShellIdentity::Namespace {
            parsing_name: "test:known".into(),
        };
        let added = ShellIdentity::Namespace {
            parsing_name: "test:added".into(),
        };
        workspace.reconcile_desktop_items([DesktopItem::new(known.clone(), "Known")]);
        let fresh = workspace.reconcile_desktop_items([
            DesktopItem::new(known, "Known"),
            DesktopItem::new(added.clone(), "Added"),
        ]);
        assert_eq!(fresh, vec![added]);
    }

    #[test]
    fn appending_skips_items_already_filed_and_keeps_grid_order() {
        let inbox = PanelId::new(1);
        let mut workspace = luciddesk_core::Workspace::new();
        workspace
            .add_panel(Panel::new(inbox, "Inbox", luciddesk_core::RectDip::default()))
            .unwrap();
        let first = ShellIdentity::Namespace {
            parsing_name: "test:first".into(),
        };
        let second = ShellIdentity::Namespace {
            parsing_name: "test:second".into(),
        };
        let fresh = workspace.reconcile_desktop_items([
            DesktopItem::new(first.clone(), "First"),
            DesktopItem::new(second.clone(), "Second"),
        ]);
        assert_eq!(fresh, vec![first.clone(), second.clone()]);
        assert_eq!(workspace.append_to_pane(inbox, &[first.clone()]), 1);
        // Re-appending an already filed item must not duplicate or reorder it.
        assert_eq!(
            workspace.append_to_pane(inbox, &[first.clone(), second.clone()]),
            1
        );
        let placed: Vec<_> = workspace
            .desktop_items()
            .iter()
            .filter_map(|item| match item.placement() {
                luciddesk_core::DesktopPlacement::Pane { pane_id, position } => {
                    Some((*pane_id, position.column, item.identity().clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(placed.len(), 2);
        assert!(placed.iter().all(|(pane, _, _)| *pane == inbox));
        assert!(placed.iter().any(|(_, column, identity)| *column == 0 && *identity == first));
        assert!(placed.iter().any(|(_, column, identity)| *column == 1 && *identity == second));
    }
}
