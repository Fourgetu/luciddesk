use super::*;

fn reconcile(workspace: &mut Workspace, inventory: Vec<DesktopItem>) {
    workspace.reconcile_desktop_items(inventory);
    let valid: Vec<_> = workspace.panels().iter().map(Panel::id).collect();
    let mut next = workspace
        .desktop_items()
        .iter()
        .filter_map(|item| match item.placement() {
            DesktopPlacement::Pane { pane_id, position } if *pane_id == PanelId::new(1) => {
                Some(position.column)
            }
            _ => None,
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    for item in workspace.desktop_items_mut() {
        if !matches!(item.placement(),DesktopPlacement::Pane{pane_id,..} if valid.contains(pane_id))
        {
            item.set_placement(DesktopPlacement::Pane {
                pane_id: PanelId::new(1),
                position: GridPosition::new(next, 0),
            });
            next = next.saturating_add(1);
        }
    }
}

fn test_state() -> PaneApp {
    let mut workspace = Workspace::new();
    for id in [1, 2] {
        workspace
            .add_panel(Panel::new(
                PanelId::new(id),
                format!("Group {id}"),
                RectDip::default(),
            ))
            .unwrap();
    }
    let inventory = ["A", "B", "C"]
        .into_iter()
        .map(|name| {
            DesktopItem::new(
                ShellIdentity::Namespace {
                    parsing_name: format!("test:{name}"),
                },
                name,
            )
        })
        .collect();
    reconcile(&mut workspace, inventory);
    let (_, receiver) = mpsc::channel();
    PaneApp {
        settings: None,
        session: None,
        workspace,
        store: WorkspaceStore::open_in_memory().unwrap(),
        views: Vec::new(),
        images: HashMap::new(),
        receiver,
    }
}

#[test]
fn snapped_content_bottom_and_scrollbar_use_the_same_row_metrics() {
    let mut model = GroupModel {
        theme: desktop_core::PanelTheme::Dark,
        dark: true,
        hovered_item: None,
        hovered_button: None,
        focused: false,
        auto_hide: false,
        reveal: 1.0,
        backdrop: desktop_core::Backdrop::Mica,
        native_material: false,
        title: "Sizing test".into(),
        items: vec![],
        icon_size: 48.0,
        spacing: (88.0, 96.0),
        selected: None,
        renaming: None,
        scroll: 0,
        collapsed: false,
        loading: false,
    };
    for label in ["Short", "Warhammer 40,000 ????"] {
        model.items = (0..10)
            .map(|i| Item {
                identity: ShellIdentity::Namespace {
                    parsing_name: format!("test-{i}"),
                },
                label: if i == 9 { label.into() } else { "Icon".into() },
                image: None,
            })
            .collect();
        let grid = model.grid(376.0, 500.0);
        let rows = model.row_contents(grid);
        let height = layout::pane_content_height(3, grid.cell_height, &rows);
        assert_eq!(
            height - (layout::HEADER + layout::PADDING + 2.0 * grid.cell_height + rows[2]),
            layout::PADDING
        );
        for reduction in [0.0, 5.0, 10.0] {
            let grid = model.grid(376.0, height - reduction);
            assert_eq!(grid.max_scroll(model.items.len()), 0);
            assert_eq!(grid.visible_rows, 3);
        }
        assert_eq!(
            model
                .grid(376.0, height - 14.0)
                .max_scroll(model.items.len()),
            1
        );
    }
}

#[test]
fn unrelated_keys_do_not_select_first_icon_or_emit_pane_focus() {
    // Real focus/default-key dispatch interacts with process-wide windowing
    // state left by other live UI fixtures. Exercise it in a fresh process,
    // while still requiring all of the native message assertions to pass.
    const ISOLATED: &str = "LUCIDPANE_KEYBOARD_TEST_CHILD";
    if std::env::var_os(ISOLATED).is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "pane::tests::unrelated_keys_do_not_select_first_icon_or_emit_pane_focus",
                "--test-threads=1",
            ])
            .env(ISOLATED, "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let timed_out = loop {
            if child.try_wait().unwrap().is_some() {
                break false;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                break true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let output = child.wait_with_output().unwrap();
        assert!(
            !timed_out && output.status.success(),
            "keyboard child: status={}, timed_out={timed_out}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    // Only this disposable test process: native failures must produce a
    // failing exit status instead of leaving a modal crash dialog behind.
    unsafe {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            GetErrorMode, SEM_NOGPFAULTERRORBOX, SetErrorMode,
        };
        SetErrorMode(GetErrorMode() | SEM_NOGPFAULTERRORBOX);
    }
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageW, WM_KEYDOWN, WM_KILLFOCUS, WM_SETFOCUS,
    };
    let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
    let model = Rc::new(RefCell::new(GroupModel {
        theme: desktop_core::PanelTheme::Dark,
        dark: true,
        hovered_item: None,
        hovered_button: None,
        focused: false,
        auto_hide: false,
        reveal: 1.0,
        backdrop: desktop_core::Backdrop::Mica,
        native_material: false,
        title: "Keyboard regression".into(),
        items: vec![],
        icon_size: 48.0,
        spacing: (88.0, 96.0),
        selected: None,
        renaming: None,
        scroll: 0,
        collapsed: false,
        loading: false,
    }));
    model.borrow_mut().items = (0..6)
        .map(|index| Item {
            identity: ShellIdentity::Namespace {
                parsing_name: format!("test:{index}"),
            },
            label: format!("Item {index}"),
            image: None,
        })
        .collect();
    let focus_events = Rc::new(std::cell::Cell::new(0));
    let observed = Rc::clone(&focus_events);
    let pane = window::create(
        RectDip::new(40.0, 40.0, 200.0, 160.0),
        Rc::clone(&model),
        move |event| {
            if matches!(event, Event::PaneItemFocus) {
                observed.set(observed.get() + 1);
            }
            false
        },
    )
    .unwrap();
    let hwnd = pane.hwnd().cast();
    for selected in [None, Some(3)] {
        model.borrow_mut().selected = selected;
        model.borrow_mut().scroll = 1;
        unsafe {
            SendMessageW(hwnd, WM_KILLFOCUS, 0, 0);
            SendMessageW(hwnd, WM_SETFOCUS, 0, 0);
        }
        let before = focus_events.get();
        // Letters, digits, modifiers, space, Tab, Backspace and unhandled function keys.
        for key in [0x41, 0x5a, 0x30, 0x10, 0x11, 0x12, 0x20, 0x09, 0x08, 0x70] {
            unsafe {
                SendMessageW(hwnd, WM_KEYDOWN, key, 0);
            }
            assert_eq!(model.borrow().selected, selected, "key={key:x}");
            assert_eq!(model.borrow().scroll, 1, "key={key:x}");
            assert_eq!(focus_events.get(), before, "key={key:x}");
        }
    }
    model.borrow_mut().selected = Some(0);
    unsafe {
        SendMessageW(hwnd, WM_KEYDOWN, 0x27, 0);
    } // Right still navigates.
    assert_eq!(model.borrow().selected, Some(1));
    unsafe {
        SendMessageW(hwnd, WM_KEYDOWN, 0x1b, 0);
    } // Escape still clears.
    assert_eq!(model.borrow().selected, None);
    unsafe {
        SendMessageW(hwnd, WM_KEYDOWN, 0x41, 0);
    }
    assert_eq!(model.borrow().selected, None);
    // Keep a real popup open past the fold duration and inspect the pane
    // before dismissing it. The old in-callback modal loop loses its ticks.
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        const RESULT: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.FoldMenuTest");
        unsafe extern "system" fn check_fold(
            hwnd: windows_sys::Win32::Foundation::HWND,
            _: u32,
            id: usize,
            _: u32,
        ) {
            unsafe {
                KillTimer(hwnd, id);
                let mut r = RECT::default();
                GetClientRect(hwnd, &raw mut r);
                let dpi =
                    windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
                let mut popup = std::ptr::null_mut();
                loop {
                    popup = FindWindowExW(
                        std::ptr::null_mut(),
                        popup,
                        std::ptr::null(),
                        windows_sys::w!("\u{5206}\u{7ec4}\u{83dc}\u{5355}"),
                    );
                    if popup.is_null() || GetWindow(popup, GW_OWNER) == hwnd {
                        break;
                    }
                }
                let passed =
                    GetWindow(popup, GW_OWNER) == hwnd && r.bottom == (300.0 * dpi).round() as i32;
                SetPropW(hwnd, RESULT, (if passed { 1usize } else { 2usize }) as _);
                if GetWindow(popup, GW_OWNER) == hwnd {
                    PostMessageW(popup, WM_CLOSE, 0, 0);
                }
            }
        }
        model.borrow_mut().collapsed = false;
        SendMessageW(hwnd, window::ANIMATE_FOLD, 0, 300);
        SetTimer(hwnd, 98, 500, Some(check_fold));
        SendMessageW(hwnd, WM_CONTEXTMENU, 0, -1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while GetPropW(hwnd, RESULT).is_null() && std::time::Instant::now() < deadline {
            let mut msg = MSG::default();
            while PeekMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&raw const msg);
                DispatchMessageW(&raw const msg);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(
            RemovePropW(hwnd, RESULT) as usize,
            1,
            "fold must finish while the popup is still open"
        );
        assert_eq!(model.borrow().reveal, 1.0);
        KillTimer(hwnd, 98);
    }
    // Client hover must appear immediately, then clear on either a
    // non-client border move or a leave notification without re-arming.
    unsafe {
        use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetClientRect, WM_MOUSEMOVE, WM_NCMOUSEMOVE,
        };
        let mut rect = windows_sys::Win32::Foundation::RECT::default();
        GetClientRect(hwnd, &raw mut rect);
        let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let x = rect.right - (24.0 * dpi) as i32;
        let y = (19.0 * dpi) as i32;
        let position = ((y as isize) << 16) | x as isize;
        for leave in [WM_NCMOUSEMOVE, WM_MOUSELEAVE] {
            SendMessageW(hwnd, WM_MOUSEMOVE, 0, position);
            assert_eq!(model.borrow().hovered_button, Some(1));
            SendMessageW(hwnd, leave, 0, 0);
            assert_eq!(model.borrow().hovered_button, None);
            assert_eq!(model.borrow().hovered_item, None);
        }
    }
    // Empty panes keep the proposed size even inside the grid magnet.
    model.borrow_mut().items.clear();
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        for dpi in [1.0, 1.25, 1.5, 2.0] {
            for edge in [
                WMSZ_LEFT,
                WMSZ_RIGHT,
                WMSZ_TOP,
                WMSZ_BOTTOM,
                WMSZ_TOPLEFT,
                WMSZ_TOPRIGHT,
                WMSZ_BOTTOMLEFT,
                WMSZ_BOTTOMRIGHT,
            ] {
                let mut rect = RECT {
                    left: 0,
                    top: 0,
                    right: (293.0 * dpi) as i32,
                    bottom: (259.0 * dpi) as i32,
                };
                let before = rect;
                SendMessageW(hwnd, WM_SIZING, edge as usize, (&raw mut rect) as isize);
                assert_eq!(
                    (rect.left, rect.top, rect.right, rect.bottom),
                    (before.left, before.top, before.right, before.bottom)
                );
            }
        }
    }
    // Simulate a leave notification consumed by the nested menu loop.
    // A hidden pane cannot be under the pointer: resync must clear both
    // stale header and item highlights without another mouse movement.
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow, WM_APP};
        ShowWindow(hwnd, SW_HIDE);
        model.borrow_mut().hovered_button = Some(1);
        model.borrow_mut().hovered_item = Some(0);
        SendMessageW(hwnd, WM_APP + 11, 0, 0);
    }
    assert_eq!(model.borrow().hovered_button, None);
    assert_eq!(model.borrow().hovered_item, None);
    window::prepare_close(hwnd);
    drop(pane);
}

