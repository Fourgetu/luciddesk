use super::{
    DesktopItem, DesktopPlacement, GridPosition, Panel, PanelId, RectDip, ShellIdentity, Workspace,
    WorkspaceError,
};
use std::path::PathBuf;

fn panel(id: u64) -> Panel {
    Panel::new(PanelId::new(id), format!("Panel {id}"), RectDip::default())
}

#[test]
fn rect_enforces_minimum_size() {
    let rect = RectDip::new(1.0, 2.0, 20.0, 30.0);
    assert!((rect.width - RectDip::MIN_WIDTH).abs() < f32::EPSILON);
    assert!((rect.height - RectDip::MIN_HEIGHT).abs() < f32::EPSILON);
}

#[test]
fn switching_content_sources_preserves_independent_preferences() {
    let mut panel = panel(1);
    let path = PathBuf::from(r"C:\folder");
    panel.set_folder(Some(path.clone()));
    panel.set_search(false);
    assert_eq!(panel.folder(), Some(path.as_path()));
    panel.set_folder_list(false);
    panel.set_locked(true);
    panel.set_collapsed(true);
    panel.set_auto_hide(true);
    panel.set_search(true);
    assert!(panel.is_search());
    assert!(panel.folder().is_none());
    assert!(!panel.collapsed());
    assert!(!panel.auto_hide());
    assert!(panel.locked());
    assert!(!panel.folder_list());
    panel.set_folder(None);
    assert!(panel.is_search());
    panel.set_search(false);
    assert!(!panel.is_search());
    assert!(panel.folder().is_none());
    panel.set_search(true);
    panel.set_folder(Some(path.clone()));
    assert!(!panel.is_search());
    assert_eq!(panel.folder(), Some(path.as_path()));
    panel.set_folder(None);
    assert!(!panel.is_search());
    assert!(panel.folder().is_none());
}

#[test]
fn workspace_rejects_duplicate_ids() {
    let result = Workspace::from_panels(vec![panel(7), panel(7)]);
    assert_eq!(result, Err(WorkspaceError::DuplicatePanel(PanelId::new(7))));
}

#[test]
fn workspace_can_add_find_and_remove_panel() {
    let mut workspace = Workspace::new();
    workspace
        .add_panel(panel(1))
        .expect("panel should be added");
    assert_eq!(workspace.panel(PanelId::new(1)).unwrap().title(), "Panel 1");
    assert!(workspace.remove_panel(PanelId::new(1)).is_some());
    assert!(workspace.panels().is_empty());
}

#[test]
fn desktop_reconciliation_preserves_membership_without_moving_files() {
    let identity = ShellIdentity::FileSystem {
        path: PathBuf::from(r"C:\Users\Test\Desktop\Editor.lnk"),
        volume_id: None,
        file_id: None,
    };
    let mut existing = DesktopItem::new(identity.clone(), "Old Editor Name");
    existing.set_placement(DesktopPlacement::Pane {
        pane_id: PanelId::new(7),
        position: GridPosition::new(2, 3),
    });
    let mut workspace = Workspace::new();
    workspace.reconcile_desktop_items([existing]);

    workspace.reconcile_desktop_items([
        DesktopItem::new(identity.clone(), "Editor"),
        DesktopItem::new(
            ShellIdentity::Namespace {
                parsing_name: "::{645FF040-5081-101B-9F08-00AA002F954E}".into(),
            },
            "Recycle Bin",
        ),
    ]);

    let editor = workspace.desktop_item(&identity).unwrap();
    assert_eq!(editor.display_name(), "Editor");
    assert_eq!(
        editor.placement(),
        &DesktopPlacement::Pane {
            pane_id: PanelId::new(7),
            position: GridPosition::new(2, 3),
        }
    );
    assert_eq!(
        identity.file_system_path(),
        Some(PathBuf::from(r"C:\Users\Test\Desktop\Editor.lnk").as_path())
    );
    assert_eq!(workspace.desktop_items().len(), 2);
}

#[test]
fn stable_file_identity_preserves_placement_across_a_rename() {
    let before = ShellIdentity::FileSystem {
        path: PathBuf::from(r"C:\Users\Test\Desktop\Before.txt"),
        volume_id: Some(11),
        file_id: Some(22),
    };
    let after = ShellIdentity::FileSystem {
        path: PathBuf::from(r"C:\Users\Test\Desktop\After.txt"),
        volume_id: Some(11),
        file_id: Some(22),
    };
    let placement = DesktopPlacement::Pane {
        pane_id: PanelId::new(3),
        position: GridPosition::new(1, 4),
    };
    let mut existing = DesktopItem::new(before, "Before");
    existing.set_placement(placement.clone());
    let mut workspace = Workspace::new();
    workspace.reconcile_desktop_items([existing]);

    workspace.reconcile_desktop_items([DesktopItem::new(after.clone(), "After")]);

    let renamed = workspace.desktop_item(&after).unwrap();
    assert_eq!(renamed.display_name(), "After");
    assert_eq!(renamed.placement(), &placement);
    assert_eq!(renamed.identity(), &after);
}
