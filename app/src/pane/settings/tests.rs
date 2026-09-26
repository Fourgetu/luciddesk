use super::*;

#[test]
fn backup_in_progress_preserves_policy_controls_and_prevents_duplicate_jobs() {
    for enabled in [false, true] {
        let mut body=scene(800.0,MIN_HEIGHT-TITLE_HEIGHT,6,true,
            (PanelTheme::System,Backdrop::Mica),desktop_core::PaneOptions::default());

        let view=recovery::View {busy:true,..Default::default()};
        layout::backup_page(&mut body,800.0,&view,recovery::Policy {enabled,..Default::default()},false);
        let toggle=body.controls.iter().find(|c|matches!(c.action,Action::BackupPolicy(0))).unwrap();
        assert!(toggle.enabled);
        assert_eq!(toggle.selected,enabled);
        assert!(body.controls.iter().filter(|c|matches!(c.action,Action::BackupPolicy(_))).all(|c|c.enabled));
        assert!(!body.controls.iter().find(|c|matches!(c.action,Action::Change(Event::CreateBackup))).unwrap().enabled);
    }
}

#[test]
fn folder_modes_fit_minimum_window_and_show_current_choice() {
    for mode in [folder::EntryMode::Inline, folder::EntryMode::Explorer] {
        let mut body = scene(800.0, MIN_HEIGHT - TITLE_HEIGHT, 8, false,
            (PanelTheme::System, Backdrop::Mica), desktop_core::PaneOptions::default());
        layout::folder_defaults(&mut body, 800.0, folder::Defaults::default(), mode);
        let s = with_titlebar(body, 800.0, false);
        for bounds in s.text.iter().map(|(r,_,_)| r).chain(s.controls.iter().map(|c| &c.bounds)) {
            assert!(bounds.right <= 800.0 && bounds.bottom <= MIN_HEIGHT);
        }
        let choices: Vec<_> = s.controls.iter().filter(|c| matches!(c.action, Action::FolderEntryMode(_))).collect();
        assert_eq!(choices.len(), 2);
        assert_eq!(choices.iter().filter(|c| c.selected).count(), 1);
        assert!(choices.iter().any(|c| c.selected && matches!(c.action, Action::FolderEntryMode(value) if value == mode)));
    }
}

#[test]
fn folder_defaults_are_saved_and_only_copied_into_new_panels() {
    let store = WorkspaceStore::open_in_memory().unwrap();
    assert_eq!(folder::Defaults::load(&store).unwrap(), folder::Defaults::default());
    let mut first = Panel::new(PanelId::new(10), "first", desktop_core::RectDip::new(0.0, 0.0, 480.0, 360.0));
    first.set_folder(Some(std::path::PathBuf::from(r"C:\first")));
    folder::Defaults::load(&store).unwrap().apply(&store, &mut first).unwrap();
    folder::Defaults { list: false, columns: 8 }.save(&store).unwrap();
    let saved = folder::Defaults::load(&store).unwrap();
    assert_eq!(saved.columns, 9);
    let mut second = Panel::new(PanelId::new(11), "second", first.rect());
    second.set_folder(Some(std::path::PathBuf::from(r"C:\second")));
    saved.apply(&store, &mut second).unwrap();
    assert!(!second.list_view());
    assert_eq!(folder::visible_columns(&store, second.id()).unwrap(), 9);
    folder::Defaults::default().save(&store).unwrap();
    assert!(first.list_view());
    assert_eq!(folder::visible_columns(&store, first.id()).unwrap(), 15);
    assert!(!second.list_view());
    assert_eq!(folder::visible_columns(&store, second.id()).unwrap(), 9);
}

#[test]
fn grid_slider_centers_default_and_scales_in_both_directions() {
    assert_eq!(grid_slider_position(100.0), 0.5);
    assert_eq!(grid_slider_value(0.5), 100.0);
    assert_eq!(grid_slider_value(0.0), grid_range().0);
    assert_eq!(grid_slider_value(1.0), grid_range().1);
    for value in 50..=200 {
        assert_eq!(grid_slider_value(grid_slider_position(value as f32)), value as f32);
    }
    assert!(grid_slider_value(0.25) < 100.0);
    assert!(grid_slider_value(0.75) > 100.0);
}

