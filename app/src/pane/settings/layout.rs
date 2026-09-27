use super::*;

pub(super) fn folder_defaults(
    s: &mut Scene,
    width: f32,
    value: folder::Defaults,
    mode: folder::EntryMode,
) {
    let mut form = SettingsForm::new(
        s,
        width,
        "设置文件夹的打开方式，以及新建文件夹面板的默认视图。",
    );
    form.section("文件夹入口");
    form.choices(
        "打开方式",
        "立即应用于所有文件夹面板，并记住选择。",
        [
            ("穿透模式", folder::EntryMode::Inline),
            ("普通模式", folder::EntryMode::Explorer),
        ]
        .into_iter()
        .map(|(label, choice)| (label, Action::FolderEntryMode(choice), mode == choice))
        .collect(),
    );
    form.info(
        "当前打开行为",
        match mode {
            folder::EntryMode::Inline => "双击或按 Enter，在面板内进入子文件夹。",
            folder::EntryMode::Explorer => "双击或按 Enter，在文件资源管理器中打开文件夹。",
        },
    );
    form.section("新建面板默认值");
    form.choices(
        "默认视图",
        "仅用于新建面板，已有面板保持原样。",
        [("图标", false), ("列表", true)]
            .into_iter()
            .map(|(label, list)| {
                (
                    label,
                    Action::FolderDefaults(folder::Defaults { list, ..value }),
                    value.list == list,
                )
            })
            .collect(),
    );
    for (label, column) in [("类型", 1), ("修改时间", 2), ("大小", 3)] {
        form.toggle(
            label,
            "列表视图中显示此列；名称始终显示。",
            value.columns & (1 << column) != 0,
            Action::FolderDefaults(folder::Defaults {
                columns: value.columns ^ (1 << column),
                ..value
            }),
        );
    }
    form.button(
        "恢复视图默认值",
        "还原新建面板的视图和显示列。",
        "恢复默认",
        Action::FolderDefaults(folder::Defaults::default()),
    );
}

