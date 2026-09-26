//! Global folder-pane activation policy. Missing preferences keep inline browsing.
use super::*;

const KEY: &str = "folder_entry_mode";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::pane) enum EntryMode {
    #[default]
    Inline,
    Explorer,
}

impl EntryMode {
    pub(in crate::pane) fn load(store: &WorkspaceStore) -> Result<Self, String> {
        let value = store.preference(KEY).map_err(|error| error.to_string())?;
        Ok(match value.as_deref() {
            Some("explorer") => Self::Explorer,
            _ => Self::Inline,
        })
    }

    pub(in crate::pane) fn save(self, store: &WorkspaceStore) -> Result<(), String> {
        store
            .save_preference(
                KEY,
                match self {
                    Self::Inline => "inline",
                    Self::Explorer => "explorer",
                },
            )
            .map_err(|error| error.to_string())
    }
}

pub(in crate::pane) fn navigation_target(
    state: &PaneApp,
    id: PanelId,
    index: usize,
) -> Result<Option<PathBuf>, String> {
    let Some(view) = state.views.iter().find(|view| view.id == id) else {
        return Ok(None);
    };
    let model = view.model.borrow();
    if model.folder.is_none() {
        return Ok(None);
    }
    let target = model
        .items
        .get(index)
        .and_then(|item| item.identity.file_system_path())
        .filter(|path| path.is_dir());
    // Read only on folder activation: settings take effect immediately without
    // per-view copies or additional work in the rendering and polling paths.
    if target.is_some() && EntryMode::load(&state.store)? == EntryMode::Inline {
        Ok(target.map(Path::to_path_buf))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_defaults_to_inline_and_survives_reopen() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("workspace.db");
        {
            let store = WorkspaceStore::open(&path).unwrap();
            assert_eq!(EntryMode::load(&store).unwrap(), EntryMode::Inline);
            EntryMode::Explorer.save(&store).unwrap();
        }
        let store = WorkspaceStore::open(&path).unwrap();
        assert_eq!(EntryMode::load(&store).unwrap(), EntryMode::Explorer);
        EntryMode::Inline.save(&store).unwrap();
        assert_eq!(EntryMode::load(&store).unwrap(), EntryMode::Inline);
        store.save_preference(KEY, "unknown").unwrap();
        assert_eq!(EntryMode::load(&store).unwrap(), EntryMode::Inline);
    }
}
