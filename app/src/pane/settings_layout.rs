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
            Rect::from_xywh(x + 18.0, 116.0, w - 258.0, 24.0),
            "圆角大小",
            1,
        );
        s.text(
            Rect::from_xywh(x + 18.0, 140.0, w - 258.0, 24.0),
            "0–24，0 为直角",
            0,
        );
        let radius = options.corner_radius;
        s.controls.push(Control {
            bounds: Rect::from_xywh(x + w - 238.0, 123.0, 180.0, 34.0),
            label: String::new(),
            action: Action::Radius(radius),
            selected: false,
            toggle: false,
        });
        s.text(
            Rect::from_xywh(x + w - 42.0, 123.0, 30.0, 34.0),
            radius.to_string(),
            1,
        );
        for (i, (title, enabled, event)) in [
            ("显示边框", options.border, Event::ToggleBorder),
            ("边缘吸附", options.snap, Event::ToggleSnap),
        ]
        .iter()
        .enumerate()
        {
            let y = 184.0 + i as f32 * 76.0;
            s.separators
                .push(Rect::from_xywh(x + 18.0, y - 6.0, w - 36.0, 1.0));
            s.text(
                Rect::from_xywh(x + 18.0, y + 16.0, w - 100.0, 32.0),
                *title,
                1,
            );
            s.controls.push(Control {
                bounds: Rect::from_xywh(x + w - 70.0, y + 20.0, 46.0, 24.0),
                label: String::new(),
                action: Action::Change(event.clone()),
                selected: *enabled,
                toggle: true,
            });
        }
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
            selected: value.enabled,
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
            selected: search_enabled,
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
            Rect::from_xywh(x, 334.0, w, 68.0),
            "仅备份布局和设置，不包含文件。",
            0,
        );
    } else {
        s.text(
            Rect::from_xywh(x + 28.0, 98.0, w - 56.0, 36.0),
            "LucidPane",
            3,
        );
        s.text(
            Rect::from_xywh(x + 28.0, 140.0, w - 56.0, 26.0),
            concat!("版本 ", env!("CARGO_PKG_VERSION")),
            0,
        );
        s.text(
            Rect::from_xywh(x + 28.0, 170.0, w - 56.0, 28.0),
            "让桌面井然有序，让文件触手可及。",
            1,
        );
        s.text(
            Rect::from_xywh(x + 28.0, 202.0, w - 56.0, 24.0),
            format!(
                "预览版 · {} · 构建 {}",
                std::env::consts::ARCH,
                env!("LUCIDPANE_BUILD_REVISION")
            ),
            0,
        );
        s.button(
            Rect::from_xywh(x + 16.0, 254.0, 132.0, 34.0),
            "项目主页",
            Action::ProjectHome,
            false,
        );
        s.text(
            Rect::from_xywh(x + 160.0, 254.0, (w - 176.0).max(1.0), 34.0),
            "MIT / Apache-2.0",
            0,
        );
    }
    s
}