// Page IDs stay stable; display order is independent of routing.
const PAGES: [(usize, &str, &str); 8] = [
    (0, "主题与材质", "\u{e790}"),
    (1, "面板布局", "\u{f0e2}"),
    (11, "字体", "\u{e8d2}"),
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
        viewport: None,
        scroll_max: 0.0,
        scroll_offset: 0.0,
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
        s.control(
            ControlKind::Navigation,
            Rect::from_xywh(12.0, y, 200.0, 38.0),
            name,
            Action::Page(id),
            page == id || (page == 7 && id == 0) || (matches!(page, 9 | 10) && id == 6),
        );
        s.text(Rect::from_xywh(30.0, y, 22.0, 38.0), icon, 5);
    }
    let x = 248.0;
    let w = (width - x - Tokens::MARGIN).min(Tokens::MAX_WIDTH);
    s.text(
        Rect::from_xywh(x, 28.0, w, 44.0),
        PAGES.iter().find(|(id, _, _)| *id == page).map_or(
            match page {
                9 => "管理备份",
                10 => "高级选项",
                _ => "配色",
            },
            |(_, name, _)| *name,
        ),
        3,
    );
    if page == 7 {
        let Backdrop::Solid { color, opacity } = appearance.1 else {
            return s;
        };
        s.control(
            ControlKind::BackButton,
            Rect::from_xywh(x + w - 86.0, 34.0, 86.0, 32.0),
            "返回",
            Action::Page(0),
            false,
        );
        let mut form =
            SettingsForm::new(&mut s, width, "选择预设颜色或精确调整 RGB，修改即时生效。");
        form.preview(
            &format!("面板预览 · {}%", (opacity * 100.0).round() as u8),
            color,
            opacity,
        );
        form.colors(color);
        form.section("自定义颜色");
        for (channel, name) in ["红 R", "绿 G", "蓝 B"].into_iter().enumerate() {
            let value = ((color >> ((2 - channel) * 8)) & 255) as u8;
            form.slider(
                name,
                "",
                Slider {
                    value: f32::from(value),
                    max: 255.0,
                    centered: false,
                    channel: Some(channel as u8),
                },
                &value.to_string(),
                Action::Channel(channel as u8, value),
            );
        }
        form.button(
            "HEX 色值",
            "输入六位十六进制颜色，按 Enter 确认。",
            &format!("#{color:06X}"),
            Action::StyleInput(false),
        );
        form.button(
            "恢复纯色设置",
            "恢复默认颜色和不透明度。",
            "恢复默认",
            Action::SolidReset,
        );
        return s;
    }
    if page == 0 {
        let mut form = SettingsForm::new(
            &mut s,
            width,
            "调整面板的主题、背景材质和通透效果，修改即时生效。",
        );
        form.section("外观");
        form.choices(
            "应用主题",
            "选择浅色、深色或跟随系统。",
            [
                ("跟随系统", PanelTheme::System),
                ("浅色", PanelTheme::Light),
                ("深色", PanelTheme::Dark),
            ]
            .into_iter()
            .map(|(name, value)| {
                (
                    name,
                    Action::Change(Event::Theme(value)),
                    appearance.0 == value,
                )
            })
            .collect(),
        );
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
        form.choices(
            "窗口材质",
            "选择面板背景的质感。",
            [
                ("亚克力", Backdrop::Acrylic),
                ("Mica", Backdrop::Mica),
                ("Mica Alt", Backdrop::MicaAlt),
                ("纯色", solid),
            ]
            .into_iter()
            .map(|(name, value)| {
                (
                    name,
                    Action::Change(Event::Material(value)),
                    appearance.1.kind() == value.kind(),
                )
            })
            .collect(),
        );
        form.section("效果与预览");
        let (color, opacity) = if let Backdrop::Solid { color, opacity } = appearance.1 {
            (color, opacity)
        } else {
            (
                if theme::is_dark(appearance.0) {
                    0x242424
                } else {
                    0xf3f3f3
                },
                0.85,
            )
        };
        form.preview("面板背景", color, opacity);
        if let Backdrop::Solid { color, opacity } = appearance.1 {
            form.button(
                "背景配色",
                &format!("当前颜色 #{color:06X}"),
                "编辑配色",
                Action::SolidColor,
            );
            let value = (opacity * 100.0).round() as u8;
            form.slider(
                "面板不透明度",
                "数值越低，背景越通透。",
                Slider::linear(f32::from(value), 100.0),
                &format!("{value}%"),
                Action::Opacity(value),
            );
            form.button(
                "恢复纯色设置",
                "恢复默认颜色和不透明度。",
                "恢复默认",
                Action::SolidReset,
            );
        }
        if let Some(value) = appearance.1.strength() {
            form.slider(
                "效果强度",
                "从通透到厚实，居中为默认。",
                Slider::centered(f32::from(value), 100.0),
                &if value == 50 {
                    "默认".into()
                } else {
                    format!("{:+}", i16::from(value) - 50)
                },
                Action::Strength(value),
            );
            form.button(
                "恢复材质效果",
                "将效果强度还原到默认值。",
                "恢复默认",
                Action::StrengthReset,
            );
        }
    } else if page == 1 {
        let mut form = SettingsForm::new(
            &mut s,
            width,
            "调整面板外形、文字和图标布局，修改即时生效。",
        );
        form.section("面板外形");
        form.slider(
            "圆角大小",
            "调整面板和标签的边角弧度。",
            Slider::linear(
                options.corner_radius,
                desktop_core::PaneOptions::MAX_CORNER_RADIUS,
            ),
            &format!("{:.1}", options.corner_radius),
            Action::Radius(options.corner_radius),
        );
        for (title, description, value, event) in [
            (
                "显示边框",
                "用细边框区分面板和桌面背景。",
                options.border,
                Event::ToggleBorder,
            ),
            (
                "边缘吸附",
                "移动面板时对齐邻近面板和屏幕边缘。",
                options.snap,
                Event::ToggleSnap,
            ),
            (
                "标题分隔线",
                "在标题栏和内容之间显示细分隔线。",
                header_divider::enabled(),
                Event::ToggleHeaderDivider,
            ),
        ] {
            form.toggle(title, description, value, Action::Change(event));
        }
        form.section("文字与图标");
        form.choices(
            "面板文字",
            "自动模式根据面板背景选择明暗。",
            [
                ("自动", desktop_core::PanelText::Auto),
                ("浅色文字", desktop_core::PanelText::Light),
                ("深色文字", desktop_core::PanelText::Dark),
            ]
            .into_iter()
            .map(|(label, value)| {
                (
                    label,
                    Action::Change(Event::SetPanelText(value)),
                    options.text == value,
                )
            })
            .collect(),
        );
        form.toggle(
            "明暗底色保护",
            "提高文字可读性；关闭后保留原始通透效果。",
            options.text_protection,
            Action::Change(Event::ToggleTextProtection),
        );
        form.slider(
            "图标网格缩放",
            "同时调整图标大小和排列间距。",
            Slider::centered(grid_slider_position(options.grid_scale), 1.0),
            &format!("{:.0}%", options.grid_scale),
            Action::GridSize(options.grid_scale),
        );
        form.button(
            "恢复面板布局",
            "将本页设置还原到默认值。",
            "恢复默认",
            Action::Change(Event::ResetPaneOptions),
        );
    } else if page == 3 {
        let value = peek::settings();
        let resolved = peek::resolved(&value);
        let mut form =
            SettingsForm::new(&mut s, width, "连接预览程序，在面板内使用快捷键预览文件。");
        form.section("预览服务");
        form.toggle_enabled(
            "启用文件预览",
            "找到可用程序后即可开启。",
            value.enabled && resolved.is_some(),
            resolved.is_some(),
            Action::PeekEnable,
        );
        form.choices(
            "预览程序",
            "选择用于打开文件预览的程序。",
            [peek::Provider::Peek, peek::Provider::QuickLook]
                .into_iter()
                .map(|provider| {
                    (
                        provider.name(),
                        Action::PreviewProvider(provider),
                        value.provider == provider,
                    )
                })
                .collect(),
        );
        let path = resolved
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("未找到 {}，请选择程序", value.provider.name()));
        form.path(
            if value.active_path().is_empty() {
                "程序路径 · 自动"
            } else {
                "程序路径"
            },
            &path,
            vec![
                ("浏览…", Action::PeekBrowse),
                ("自动检测", Action::PeekDetect),
            ],
        );
        form.section("快捷键");
        form.shortcut(
            "预览快捷键",
            "仅在面板内生效。",
            &peek::shortcut_label(&value),
            Action::PeekShortcut,
            Action::PeekReset,
        );
    } else if page == 4 {
        let value = everything_settings::settings();
        let resolved = everything_settings::resolved(&value);
        let mut form = SettingsForm::new(&mut s, width, "连接 Everything，快速搜索本机文件。");
        form.section("搜索服务");
        form.toggle_enabled(
            "启用搜索面板",
            "找到 Everything 程序后即可开启。",
            search_enabled && resolved.is_some(),
            resolved.is_some(),
            Action::Change(Event::ToggleSearch),
        );
        let path = resolved
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "未找到 Everything，请选择程序".into());
        form.path(
            if value.path.is_empty() {
                "程序路径 · 自动"
            } else {
                "程序路径"
            },
            &path,
            vec![
                ("浏览…", Action::EverythingBrowse),
                ("自动检测", Action::EverythingDetect),
                ("启动", Action::EverythingLaunch),
            ],
        );
        form.section("快捷键");
        let status = search_hotkey::status();
        form.shortcut(
            "全局快捷键",
            if status.is_empty() {
                "在任意应用中唤起搜索面板。"
            } else {
                &status
            },
            &search_hotkey::label(search_hotkey::settings()),
            Action::SearchShortcut,
            Action::SearchReset,
        );
    }

    s
}

