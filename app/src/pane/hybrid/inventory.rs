//! Source membership is independent of the reduced Explorer view.
use desktop_core::ShellIdentity;
use desktop_shell::DesktopShellItem;

pub(super) struct Inventory {
    pub icon_size: i32,
    pub spacing: (i32, i32),
    pub dpi: u32,
    pub items: Vec<DesktopShellItem>,
}
// Stable file IDs deliberately survive rename. An in-flight audit of the old
// parsing name must nevertheless not overwrite a newly committed identity.
pub(super) fn revision_keys(managed: &[ShellIdentity]) -> Vec<String> {
    managed
        .iter()
        .map(|identity| {
            format!(
                "{}\0{}",
                identity.persistent_key(),
                identity.activation_name().to_string_lossy()
            )
        })
        .collect()
}

pub(super) fn capture(managed: &[ShellIdentity]) -> Result<Inventory, String> {
    let native = desktop_shell::native_desktop_snapshot()?;
    if !native.is_complete() {
        return Err(crate::i18n::text("ui-desktop-is-changing-group-refresh-deferred").into());
    }
    let mut items: Vec<_> = native.items.into_iter().map(|(item, _, _)| item).collect();
    if !managed.is_empty() {
        let source =
            desktop_shell::enumerate_desktop_source().map_err(|error| error.to_string())?;
        merge_managed(&mut items, source, managed);
    }
    items.sort_by_cached_key(|item| item.identity.persistent_key());
    let mut seen = std::collections::HashSet::new();
    if !items
        .iter()
        .all(|item| seen.insert(item.identity.persistent_key()))
    {
        return Err(crate::i18n::text("ui-duplicate-desktop-identities-group-refresh-deferred").into());
    }
    Ok(Inventory {
        icon_size: native.icon_size,
        spacing: native.spacing,
        dpi: native.dpi,
        items,
    })
}

fn merge_managed(
    visible: &mut Vec<DesktopShellItem>,
    source: Vec<DesktopShellItem>,
    managed: &[ShellIdentity],
) {
    for item in source {
        if managed
            .iter()
            .any(|identity| identity.equivalent_to(&item.identity))
            && !visible
                .iter()
                .any(|existing| existing.identity.equivalent_to(&item.identity))
        {
            visible.push(item);
        }
    }
}

pub(super) fn same(a: &Inventory, b: &Inventory) -> bool {
    a.icon_size == b.icon_size
        && a.spacing == b.spacing
        && a.dpi == b.dpi
        && a.items.len() == b.items.len()
        && a.items.iter().zip(&b.items).all(|(a, b)| {
            a.identity == b.identity && a.display_name == b.display_name && a.modified == b.modified
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(name: &str, file_id: u128) -> DesktopShellItem {
        DesktopShellItem {
            identity: ShellIdentity::FileSystem {
                path: name.into(),
                volume_id: Some(1),
                file_id: Some(file_id),
            },
            display_name: name.into(),
            attributes: Default::default(),
            modified: None,
            size: None,
        }
    }
    #[test]
    fn audit_revision_changes_on_rename_even_when_file_id_does_not() {
        let old = item("old.lnk", 1).identity;
        let renamed = item("new.lnk", 1).identity;
        assert_eq!(old.persistent_key(), renamed.persistent_key());
        assert_ne!(revision_keys(&[old]), revision_keys(&[renamed]));
    }
    #[test]
    fn filtered_members_survive_and_rename_while_deleted_members_do_not() {
        let old = item("old.lnk", 1);
        let renamed = item("renamed.lnk", 1);
        let deleted = item("deleted.lnk", 2);
        let visible = item("visible.lnk", 3);
        let unrelated = item("desktop.ini", 4);
        let mut combined = vec![visible.clone()];
        merge_managed(
            &mut combined,
            vec![renamed.clone(), visible, unrelated],
            &[old.identity, deleted.identity],
        );
        assert_eq!(combined.len(), 2);
        assert!(
            combined
                .iter()
                .any(|entry| entry.identity == renamed.identity)
        );
    }
}
