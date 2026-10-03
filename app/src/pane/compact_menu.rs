//! Windows 11 item-menu preference, included in workspace metadata backups.
use std::sync::atomic::{AtomicBool, Ordering};
static ENABLED: AtomicBool = AtomicBool::new(true);
const KEY: &str = "pane_compact_menu";
pub(super) fn enabled() -> bool { ENABLED.load(Ordering::Relaxed) }

pub(super) fn load(store: &luciddesk_storage::WorkspaceStore) -> Result<(), String> {
    let value = store.preference(KEY).map_err(|e| e.to_string())?;
    ENABLED.store(value.as_deref() != Some("false"), Ordering::Relaxed);
    Ok(())
}
pub(super) fn save(store: &luciddesk_storage::WorkspaceStore, value: bool) -> Result<(), String> {
    store.save_preference(KEY, if value { "true" } else { "false" }).map_err(|e| e.to_string())?;
    ENABLED.store(value, Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_on_and_persists_disabled() {
        let original = enabled();
        let store = luciddesk_storage::WorkspaceStore::open_in_memory().unwrap();
        load(&store).unwrap();
        assert!(enabled());
        save(&store, false).unwrap();
        load(&store).unwrap();
        assert!(!enabled());
        assert_eq!(store.preference(KEY).unwrap().as_deref(), Some("false"));
        save(&store, true).unwrap();
        assert!(enabled());
        ENABLED.store(original, Ordering::Relaxed);
    }
}
