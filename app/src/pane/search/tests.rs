#[test]
fn compact_search_edges_resize_and_icon_drags_at_each_scale() {
    use super::*;
    for scale in [1.0, 1.5, 2.0] {
        let bounds = RECT {
            right: (360.0 * scale) as i32,
            bottom: (TOP * scale) as i32,
            ..Default::default()
        };
        for (x, y, expected) in [
            (2.0, 20.0, HTLEFT),
            (358.0, 20.0, HTRIGHT),
            (24.0, 20.0, HTCAPTION),
            (100.0, 20.0, HTCLIENT),
        ] {
            let point = POINT {
                x: (x * scale) as i32,
                y: (y * scale) as i32,
            };
            assert_eq!(search_frame_hit(bounds, point, scale, false), expected);
        }
        assert_eq!(
            search_frame_hit(
                bounds,
                POINT {
                    x: (24.0 * scale) as i32,
                    y: (20.0 * scale) as i32
                },
                scale,
                true
            ),
            HTCLIENT
        );
    }
}

use super::*;
fn populated_search() -> Search {
    let mut state = Search::new();
    state.change("x".into());
    state.entries = (0..30)
        .map(|i| Entry {
            path: format!("C:\\{i}").into(),
            folder: false,
        })
        .collect();
    state
}
#[test]
fn viewport_fits_screen_edges_and_reduces_rows_at_high_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let mut state = populated_search();
        let work = RECT {
            left: -1280,
            top: -100,
            right: 0,
            bottom: 300,
        };
        let current = RECT {
            left: -300,
            top: 250,
            right: 200,
            bottom: 306,
        };
        state.select(29, false, false);
        let fitted = fit_search(work, current, scale, &mut state);
        assert!(fitted.left >= work.left && fitted.right <= work.right);
        assert!(fitted.top >= work.top && fitted.bottom <= work.bottom);
        assert!(state.visible_rows < VISIBLE);
        assert!(state.scroll <= 29 && state.scroll + state.visible_rows > 29);
        assert!(
            (fitted.bottom - fitted.top) as f32
                >= (TOP + ROW * state.visible_rows as f32) * scale
        );
        state.change(String::new());
        let compact = fit_search(work, fitted, scale, &mut state);
        assert_eq!(compact.bottom - compact.top, (TOP * scale).round() as i32);
    }
}
#[test]
fn keyboard_range_extension_keeps_disjoint_selection_and_ctrl_only_focus() {
    let mut state = populated_search();
    state.select(0, false, false);
    state.select(10, true, false);
    state.move_focus(12, true, true);
    assert_eq!(state.selection, BTreeSet::from([0, 10, 11, 12]));
    state.move_focus(15, true, false);
    assert_eq!(state.focused, Some(15));
    assert_eq!(state.selection, BTreeSet::from([0, 10, 11, 12]));
    state.move_focus(16, false, true);
    assert_eq!(state.selection, BTreeSet::from([15, 16]));
}
#[test]
fn input_colors_survive_owner_callback_reentry() {
    let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
    const REENTER: u32 = WM_APP + 121;
    let owner = windows_window::Window::new("Search color regression")
        .on_message(|raw, msg, wp, _| {
            if msg == REENTER {
                let hwnd = raw.cast();
                return Some(unsafe {
                    SendMessageW(hwnd, WM_CTLCOLOREDIT, wp, edit(hwnd) as isize)
                });
            }
            None
        })
        .create()
        .unwrap();
    let hwnd = owner.hwnd().cast();
    let mut editor = Editor::new(hwnd).unwrap();
    for dark in [true, false] {
        editor.line_height = 1;
        editor.appearance(hwnd, dark);
        unsafe {
            let mut bounds = RECT::default();
            GetWindowRect(editor.hwnd, &raw mut bounds);
            MapWindowPoints(std::ptr::null_mut(), hwnd, (&raw mut bounds).cast(), 2);
            assert_eq!(bounds.bottom - bounds.top, editor.line_height);
            assert!(
                (bounds.top + bounds.bottom - (TOP * scale(hwnd)).round() as i32).abs() <= 1
            );
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            assert!(!dc.is_null());
            SetBkColor(dc, 0xffffff);
            let brush = SendMessageW(hwnd, REENTER, dc as usize, 0);
            assert_ne!(brush, 0);
            assert_eq!(GetBkColor(dc), if dark { 0x202020 } else { 0xf5f5f5 });
            assert_eq!(GetDCBrushColor(dc), GetBkColor(dc));
            DeleteDC(dc);
        }
    }
}
#[test]
#[ignore = "requires Everything and an interactive desktop"]
fn live_compact_query_and_clear() {
    let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
    let app = Rc::new(RefCell::new(super::super::tests::test_state()));
    app.borrow_mut().workspace.set_appearance(
        desktop_core::PanelTheme::Dark,
        desktop_core::Backdrop::Translucent { opacity: 1.0 },
    );
    super::super::handle(&app, desktop_core::PanelId::new(0), Event::EnableSearch).unwrap();
    let hwnd = app.borrow().views[0].window.hwnd().cast();
    unsafe {
        let mut key = 0;
        let mut alpha = 0;
        let mut flags = 0;
        assert_ne!(
            GetLayeredWindowAttributes(
                edit(hwnd),
                &raw mut key,
                &raw mut alpha,
                &raw mut flags
            ),
            0
        );
        assert_eq!(key, 0x202020);
        assert_eq!(flags, LWA_COLORKEY);
        SetFocus(hwnd);
        let x = (80.0 * scale(hwnd)) as isize;
        let y = (28.0 * scale(hwnd)) as isize;
        SendMessageW(hwnd, WM_LBUTTONDOWN, 1, x | (y << 16));
        assert_eq!(GetFocus(), edit(hwnd));
        assert_eq!(GetCapture(), edit(hwnd));
        SendMessageW(edit(hwnd), WM_LBUTTONUP, 0, 0);
    }
    // Widths outside the old 320..640 range survive resize and persistence.
    for width in [280.0, 720.0] {
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                (width * scale(hwnd)) as i32,
                (TOP * scale(hwnd)) as i32,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            SendMessageW(hwnd, WM_EXITSIZEMOVE, 0, 0);
        }
        let saved = app.borrow().store.load_workspace().unwrap();
        let panel = saved.panels().iter().find(|p| p.is_search()).unwrap();
        assert!((panel.rect().width - width).abs() < 1.0);
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let mut monitor = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        assert_ne!(
            GetMonitorInfoW(
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                &raw mut monitor
            ),
            0
        );
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            monitor.rcWork.left + 40,
            monitor.rcWork.bottom - (TOP * scale(hwnd)) as i32,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    let query = wide(&format!("path:\"{}\" Cargo.toml", root.display()));
    unsafe {
        SetWindowTextW(edit(hwnd), query.as_ptr());
    }
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut bounds = RECT::default();
    loop {
        windows_window::pump();
        unsafe {
            GetWindowRect(hwnd, &raw mut bounds);
        }
        if (bounds.bottom - bounds.top) as f32 / scale(hwnd) > TOP + 64.0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "search did not expand to results"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    unsafe {
        assert!(bounds.top >= monitor.rcWork.top && bounds.bottom <= monitor.rcWork.bottom);
        let mut editor_bounds = RECT::default();
        GetWindowRect(edit(hwnd), &raw mut editor_bounds);
        assert!(
            (editor_bounds.top + editor_bounds.bottom
                - 2 * bounds.top
                - (TOP * scale(hwnd)).round() as i32)
                .abs()
                <= 1
        );
        SetWindowTextW(edit(hwnd), windows_sys::w!(""));
    }
    windows_window::pump();
    unsafe {
        GetWindowRect(hwnd, &raw mut bounds);
    }
    assert_eq!(bounds.bottom - bounds.top, (TOP * scale(hwnd)) as i32);
}

#[test]
fn compact_render_keeps_rows_and_border_inside_the_pane() {
    let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
    let app = Rc::new(RefCell::new(super::super::tests::test_state()));
    app.borrow_mut().workspace.set_appearance(
        desktop_core::PanelTheme::Dark,
        desktop_core::Backdrop::Translucent { opacity: 1.0 },
    );
    super::super::handle(&app, desktop_core::PanelId::new(0), Event::EnableSearch).unwrap();
    let mut model = app.borrow().views[0].model.borrow().clone();
    let fixture = windows_window::Window::new("Search render fixture")
        .style(WS_POPUP)
        .size(480, 180)
        .on_message(|_, message, _, _| if message == WM_DESTROY { Some(0) } else { None })
        .create()
        .unwrap();
    let hwnd = fixture.hwnd().cast();

    model.dark = true;
    model.backdrop = desktop_core::Backdrop::Translucent { opacity: 1.0 };
    let mut state = Search::new();
    state.change("LucidPane".into());
    state.entries = vec![
        Entry {
            path: r"E:\Project\LucidPane\README.md".into(),
            folder: false,
        },
        Entry {
            path: r"E:\Project\LucidPane\docs".into(),
            folder: true,
        },
    ];
    state.select(0, false, false);
    resize(hwnd, &mut state);
    let mut drawing = Drawing::new(hwnd).unwrap();
    drawing.paint(hwnd, &model, &state).unwrap();
    let pixels = drawing.surface.readback().unwrap();
    let mut r = RECT::default();
    unsafe {
        GetClientRect(hwnd, &raw mut r);
    }
    assert_eq!(pixels.len(), (r.right * r.bottom * 4) as usize);
    assert!(
        pixels
            .chunks_exact(4)
            .any(|p| p[0] > 180 && p[1] > 180 && p[2] > 180 && p[3] > 200)
    );
    if let Some(dir) = std::env::var_os("LUCIDPANE_RENDER_OUTPUT") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("search.bgra"), pixels).unwrap();
        std::fs::write(
            dir.join("search-size.txt"),
            format!("{} {}", r.right, r.bottom),
        )
        .unwrap();
    }
}
#[test]
fn empty_query_collapses_and_rejects_pending_results() {
    let mut state = Search::new();
    assert_eq!(state.height(), TOP);
    assert!(state.due.is_none());
    state.change("example".into());
    let old = state.generation;
    assert!(state.height() > TOP);
    state.change(" ".into());
    assert_eq!(state.height(), TOP);
    assert!(state.due.is_none());
    assert!(!state.accept(
        old,
        Ok(Page {
            offset: 0,
            total: 1,
            entries: vec![Entry {
                path: "C:\\x".into(),
                folder: false,
            }]
        })
    ));
    assert!(state.entries.is_empty());
}
#[test]
fn results_expand_to_eight_rows_and_selection_scrolls() {
    let mut state = Search::new();
    state.change("x".into());
    let entries = (0..20)
        .map(|i| Entry {
            path: format!("C:\\{i}").into(),
            folder: false,
        })
        .collect();
    assert!(state.accept(
        state.generation,
        Ok(Page {
            offset: 0,
            total: 20,
            entries
        })
    ));
    assert_eq!(state.height(), TOP + ROW * VISIBLE as f32 + 12.0);
    state.select(0, false, false);
    state.select(10, false, true);
    assert_eq!(state.selection.len(), 11);
    assert_eq!(state.scroll, 3);
}

#[test]
fn clearing_search_releases_large_result_buffer_and_rejects_stale_pages() {
    let mut state = populated_search();
    state.entries.reserve(100_000);
    let previous = state.generation;
    assert!(state.entries.capacity() >= 100_000);
    state.change(String::new());
    assert_eq!(state.entries.capacity(), 0);
    assert!(!state.accept(previous, Ok(Page {
        total: 1, offset: 0,
        entries: vec![Entry { path: r"C:\stale".into(), folder: false }],
    })));
    assert!(state.entries.is_empty());
}
