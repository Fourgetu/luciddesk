use super::*;

pub(super) fn folder_defaults(s: &mut Scene, width: f32, value: folder::Defaults) {
    let x = 248.0;
    let w = width - x - 24.0;
    s.text(Rect::from_xywh(x, 80.0, w, 28.0), "仅用于新建文件夹面板，已有面板保持原样。", 0);
    s.text(Rect::from_xywh(x, 130.0, w - 200.0, 34.0), "默认视图", 1);
    for (i, (label, list)) in [("图标", false), ("列表", true)].into_iter().enumerate() {
        s.button(Rect::from_xywh(x + w - 184.0 + i as f32 * 96.0,
            130.0, 88.0, 34.0), label,
            Action::FolderDefaults(folder::Defaults { list, ..value }), value.list == list);
    }
    s.separators.push(Rect::from_xywh(x, 184.0, w, 1.0));
    s.text(Rect::from_xywh(x, 202.0, w, 28.0), "列表默认显示列", 1);
    s.text(Rect::from_xywh(x, 232.0, w, 24.0), "名称始终显示；切换到列表视图时生效。", 0);
    for (i, (label, column)) in [("类型", 1), ("修改时间", 2), ("大小", 3)].into_iter().enumerate() {
        let y = 270.0 + i as f32 * 42.0;
        s.text(Rect::from_xywh(x, y, w - 70.0, 28.0), label, 1);
        s.controls.push(Control {
            bounds: Rect::from_xywh(x + w - 46.0, y + 2.0, 46.0, 24.0),
            label: String::new(),
            action: Action::FolderDefaults(folder::Defaults { columns: value.columns ^ (1 << column), ..value }),
            selected: value.columns & (1 << column) != 0,
            enabled: true, toggle: true,
        });
    }
    s.button(Rect::from_xywh(x, 404.0, 112.0, 34.0), "恢复默认",
        Action::FolderDefaults(folder::Defaults::default()), false);
}

// Page IDs stay stable; display order is independent of routing.
const PAGES: [(usize, &str, &str); 7] = [
    (0, "主题与材质", "\u{e790}"),
    (1, "面板布局", "\u{f0e2}"),
    (8, "文件夹面板", "\u{e8b7}"),
    (4, "Everything 搜索", "\u{e721}"),
    (3, "文件预览", "\u{e890}"),
    (6, "备份与恢复", "\u{e81c}"),
    (5, "关于", "\u{e946}"),
];