#[test]
fn radius_drag_preserves_fractional_values() {
    let bounds = Rect::from_xywh(0.0, 0.0, 180.0, 34.0);
    let first = radius_from_pointer(bounds, 80.0);
    let next = radius_from_pointer(bounds, 81.0);
    assert!(first.fract() != 0.0);
    assert!(next > first && next - first < 1.0);
    assert_eq!(radius_from_pointer(bounds, -10.0), 0.0);
    assert_eq!(radius_from_pointer(bounds, 190.0), 24.0);
}

#[test]
fn initial_library_show_remains_hidden_until_prepared() {
    let prepared = Rc::new(std::cell::Cell::new(false));
    let callback_prepared = Rc::clone(&prepared);
    let window = windows_window::Window::new("LucidPane initial visibility test")
        .style(WS_OVERLAPPEDWINDOW)
        .on_message(move |_, msg, _, lp| {
            if unsafe { defer_show(msg, lp, callback_prepared.get()) }
                || msg == WM_DESTROY
            {
                Some(0)
            } else {
                None
            }
        })
        .create()
        .unwrap();
    unsafe {
        assert_eq!(IsWindowVisible(window.hwnd().cast()), 0);
        prepared.set(true);
        ShowWindow(window.hwnd().cast(), SW_SHOWNOACTIVATE);
        assert_ne!(IsWindowVisible(window.hwnd().cast()), 0);
    }
}

#[test]
fn settings_opacity_is_independent_and_rgb_preserves_other_channels() {
    for dark in [false, true] {
        for color in [0x123456, 0x7d4441, 0xffffff] {
            for opacity in [0.0, 0.5, 1.0] {
                assert_eq!(
                    settings_backdrop(Backdrop::Solid { color, opacity }, dark),
                    Backdrop::Solid {
                        color: if dark { 0x202020 } else { 0xf3f3f3 },
                        opacity: 1.0
                    }
                );
            }
        }
    }
    assert_eq!(
        settings_backdrop(Backdrop::Acrylic, true),
        Backdrop::Acrylic
    );
    for base in [Backdrop::Acrylic, Backdrop::Mica, Backdrop::MicaAlt] {
        for strength in [0, 50, 100] {
            assert_eq!(settings_backdrop(base.with_strength(strength), true), base);
        }
    }
    assert_eq!(color_channel(0x123456, 0, 255), 0xff3456);
    assert_eq!(color_channel(0x123456, 1, 0), 0x120056);
    assert_eq!(color_channel(0x123456, 2, 255), 0x1234ff);
    let picker = with_titlebar(
        scene(
            800.0,
            480.0 - TITLE_HEIGHT,
            7,
            false,
            (
                PanelTheme::Dark,
                Backdrop::Solid {
                    color: 0x123456,
                    opacity: 0.5,
                },
            ),
            Default::default(),
        ),
        800.0,
        false,
    );
    assert!(
        picker
            .controls
            .iter()
            .all(|c| c.bounds.right <= 800.0 && c.bounds.bottom <= 480.0)
    );
    assert_eq!(
        picker
            .controls
            .iter()
            .filter(|c| matches!(c.action, Action::Channel(_, _)))
            .count(),
        3
    );
    assert_eq!(picker.previews.len(), 1);
}

#[test]
fn solid_controls_fit_minimum_settings_size() {
    let s = with_titlebar(
        scene(
            800.0,
            480.0 - TITLE_HEIGHT,
            0,
            false,
            (
                PanelTheme::Dark,
                Backdrop::Solid {
                    color: 0x24364b,
                    opacity: 0.5,
                },
            ),
            Default::default(),
        ),
        800.0,
        false,
    );
    for control in &s.controls {
        assert!(control.bounds.right <= 800.0 && control.bounds.bottom <= 480.0);
    }
    assert!(
        s.controls
            .iter()
            .any(|c| matches!(c.action, Action::Opacity(50)))
    );
}

