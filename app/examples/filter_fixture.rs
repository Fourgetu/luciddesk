//! Creates/checks a disposable workspace for production application smoke testing.
use desktop_core::{
    DesktopItem, DesktopPlacement, GridPosition, Panel, PanelId, RectDip, Workspace,
};
use desktop_storage::WorkspaceStore;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _sta = desktop_shell::ShellApartment::initialize_sta()?;
    let path = std::env::args()
        .nth(1)
        .ok_or("Expected disposable workspace.db path")?;
    let mut store = WorkspaceStore::open(std::path::Path::new(&path))?;
    if std::env::args().any(|arg| arg == "--check") {
        let workspace = store.load_workspace()?;
        let count = workspace
            .desktop_items()
            .iter()
            .filter(|item| matches!(item.placement(), DesktopPlacement::Pane { .. }))
            .count();
        println!(
            "persisted_pane_members={count} total={}",
            workspace.desktop_items().len()
        );
        assert_eq!(count, 2);
        return Ok(());
    }
    let snapshot = desktop_shell::native_desktop_snapshot()?;
    let mut workspace = Workspace::new();
    let id = PanelId::new(1);
    workspace.add_panel(Panel::new(
        id,
        "Filter smoke test",
        RectDip::new(650.0, 100.0, 440.0, 380.0),
    ))?;
    workspace.reconcile_desktop_items(
        snapshot
            .items
            .iter()
            .map(|(item, _, _)| DesktopItem::new(item.identity.clone(), item.display_name.clone())),
    );
    let targets: Vec<_> = snapshot
        .items
        .iter()
        .skip(snapshot.items.len() / 2)
        .filter(|(item, _, _)| item.identity.file_system_path().is_some())
        .take(2)
        .map(|(item, _, _)| item.identity.clone())
        .collect();
    assert_eq!(targets.len(), 2);
    for (index, identity) in targets.iter().enumerate() {
        workspace
            .desktop_item_mut(identity)
            .unwrap()
            .set_placement(DesktopPlacement::Pane {
                pane_id: id,
                position: GridPosition::new(index as u32, 0),
            });
    }
    store.save_workspace(&workspace)?;
    println!(
        "fixture_ready total={} pane_members=2",
        workspace.desktop_items().len()
    );
    Ok(())
}
