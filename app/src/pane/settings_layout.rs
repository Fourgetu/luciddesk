use super::*;

pub(super) fn scene(
    width: f32,
    _height: f32,
    page: usize,
    panel: Option<&Panel>,
    count: usize,
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
        ("关于", "\u{e946}"),
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
        ["个性化", "面板外观", "分组行为", "Peek 速览", "关于"][page],
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
        let bw = (w - 24.0) / 3.0;
        for (j, (name, value)) in [
            ("亚克力", Backdrop::Acrylic),
            ("Mica", Backdrop::Mica),
            ("Mica Alt", Backdrop::MicaAlt),
        ]
        .iter()
        .enumerate()
        {
            s.button(
                Rect::from_xywh(x + j as f32 * (bw + 12.0), 228.0 + extra, bw, 128.0),
                name,
                Action::Change(Event::Material(*value)),
                appearance.1 == *value,
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
                "连接本机 Everything 1.4+ · 每页 200 项\nCtrl+L 搜索，F5 刷新，Ctrl+Enter 打开位置",
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
    } else {
        s.cards.push(Rect::from_xywh(x, 88.0, w, 152.0));
        s.text(
            Rect::from_xywh(x + 28.0, 104.0, w - 56.0, 40.0),
            "LucidPane",
            3,
        );
        s.text(
            Rect::from_xywh(x + 28.0, 152.0, w - 56.0, 26.0),
            concat!("版本 ", env!("CARGO_PKG_VERSION")),
            0,
        );
        s.text(
            Rect::from_xywh(x + 28.0, 190.0, w - 56.0, 28.0),
            "\u{6df7}\u{5408}\u{684c}\u{9762}\u{6a21}\u{5f0f}",
            1,
        );
    }
    s
}