#[test]
fn panel_options_text_fits_default_and_minimum_window() {
    for (width, height) in [(800.0, MIN_HEIGHT), (900.0, DEFAULT_HEIGHT as f32)] {
        let s = with_titlebar(
            scene(
                width,
                height - TITLE_HEIGHT,
                1,
                false,
                (PanelTheme::Dark, Backdrop::Mica),
                Default::default(),
            ),
            width,
            false,
        );
        for (bounds, text, _) in &s.text {
            assert!(bounds.bottom <= height - 16.0, "Clipped text: {text}");
            assert!(bounds.right <= width, "Clipped text: {text}");
        }
        for control in &s.controls {
            assert!(control.bounds.bottom <= height - 16.0);
        }
    }
}

#[test]
fn solid_inputs_validate_color_and_opacity_without_changing_other_channels() {
    let solid = Backdrop::Solid {
        color: 0x123456,
        opacity: 0.85,
    };
    assert_eq!(
        edited_solid(solid, false, "#A1b2C3"),
        Some(Backdrop::Solid {
            color: 0xa1b2c3,
            opacity: 0.85
        })
    );
    for value in ["0", "50%", "100"] {
        assert!(edited_solid(solid, true, value).is_some());
    }
    for value in ["101", "-1", "NaN", ""] {
        assert!(edited_solid(solid, true, value).is_none());
    }
    for value in ["123", "GG0000", "1234567"] {
        assert!(edited_solid(solid, false, value).is_none());
    }
}

#[test]
fn switch_thumb_stays_centered_with_equal_end_insets() {
    for (width, height) in [(42.0, 22.0), (48.0, 24.0)] {
        let bounds = Rect::from_xywh(100.0, 50.0, width, height);
        for position in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let thumb = toggle_thumb(bounds, position);
            assert_eq!(thumb.center.y, 50.0 + height / 2.0);
            assert!(thumb.center.x - thumb.radius_x >= bounds.left + 4.0);
            assert!(thumb.center.x + thumb.radius_x <= bounds.right - 4.0);
        }
        assert_eq!(
            toggle_thumb(bounds, 0.0).center.x - toggle_thumb(bounds, 0.0).radius_x,
            bounds.left + 4.0
        );
        assert_eq!(
            toggle_thumb(bounds, 1.0).center.x + toggle_thumb(bounds, 1.0).radius_x,
            bounds.right - 4.0
        );
    }
}

#[test]
fn switch_motion_reverses_continuously_and_respects_disabled_animation() {
    let now = std::time::Instant::now();
    let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
    let mut motion = ToggleMotion::settled(0.0, now);
    assert_eq!(motion.retarget(1.0, now, true), 0.0);
    let halfway = now + std::time::Duration::from_millis(80);
    let value = motion.sample(halfway).unwrap();
    assert!(value > 0.0 && value < 1.0);
    assert_eq!(motion.retarget(0.0, halfway, true), value);
    assert_eq!(
        motion
            .sample(halfway + std::time::Duration::from_millis(160))
            .unwrap(),
        0.0
    );
    assert_eq!(motion.retarget(1.0, halfway, false), 1.0);
}

#[test]
fn custom_frame_keeps_caption_buttons_and_resize_edges_separate() {
    assert_eq!(frame_hit(80.0, 16.0, 1040.0, 760.0, false), HTCAPTION);
    assert_eq!(frame_hit(1020.0, 16.0, 1040.0, 760.0, false), HTCLIENT);
    assert_eq!(frame_hit(2.0, 2.0, 1040.0, 760.0, false), HTTOPLEFT);
    assert_eq!(
        frame_hit(1038.0, 758.0, 1040.0, 760.0, false),
        HTBOTTOMRIGHT
    );
    assert_eq!(frame_hit(80.0, 2.0, 1040.0, 760.0, true), HTCAPTION);
    let s = with_titlebar(
        scene(
            1040.0,
            728.0,
            0,
            false,
            (PanelTheme::Dark, Backdrop::Mica),
            desktop_core::PaneOptions::default(),
        ),
        1040.0,
        false,
    );
    assert_eq!(
        s.controls
            .iter()
            .filter(|c| matches!(c.action, Action::Window(_)))
            .count(),
        3
    );
}
#[test]
fn about_page_fits_minimum_window() {
    let width = 800.0;
    let height = MIN_HEIGHT;
    let mut body = scene(width, height - TITLE_HEIGHT, 5, true,
        (PanelTheme::Dark, Backdrop::Mica), desktop_core::PaneOptions::default());
    layout::about_status(&mut body, width, "桌面分组已连接", true);
    let s = with_titlebar(body, width, false);
    for bounds in s.text.iter().map(|(bounds, _, _)| bounds).chain(s.app_icon.iter())
        .chain(s.cards.iter()).chain(s.controls.iter().map(|c| &c.bounds)) {
        assert!(bounds.left >= 0.0 && bounds.top >= 0.0
            && bounds.right <= width && bounds.bottom <= height);
    }
}

