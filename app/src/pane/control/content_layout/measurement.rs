//! Content counts, grid metrics and folder readiness snapshots.
use super::*;

#[derive(Clone, Debug)]
pub(in crate::pane::control) struct FolderSnapshot {
    pub(super) root: std::path::PathBuf,
    pub(super) count: usize,
    pub(super) ready: bool,
}
pub(in crate::pane::control) type FolderSnapshots = HashMap<PanelId, FolderSnapshot>;
pub(in crate::pane::control) fn snapshots(state: &PaneApp) -> FolderSnapshots {
    state
        .folders
        .iter()
        .filter_map(|(id, source)| {
            let root = state.workspace.panel(*id)?.folder()?.to_path_buf();
            Some((
                *id,
                FolderSnapshot {
                    root,
                    count: source.items.len(),
                    ready: !source.loading && source.status.is_none(),
                },
            ))
        })
        .collect()
}
fn folder_count(w: &Workspace, id: PanelId, folders: &FolderSnapshots) -> Result<usize, String> {
    let source = folders
        .get(&id)
        .ok_or("folder is not loaded; query folder get and wait before fitting")?;
    if !source.ready {
        return Err("folder snapshot is loading or failed; query folder get before fitting".into());
    }
    if w.panel(id).and_then(Panel::folder) != Some(source.root.as_path()) {
        return Err(
            "folder mapping changed; apply it and wait for the new snapshot before fitting".into(),
        );
    }
    Ok(source.count)
}
pub(in crate::pane::control) fn live_query(state: &PaneApp, id: PanelId) -> serde_json::Value {
    let panel = state.workspace.panel(id).unwrap();
    if panel.folder().is_none() {
        return query(&state.workspace, id);
    }
    let source = state.folders.get(&id);
    let ready = source.is_some_and(|s| !s.loading && s.status.is_none());
    let n = source.map_or(0, |s| s.items.len());
    let columns = if panel.list_view() {
        1
    } else {
        ((panel.rect().width - layout::PADDING * 2.0) / metrics(&state.workspace).cell_width)
            .floor()
            .max(1.0) as usize
    };
    let rows = n.div_ceil(columns);
    let height = if panel.list_view() {
        layout::HEADER + layout::LIST_HEADER + layout::PADDING + rows as f32 * layout::LIST_ROW
    } else {
        layout::HEADER + layout::PADDING * 2.0 + rows as f32 * metrics(&state.workspace).cell_height
    };
    json!({"supported":true,"ready":ready,"view":if panel.list_view(){"list"}else{"icons"},"item_count":n,"icon_columns":if panel.list_view(){serde_json::Value::Null}else{json!(columns)},"content_rows":rows,"current_path":source.map(|s|&s.path),"required_height_dip":if ready{json!(height.max(RectDip::MIN_HEIGHT))}else{serde_json::Value::Null},"gap_px":snap::GAP_PX})
}

pub(super) fn members(w: &Workspace, id: PanelId) -> Vec<PanelId> {
    w.tab_group(id)
        .map_or_else(|| vec![id], |g| g.members.clone())
}
pub(super) fn count(w: &Workspace, id: PanelId) -> usize {
    w.desktop_items()
        .iter()
        .filter(
            |i| matches!(i.placement(), DesktopPlacement::Pane { pane_id, .. } if *pane_id == id),
        )
        .count()
}
pub(super) fn metrics(w: &Workspace) -> layout::Grid {
    layout::desktop_grid(
        0.0,
        0.0,
        layout::DESKTOP_ICON_SIZE,
        w.pane_options().grid_scale,
    )
}
#[cfg(test)]
pub(super) fn size(w: &Workspace, id: PanelId, columns: u32) -> Result<(f32, f32), String> {
    size_with_folders(w, id, columns, &HashMap::new(), None)
}
pub(super) fn size_with_folders(
    w: &Workspace,
    id: PanelId,
    columns: u32,
    folders: &FolderSnapshots,
    max_rows: Option<u32>,
) -> Result<(f32, f32), String> {
    if !(1..=64).contains(&columns) {
        return Err("icon_columns must be 1..64".into());
    }
    for member in members(w, id) {
        let p = w.panel(member).ok_or("panel does not exist")?;
        if p.is_search() {
            return Err("content fitting does not support dynamic search panels".into());
        }
        if p.locked() {
            return Err("explicitly unlock the panel before fitting or arranging".into());
        }
        if p.collapsed() {
            return Err("expand the panel before fitting or arranging".into());
        }
    }
    if max_rows.is_some_and(|n| n == 0 || n > 10000) {
        return Err("max_rows must be 1..10000".into());
    }
    let panel = w.panel(id).unwrap();
    if panel.folder().is_some() && panel.list_view() {
        let rows = folder_count(w, id, folders)?.min(max_rows.map_or(usize::MAX, |n| n as usize));
        return Ok((
            panel.rect().width,
            (layout::HEADER
                + layout::LIST_HEADER
                + layout::PADDING
                + rows as f32 * layout::LIST_ROW)
                .max(RectDip::MIN_HEIGHT),
        ));
    }
    let g = metrics(w);
    let width = (layout::PADDING * 2.0 + columns as f32 * g.cell_width).max(RectDip::MIN_WIDTH);
    let actual = ((width - layout::PADDING * 2.0) / g.cell_width)
        .floor()
        .max(1.0) as usize;
    let n = if panel.folder().is_some() {
        folder_count(w, id, folders)?
    } else {
        members(w, id)
            .into_iter()
            .map(|id| count(w, id))
            .max()
            .unwrap_or(0)
    };
    let rows = n
        .div_ceil(actual)
        .min(max_rows.map_or(usize::MAX, |n| n as usize));
    Ok((
        width,
        (layout::HEADER + layout::PADDING * 2.0 + rows as f32 * g.cell_height)
            .max(RectDip::MIN_HEIGHT),
    ))
}
pub(in crate::pane::control) fn query(w: &Workspace, id: PanelId) -> serde_json::Value {
    let p = w.panel(id).unwrap();
    if p.is_search() || p.folder().is_some() {
        return json!({"supported":false,"reason":"desktop panels only"});
    }
    let g = metrics(w);
    let columns = ((p.rect().width - layout::PADDING * 2.0) / g.cell_width)
        .floor()
        .max(1.0) as usize;
    let n = count(w, id);
    let window_count = members(w, id)
        .into_iter()
        .map(|id| count(w, id))
        .max()
        .unwrap_or(0);
    json!({"supported":true,"item_count":n,"window_item_count":window_count,"icon_columns":columns,"content_rows":n.div_ceil(columns),
        "cell_dip":{"width":g.cell_width,"height":g.cell_height},"icon_size_dip":g.icon_size,
        "minimum_size_dip":{"width":RectDip::MIN_WIDTH,"height":RectDip::MIN_HEIGHT},"gap_px":snap::GAP_PX,
        "required_height_dip":(layout::HEADER+layout::PADDING*2.0+window_count.div_ceil(columns) as f32*g.cell_height).max(RectDip::MIN_HEIGHT)})
}
pub(super) fn pixel_size(
    w: &Workspace,
    id: PanelId,
    columns: u32,
    scale: f32,
    folders: &FolderSnapshots,
    max_rows: Option<u32>,
) -> Result<(f32, f32), String> {
    let (width, height) = size_with_folders(w, id, columns, folders, max_rows)?;
    // Ceil avoids losing the last cell to fractional-DPI rounding.
    Ok(((width * scale).ceil(), (height * scale).ceil()))
}