pub(super) fn scene(
    width: f32,
    _height: f32,
    page: usize,
    search_enabled: bool,
    appearance: (PanelTheme, Backdrop),
    options: desktop_core::PaneOptions,
) -> Scene {
    let mut s = Scene {
        text: vec![],
        cards: vec![],
        separators: vec![],
        controls: vec![],
        previews: vec![],
        app_icon: None,
    };
    s.text(Rect::from_xywh(28.0, 26.0, 172.0, 32.0), "LucidPane", 2);
    for (position, &(id, name, icon)) in PAGES.iter().enumerate() {
        // Keep spacing tied to semantic groups, not insertion positions.
        let gap = match id {
            4 | 3 => 10.0,
            6 | 5 => 20.0,
            _ => 0.0,
        };
        let y = 78.0 + position as f32 * 42.0 + gap;
        s.button(
            Rect::from_xywh(12.0, y, 200.0, 38.0),
            name,
            Action::Page(id),
            page == id || (page == 7 && id == 0) || (matches!(page,9|10) && id==6),
        );
        s.text(Rect::from_xywh(30.0, y, 22.0, 38.0), icon, 5);
    }
    let x = 248.0;
    let w = width - x - 24.0;
    s.text(
        Rect::from_xywh(x, 28.0, w, 44.0),
        PAGES
            .iter()
            .find(|(id, _, _)| *id == page)
            .map_or(match page {9=>"管理备份",10=>"高级选项",_=>"配色"}, |(_, name, _)| *name),
        3,
    );
    if page == 7 {
        let Backdrop::Solid { color, opacity } = appearance.1 else {
            return s;
        };
        s.button(
            Rect::from_xywh(x + w - 86.0, 34.0, 86.0, 32.0),
            "返回",
            Action::Page(0),
            false,
        );
        let pw = 184.0;
        s.cards.push(Rect::from_xywh(x, 116.0, pw, 120.0));
        s.previews.push((
            Rect::from_xywh(x + 12.0, 128.0, pw - 24.0, 72.0),
            color,
            opacity,
        ));
        s.text(
            Rect::from_xywh(x + 14.0, 206.0, pw - 28.0, 22.0),
            format!("面板预览 · {}%", (opacity * 100.0).round() as u8),
            0,
        );
        s.separators.push(Rect::from_xywh(x, 244.0, w, 1.0));
        let px = x + pw + 20.0;
        let palette_width = w - pw - 20.0;
        s.text(
            Rect::from_xywh(px, 116.0, palette_width, 24.0),
            "预设配色",
            1,
        );
        for (i, value) in [
            0x181b20, 0xf5f6f8, 0x24364b, 0x32463d, 0x51405c, 0x5b3838, 0x745839, 0x416c78,
        ]
        .into_iter()
        .enumerate()
        {
            let cw = (palette_width - 24.0) / 4.0;
            s.button(
                Rect::from_xywh(
                    px + (i % 4) as f32 * (cw + 8.0),
                    148.0 + (i / 4) as f32 * 46.0,
                    cw,
                    36.0,
                ),
                "",
                Action::ColorPreset(value),
                color == value,
            );
        }
        for (channel, name) in ["红 R", "绿 G", "蓝 B"].into_iter().enumerate() {
            let value = ((color >> ((2 - channel) * 8)) & 255) as u8;
            let y = 258.0 + channel as f32 * 40.0;
            s.text(Rect::from_xywh(x + 16.0, y, 48.0, 30.0), name, 0);
            s.controls.push(Control {
                bounds: Rect::from_xywh(x + 74.0, y, w - 150.0, 30.0),
                label: String::new(),
                action: Action::Channel(channel as u8, value),
                selected: false,
                enabled: true,
                toggle: false,
            });
            s.text(
                Rect::from_xywh(x + w - 52.0, y, 40.0, 30.0),
                value.to_string(),
                0,
            );
        }
        s.text(Rect::from_xywh(x, 396.0, 52.0, 32.0), "HEX", 0);
        s.button(
            Rect::from_xywh(x + 54.0, 396.0, 126.0, 32.0),
            &format!("#{color:06X}"),
            Action::StyleInput(false),
            false,
        );
        s.text(
            Rect::from_xywh(x + 192.0, 396.0, w - 304.0, 32.0),
            "Enter 确认",
            0,
        );
        s.button(
            Rect::from_xywh(x + w - 104.0, 396.0, 104.0, 32.0),
            "恢复默认",
            Action::SolidReset,
            false,
        );
        return s;
    }
    if page == 0 {
        let stacked = w < 470.0;
        let extra = if stacked { 44.0 } else { 0.0 };
        s.text(Rect::from_xywh(x + 16.0, 124.0, 28.0, 32.0), "\u{e793}", 4);
        s.text(Rect::from_xywh(x + 54.0, 124.0, 120.0, 32.0), "应用主题", 1);
        let bw = 86.0;
        for (j, (name, value)) in [
            ("跟随系统", PanelTheme::System),
            ("浅色", PanelTheme::Light),
            ("深色", PanelTheme::Dark),
        ]
        .iter()
        .enumerate()
        {
            s.button(
                Rect::from_xywh(
                    if stacked { x + 16.0 } else { x + w - 282.0 } + j as f32 * 90.0,
                    if stacked { 164.0 } else { 123.0 },
                    bw,
                    34.0,
                ),
                name,
                Action::Change(Event::Theme(*value)),
                appearance.0 == *value,
            );
        }
        s.text(Rect::from_xywh(x, 192.0 + extra, w, 28.0), "窗口材质", 1);
        let solid = if matches!(appearance.1, Backdrop::Solid { .. }) {
            appearance.1
        } else {
            Backdrop::Solid {
                color: if theme::is_dark(appearance.0) {
                    0x181b20
                } else {
                    0xf5f6f8
                },
                opacity: 0.85,
            }
        };
        let bw = (w - 36.0) / 4.0;
        for (j, (name, value)) in [
            ("亚克力", Backdrop::Acrylic),
            ("Mica", Backdrop::Mica),
            ("Mica Alt", Backdrop::MicaAlt),
            ("纯色", solid),
        ]
        .iter()
        .enumerate()
        {
            s.button(
                Rect::from_xywh(x + j as f32 * (bw + 12.0), 228.0 + extra, bw, 108.0),
                name,
                Action::Change(Event::Material(*value)),
                appearance.1.kind() == value.kind(),
            );
        }
        if let Backdrop::Solid { color, opacity } = appearance.1 {
            let y = 364.0 + extra;
            s.previews
                .push((Rect::from_xywh(x + 14.0, y, 30.0, 30.0), color, opacity));
            s.button(
                Rect::from_xywh(x + 54.0, y, 112.0, 30.0),
                &format!("#{color:06X}"),
                Action::StyleInput(false),
                false,
            );
            s.button(
                Rect::from_xywh(x + 178.0, y, 100.0, 30.0),
                "编辑配色",
                Action::SolidColor,
                false,
            );
            s.button(
                Rect::from_xywh(x + w - 114.0, y, 100.0, 30.0),
                "恢复默认",
                Action::SolidReset,
                false,
            );
            let value = (opacity * 100.0).round() as u8;
            s.text(
                Rect::from_xywh(x + 14.0, y + 42.0, 100.0, 30.0),
                "面板不透明度",
                0,
            );
            s.controls.push(Control {
                bounds: Rect::from_xywh(x + 124.0, y + 42.0, w - 224.0, 30.0),
                label: String::new(),
                action: Action::Opacity(value),
                selected: false,
                enabled: true,
                toggle: false,
            });
            s.button(
                Rect::from_xywh(x + w - 86.0, y + 42.0, 72.0, 30.0),
                &format!("{value}%"),
                Action::StyleInput(true),
                false,
            );
        }
        if let Some(value) = appearance.1.strength() {
            let y = 364.0 + extra;
            s.text(Rect::from_xywh(x + 14.0, y, 100.0, 30.0), "效果强度", 0);
            s.controls.push(Control {
                bounds: Rect::from_xywh(x + 124.0, y, w - 224.0, 30.0),
                label: String::new(),
                action: Action::Strength(value),
                selected: false,
                enabled: true,
                toggle: false,
            });
            s.text(
                Rect::from_xywh(x + w - 72.0, y, 58.0, 30.0),
                if value == 50 {
                    "默认".to_string()
                } else {
                    format!("{:+}", i16::from(value) - 50)
                },
                1,
            );
            s.text(
                Rect::from_xywh(x + 14.0, y + 42.0, w - 142.0, 30.0),
                "通透 ↔ 厚实",
                0,
            );
            s.button(
                Rect::from_xywh(x + w - 114.0, y + 42.0, 100.0, 30.0),
                "恢复默认",
                Action::StrengthReset,
                false,
            );
        }
    } else if page == 1 {
        s.button(
            Rect::from_xywh(x + w - 110.0, 34.0, 110.0, 32.0),
            "恢复默认",
            Action::Change(Event::ResetPaneOptions),
            false,
        );
        s.text(
            Rect::from_xywh(x + 18.0, 104.0, w - 258.0, 34.0),
            "圆角大小",
            1,
        );
        let radius = options.corner_radius;
        s.controls.push(Control {
            bounds: Rect::from_xywh(x + w - 248.0, 104.0, 180.0, 34.0),
            label: String::new(),
            action: Action::Radius(radius),
            selected: false,
            enabled: true,
            toggle: false,
        });
        s.text(
            Rect::from_xywh(x + w - 58.0, 104.0, 46.0, 34.0),
            format!("{radius:.1}"),
            1,
        );
        for (i, (title, enabled, event)) in [
            ("显示边框", options.border, Event::ToggleBorder),
            ("边缘吸附", options.snap, Event::ToggleSnap),
        ]
        .iter()
        .enumerate()
        {
            let y = 164.0 + i as f32 * 56.0;
            s.separators
                .push(Rect::from_xywh(x + 18.0, y - 6.0, w - 36.0, 1.0));
            s.text(
                Rect::from_xywh(x + 18.0, y + 8.0, w - 100.0, 32.0),
                *title,
                1,
            );
            s.controls.push(Control {
                bounds: Rect::from_xywh(x + w - 70.0, y + 12.0, 46.0, 24.0),
                label: String::new(),
                action: Action::Change(event.clone()),
                selected: *enabled,
                enabled: true,
                toggle: true,
            });
        }
        s.separators
            .push(Rect::from_xywh(x + 18.0, 270.0, w - 36.0, 1.0));
        s.text(
            Rect::from_xywh(x + 18.0, 286.0, w - 36.0, 28.0),
            "面板文字",
            1,
        );
        let choice_width = (w - 52.0) / 3.0;
        for (i, (label, value)) in [
            ("自动", desktop_core::PanelText::Auto),
            ("浅色文字", desktop_core::PanelText::Light),
            ("深色文字", desktop_core::PanelText::Dark),
        ]
        .into_iter()
        .enumerate()
        {
            s.button(
                Rect::from_xywh(
                    x + 18.0 + i as f32 * (choice_width + 8.0),
                    322.0,
                    choice_width,
                    34.0,
                ),
                label,
                Action::Change(Event::SetPanelText(value)),
                options.text == value,
            );
        }
        s.text(
            Rect::from_xywh(x + 18.0, 372.0, w - 100.0, 28.0),
            "明暗底色保护",
            1,
        );
        s.controls.push(Control {
            bounds: Rect::from_xywh(x + w - 70.0, 374.0, 46.0, 24.0),
            label: String::new(),
            action: Action::Change(Event::ToggleTextProtection),
            selected: options.text_protection,
            enabled: true,
            toggle: true,
        });
        s.text(
            Rect::from_xywh(x + 18.0, 410.0, w - 36.0, 26.0),
            "关闭后保留原始通透效果，文字颜色仍按上方选择。",
            0,
        );
        let value = options.grid_scale;
        s.text(Rect::from_xywh(x + 18.0, 446.0, w - 258.0, 34.0), "图标网格缩放", 1);
        s.controls.push(Control {
            bounds: Rect::from_xywh(x + w - 248.0, 446.0, 168.0, 34.0),
            label: String::new(), action: Action::GridSize(value),
            selected: false, enabled: true, toggle: false,
        });
        s.text(Rect::from_xywh(x + w - 74.0, 446.0, 66.0, 34.0), format!("{value:.0}%"), 0);
    } else if page == 3 {
        for y in [198.0, 348.0] {
            s.separators
                .push(Rect::from_xywh(x + 18.0, y, w - 36.0, 1.0));
        }
        let value = peek::settings();
        s.text(
            Rect::from_xywh(x + 18.0, 104.0, w - 110.0, 32.0),
            "启用文件预览",
            1,
        );
        s.controls.push(Control {
            bounds: Rect::from_xywh(x + w - 70.0, 108.0, 46.0, 24.0),
            label: String::new(),
            action: Action::PeekEnable,
            selected: value.enabled && peek::resolved(&value).is_some(),
            enabled: peek::resolved(&value).is_some(),
            toggle: true,
        });
        s.text(
            Rect::from_xywh(x + 18.0, 214.0, w - 36.0, 26.0),
            if value.active_path().is_empty() {
                "程序路径 · 自动"
            } else {
                "程序路径"
            },
            1,
        );
        for (i, provider) in [peek::Provider::Peek, peek::Provider::QuickLook]
            .into_iter()
            .enumerate()
        {
            s.button(
                Rect::from_xywh(x + 18.0 + i as f32 * 130.0, 154.0, 120.0, 32.0),
                provider.name(),
                Action::PreviewProvider(provider),
                value.provider == provider,
            );
        }
        let path = peek::resolved(&value)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("未找到 {}，请选择程序", value.provider.name()));
        s.text(Rect::from_xywh(x + 18.0, 248.0, w - 36.0, 36.0), path, 0);
        s.button(
            Rect::from_xywh(x + 18.0, 294.0, 110.0, 34.0),
            "浏览…",
            Action::PeekBrowse,
            false,
        );
        s.button(
            Rect::from_xywh(x + 140.0, 294.0, 110.0, 34.0),
            "自动检测",
            Action::PeekDetect,
            false,
        );
        s.text(Rect::from_xywh(x + 18.0, 362.0, 90.0, 26.0), "快捷键", 1);
        s.button(
            Rect::from_xywh(x + 114.0, 364.0, w - 250.0, 34.0),
            &peek::shortcut_label(&value),
            Action::PeekShortcut,
            false,
        );
        s.button(
            Rect::from_xywh(x + w - 124.0, 364.0, 106.0, 34.0),
            "恢复默认",
            Action::PeekReset,
            false,
        );
        s.text(
            Rect::from_xywh(x + 18.0, 410.0, w - 36.0, 24.0),
            "仅在面板内生效",
            0,
        );
    } else if page == 4 {
        for y in [248.0] {
            s.separators
                .push(Rect::from_xywh(x + 18.0, y, w - 36.0, 1.0));
        }
        let value = everything_settings::settings();
        s.text(
            Rect::from_xywh(x + 18.0, 104.0, w - 110.0, 32.0),
            "启用搜索面板",
            1,
        );
        s.controls.push(Control {
            bounds: Rect::from_xywh(x + w - 70.0, 108.0, 46.0, 24.0),
            label: String::new(),
            action: Action::Change(Event::ToggleSearch),
            selected: search_enabled && everything_settings::resolved(&value).is_some(),
            enabled: everything_settings::resolved(&value).is_some(),
            toggle: true,
        });
        s.text(
            Rect::from_xywh(x + 18.0, 174.0, 90.0, 30.0),
            "全局快捷键",
            1,
        );
        s.button(
            Rect::from_xywh(x + 114.0, 172.0, w - 250.0, 34.0),
            &search_hotkey::label(search_hotkey::settings()),
            Action::SearchShortcut,
            false,
        );
        s.button(
            Rect::from_xywh(x + w - 124.0, 172.0, 106.0, 34.0),
            "恢复默认",
            Action::SearchReset,
            false,
        );
        s.text(
            Rect::from_xywh(x + 18.0, 210.0, w - 36.0, 24.0),
            search_hotkey::status(),
            0,
        );
        s.text(
            Rect::from_xywh(x + 18.0, 264.0, w - 36.0, 26.0),
            if value.path.is_empty() {
                "程序路径 · 自动"
            } else {
                "程序路径"
            },
            1,
        );
        let path = everything_settings::resolved(&value)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "未找到 Everything，请选择程序".into());
        s.text(Rect::from_xywh(x + 18.0, 294.0, w - 36.0, 24.0), path, 0);
        s.button(
            Rect::from_xywh(x + 18.0, 322.0, 110.0, 34.0),
            "浏览…",
            Action::EverythingBrowse,
            false,
        );
        s.button(
            Rect::from_xywh(x + 140.0, 322.0, 110.0, 34.0),
            "自动检测",
            Action::EverythingDetect,
            false,
        );
        s.button(
            Rect::from_xywh(x + w - 128.0, 322.0, 110.0, 34.0),
            "启动",
            Action::EverythingLaunch,
            false,
        );
    } else if page == 6 {
        backup_page(&mut s, width, &recovery::View::default(), recovery::Policy::default(), 0, false);
    } else if page == 5 {
        s.app_icon = Some(Rect::from_xywh(x, 106.0, 64.0, 64.0));
        s.text(
            Rect::from_xywh(x + 84.0, 102.0, w - 84.0, 42.0),
            "LucidPane",
            3,
        );
        s.text(
            Rect::from_xywh(x + 84.0, 148.0, w - 84.0, 26.0),
            concat!("v", env!("CARGO_PKG_VERSION"), " · 预览版"),
            0,
        );
        s.text(
            Rect::from_xywh(x, 190.0, w, 28.0),
            "让桌面井然有序，让文件触手可及。",
            1,
        );
        s.text(
            Rect::from_xywh(x + 132.0, 230.0, w - 132.0, 34.0),
            "MIT / Apache-2.0",
            0,
        );
        s.button(
            Rect::from_xywh(x, 230.0, 116.0, 34.0),
            "项目主页",
            Action::ProjectHome,
            false,
        );

        s.separators.push(Rect::from_xywh(x, 286.0, w, 1.0));
        let revision = env!("LUCIDPANE_BUILD_REVISION");
        let build = if revision == "unknown" {
            "本地构建".to_owned()
        } else {
            format!("构建 {revision}")
        };
        s.text(Rect::from_xywh(x, 302.0, 76.0, 28.0), "版本信息", 0);
        s.text(
            Rect::from_xywh(x + 96.0, 302.0, w - 96.0, 28.0),
            format!("{} · {build}", std::env::consts::ARCH),
            1,
        );
        s.text(Rect::from_xywh(x, 340.0, 76.0, 28.0), "操作系统", 0);
        s.text(
            Rect::from_xywh(x + 96.0, 340.0, w - 96.0, 28.0),
            crate::diagnostics::system().summary(),
            1,
        );

        s.separators.push(Rect::from_xywh(x, 388.0, w, 1.0));
        s.text(Rect::from_xywh(x, 404.0, 76.0, 32.0), "桌面连接", 0);
    }
    s
}