#[test]
fn backup_config_controls_fit_minimum_window() {
    let mut body=scene(800.0,MIN_HEIGHT-TITLE_HEIGHT,6,true,
        (PanelTheme::System,Backdrop::Mica),desktop_core::PaneOptions::default());

    layout::backup_page(&mut body,800.0,&recovery::View::default(),recovery::Policy::default(),true);
    let s=with_titlebar(body,800.0,false);
    for bounds in s.text.iter().map(|(r,_,_)|r).chain(s.controls.iter().map(|c|&c.bounds)) {
        assert!(bounds.right<=800.0 && bounds.bottom<=MIN_HEIGHT);
    }
    assert!(s.controls.iter().any(|c|matches!(c.action,Action::Change(Event::ReloadConfig))));
}

#[test]
fn settings_layout_and_rendering_at_multiple_scales() {
    let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
    let painter = Painter::new().unwrap();
    let export_snapshots = std::env::var_os("LUCIDPANE_TEST_EXPORT_SNAPSHOTS").is_some();
    {
        let device = windows_canvas::GpuDevice::new_warp().unwrap();
        for scale in [1.0, 1.5, 2.0] {
            for page in [0, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11] {
                for dark in [false, true] {
                    let mut body = scene(
                            940.0,
                            620.0 - TITLE_HEIGHT,
                            page,
                            true,
                            (
                                PanelTheme::System,
                                if page == 0 {
                                    Backdrop::Acrylic.with_strength(65)
                                } else if page == 7 {
                                    Backdrop::Solid {
                                        color: 0x24364b,
                                        opacity: 0.85,
                                    }
                                } else {
                                    Backdrop::Mica
                                },
                            ),
                            desktop_core::PaneOptions::default(),
                        );
                    if matches!(page,6|9|10) {

                        let view=recovery::View {status:"手动备份成功 · 上次备份：今天 14:32".into(),records:(0..5).map(|i|recovery::Record {
                            path:std::path::PathBuf::from(format!("backup-{i}.db")),date:"2026/09/14 14:32".into(),kind:if i==0{"手动"}else{"自动"},bytes:131072,
                        }).collect(),..Default::default()};
                        if page==9 {layout::backup_history(&mut body,940.0,&view,0);}else{layout::backup_page(&mut body,940.0,&view,recovery::Policy::default(),page==10);}
                    }
                    if page == 11 { layout::fonts(&mut body, 940.0, &fonts::installed(), 0); }
                    if page == 8 { layout::folder_defaults(&mut body, 940.0, folder::Defaults::default(), if dark { folder::EntryMode::Explorer } else { folder::EntryMode::Inline }); }
                    if page == 5 {
                        layout::about_status(&mut body, 940.0, "桌面分组已连接", false);
                    }
                    let s = with_titlebar(body, 940.0, false);
                    for c in &s.controls {
                        assert!(
                            c.bounds.left >= 0.0
                                && c.bounds.top >= 0.0
                                && c.bounds.right <= 940.0
                                && c.bounds.bottom <= 620.0
                        );
                        assert!(contains(
                            &c.bounds,
                            (c.bounds.left + c.bounds.right) / 2.0,
                            (c.bounds.top + c.bounds.bottom) / 2.0
                        ));
                    }
                    let width = (940.0 * scale) as u32;
                    let height = (620.0 * scale) as u32;
                    let bitmap =
                        super::super::canvas::Offscreen::new(&device, width, height).unwrap();
                    let target = bitmap.target.clone();
                    painter
                        .paint(
                            &target,
                            &s,
                            940.0,
                            620.0,
                            scale,
                            dark,
                            false,
                            None,
                            None,
                            &std::collections::HashMap::new(),
                        )
                        .unwrap();
                    let pixels = bitmap.pixels().unwrap();
                    assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
                    assert_eq!(pixels[0] < 128, dark);
                    if export_snapshots && scale == 1.0 {
                        // Standalone raster for visual review, independent of the live desktop.
                        let mut bmp = vec![0u8; 54];
                        bmp[0..2].copy_from_slice(b"BM");
                        bmp[2..6].copy_from_slice(&(54 + pixels.len() as u32).to_le_bytes());
                        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
                        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
                        bmp[18..22].copy_from_slice(&(width as i32).to_le_bytes());
                        bmp[22..26].copy_from_slice(&(-(height as i32)).to_le_bytes());
                        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
                        bmp[28..30].copy_from_slice(&32u16.to_le_bytes());
                        bmp.extend(pixels);
                        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("../target")
                            .join(match (page, dark) {
                                (11, true) => "settings-fonts-dark.bmp",
                                (11, false) => "settings-fonts-light.bmp",
                                (8, true) => "settings-folder-dark.bmp",
                                (8, false) => "settings-folder-light.bmp",
                                (3, true) => "settings-peek-dark.bmp",
                                (3, false) => "settings-peek-light.bmp",
                                (4, true) => "settings-search-dark.bmp",
                                (4, false) => "settings-search-light.bmp",
                                (9, true) => "settings-backup-history-dark.bmp",
                                (9, false) => "settings-backup-history-light.bmp",
                                (10, true) => "settings-backup-advanced-dark.bmp",
                                (10, false) => "settings-backup-advanced-light.bmp",
                                (6, true) => "settings-backup-dark.bmp",
                                (6, false) => "settings-backup-light.bmp",
                                (7, true) => "settings-colors-dark.bmp",
                                (7, false) => "settings-colors-light.bmp",
                                (5, true) => "settings-about-dark.bmp",
                                (5, false) => "settings-about-light.bmp",
                                (1, true) => "settings-pane-dark.bmp",
                                (1, false) => "settings-pane-light.bmp",
                                (_, true) => "settings-dark.bmp",
                                (_, false) => "settings-light.bmp",
                            });
                        std::fs::write(path, bmp).unwrap();
                    }
                    if page == 0 {
                        painter
                            .paint(
                                &target,
                                &s,
                                940.0,
                                620.0,
                                scale,
                                dark,
                                true,
                                None,
                                None,
                                &std::collections::HashMap::new(),
                            )
                            .unwrap();
                        let overlay = bitmap.pixels().unwrap();
                        assert_eq!(
                            overlay[3], 0,
                            "the native material must remain visible beneath the sidebar"
                        );
                        let card_at = (((190.0 * scale) as u32 * width
                            + (270.0 * scale) as u32)
                            * 4) as usize;
                        assert!(
                            overlay[card_at + 3] > 0 && overlay[card_at + 3] < 255,
                            "settings content must retain material transparency"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn font_picker_fits_minimum_window_and_exposes_paging_and_reset() {
    let names: Vec<_> = (0..19).map(|i| format!("Font {i}")).collect();
    for offset in [0, 7, 14] {
        let mut body = scene(800.0, MIN_HEIGHT - TITLE_HEIGHT, 11, false,
            (PanelTheme::Dark, Backdrop::Mica), desktop_core::PaneOptions::default());
        layout::fonts(&mut body, 800.0, &names, offset);
        let s = with_titlebar(body, 800.0, false);
        for r in s.text.iter().map(|(r,_,_)| r).chain(s.controls.iter().map(|c| &c.bounds)) {
            assert!(r.right <= 800.0 && r.bottom <= MIN_HEIGHT);
        }
        assert_eq!(s.controls.iter().filter(|c| matches!(&c.action, Action::Font(name) if name.starts_with("Font "))).count(), (names.len() - offset).min(7));
        assert!(s.controls.iter().any(|c| matches!(&c.action, Action::Font(name) if name == assets::UI_FONT)));
    }
}
