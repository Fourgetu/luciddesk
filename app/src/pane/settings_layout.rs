use super::*;

pub(super) fn scene(
    width: f32,
    _height: f32,
    page: usize,
    panel: Option<&Panel>,
    count: usize,
    search_enabled: bool,
    appearance: (PanelTheme, Backdrop),
    options: desktop_core::PaneOptions,
) -> Scene {
    let mut s = Scene {
        text: vec![],
        cards: vec![],
        controls: vec![],
    };
    s.text(Rect::from_xywh(28.0, 26.0, 172.0, 32.0), "LucidPane", 2);
    for (i, (name, icon)) in [
        ("个性化", "\u{e790}"),
        ("面板外观", "\u{e790}"),
        ("分组行为", "\u{e8b7}"),
        ("Peek 速览", "\u{e721}"),
        ("Everything 搜索", "\u{e721}"),
        ("关于", "\u{e946}"),
        ("备份与恢复", "\u{e74e}"),
    ]
    .iter()
    .enumerate()
    {
        let y = 78.0 + i as f32 * 42.0;
        s.button(
            Rect::from_xywh(12.0, y, 200.0, 38.0),
            name,
            Action::Page(i),
            page == i,
        );
        s.text(Rect::from_xywh(30.0, y, 22.0, 38.0), *icon, 4);
    }
    let x = 248.0;
    let w = width - x - 24.0;
    s.text(
        Rect::from_xywh(x, 28.0, w, 44.0),
        [
            "个性化",
            "面板外观",
            "分组行为",
            "Peek 速览",
            "Everything 搜索",
            "关于",
            "备份与恢复",
        ][page],
        3,
    );
    if page == 0 {
        s.text(
            Rect::from_xywh(x, 76.0, w, 24.0),
            format!("所有分组 · {count}"),
            0,
        );
        let stacked = w < 470.0;
        let extra = if stacked { 44.0 } else { 0.0 };
        s.cards.push(Rect::from_xywh(x, 108.0, w, 64.0 + extra));
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
                Rect::from_xywh(x + j as f32 * (bw + 12.0), 228.0 + extra, bw, 128.0),
                name,
                Action::Change(Event::Material(*value)),
                appearance.1.kind() == value.kind(),
            );
        }
        if let Backdrop::Solid { color, opacity } = appearance.1 {
            let y = 366.0 + extra;
            s.button(
                Rect::from_xywh(x, y, 112.0, 30.0),
                &format!("#{color:06X}"),
                Action::StyleInput(false),
                false,
            );
            s.button(
                Rect::from_xywh(x + 124.0, y, 100.0, 30.0),
                "选择颜色",
                Action::SolidColor,
                false,
            );
            s.button(
                Rect::from_xywh(x + w - 100.0, y, 100.0, 30.0),
                "恢复默认",
                Action::SolidReset,
                false,
            );
            let value = (opacity * 100.0).round() as u8;
            s.text(Rect::from_xywh(x, y + 40.0, 100.0, 30.0), "不透明度", 0);
            s.controls.push(Control {
                bounds: Rect::from_xywh(x + 106.0, y + 40.0, w - 196.0, 30.0),
                label: String::new(),
                action: Action::Opacity(value),
                selected: false,
                toggle: false,
            });
            s.button(
                Rect::from_xywh(x + w - 78.0, y + 40.0, 78.0, 30.0),
                &format!("{value}%"),
                Action::StyleInput(true),
                false,
            );
        }
    } else if page == 1 {
        s.text(
            Rect::from_xywh(x, 76.0, w, 24.0),
            format!("所有分组 · {count}"),
            0,
        );
        s.cards.push(Rect::from_xywh(x, 108.0, w, 64.0));
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
        for (i, (title, description, enabled, event)) in [
            (
                "边框",
                "显示面板轮廓线",
                options.border,
                Event::ToggleBorder,
            ),
            (
                "自动吸附",
                "移动时对齐面板与屏幕边缘",
                options.snap,
                Event::ToggleSnap,
            ),
        ]
        .iter()
        .enumerate()
        {
            let y = 184.0 + i as f32 * 76.0;
            s.cards.push(Rect::from_xywh(x, y, w, 64.0));
            s.text(
                Rect::from_xywh(x + 18.0, y + 8.0, w - 100.0, 24.0),
                *title,
                1,
            );
            s.text(
                Rect::from_xywh(x + 18.0, y + 32.0, w - 100.0, 24.0),
                *description,
                0,
            );
            s.controls.push(Control {
                bounds: Rect::from_xywh(x + w - 70.0, y + 20.0, 46.0, 24.0),
                label: String::new(),
                action: Action::Change(event.clone()),
                selected: *enabled,
                toggle: true,
            });
        }
    } else if page == 2 {
        let Some(panel) = panel else {
            s.cards.push(Rect::from_xywh(x, 88.0, w, 116.0));
            s.text(
                Rect::from_xywh(x + 24.0, 100.0, w - 48.0, 32.0),
                "还没有分组",
                2,
            );
            s.button(
                Rect::from_xywh(x + 24.0, 150.0, 132.0, 36.0),
                "新建分组",
                Action::Change(Event::New),
                true,
            );
            return s;
        };
        s.cards.push(Rect::from_xywh(x, 88.0, w, 64.0));
        s.text(
            Rect::from_xywh(x + 24.0, 94.0, w - 150.0, 22.0),
            "正在调整的分组",
            0,
        );
        s.text(
            Rect::from_xywh(x + 24.0, 116.0, w - 150.0, 28.0),
            panel.title(),
            1,
        );
        if count > 1 {
            s.button(
                Rect::from_xywh(x + w - 100.0, 103.0, 34.0, 34.0),
                "‹",
                Action::Previous,
                false,
            );
            s.button(
                Rect::from_xywh(x + w - 54.0, 103.0, 34.0, 34.0),
                "›",
                Action::Next,
                false,
            );
        }
        for (i, (title, description, icon, enabled, event)) in [
            (
                "始终置顶",
                "将此分组显示在其他窗口上方",
                "\u{e718}",
                panel.always_on_top(),
                Event::ToggleTopmost,
            ),
            (
                "自动收起",
                "鼠标移入展开，离开时自动收起",
                "\u{e8a7}",
                panel.auto_hide(),
                Event::ToggleAutoHide,
            ),
        ]
        .iter()
        .enumerate()
        {
            if panel.is_search() && matches!(event, Event::ToggleAutoHide) {
                continue;
            }
            let y = 164.0 + i as f32 * 76.0;
            s.cards.push(Rect::from_xywh(x, y, w, 64.0));
            s.text(Rect::from_xywh(x + 22.0, y + 16.0, 28.0, 32.0), *icon, 4);
            s.text(
                Rect::from_xywh(x + 66.0, y + 8.0, w - 180.0, 24.0),
                *title,
                1,
            );
            s.text(
                Rect::from_xywh(x + 66.0, y + 32.0, w - 180.0, 22.0),
                *description,
                0,
            );
            s.controls.push(Control {
                bounds: Rect::from_xywh(x + w - 70.0, y + 20.0, 46.0, 24.0),
                label: String::new(),
                action: Action::Change(event.clone()),
                selected: *enabled,
                toggle: true,
            });
        }
        if panel.is_search() {
            s.cards.push(Rect::from_xywh(x, 240.0, w, 110.0));
            s.text(
                Rect::from_xywh(x + 18.0, 252.0, w - 36.0, 28.0),
                "Everything 搜索面板",
                1,
            );
            s.text(
                Rect::from_xywh(x + 18.0, 286.0, w - 36.0, 52.0),
                "输入后展开结果，清空后收回\nCtrl+L 搜索，F5 刷新，Ctrl+Enter 打开位置",
                0,
            );
        }
        if let Some(path) = panel.folder() {
            s.button(
                Rect::from_xywh(x + w - 128.0, 324.0, 110.0, 26.0),
                if panel.folder_list() {
                    "切换图标视图"
                } else {
                    "切换列表视图"
                },
                Action::Change(Event::ToggleFolderView),
                false,
            );
            s.cards.push(Rect::from_xywh(x, 316.0, w, 108.0));
            s.text(
                Rect::from_xywh(x + 18.0, 324.0, w - 156.0, 26.0),
                "文件夹面板 · 源文件夹",
                1,
            );
            s.text(
                Rect::from_xywh(x + 18.0, 354.0, w - 36.0, 26.0),
                path.to_string_lossy(),
                0,
            );
            s.button(
                Rect::from_xywh(x + 18.0, 386.0, 120.0, 30.0),
                "打开文件夹",
                Action::Change(Event::OpenFolder),
                false,
            );
            s.button(
                Rect::from_xywh(x + 150.0, 386.0, 120.0, 30.0),
                "更换文件夹…",
                Action::Change(Event::ChangeFolder),
                false,
            );
        }
    } else if page == 3 {
        let value = peek::settings();
        s.cards.push(Rect::from_xywh(x, 88.0, w, 64.0));
        s.text(
            Rect::from_xywh(x + 18.0, 96.0, w - 110.0, 24.0),
            "启用 Peek",
            1,
        );
        s.text(
            Rect::from_xywh(x + 18.0, 122.0, w - 110.0, 22.0),
            "在分组中使用快捷键预览选中的项目",
            0,
        );
        s.controls.push(Control {
            bounds: Rect::from_xywh(x + w - 70.0, 108.0, 46.0, 24.0),
            label: String::new(),
            action: Action::PeekEnable,
            selected: value.enabled,
            toggle: true,
        });
        s.cards.push(Rect::from_xywh(x, 164.0, w, 138.0));
        s.text(
            Rect::from_xywh(x + 18.0, 174.0, w - 36.0, 26.0),
            if value.path.is_empty() {
                "Peek 路径 · 自动检测"
            } else {
                "Peek 路径 · 自定义"
            },
            1,
        );
        let path = peek::resolved(&value)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "未检测到 Peek，请安装 PowerToys 或浏览选择程序".into());
        s.text(Rect::from_xywh(x + 18.0, 208.0, w - 36.0, 36.0), path, 0);
        s.button(
            Rect::from_xywh(x + 18.0, 254.0, 110.0, 34.0),
            "浏览…",
            Action::PeekBrowse,
            false,
        );
        s.button(
            Rect::from_xywh(x + 140.0, 254.0, 110.0, 34.0),
            "自动检测",
            Action::PeekDetect,
            false,
        );
        s.cards.push(Rect::from_xywh(x, 314.0, w, 96.0));
        s.text(Rect::from_xywh(x + 18.0, 322.0, 90.0, 26.0), "快捷键", 1);
        s.button(
            Rect::from_xywh(x + 114.0, 324.0, w - 250.0, 34.0),
            &peek::shortcut_label(&value),
            Action::PeekShortcut,
            false,
        );
        s.button(
            Rect::from_xywh(x + w - 124.0, 324.0, 106.0, 34.0),
            "恢复默认",
            Action::PeekReset,
            false,
        );
        s.text(
            Rect::from_xywh(x + 18.0, 370.0, w - 36.0, 24.0),
            "点击录入快捷键，Esc 取消；仅在分组中生效",
            0,
        );
    } else if page == 4 {
        let value = everything_settings::settings();
        s.cards.push(Rect::from_xywh(x, 88.0, w, 64.0));
        s.text(
            Rect::from_xywh(x + 18.0, 96.0, w - 110.0, 24.0),
            "搜索时自动启动",
            1,
        );
        s.text(
            Rect::from_xywh(x + 18.0, 122.0, w - 110.0, 22.0),
            "Everything 未运行时在后台启动",
            0,
        );
        s.controls.push(Control {
            bounds: Rect::from_xywh(x + w - 70.0, 108.0, 46.0, 24.0),
            label: String::new(),
            action: Action::EverythingAutoStart,
            selected: value.auto_start,
            toggle: true,
        });
        s.cards.push(Rect::from_xywh(x, 164.0, w, 112.0));
        s.text(
            Rect::from_xywh(x + 18.0, 174.0, w - 36.0, 26.0),
            if value.path.is_empty() {
                "Everything 路径 · 自动检测"
            } else {
                "Everything 路径 · 自定义"
            },
            1,
        );
        let path = everything_settings::resolved(&value)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "未检测到 Everything，请浏览选择程序".into());
        s.text(Rect::from_xywh(x + 18.0, 204.0, w - 36.0, 24.0), path, 0);
        s.button(
            Rect::from_xywh(x + 18.0, 232.0, 110.0, 34.0),
            "浏览…",
            Action::EverythingBrowse,
            false,
        );
        s.button(
            Rect::from_xywh(x + 140.0, 232.0, 110.0, 34.0),
            "自动检测",
            Action::EverythingDetect,
            false,
        );
        s.cards.push(Rect::from_xywh(x, 288.0, w, 64.0));
        s.button(
            Rect::from_xywh(x + 18.0, 302.0, 150.0, 34.0),
            "启动 Everything",
            Action::EverythingLaunch,
            false,
        );
        s.text(
            Rect::from_xywh(x + 180.0, 302.0, w - 268.0, 34.0),
            "启用搜索面板",
            1,
        );
        s.controls.push(Control {
            bounds: Rect::from_xywh(x + w - 70.0, 307.0, 46.0, 24.0),
            label: String::new(),
            action: Action::Change(Event::ToggleSearch),
            selected: search_enabled,
            toggle: true,
        });
        s.cards.push(Rect::from_xywh(x, 364.0, w, 78.0));
        s.text(Rect::from_xywh(x + 18.0, 372.0, 90.0, 26.0), "全局唤起", 1);
        s.button(
            Rect::from_xywh(x + 114.0, 374.0, w - 250.0, 34.0),
            &search_hotkey::label(search_hotkey::settings()),
            Action::SearchShortcut,
            false,
        );
        s.button(
            Rect::from_xywh(x + w - 124.0, 374.0, 106.0, 34.0),
            "恢复默认",
            Action::SearchReset,
            false,
        );
        s.text(
            Rect::from_xywh(x + 18.0, 412.0, w - 36.0, 24.0),
            search_hotkey::status(),
            0,
        );
    } else if page == 6 {
        s.text(
            Rect::from_xywh(x, 88.0, w, 60.0),
            "每 5 分钟自动备份变更后的配置，保留最近 10 份。",
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
            "包含布局、分组归属和设置，不包含原文件。\n恢复前会保留当前配置的备份。",
            0,
        );
    } else {
        s.cards.push(Rect::from_xywh(x, 88.0, w, 144.0));
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
        s.text(
            Rect::from_xywh(x + 16.0, 244.0, w - 32.0, 52.0),
            "桌面分组与文件夹映射 · Everything 搜索\n键盘操作与 Peek 速览 · 布局备份与恢复",
            0,
        );
        s.button(
            Rect::from_xywh(x + 16.0, 306.0, 132.0, 34.0),
            "项目主页",
            Action::ProjectHome,
            false,
        );
        s.text(
            Rect::from_xywh(x + 160.0, 306.0, (w - 176.0).max(1.0), 34.0),
            "MIT / Apache-2.0",
            0,
        );
    }
    s
}