#[test]
fn pane_layer_switch_and_wallpaper_material_initialize() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongW, WS_EX_TOPMOST};
    let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
    let model = Rc::new(RefCell::new(GroupModel {
        theme: desktop_core::PanelTheme::Dark,
        dark: true,
        hovered_item: None,
        hovered_button: None,
        focused: false,
        auto_hide: false,
        reveal: 1.0,
        backdrop: desktop_core::Backdrop::Mica,
        native_material: false,
        title: "Layer test".into(),
        items: vec![],
        icon_size: 48.0,
        spacing: (88.0, 96.0),
        selected: None,
        renaming: None,
        scroll: 0,
        collapsed: false,
        loading: false,
    }));
    let pane = window::create(
        RectDip::new(40.0, 40.0, 200.0, 160.0),
        Rc::clone(&model),
        |_| false,
    )
    .unwrap();
    assert!(
        model.borrow().native_material,
        "System wallpaper brush was unavailable"
    );
    let hwnd = pane.hwnd().cast();
    let mut frame_enabled = 1i32;
    unsafe {
        windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
            windows::Win32::Foundation::HWND(hwnd),
            windows::Win32::Graphics::Dwm::DWMWA_NCRENDERING_ENABLED,
            (&raw mut frame_enabled).cast(),
            4,
        )
        .unwrap();
    }
    assert_eq!(
        frame_enabled, 0,
        "Pane must not use the DWM activation frame"
    );
    for material in [
        desktop_core::Backdrop::Mica,
        desktop_core::Backdrop::MicaAlt,
        desktop_core::Backdrop::Acrylic,
    ] {
        for dark in [false, true] {
            {
                let mut m = model.borrow_mut();
                m.backdrop = material;
                m.dark = dark;
            }
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(hwnd, 0x0008, 0, 0); // WM_KILLFOCUS
                windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(hwnd, 0x000f, 0, 0); // WM_PAINT
            }
            assert!(
                model.borrow().native_material,
                "Composition material failed after focus loss"
            );
        }
    }
    window::set_layer(hwnd, false);
    window::set_layer(hwnd, true);
    assert_ne!(
        unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST,
        0
    );
    window::set_layer(hwnd, false);
    assert_eq!(
        unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST,
        0
    );
    window::prepare_close(hwnd);
    drop(pane);
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            IsWindow, MSG, PM_REMOVE, PeekMessageW, WM_QUIT,
        };
        assert_eq!(IsWindow(hwnd), 0);
        let mut message = MSG::default();
        assert_eq!(
            PeekMessageW(
                &raw mut message,
                std::ptr::null_mut(),
                WM_QUIT,
                WM_QUIT,
                PM_REMOVE
            ),
            0,
            "closing one pane must not quit the app"
        );
    }
}

