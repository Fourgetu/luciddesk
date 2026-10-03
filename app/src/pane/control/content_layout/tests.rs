use super::measurement::{count, size_with_folders};
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
