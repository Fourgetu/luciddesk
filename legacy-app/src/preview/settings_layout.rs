use super::*;

pub(super) fn scene(
    width: f32,
    _height: f32,
    page: usize,
    panel: Option<&Panel>,
    count: usize,
    hybrid: bool,
    appearance: (PanelTheme, Backdrop),
) -> Scene {
    let mut s = Scene {
        text: vec![],
        cards: vec![],
        controls: vec![],
    };
    s.text(rect(28.0, 26.0, 172.0, 32.0), "LucidPane", 2);
    for (i, (name, icon)) in [
        ("个性化", "\u{e790}"),
        ("分组行为", "\u{e8b7}"),
        ("关于", "\u{e946}"),
    ]
    .iter()
    .enumerate()
    {
        let y = 78.0 + i as f32 * 42.0;
        s.button(rect(12.0, y, 200.0, 38.0), name, Action::Page(i), page == i);
        s.text(rect(30.0, y, 22.0, 38.0), *icon, 4);
    }
    let x = 248.0;
    let w = width - x - 24.0;
    s.text(
        rect(x, 28.0, w, 44.0),
        ["个性化", "分组行为", "关于"][page],
        3,
    );
    if page == 0 {
        s.text(rect(x, 76.0, w, 24.0), format!("所有分组 · {count}"), 0);
        let stacked = w < 470.0;
        let extra = if stacked { 44.0 } else { 0.0 };
        s.cards.push(rect(x, 108.0, w, 64.0 + extra));
        s.text(rect(x + 16.0, 124.0, 28.0, 32.0), "\u{e793}", 4);
        s.text(rect(x + 54.0, 124.0, 120.0, 32.0), "应用主题", 1);
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
                rect(if stacked { x + 16.0 } else { x + w - 282.0 } + j as f32 * 90.0,
                    if stacked { 164.0 } else { 123.0 }, bw, 34.0),
                name,
                Action::Change(Event::Theme(*value)),
                appearance.0 == *value,
            );
        }
        s.text(rect(x, 192.0 + extra, w, 28.0), "窗口材质", 1);
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
                rect(x + j as f32 * (bw + 12.0), 228.0 + extra, bw, 128.0),
                name,
                Action::Change(Event::Material(*value)),
                appearance.1 == *value,
            );
        }
    } else if page == 1 {
        let Some(panel) = panel else {
            s.cards.push(rect(x, 88.0, w, 116.0));
            s.text(rect(x + 24.0, 100.0, w - 48.0, 32.0), "还没有分组", 2);
            s.button(
                rect(x + 24.0, 150.0, 132.0, 36.0),
                "新建分组",
                Action::Change(Event::New),
                true,
            );
            return s;
        };
        s.cards.push(rect(x, 88.0, w, 64.0));
        s.text(rect(x + 24.0, 94.0, w - 150.0, 22.0), "正在调整的分组", 0);
        s.text(rect(x + 24.0, 116.0, w - 150.0, 28.0), panel.title(), 1);
        if count > 1 {
            s.button(
                rect(x + w - 100.0, 103.0, 34.0, 34.0),
                "‹",
                Action::Previous,
                false,
            );
            s.button(
                rect(x + w - 54.0, 103.0, 34.0, 34.0),
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
            let y = 164.0 + i as f32 * 76.0;
            s.cards.push(rect(x, y, w, 64.0));
            s.text(rect(x + 22.0, y + 16.0, 28.0, 32.0), *icon, 4);
            s.text(rect(x + 66.0, y + 8.0, w - 180.0, 24.0), *title, 1);
            s.text(rect(x + 66.0, y + 32.0, w - 180.0, 22.0), *description, 0);
            s.controls.push(Control {
                bounds: rect(x + w - 70.0, y + 20.0, 46.0, 24.0),
                label: String::new(),
                action: Action::Change(event.clone()),
                selected: *enabled,
                toggle: true,
            });
        }
    } else {
        s.cards.push(rect(x, 88.0, w, 152.0));
        s.text(rect(x + 28.0, 104.0, w - 56.0, 40.0), "LucidPane", 3);
        s.text(
            rect(x + 28.0, 152.0, w - 56.0, 26.0),
            concat!("版本 ", env!("CARGO_PKG_VERSION")),
            0,
        );
        s.text(
            rect(x + 28.0, 190.0, w - 56.0, 28.0),
            if hybrid {
                "混合桌面模式"
            } else {
                "桌面分组模式"
            },
            1,
        );
    }
    s
}