#[test]
fn desktop_sort_preserves_pane_order_after_a_gap_and_legacy_grid() {
    let mut state = test_state();
    let id = PanelId::new(1);
    // Two rows from the former native-pane layout share a column.
    for (item, position) in state.workspace.desktop_items_mut().iter_mut().zip([
        GridPosition::new(0, 0),
        GridPosition::new(1, 0),
        GridPosition::new(0, 1),
    ]) {
        item.set_placement(DesktopPlacement::Pane {
            pane_id: id,
            position,
        });
    }
    let names = |s: &PaneApp| {
        items_for(s, id)
            .into_iter()
            .map(|i| i.label)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&state), ["A", "B", "C"]);
    normalize_pane_orders(&mut state);
    let mut inventory = state.workspace.desktop_items().to_vec();
    inventory.reverse();
    state.workspace.reconcile_desktop_items(inventory);
    assert_eq!(names(&state), ["A", "B", "C"]);
    let a = items_for(&state, id)[0].identity.clone();
    state
        .workspace
        .desktop_item_mut(&a)
        .unwrap()
        .set_placement(DesktopPlacement::default());
    normalize_pane_orders(&mut state);
    let at = items_for(&state, id).len();
    state
        .workspace
        .desktop_item_mut(&a)
        .unwrap()
        .set_placement(DesktopPlacement::Pane {
            pane_id: id,
            position: GridPosition::new(at as u32, 0),
        });
    let mut inventory = state.workspace.desktop_items().to_vec();
    inventory.reverse();
    state.workspace.reconcile_desktop_items(inventory);
    assert_eq!(names(&state), ["B", "C", "A"]);
    state.store.save_workspace(&state.workspace).unwrap();
    state.workspace = state.store.load_workspace().unwrap();
    assert_eq!(names(&state), ["B", "C", "A"]);
}

