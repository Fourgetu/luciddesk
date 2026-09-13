use super::*;

// Page IDs stay stable; display order is independent of routing.
const PAGES: [(usize, &str, &str); 6] = [
    (0, "主题与材质", "\u{e790}"),
    (1, "面板布局", "\u{f0e2}"),
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
        let gap = if position >= 4 {
            20.0
        } else if position >= 2 {
            10.0
        } else {
            0.0
        };
        let y = 78.0 + position as f32 * 42.0 + gap;
        s.button(
            Rect::from_xywh(12.0, y, 200.0, 38.0),
            name,
            Action::Page(id),
            page == id || (page == 7 && id == 0),
        );
        s.text(Rect::from_xywh(30.0, y, 22.0, 38.0), icon, 4);
    }
    let x = 248.0;
    let w = width - x - 24.0;
    s.text(
        Rect::from_xywh(x, 28.0, w, 44.0),
        PAGES
            .iter()
            .find(|(id, _, _)| *id == page)
            .map_or("配色", |(_, name, _)| *name),
        3,
    );
    if page == 7 {
        let Backdrop::Solid { color, opacity } = appearance.1 else {
            return s;
        };
        s.button(
            Rect::from_xywh(x + w - 86.0, 34.0, 86.0, 32.0),
            "‹ 返回",
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
        s.text(
            Rect::from_xywh(x, 88.0, w, 60.0),
            "自动备份 · 5 分钟 · 最近 10 份",
            0,
        );
        for (i, (label, event)) in [
            ("导出配置…", Event::ExportBackup),
            ("恢复配置…", Event::RestoreBackup),
            ("打开备份文件夹", Event::OpenBackups),
        ]
        .into_iter()
        .enumerate()
        {
            s.button(
                Rect::from_xywh(x, 164.0 + i as f32 * 52.0, 190.0, 38.0),
                label,
                Action::Change(event),
                false,
            );
        }
        s.text(
            Rect::from_xywh(x, 322.0, w, 44.0),
            "备份包含全局配置和布局，不包含桌面文件。",
            0,
        );
        s.text(Rect::from_xywh(x,390.0,w,26.0),"config.toml · 外部修改后重新加载",0);
        s.button(Rect::from_xywh(x,430.0,150.0,34.0),"打开配置目录",Action::Change(Event::OpenConfigDirectory),false);
        s.button(Rect::from_xywh(x+162.0,430.0,150.0,34.0),"重新加载配置",Action::Change(Event::ReloadConfig),false);
    } else {
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
