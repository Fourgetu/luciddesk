//! One-shot ordinary pane ordering shared by menus and the control API.
use super::*;

pub(super) fn apply(workspace: &mut Workspace, id: PanelId, descending: bool) -> Result<bool, String> {
    let panel = workspace.panel(id).ok_or("panel does not exist")?;
    if !panel.supports_tabs() { return Err("only desktop panels support name sorting".into()); }
    if panel.locked() { return Err("panel is locked".into()); }
    let mut entries: Vec<_> = workspace.desktop_items().iter()
        .filter_map(|item| match item.placement() {
            DesktopPlacement::Pane { pane_id, position } if *pane_id == id => Some((
                item.display_name().encode_utf16().chain(Some(0)).collect::<Vec<_>>(),
                item.identity().persistent_key(), (position.row, position.column))),
            _ => None,
        }).collect();
    entries.sort_unstable_by(|a,b| a.2.cmp(&b.2).then_with(|| a.1.cmp(&b.1)));
    let before: Vec<_> = entries.iter().map(|e|e.1.clone()).collect();
    entries.sort_unstable_by(|a,b| {
        let cmp = unsafe { windows_sys::Win32::UI::Shell::StrCmpLogicalW(a.0.as_ptr(), b.0.as_ptr()) }.cmp(&0)
            .then_with(|| a.0.cmp(&b.0)).then_with(|| a.1.cmp(&b.1));
        if descending { cmp.reverse() } else { cmp }
    });
    if entries.iter().map(|e| &e.1).eq(before.iter()) { return Ok(false); }
    let positions: HashMap<_,_> = entries.into_iter().enumerate().map(|(at,e)|(e.1,at)).collect();
    for item in workspace.desktop_items_mut() {
        if let Some(&at) = positions.get(&item.identity().persistent_key()) {
            item.set_placement(DesktopPlacement::Pane { pane_id: id, position: GridPosition::new(at as u32,0) });
        }
    }
    Ok(true)
}