#[test]
fn moving_between_groups_keeps_identity_unique_and_persists_order() {
    let mut state = test_state();
    let before: Vec<_> = state
        .workspace
        .desktop_items()
        .iter()
        .map(|i| i.identity().clone())
        .collect();
    transfer(&mut state, PanelId::new(1), 1, PanelId::new(2), 0).unwrap();
    assert_eq!(
        items_for(&state, PanelId::new(1))
            .iter()
            .map(|i| i.label.as_str())
            .collect::<Vec<_>>(),
        ["A", "C"]
    );
    assert_eq!(items_for(&state, PanelId::new(2))[0].label, "B");
    assert_eq!(
        state
            .workspace
            .desktop_items()
            .iter()
            .map(|i| i.identity().clone())
            .collect::<Vec<_>>(),
        before
    );
    state.workspace = state.store.load_workspace().unwrap();
    assert_eq!(items_for(&state, PanelId::new(2))[0].label, "B");
    transfer(&mut state, PanelId::new(2), 0, PanelId::new(1), 1).unwrap();
    assert_eq!(
        items_for(&state, PanelId::new(1))
            .iter()
            .map(|i| i.label.as_str())
            .collect::<Vec<_>>(),
        ["A", "B", "C"]
    );
    assert!(items_for(&state, PanelId::new(2)).is_empty());
}