// Shared by the live page and raster tests so status and actions use the same layout.
pub(super) fn about_status(s: &mut Scene, width: f32, status: &str, copied: bool) {
    let x = 248.0;
    let w = width - x - 24.0;
    s.text(Rect::from_xywh(x + 96.0, 404.0, w - 96.0, 32.0), status, 1);
    s.button(
        Rect::from_xywh(x, 454.0, 144.0, 34.0),
        "重新连接桌面",
        Action::Change(Event::RetryDesktop),
        false,
    );
    s.button(
        Rect::from_xywh(x + 156.0, 454.0, 116.0, 34.0),
        if copied { "已复制" } else { "复制诊断" },
        Action::CopyDiagnostics,
        false,
    );
}


fn backup_row(s: &mut Scene, x:f32, y:f32, w:f32, label:&str, action:Action) {
    s.button(Rect::from_xywh(x,y,w,38.0),label,action,false);
}
pub(super) fn backup_page(s: &mut Scene, width: f32, view: &recovery::View,
    policy: recovery::Policy, _offset: usize, advanced: bool) {
    let x=248.0; let w=width-x-24.0;
    if advanced {
        backup_row(s,x,80.0,w,"返回备份与恢复",Action::Page(6));
        for (i,(label,event)) in [("打开配置目录",Event::OpenConfigDirectory),("重新加载配置",Event::ReloadConfig),("导出当前配置…",Event::ExportBackup)].into_iter().enumerate() {
            backup_row(s,x,140.0+i as f32*46.0,w,label,Action::Change(event));
        }
    } else {
        let status=if view.status.is_empty(){"尚无备份"}else{&view.status};
        s.text(Rect::from_xywh(x+10.0,80.0,w-20.0,30.0),status,0);
        if view.status.contains("失败") {backup_row(s,x,80.0,w,"",Action::BackupStatus);}
        s.text(Rect::from_xywh(x+10.0,126.0,w-80.0,32.0),"自动备份",1);
        s.controls.push(Control {bounds:Rect::from_xywh(x+w-56.0,130.0,46.0,24.0),label:String::new(),action:Action::BackupPolicy(0),selected:policy.enabled,toggle:true,enabled:true});
        s.text(Rect::from_xywh(x+10.0,166.0,w-246.0,38.0),"备份间隔",1);
        s.button(Rect::from_xywh(x+w-226.0,169.0,216.0,32.0),&format!("{} 分钟",policy.minutes),Action::BackupPolicy(1),false);
        s.text(Rect::from_xywh(x+10.0,208.0,w-246.0,38.0),"保留自动备份",1);
        s.button(Rect::from_xywh(x+w-226.0,211.0,216.0,32.0),&format!("最近 {} 份",policy.keep),Action::BackupPolicy(2),false);
        s.separators.push(Rect::from_xywh(x,258.0,w,1.0));
        backup_row(s,x,274.0,w,"立即备份",Action::Change(Event::CreateBackup));
        backup_row(s,x,316.0,w,"从文件恢复…",Action::Change(Event::RestoreBackup));
        backup_row(s,x,358.0,w,"管理备份",Action::Page(9));
        let next=if let Some(path)=&view.undo {
            backup_row(s,x,400.0,w,"撤销本次恢复…",Action::Change(Event::RestoreBackupPath(path.clone())));442.0
        } else {400.0};
        backup_row(s,x,next,w,"高级选项",Action::BackupAdvanced);
        s.text(Rect::from_xywh(x+10.0,next+48.0,w-20.0,28.0),"仅保存配置和布局，不包含实际文件。",0);
    }
    if view.busy {for control in &mut s.controls {if control.bounds.left>=x && !matches!(control.action,Action::BackupPolicy(_)|Action::Page(_)|Action::BackupAdvanced|Action::BackupStatus){control.enabled=false;}}}
}
pub(super) fn backup_history(s:&mut Scene,width:f32,view:&recovery::View,offset:usize) {
    let x=248.0;let w=width-x-24.0;
    backup_row(s,x,80.0,w/2.0,"返回备份与恢复",Action::Page(6));
    backup_row(s,x+w/2.0,80.0,w/2.0,"打开备份文件夹",Action::Change(Event::OpenBackups));
    s.text(Rect::from_xywh(x+10.0,124.0,w-20.0,26.0),"最新在前 · 点击记录可恢复、导出或删除",0);
    for (i,record) in view.records.iter().skip(offset).take(6).enumerate() {
        let y=160.0+i as f32*44.0;
        backup_row(s,x,y,w,&record.date,Action::BackupRecord(record.path.clone()));
        s.text(Rect::from_xywh(x+180.0,y,100.0,38.0),record.kind,0);
        s.text(Rect::from_xywh(x+w-128.0,y,118.0,38.0),folder::size_text(Some(record.bytes),false),0);
    }
    if view.records.is_empty(){s.text(Rect::from_xywh(x+10.0,172.0,w-20.0,32.0),"暂无备份",0);}
    s.text(Rect::from_xywh(x+10.0,446.0,w-190.0,32.0),format!("共 {} 份 · 手动备份不自动清理",view.records.len()),0);
    if offset>0 {backup_row(s,x+w-180.0,446.0,86.0,"上一页",Action::BackupPage(-1));}
    if offset+6<view.records.len(){backup_row(s,x+w-90.0,446.0,90.0,"下一页",Action::BackupPage(1));}
    if view.busy {for c in &mut s.controls {if matches!(c.action,Action::BackupRecord(_)){c.enabled=false;}}}
}