// Shared by the live page and raster tests so status and actions use the same layout.
pub(super) fn about_status(s: &mut Scene, width: f32, status: &str, copied: bool) {
    let mut form = SettingsForm::new(s, width, "让桌面井然有序，让文件触手可及。");
    form.brand();
    form.button(
        "开源项目",
        "MIT / Apache-2.0",
        "项目主页",
        Action::ProjectHome,
    );
    form.section("版本与系统");
    let revision = env!("LUCIDPANE_BUILD_REVISION");
    let build = if revision == "unknown" {
        "本地构建".to_owned()
    } else {
        format!("构建 {revision}")
    };
    form.info("版本信息", &format!("{} · {build}", std::env::consts::ARCH));
    form.info("操作系统", &crate::diagnostics::system().summary());
    form.section("桌面连接");
    form.actions(
        "连接状态",
        status,
        vec![
            ("重新连接", Action::Change(Event::RetryDesktop)),
            (
                if copied { "已复制" } else { "复制诊断" },
                Action::CopyDiagnostics,
            ),
        ],
    );
}

pub(super) fn backup_page(
    s: &mut Scene,
    width: f32,
    view: &recovery::View,
    policy: recovery::Policy,
    advanced: bool,
) {
    let first_control = s.controls.len();
    let mut form = SettingsForm::new(s, width, "备份仅保存配置和布局，不包含实际文件。");
    if advanced {
        form.back("返回备份与恢复", Action::Page(6));
        form.section("配置维护");
        form.button(
            "配置目录",
            "打开本地配置文件所在位置。",
            "打开目录",
            Action::Change(Event::OpenConfigDirectory),
        );
        form.button(
            "重新加载配置",
            "读取磁盘上的配置并应用到当前工作区。",
            "重新加载",
            Action::Change(Event::ReloadConfig),
        );
        form.button(
            "导出当前配置",
            "选择位置保存一份配置副本。",
            "导出…",
            Action::Change(Event::ExportBackup),
        );
    } else {
        let status = if view.status.is_empty() {
            "尚无备份"
        } else {
            &view.status
        };
        if view.status.contains("失败") {
            form.button("备份状态", status, "查看详情", Action::BackupStatus);
        } else {
            form.info("备份状态", status);
        }
        form.section("自动备份");
        form.toggle(
            "自动备份",
            "按设定间隔保存配置和布局。",
            policy.enabled,
            Action::BackupPolicy(0),
        );
        form.combo(
            "备份间隔",
            "自动备份的执行频率。",
            &format!("{} 分钟", policy.minutes),
            Action::BackupPolicy(1),
        );
        form.combo(
            "保留自动备份",
            "超出数量的旧自动备份会被清理。",
            &format!("最近 {} 份", policy.keep),
            Action::BackupPolicy(2),
        );
        form.section("备份与恢复");
        form.actions(
            "手动操作",
            "立即创建备份，或从已有备份文件恢复。",
            vec![
                ("立即备份", Action::Change(Event::CreateBackup)),
                ("从文件恢复…", Action::Change(Event::RestoreBackup)),
            ],
        );
        form.link(
            "管理备份",
            "查看、恢复、导出或删除已有备份。",
            Action::Page(9),
        );
        if let Some(path) = &view.undo {
            form.button(
                "撤销本次恢复",
                "恢复到本次还原操作之前的配置。",
                "撤销恢复…",
                Action::Change(Event::RestoreBackupPath(path.clone())),
            );
        }
        form.link(
            "高级选项",
            "管理配置目录、重新加载或导出配置。",
            Action::BackupAdvanced,
        );
    }
    if view.busy {
        for control in &mut s.controls[first_control..] {
            if !matches!(
                control.action,
                Action::BackupPolicy(_)
                    | Action::Page(_)
                    | Action::BackupAdvanced
                    | Action::BackupStatus
            ) {
                control.enabled = false;
            }
        }
    }
}
pub(super) fn backup_history(s: &mut Scene, width: f32, view: &recovery::View, offset: usize) {
    let mut form = SettingsForm::new(s, width, "最新备份在前；手动备份不会自动清理。");
    form.back("返回备份与恢复", Action::Page(6));
    form.button(
        "备份文件夹",
        "在文件资源管理器中查看本地备份。",
        "打开目录",
        Action::Change(Event::OpenBackups),
    );
    form.section("备份记录");
    for record in view.records.iter().skip(offset).take(6) {
        form.button(
            &record.date,
            &format!(
                "{} · {}",
                record.kind,
                folder::size_text(Some(record.bytes), false)
            ),
            "管理…",
            Action::BackupRecord(record.path.clone()),
        );
    }
    if view.records.is_empty() {
        form.info("暂无备份", "创建第一份备份后，记录会显示在这里。");
    }
    form.pager(
        &format!(
            "{} / {} · 共 {} 份",
            offset / 6 + 1,
            view.records.len().div_ceil(6).max(1),
            view.records.len()
        ),
        (Action::BackupPage(-1), offset > 0),
        (Action::BackupPage(1), offset + 6 < view.records.len()),
    );
    if view.busy {
        for c in &mut s.controls {
            if matches!(c.action, Action::BackupRecord(_)) {
                c.enabled = false;
            }
        }
    }
}

pub(super) fn fonts(s: &mut Scene, width: f32, choices: &[String], offset: usize) {
    let selected = super::super::fonts::family();
    let mut form = SettingsForm::new(s, width, "本机可缩放字体，已检查常用中英文字符。");
    form.info("当前字体", &selected);
    form.section("可用字体");
    for name in choices.iter().skip(offset).take(7) {
        form.option(name, Action::Font(name.clone()), name == &selected);
    }
    if choices.is_empty() {
        form.info("暂无可用字体", "可恢复默认字体继续使用。");
    }
    form.pager(
        &format!(
            "{} / {} · {} 种字体",
            offset / 7 + 1,
            choices.len().div_ceil(7).max(1),
            choices.len()
        ),
        (Action::FontPage(-1), offset > 0),
        (Action::FontPage(1), offset + 7 < choices.len()),
    );
    form.button(
        "恢复默认字体",
        "使用应用内置的默认字体。",
        "恢复默认",
        Action::Font(super::super::assets::UI_FONT.into()),
    );
}