#[test]
fn closing_groups_releases_items_and_persists_an_empty_workspace() {
    let mut state = test_state();
    transfer(&mut state, PanelId::new(1), 0, PanelId::new(2), 0).unwrap();
    let identities: Vec<_> = state
        .workspace
        .desktop_items()
        .iter()
        .map(|item| item.identity().clone())
        .collect();
    let state = Rc::new(RefCell::new(state));
    handle(&state, PanelId::new(1), Event::ClosePane).unwrap();
    {
        let s = state.borrow();
        assert!(s.workspace.panel(PanelId::new(1)).is_none());
        assert_eq!(items_for(&s, PanelId::new(2))[0].label, "A");
        assert_eq!(
            s.workspace
                .desktop_items()
                .iter()
                .filter(|item| matches!(item.placement(), DesktopPlacement::FreeDesktop { .. }))
                .count(),
            2
        );
    }
    handle(&state, PanelId::new(2), Event::ClosePane).unwrap();
    let s = state.borrow();
    let loaded = s.store.load_workspace().unwrap();
    assert!(loaded.panels().is_empty());
    assert_eq!(
        loaded
            .desktop_items()
            .iter()
            .map(|item| item.identity().clone())
            .collect::<Vec<_>>(),
        identities
    );
    assert!(
        loaded
            .desktop_items()
            .iter()
            .all(|item| matches!(item.placement(), DesktopPlacement::FreeDesktop { .. }))
    );
}

