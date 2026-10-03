//! Content sizing uses the renderer's grid; layout is previewed before one commit.
use super::*;
use luciddesk_api::{Operation, SnapAlign, SnapSide};
use luciddesk_window::MonitorDescriptor;

#[derive(Clone, Debug)]
pub(super) struct FolderSnapshot {
    root: std::path::PathBuf,
    count: usize,
    ready: bool,
}
pub(super) type FolderSnapshots = HashMap<PanelId, FolderSnapshot>;
pub(super) fn snapshots(state: &PaneApp) -> FolderSnapshots {
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
pub(super) fn live_query(state: &PaneApp, id: PanelId) -> serde_json::Value {
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

fn members(w: &Workspace, id: PanelId) -> Vec<PanelId> {
    w.tab_group(id)
        .map_or_else(|| vec![id], |g| g.members.clone())
}
fn count(w: &Workspace, id: PanelId) -> usize {
    w.desktop_items()
        .iter()
        .filter(
            |i| matches!(i.placement(), DesktopPlacement::Pane { pane_id, .. } if *pane_id == id),
        )
        .count()
}
fn metrics(w: &Workspace) -> layout::Grid {
    layout::desktop_grid(
        0.0,
        0.0,
        layout::DESKTOP_ICON_SIZE,
        w.pane_options().grid_scale,
    )
}
#[cfg(test)]
fn size(w: &Workspace, id: PanelId, columns: u32) -> Result<(f32, f32), String> {
    size_with_folders(w, id, columns, &HashMap::new(), None)
}
fn size_with_folders(
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
pub(super) fn query(w: &Workspace, id: PanelId) -> serde_json::Value {
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
fn pixel_size(
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
fn wire(id: PanelId, m: &MonitorDescriptor, x: f32, y: f32, width: f32, height: f32) -> Operation {
    let scale = m.dpi as f32 / 96.0;
    Operation::Geometry {
        pane_id: id.get().to_string(),
        monitor_id: m.id.as_str().into(),
        x: x / scale,
        y: y / scale,
        width: width / scale,
        height: height / scale,
    }
}
fn parse(raw: &str) -> Result<PanelId, String> {
    raw.parse::<u64>()
        .ok()
        .filter(|v| *v > 0)
        .map(PanelId::new)
        .ok_or("invalid panel ID".into())
}
fn current(
    w: &Workspace,
    id: PanelId,
    positions: &HashMap<PanelId, RectDip>,
    monitors: &[MonitorDescriptor],
) -> Result<RectDip, String> {
    if let Some(r) = positions.get(&id) {
        return Ok(*r);
    }
    let r = w.panel(id).ok_or("panel does not exist")?.rect();
    let scale = monitors.first().ok_or("no monitor available")?.dpi as f32 / 96.0;
    Ok(RectDip {
        x: r.x * scale,
        y: r.y * scale,
        width: r.width * scale,
        height: r.height * scale,
    })
}
fn overlaps(a: RectDip, b: RectDip) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}
#[cfg(test)]
fn expand(
    w: &Workspace,
    store: &WorkspaceStore,
    op: &Operation,
    monitors: &[MonitorDescriptor],
    pending: &HashMap<PanelId, RectDip>,
) -> Result<Vec<Operation>, String> {
    expand_with_folders(w, store, op, monitors, pending, &HashMap::new())
}
pub(super) fn expand_with_folders(
    w: &Workspace,
    store: &WorkspaceStore,
    op: &Operation,
    monitors: &[MonitorDescriptor],
    pending: &HashMap<PanelId, RectDip>,
    folders: &FolderSnapshots,
) -> Result<Vec<Operation>, String> {
    let mut positions: HashMap<_, _> = store
        .monitor_layout(&display_layout::key(monitors))
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    positions.extend(pending.iter().map(|(k, v)| (*k, *v)));
    let mut result = Vec::new();
    match op {
        Operation::Snap {
            pane_id,
            target_pane_id,
            side,
            align,
            icon_columns,
        } => {
            let id = parse(pane_id)?;
            let target = parse(target_pane_id)?;
            let moving = members(w, id);
            let anchors = members(w, target);
            if moving.iter().any(|id| anchors.contains(id)) {
                return Err("cannot snap a panel to its own shared window".into());
            }
            for member in moving.iter().chain(anchors.iter()) {
                let p = w.panel(*member).ok_or("panel does not exist")?;
                if p.is_search() {
                    return Err("relative snapping requires fixed-height desktop or folder panels; use geometry for search".into());
                }
                if p.collapsed() {
                    return Err("expand both panels before relative snapping".into());
                }
            }
            let anchor = current(w, target, &positions, monitors)?;
            let description = geometry::describe(anchor, monitors);
            let m = monitors
                .iter()
                .find(|m| Some(m.id.as_str()) == description["monitor_id"].as_str())
                .ok_or("target monitor unavailable")?;
            let scale = m.dpi as f32 / 96.0;
            let (width, height) = if let Some(columns) = icon_columns {
                pixel_size(w, id, *columns, scale, folders, None)?
            } else {
                let r = w.panel(id).unwrap().rect();
                ((r.width * scale).round(), (r.height * scale).round())
            };
            let gap = snap::GAP_PX as f32;
            let offset = |available: f32, extent: f32| match align {
                SnapAlign::Start => 0.0,
                SnapAlign::Center => ((available - extent) / 2.0).round(),
                SnapAlign::End => available - extent,
            };
            let (x, y) = match side {
                SnapSide::Left => (
                    anchor.x - width - gap,
                    anchor.y + offset(anchor.height, height),
                ),
                SnapSide::Right => (
                    anchor.x + anchor.width + gap,
                    anchor.y + offset(anchor.height, height),
                ),
                SnapSide::Top => (
                    anchor.x + offset(anchor.width, width),
                    anchor.y - height - gap,
                ),
                SnapSide::Bottom => (
                    anchor.x + offset(anchor.width, width),
                    anchor.y + anchor.height + gap,
                ),
            };
            let placed = RectDip {
                x,
                y,
                width,
                height,
            };
            for panel in w
                .panels()
                .iter()
                .filter(|p| !moving.contains(&p.id()) && !anchors.contains(&p.id()))
            {
                if overlaps(placed, current(w, panel.id(), &positions, monitors)?) {
                    return Err(format!(
                        "snap would overlap panel {}; move it first",
                        panel.id().get()
                    ));
                }
            }
            let proposed = wire(
                id,
                m,
                x - m.work_area.x as f32,
                y - m.work_area.y as f32,
                width,
                height,
            );
            if let Operation::Geometry {
                x,
                y,
                width,
                height,
                ..
            } = &proposed
            {
                geometry::convert(
                    m,
                    RectDip {
                        x: *x,
                        y: *y,
                        width: *width,
                        height: *height,
                    },
                )?;
            }
            result.push(proposed);
        }
        Operation::Fit { pane_id, .. } | Operation::FolderFit { pane_id, .. } => {
            let p = w.panel(parse(pane_id)?).ok_or("panel does not exist")?;
            let (columns, max_rows) = match op {
                Operation::Fit { icon_columns, .. } => (*icon_columns, None),
                Operation::FolderFit {
                    icon_columns,
                    max_rows,
                    ..
                } => {
                    if p.folder().is_none() {
                        return Err("folder.fit requires a folder panel".into());
                    }
                    if p.list_view() && icon_columns.is_some() {
                        return Err(
                            "icon_columns is only valid in icon view; omit it for list view".into(),
                        );
                    }
                    (
                        icon_columns.unwrap_or(
                            ((p.rect().width - layout::PADDING * 2.0) / metrics(w).cell_width)
                                .floor()
                                .clamp(1.0, 64.0) as u32,
                        ),
                        *max_rows,
                    )
                }
                _ => unreachable!(),
            };
            let id = parse(pane_id)?;
            let old = current(w, id, &positions, monitors)?;
            let description = geometry::describe(old, monitors);
            let m = monitors
                .iter()
                .find(|m| Some(m.id.as_str()) == description["monitor_id"].as_str())
                .ok_or("no monitor available")?;
            let (width, height) =
                pixel_size(w, id, columns, m.dpi as f32 / 96.0, folders, max_rows)?;
            if width > m.work_area.width as f32 || height > m.work_area.height as f32 {
                return Err(
                    "content does not fit the monitor; choose more icon columns or split the panel"
                        .into(),
                );
            }
            let x = (old.x - m.work_area.x as f32).clamp(0.0, m.work_area.width as f32 - width);
            let y = (old.y - m.work_area.y as f32).clamp(0.0, m.work_area.height as f32 - height);
            result.push(wire(id, m, x, y, width, height));
        }
        Operation::Arrange {
            monitor_id,
            columns,
            icon_columns,
        } => {
            if columns.is_empty()
                || columns.len() > 16
                || columns.iter().any(Vec::is_empty)
                || columns.iter().map(Vec::len).sum::<usize>() > 256
            {
                return Err("expected 1..16 nonempty panel columns, at most 256 panels".into());
            }
            let m = monitors
                .iter()
                .find(|m| m.id.as_str() == monitor_id)
                .ok_or("unknown monitor ID")?;
            let scale = m.dpi as f32 / 96.0;
            let mut selected = std::collections::HashSet::new();
            let mut sizes = Vec::new();
            for column in columns {
                let mut cells = Vec::new();
                for raw in column {
                    let id = parse(raw)?;
                    let (width, height) = pixel_size(w, id, *icon_columns, scale, folders, None)?;
                    for member in members(w, id) {
                        if !selected.insert(member) {
                            return Err(
                                "duplicate panel or shared tab window in arrangement".into()
                            );
                        }
                    }
                    cells.push((id, width, height));
                }
                sizes.push(cells);
            }
            let gap = snap::GAP_PX as f32;
            let mut right = m.work_area.width as f32 - gap;
            let mut rectangles = Vec::new();
            // Input columns are left-to-right, panes within each are top-to-bottom.
            for column in sizes.iter().rev() {
                let width = column.iter().map(|(_, w, _)| *w).fold(0.0, f32::max);
                let x = right - width;
                let mut y = gap;
                if x < gap {
                    return Err(
                        "arrangement exceeds work area width; use fewer panel columns".into(),
                    );
                }
                for &(id, width, height) in column {
                    if y + height > m.work_area.height as f32 - gap {
                        return Err("arrangement exceeds work area height; use more panel columns or icon columns".into());
                    }
                    result.push(wire(id, m, x, y, width, height));
                    rectangles.push(RectDip {
                        x: m.work_area.x as f32 + x,
                        y: m.work_area.y as f32 + y,
                        width,
                        height,
                    });
                    y += height + gap;
                }
                right = x - gap;
            }
            for p in w.panels().iter().filter(|p| !selected.contains(&p.id())) {
                let r = current(w, p.id(), &positions, monitors)?;
                if rectangles.iter().any(|placed| overlaps(*placed, r)) {
                    return Err(format!(
                        "arrangement overlaps unselected panel {}; include or move it first",
                        p.id().get()
                    ));
                }
            }
        }
        _ => return Err("not a content layout operation".into()),
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn monitor() -> MonitorDescriptor {
        let rect = luciddesk_window::PixelRect {
            x: -3840,
            y: 0,
            width: 3840,
            height: 2088,
        };
        MonitorDescriptor {
            id: luciddesk_core::MonitorId::new("test"),
            bounds: rect,
            work_area: rect,
            dpi: 144,
            primary: true,
        }
    }
    #[test]
    fn folder_sizing_uses_ready_snapshot_and_list_or_icon_metrics() {
        let mut s = super::super::super::tests::test_state();
        let id = PanelId::new(1);
        let root = std::env::temp_dir();
        s.workspace
            .panel_mut(id)
            .unwrap()
            .set_folder(Some(root.clone()));
        let mut folders = HashMap::from([(
            id,
            FolderSnapshot {
                root,
                count: 17,
                ready: false,
            },
        )]);
        assert!(size_with_folders(&s.workspace, id, 4, &folders, None).is_err());
        folders.get_mut(&id).unwrap().ready = true;
        s.workspace.panel_mut(id).unwrap().set_list_view(false);
        let (width, height) = size_with_folders(&s.workspace, id, 4, &folders, None).unwrap();
        assert_eq!(width, 376.0);
        assert_eq!(height, 544.0);
        assert_eq!(
            size_with_folders(&s.workspace, id, 4, &folders, Some(2))
                .unwrap()
                .1,
            256.0
        );
        s.workspace.panel_mut(id).unwrap().set_list_view(true);
        assert_eq!(
            size_with_folders(&s.workspace, id, 4, &folders, Some(5)).unwrap(),
            (s.workspace.panel(id).unwrap().rect().width, 240.0)
        );
        assert_eq!(
            folders[&id].count, 17,
            "viewport cap does not truncate inventory"
        );
        s.workspace
            .panel_mut(id)
            .unwrap()
            .set_folder(Some(std::env::temp_dir().join("changed")));
        assert!(size_with_folders(&s.workspace, id, 4, &folders, None).is_err());
    }
    #[test]
    fn mixed_folder_arrangement_and_folder_fit_validate_before_saving() {
        let mut s = super::super::super::tests::test_state();
        let id = PanelId::new(1);
        let root = std::env::temp_dir();
        s.workspace
            .panel_mut(id)
            .unwrap()
            .set_folder(Some(root.clone()));
        s.workspace.panel_mut(id).unwrap().set_list_view(true);
        let folders = HashMap::from([(
            id,
            FolderSnapshot {
                root,
                count: 8,
                ready: true,
            },
        )]);
        let m = monitor();
        let before = s.store.change_count();
        let op = Operation::Arrange {
            monitor_id: "test".into(),
            columns: vec![vec!["1".into(), "2".into()]],
            icon_columns: 6,
        };
        assert_eq!(
            expand_with_folders(
                &s.workspace,
                &s.store,
                &op,
                &[m.clone()],
                &HashMap::new(),
                &folders
            )
            .unwrap()
            .len(),
            2
        );
        for op in [
            Operation::FolderFit {
                pane_id: "1".into(),
                icon_columns: Some(4),
                max_rows: None,
            },
            Operation::FolderFit {
                pane_id: "1".into(),
                icon_columns: None,
                max_rows: Some(0),
            },
        ] {
            assert!(
                expand_with_folders(
                    &s.workspace,
                    &s.store,
                    &op,
                    &[m.clone()],
                    &HashMap::new(),
                    &folders
                )
                .is_err()
            );
        }
        assert_eq!(s.store.change_count(), before);
    }
    #[test]
    fn relative_snap_has_fixed_gap_in_all_directions_and_alignments() {
        let s = super::super::super::tests::test_state();
        let m = monitor();
        let anchor = RectDip {
            x: -2200.0,
            y: 800.0,
            width: 900.0,
            height: 600.0,
        };
        let positions = HashMap::from([(PanelId::new(2), anchor)]);
        for side in [
            SnapSide::Left,
            SnapSide::Right,
            SnapSide::Top,
            SnapSide::Bottom,
        ] {
            for align in [SnapAlign::Start, SnapAlign::Center, SnapAlign::End] {
                let op = Operation::Snap {
                    pane_id: "1".into(),
                    target_pane_id: "2".into(),
                    side,
                    align,
                    icon_columns: Some(6),
                };
                let ops = expand(&s.workspace, &s.store, &op, &[m.clone()], &positions).unwrap();
                let Operation::Geometry {
                    x,
                    y,
                    width,
                    height,
                    ..
                } = ops[0]
                else {
                    panic!()
                };
                let (_, r) = geometry::convert(
                    &m,
                    RectDip {
                        x,
                        y,
                        width,
                        height,
                    },
                )
                .unwrap();
                let (gap, delta, available, extent) = match side {
                    SnapSide::Left => (
                        anchor.x - r.x - r.width,
                        r.y - anchor.y,
                        anchor.height,
                        r.height,
                    ),
                    SnapSide::Right => (
                        r.x - anchor.x - anchor.width,
                        r.y - anchor.y,
                        anchor.height,
                        r.height,
                    ),
                    SnapSide::Top => (
                        anchor.y - r.y - r.height,
                        r.x - anchor.x,
                        anchor.width,
                        r.width,
                    ),
                    SnapSide::Bottom => (
                        r.y - anchor.y - anchor.height,
                        r.x - anchor.x,
                        anchor.width,
                        r.width,
                    ),
                };
                assert_eq!(gap, snap::GAP_PX as f32);
                let expected = match align {
                    SnapAlign::Start => 0.0,
                    SnapAlign::Center => ((available - extent) / 2.0).round(),
                    SnapAlign::End => available - extent,
                };
                assert_eq!(delta, expected);
            }
        }
    }
    #[test]
    fn relative_snap_rejects_self_and_offscreen_and_uses_pending_target() {
        let s = super::super::super::tests::test_state();
        let m = monitor();
        let op = |target: &str, side| Operation::Snap {
            pane_id: "1".into(),
            target_pane_id: target.into(),
            side,
            align: SnapAlign::Start,
            icon_columns: None,
        };
        assert!(
            expand(
                &s.workspace,
                &s.store,
                &op("1", SnapSide::Left),
                &[m.clone()],
                &HashMap::new()
            )
            .is_err()
        );
        let positions = HashMap::from([(
            PanelId::new(2),
            RectDip {
                x: -3840.0,
                y: 100.0,
                width: 720.0,
                height: 540.0,
            },
        )]);
        assert!(
            expand(
                &s.workspace,
                &s.store,
                &op("2", SnapSide::Left),
                &[m.clone()],
                &positions
            )
            .is_err()
        );
        let ops = expand(
            &s.workspace,
            &s.store,
            &op("2", SnapSide::Right),
            &[m.clone()],
            &positions,
        )
        .unwrap();
        let Operation::Geometry { x, .. } = ops[0] else {
            panic!()
        };
        assert_eq!((x * 1.5).round(), 725.0);
    }
    #[test]
    fn arrange_matches_grid_and_native_snap_gap_without_writes() {
        let s = super::super::super::tests::test_state();
        let m = monitor();
        let before = s.store.change_count();
        let ops = expand(
            &s.workspace,
            &s.store,
            &Operation::Arrange {
                monitor_id: "test".into(),
                columns: vec![vec!["1".into(), "2".into()]],
                icon_columns: 6,
            },
            &[m.clone()],
            &HashMap::new(),
        )
        .unwrap();
        let rects: Vec<_> = ops
            .iter()
            .map(|op| match op {
                Operation::Geometry {
                    pane_id,
                    x,
                    y,
                    width,
                    height,
                    ..
                } => {
                    let (_, px) = geometry::convert(
                        &m,
                        RectDip {
                            x: *x,
                            y: *y,
                            width: *width,
                            height: *height,
                        },
                    )
                    .unwrap();
                    let grid = layout::desktop_grid(
                        *width,
                        *height,
                        layout::DESKTOP_ICON_SIZE,
                        s.workspace.pane_options().grid_scale,
                    );
                    assert!(grid.columns >= 6);
                    assert!(
                        grid.columns * grid.visible_rows
                            >= count(&s.workspace, parse(pane_id).unwrap())
                    );
                    px
                }
                _ => panic!(),
            })
            .collect();
        assert_eq!(
            rects[1].y - rects[0].y - rects[0].height,
            snap::GAP_PX as f32
        );
        assert_eq!(
            m.work_area.x as f32 + m.work_area.width as f32 - rects[0].x - rects[0].width,
            snap::GAP_PX as f32
        );
        assert_eq!(s.store.change_count(), before);
    }
    #[test]
    fn layout_rejects_overflow_duplicates_and_unsupported_content() {
        let mut s = super::super::super::tests::test_state();
        let m = monitor();
        let op = |columns, icon_columns| Operation::Arrange {
            monitor_id: "test".into(),
            columns,
            icon_columns,
        };
        for invalid in [
            op(vec![], 6),
            op(vec![vec!["1".into(), "1".into()]], 6),
            op(vec![vec!["1".into()]], 0),
            op(vec![vec!["1".into()]], 64),
        ] {
            assert!(
                expand(
                    &s.workspace,
                    &s.store,
                    &invalid,
                    &[m.clone()],
                    &HashMap::new()
                )
                .is_err()
            );
        }
        s.workspace
            .panel_mut(PanelId::new(1))
            .unwrap()
            .set_locked(true);
        assert!(size(&s.workspace, PanelId::new(1), 6).is_err());
        s.workspace
            .panel_mut(PanelId::new(1))
            .unwrap()
            .set_locked(false);
        s.workspace
            .panel_mut(PanelId::new(1))
            .unwrap()
            .set_folder(Some(std::env::temp_dir()));
        assert!(size(&s.workspace, PanelId::new(1), 6).is_err());
    }
    #[test]
    fn arranging_subset_never_covers_unselected_panel() {
        let s = super::super::super::tests::test_state();
        let m = monitor();
        let positions = HashMap::from([(
            PanelId::new(2),
            RectDip {
                x: -1000.0,
                y: 0.0,
                width: 1000.0,
                height: 1000.0,
            },
        )]);
        let op = Operation::Arrange {
            monitor_id: "test".into(),
            columns: vec![vec!["1".into()]],
            icon_columns: 6,
        };
        assert!(
            expand(&s.workspace, &s.store, &op, &[m], &positions)
                .unwrap_err()
                .contains("unselected panel")
        );
    }
    #[test]
    fn fit_keeps_position_and_rounds_outward_at_fractional_dpi() {
        let s = super::super::super::tests::test_state();
        let mut m = monitor();
        m.dpi = 120;
        let positions = HashMap::from([(
            PanelId::new(1),
            RectDip {
                x: -3500.0,
                y: 100.0,
                width: 600.0,
                height: 450.0,
            },
        )]);
        let ops = expand(
            &s.workspace,
            &s.store,
            &Operation::Fit {
                pane_id: "1".into(),
                icon_columns: 5,
            },
            &[m.clone()],
            &positions,
        )
        .unwrap();
        let Operation::Geometry {
            x,
            y,
            width,
            height,
            ..
        } = ops[0]
        else {
            panic!()
        };
        let (_, px) = geometry::convert(
            &m,
            RectDip {
                x,
                y,
                width,
                height,
            },
        )
        .unwrap();
        assert_eq!(px.x, -3500.0);
        assert_eq!(px.y, 100.0);
        let grid = layout::desktop_grid(
            width,
            height,
            layout::DESKTOP_ICON_SIZE,
            s.workspace.pane_options().grid_scale,
        );
        assert!(grid.columns >= 5);
        assert!(grid.visible_rows * grid.columns >= count(&s.workspace, PanelId::new(1)));
    }
}