#[test]
fn appearance_is_global_while_behavior_remains_per_group() {
    let state = Rc::new(RefCell::new(test_state()));
    let id = PanelId::new(1);
    let other = state
        .borrow()
        .workspace
        .panel(PanelId::new(2))
        .unwrap()
        .clone();
    let before = state.borrow().workspace.panel(id).unwrap().clone();
    for event in [
        Event::Theme(desktop_core::PanelTheme::Dark),
        Event::Material(desktop_core::Backdrop::Acrylic),
        Event::ToggleAutoHide,
        Event::ToggleTopmost,
    ] {
        handle(&state, id, event).unwrap();
    }
    let s = state.borrow();
    let stored = s.store.load_workspace().unwrap();
    let panel = stored.panel(id).unwrap();
    assert_eq!(panel.theme(), desktop_core::PanelTheme::Dark);
    assert_eq!(panel.backdrop(), desktop_core::Backdrop::Acrylic);
    assert_eq!(panel.auto_hide(), !before.auto_hide());
    assert_eq!(panel.always_on_top(), !before.always_on_top());
    let other_stored = stored.panel(PanelId::new(2)).unwrap();
    assert_eq!(other_stored.theme(), desktop_core::PanelTheme::Dark);
    assert_eq!(other_stored.backdrop(), desktop_core::Backdrop::Acrylic);
    assert_eq!(other_stored.auto_hide(), other.auto_hide());
    assert_eq!(other_stored.always_on_top(), other.always_on_top());
    assert_eq!(
        stored.appearance(),
        Some((
            desktop_core::PanelTheme::Dark,
            desktop_core::Backdrop::Acrylic
        ))
    );
}

#[test]
fn panel_menu_appearance_targets_only_its_panel_and_survives_reload() {
    let state = Rc::new(RefCell::new(test_state()));
    let id = PanelId::new(1);
    handle(&state, id, Event::Theme(desktop_core::PanelTheme::Dark)).unwrap();
    handle(&state, id, Event::Material(desktop_core::Backdrop::Mica)).unwrap();
    handle(
        &state,
        id,
        Event::PanelMaterial(desktop_core::Backdrop::Acrylic),
    )
    .unwrap();
    handle(
        &state,
        id,
        Event::PanelTheme(desktop_core::PanelTheme::Light),
    )
    .unwrap();
    let s = state.borrow();
    for workspace in [s.workspace.clone(), s.store.load_workspace().unwrap()] {
        let current = workspace.panel(id).unwrap();
        let other = workspace.panel(PanelId::new(2)).unwrap();
        assert_eq!(current.backdrop(), desktop_core::Backdrop::Acrylic);
        assert_eq!(current.theme(), desktop_core::PanelTheme::Light);
        assert_eq!(other.backdrop(), desktop_core::Backdrop::Mica);
        assert_eq!(other.theme(), desktop_core::PanelTheme::Dark);
        assert_eq!(
            workspace.appearance(),
            Some((desktop_core::PanelTheme::Dark, desktop_core::Backdrop::Mica))
        );
    }
}

#[test]
fn settings_window_applies_clicks_and_closes_without_exiting() {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
    let state = Rc::new(RefCell::new(test_state()));
    settings::show(&state, PanelId::new(1)).unwrap();
    let hwnd = state.borrow().settings.as_ref().unwrap().hwnd().cast();
    unsafe {
        let mut outer = RECT::default();
        let mut client = RECT::default();
        GetWindowRect(hwnd, &raw mut outer);
        GetClientRect(hwnd, &raw mut client);
        let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let mut monitor = windows_sys::Win32::Graphics::Gdi::MONITORINFO {
            cbSize: size_of::<windows_sys::Win32::Graphics::Gdi::MONITORINFO>() as u32,
            ..Default::default()
        };
        windows_sys::Win32::Graphics::Gdi::GetMonitorInfoW(
            windows_sys::Win32::Graphics::Gdi::MonitorFromWindow(
                hwnd,
                windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
            ),
            &raw mut monitor,
        );
        assert_eq!(
            client.right,
            ((900.0 * dpi).round() as i32).min(monitor.rcWork.right - monitor.rcWork.left),
            "initial width must already use DPI before resizing"
        );
        assert_eq!(
            client.bottom,
            ((520.0 * dpi).round() as i32).min(monitor.rcWork.bottom - monitor.rcWork.top),
            "initial height must fit the monitor"
        );
        assert_eq!(
            outer.right - outer.left,
            client.right,
            "no system side frame"
        );
        assert_eq!(
            outer.bottom - outer.top,
            client.bottom,
            "no system caption band"
        );
        SendMessageW(hwnd, WM_SYSCOMMAND, SC_MAXIMIZE as usize, 0);
        assert_ne!(IsZoomed(hwnd), 0);
        SendMessageW(hwnd, WM_SYSCOMMAND, SC_RESTORE as usize, 0);
        assert_eq!(IsZoomed(hwnd), 0);
        {
            // Explorer/Hook can synchronously send these messages while a
            // desktop update holds the mutable workspace borrow.
            let _updating = state.borrow_mut();
            SendMessageW(hwnd, WM_NCHITTEST, 0, 0);
            SendMessageW(hwnd, WM_ACTIVATE, WA_INACTIVE as usize, 0);
            SendMessageW(hwnd, WM_PAINT, 0, 0);
        }
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let mut bounds = RECT::default();
        GetClientRect(hwnd, &raw mut bounds);
        let x = bounds.right - (80.0 * scale) as i32;
        let y = (172.0 * scale) as i32;
        let point = ((y as isize) << 16) | (x as isize & 0xffff);
        SendMessageW(hwnd, WM_LBUTTONDOWN, 1, point);
        SendMessageW(hwnd, WM_LBUTTONUP, 0, point);
        assert_eq!(
            state
                .borrow()
                .store
                .load_workspace()
                .unwrap()
                .panel(PanelId::new(1))
                .unwrap()
                .theme(),
            desktop_core::PanelTheme::Dark
        );
        SendMessageW(hwnd, WM_CLOSE, 0, 0);
        assert_eq!(IsWindow(hwnd), 0);
        assert!(state.borrow().settings.is_none());
        for _ in 0..3 {
            settings::show(&state, PanelId::new(2)).unwrap();
            let reopened = state.borrow().settings.as_ref().unwrap().hwnd().cast();
            assert_ne!(IsWindowVisible(reopened), 0);
            SendMessageW(reopened, WM_KEYDOWN, 0x1b, 0);
            let mut message = MSG::default();
            while PeekMessageW(&raw mut message, reopened, WM_CLOSE, WM_CLOSE, PM_REMOVE) != 0 {
                DispatchMessageW(&raw const message);
            }
            assert_eq!(IsWindow(reopened), 0);
            assert!(state.borrow().settings.is_none());
            assert_eq!(
                PeekMessageW(
                    &raw mut message,
                    std::ptr::null_mut(),
                    WM_QUIT,
                    WM_QUIT,
                    PM_REMOVE
                ),
                0,
                "settings destruction must not quit the app"
            );
        }
    }
}

#[test]
fn reconciliation_preserves_groups_and_appends_new_items_after_existing_order() {
    let mut state = test_state();
    transfer(&mut state, PanelId::new(1), 0, PanelId::new(2), 0).unwrap();
    let mut fresh = state.workspace.desktop_items().to_vec();
    fresh.push(DesktopItem::new(
        ShellIdentity::Namespace {
            parsing_name: "test:D".into(),
        },
        "D",
    ));
    reconcile(&mut state.workspace, fresh);
    assert_eq!(items_for(&state, PanelId::new(2))[0].label, "A");
    assert_eq!(
        items_for(&state, PanelId::new(1))
            .iter()
            .map(|i| i.label.as_str())
            .collect::<Vec<_>>(),
        ["B", "C", "D"]
    );
    let before = state.workspace.clone();
    assert!(transfer(&mut state, PanelId::new(1), 0, PanelId::new(999), 0).is_err());
    assert_eq!(state.workspace, before);
}
