//! Reusable settings layout primitives; pages supply values and actions, never coordinates.
use super::*;

pub(super) struct Tokens;
impl Tokens {
    pub const CONTENT_X: f32 = 248.0;
    pub const MARGIN: f32 = 24.0;
    pub const GAP: f32 = 8.0;
    pub const INSET: f32 = 16.0;
    pub const CARD_RADIUS: f32 = 8.0;
    pub const CONTROL_HEIGHT: f32 = 32.0;
    pub const MAX_WIDTH: f32 = 1000.0;
    pub const VALUE_TEXT: usize = 7;
}

pub(super) struct Palette {
    pub background: u32,
    pub ink: u32,
    pub muted: u32,
    pub border: u32,
    pub accent: u32,
}
impl Palette {
    pub fn for_theme(dark: bool) -> Self {
        if dark {
            Self {
                background: 0x202020,
                ink: 0xf5f5f5,
                muted: 0xadadad,
                border: 0x424242,
                accent: 0x76b9ed,
            }
        } else {
            Self {
                background: 0xf3f3f3,
                ink: 0x202020,
                muted: 0x666666,
                border: 0xdfdfdf,
                accent: 0x0067c0,
            }
        }
    }
}

pub(super) struct SettingsForm<'a> {
    scene: &'a mut Scene,
    x: f32,
    width: f32,
    y: f32,
}
impl<'a> SettingsForm<'a> {
    pub fn new(scene: &'a mut Scene, width: f32, description: &str) -> Self {
        let width = (width - Tokens::CONTENT_X - Tokens::MARGIN).min(Tokens::MAX_WIDTH);
        scene.text(
            Rect::from_xywh(Tokens::CONTENT_X, 76.0, width, 36.0),
            description,
            6,
        );
        Self {
            scene,
            x: Tokens::CONTENT_X,
            width,
            y: 120.0,
        }
    }
    pub fn section(&mut self, title: &str) {
        if self.y > 120.0 {
            self.y += 16.0;
        }
        self.scene
            .text(Rect::from_xywh(self.x, self.y, self.width, 28.0), title, 1);
        self.y += 36.0;
    }
    /// Reserve a common trailing column. Long descriptions use the full card width.
    fn card(&mut self, title: &str, description: &str, action_width: f32) -> Rect {
        self.card_layout(title, description, action_width, false)
    }
    fn card_layout(
        &mut self,
        title: &str,
        description: &str,
        action_width: f32,
        force_stacked: bool,
    ) -> Rect {
        let column = action_width.max(232.0);
        let stacked =
            force_stacked || self.width < column + 260.0 || description.chars().count() > 60;
        let text_width = if stacked || action_width == 0.0 {
            self.width - 2.0 * Tokens::INSET
        } else {
            self.width - column - 3.0 * Tokens::INSET
        };
        // Reserve conservative line space for mixed Chinese text and Windows paths.
        let units: f32 = description
            .chars()
            .map(|c| if c.is_ascii() { 7.0 } else { 13.0 })
            .sum();
        let description_height = if description.is_empty() {
            0.0
        } else {
            (units / text_width.max(1.0)).ceil().max(1.0) * 20.0
        };
        let text_height = 24.0
            + if description.is_empty() {
                0.0
            } else {
                4.0 + description_height
            };
        let height = (text_height
            + 2.0 * Tokens::INSET
            + if stacked && action_width > 0.0 {
                12.0 + Tokens::CONTROL_HEIGHT
            } else {
                0.0
            })
        .max(80.0);
        self.scene
            .cards
            .push(Rect::from_xywh(self.x, self.y, self.width, height));
        self.scene.text(
            Rect::from_xywh(
                self.x + Tokens::INSET,
                self.y
                    + if description.is_empty() && !stacked {
                        (height - 24.0) / 2.0
                    } else {
                        Tokens::INSET
                    },
                text_width,
                24.0,
            ),
            title,
            1,
        );
        if !description.is_empty() {
            self.scene.text(
                Rect::from_xywh(
                    self.x + Tokens::INSET,
                    self.y + Tokens::INSET + 28.0,
                    text_width,
                    description_height,
                ),
                description,
                6,
            );
        }
        let action = Rect::from_xywh(
            self.x + self.width - Tokens::INSET - action_width,
            self.y
                + if stacked {
                    height - Tokens::INSET - Tokens::CONTROL_HEIGHT
                } else {
                    (height - Tokens::CONTROL_HEIGHT) / 2.0
                },
            action_width,
            Tokens::CONTROL_HEIGHT,
        );
        self.y += height + Tokens::GAP;
        action
    }
    pub fn info(&mut self, title: &str, description: &str) {
        self.card(title, description, 0.0);
    }
    pub fn toggle_enabled(
        &mut self,
        title: &str,
        description: &str,
        selected: bool,
        enabled: bool,
        action: Action,
    ) {
        self.toggle(title, description, selected, action);
        self.scene.controls.last_mut().unwrap().enabled = enabled;
    }
    pub fn actions(&mut self, title: &str, description: &str, actions: Vec<(&str, Action)>) {
        self.action_group(title, description, actions, false);
    }
    pub fn path(&mut self, title: &str, path: &str, actions: Vec<(&str, Action)>) {
        self.action_group(title, path, actions, true);
    }
    fn action_group(
        &mut self,
        title: &str,
        description: &str,
        actions: Vec<(&str, Action)>,
        stacked: bool,
    ) {
        let width = (actions.len() as f32 * 120.0 - Tokens::GAP).min(self.width - 32.0);
        let r = self.card_layout(title, description, width, stacked);
        let count = actions.len();
        let cell = (width - Tokens::GAP * (count - 1) as f32) / count as f32;
        for (i, (label, action)) in actions.into_iter().enumerate() {
            self.scene.button(
                Rect::from_xywh(
                    r.left + i as f32 * (cell + Tokens::GAP),
                    r.top,
                    cell,
                    Tokens::CONTROL_HEIGHT,
                ),
                label,
                action,
                false,
            );
        }
    }
    pub fn shortcut(
        &mut self,
        title: &str,
        description: &str,
        label: &str,
        action: Action,
        reset: Action,
    ) {
        let r = self.card(title, description, 352.0);
        self.scene.button(
            Rect::from_xywh(r.left, r.top, 232.0, 32.0),
            label,
            action,
            false,
        );
        self.scene.button(
            Rect::from_xywh(r.left + 240.0, r.top, 112.0, 32.0),
            "恢复默认",
            reset,
            false,
        );
    }
    pub fn link(&mut self, title: &str, description: &str, action: Action) {
        let r = self.card(title, description, 112.0);
        self.scene
            .forward_row(r.left, r.top, r.right - r.left, "打开", action);
        self.scene.controls.last_mut().unwrap().bounds = r;
    }
    pub fn combo(&mut self, title: &str, description: &str, label: &str, action: Action) {
        let r = self.card(title, description, 232.0);
        self.scene
            .control(ControlKind::Combo, r, label, action, false);
    }
    pub fn back(&mut self, label: &str, action: Action) {
        self.scene
            .back_row(self.x, self.y, self.width, label, action);
        self.y += 48.0;
    }
    pub fn option(&mut self, label: &str, action: Action, selected: bool) {
        self.scene.row(self.x, self.y, self.width, label, action);
        let control = self.scene.controls.last_mut().unwrap();
        control.selected = selected;
        control.bounds.bottom = self.y + 40.0;
        self.y += 48.0;
    }
    pub fn pager(&mut self, label: &str, previous: (Action, bool), next: (Action, bool)) {
        let r = self.card("分页", label, 232.0);
        for (i, (label, (action, enabled))) in [("上一页", previous), ("下一页", next)]
            .into_iter()
            .enumerate()
        {
            self.scene.button(
                Rect::from_xywh(r.left + i as f32 * 120.0, r.top, 112.0, 32.0),
                label,
                action,
                false,
            );
            self.scene.controls.last_mut().unwrap().enabled = enabled;
        }
    }
    pub fn brand(&mut self) {
        self.scene
            .cards
            .push(Rect::from_xywh(self.x, self.y, self.width, 112.0));
        self.scene.app_icon = Some(Rect::from_xywh(self.x + 16.0, self.y + 24.0, 64.0, 64.0));
        self.scene.text(
            Rect::from_xywh(self.x + 96.0, self.y + 24.0, self.width - 112.0, 32.0),
            "LucidPane",
            2,
        );
        self.scene.text(
            Rect::from_xywh(self.x + 96.0, self.y + 60.0, self.width - 112.0, 28.0),
            concat!("v", env!("CARGO_PKG_VERSION"), " · 预览版"),
            6,
        );
        self.y += 120.0;
    }
    pub fn colors(&mut self, color: u32) {
        let r = self.card("预设配色", "选择颜色后即时预览。", 352.0);
        for (i, value) in [
            0x181b20, 0xf5f6f8, 0x24364b, 0x32463d, 0x51405c, 0x5b3838, 0x745839, 0x416c78,
        ]
        .into_iter()
        .enumerate()
        {
            self.scene.button(
                Rect::from_xywh(r.left + i as f32 * 45.0, r.top, 37.0, 32.0),
                "",
                Action::ColorPreset(value),
                color == value,
            );
        }
    }
    pub fn toggle(&mut self, title: &str, description: &str, selected: bool, action: Action) {
        let r = self.card(title, description, 46.0);
        self.scene.toggle(
            Rect::from_xywh(r.left, r.top + 4.0, 46.0, 24.0),
            action,
            selected,
            true,
        );
    }
    pub fn choices(&mut self, title: &str, description: &str, choices: Vec<(&str, Action, bool)>) {
        let r = self.card(
            title,
            description,
            (choices.len() as f32 * 88.0).min(self.width - 32.0),
        );
        let width =
            (r.right - r.left - Tokens::GAP * (choices.len() - 1) as f32) / choices.len() as f32;
        for (index, (label, action, selected)) in choices.into_iter().enumerate() {
            self.scene.button(
                Rect::from_xywh(
                    r.left + index as f32 * (width + Tokens::GAP),
                    r.top,
                    width,
                    32.0,
                ),
                label,
                action,
                selected,
            );
        }
    }
    pub fn slider(
        &mut self,
        title: &str,
        description: &str,
        value: Slider,
        label: &str,
        action: Action,
    ) {
        let r = self.card(title, description, 232.0);
        // Reserve only the space needed by the value's format. Keep it stable
        // while dragging so changing digit counts cannot move the slider endpoint.
        let value_width = if matches!(action, Action::GridSize(_) | Action::Opacity(_)) {
            40.0
        } else {
            32.0
        };
        let gap = 4.0;
        self.scene.slider(
            Rect::from_xywh(r.left, r.top, r.right - r.left - value_width - gap, 32.0),
            value,
            action,
        );
        self.scene.text(
            Rect::from_xywh(r.right - value_width, r.top, value_width, 32.0),
            label,
            Tokens::VALUE_TEXT,
        );
    }
    pub fn button(&mut self, title: &str, description: &str, label: &str, action: Action) {
        let r = self.card(title, description, 112.0);
        self.scene.button(r, label, action, false);
    }
    pub fn preview(&mut self, name: &str, color: u32, opacity: f32) {
        self.material_preview(name, Backdrop::Solid { color, opacity });
    }
    pub fn material_preview(&mut self, name: &str, material: Backdrop) {
        self.scene
            .cards
            .push(Rect::from_xywh(self.x, self.y, self.width, 152.0));
        self.scene.previews.push((
            Rect::from_xywh(self.x + 16.0, self.y + 16.0, 200.0, 120.0),
            material,
        ));
        self.scene.text(
            Rect::from_xywh(self.x + 232.0, self.y + 16.0, self.width - 248.0, 28.0),
            name,
            1,
        );
        let description = match material.base() {
            Backdrop::Mica => "柔和的壁纸色调，保持内容清晰。",
            Backdrop::MicaAlt => "更明显的壁纸色调与深一层的底色。",
            Backdrop::Solid { .. } => "自定义颜色与不透明度。",
            _ => "磨砂玻璃质感，透出背景层次。",
        };
        self.scene.text(
            Rect::from_xywh(self.x + 232.0, self.y + 48.0, self.width - 248.0, 40.0),
            description,
            6,
        );
        self.scene.text(
            Rect::from_xywh(self.x + 232.0, self.y + 92.0, self.width - 248.0, 44.0),
            "示意预览 · 实际效果随桌面背景变化",
            6,
        );
        self.y += 160.0;
    }
}

impl Scene {
    pub fn scroll_to(&mut self, width: f32, height: f32, offset: &mut f32) {
        let viewport = Rect::from_xywh(
            Tokens::CONTENT_X,
            TITLE_HEIGHT,
            width - Tokens::CONTENT_X,
            height - TITLE_HEIGHT,
        );
        let bottom = self
            .text
            .iter()
            .map(|(r, _, _)| r)
            .chain(self.cards.iter())
            .chain(
                self.controls
                    .iter()
                    .filter(|c| !matches!(c.kind, ControlKind::Caption))
                    .map(|c| &c.bounds),
            )
            .filter(|r| r.left >= Tokens::CONTENT_X)
            .map(|r| r.bottom)
            .chain(self.app_icon.iter().map(|r| r.bottom))
            .fold(viewport.top, f32::max);
        self.scroll_max = (bottom + Tokens::MARGIN - viewport.bottom).max(0.0);
        *offset = offset.clamp(0.0, self.scroll_max);
        self.scroll_offset = *offset;
        self.viewport = Some(viewport);
        let translate = |r: &mut Rect| {
            if r.left >= Tokens::CONTENT_X {
                r.top -= *offset;
                r.bottom -= *offset;
            }
        };
        for (r, _, _) in &mut self.text {
            translate(r);
        }
        for r in self.cards.iter_mut().chain(self.separators.iter_mut()) {
            translate(r);
        }
        for (r, _) in &mut self.previews {
            translate(r);
        }
        if let Some(r) = &mut self.app_icon {
            translate(r);
        }
        for c in &mut self.controls {
            if !matches!(c.kind, ControlKind::Caption) {
                translate(&mut c.bounds);
            }
        }
    }
    pub fn scroll_thumb(&self) -> Option<Rect> {
        let viewport = self.viewport?;
        if self.scroll_max <= 0.0 {
            return None;
        }
        let track = viewport.bottom - viewport.top - 16.0;
        let height = (track * track / (track + self.scroll_max)).max(28.0);
        let top = viewport.top + 8.0 + (track - height) * self.scroll_offset / self.scroll_max;
        Some(Rect::from_xywh(viewport.right - 10.0, top, 4.0, height))
    }
    pub fn accepts_pointer(&self, c: &Control, x: f32, y: f32) -> bool {
        contains(&c.bounds, x, y)
            && (matches!(c.kind, ControlKind::Caption)
                || c.bounds.left < Tokens::CONTENT_X
                || self.viewport.is_none_or(|v| contains(&v, x, y)))
    }
}

pub(super) struct ContentClip<'a, 'b> {
    pass: &'a canvas::DrawPass<'b>,
    active: bool,
}
impl<'a, 'b> ContentClip<'a, 'b> {
    pub fn new(pass: &'a canvas::DrawPass<'b>, scene: &Scene, content: bool) -> Self {
        let active = content && scene.viewport.is_some();
        if active {
            pass.push_clip(&scene.viewport.unwrap());
        }
        Self { pass, active }
    }
}
impl Drop for ContentClip<'_, '_> {
    fn drop(&mut self) {
        if self.active {
            self.pass.pop_clip();
        }
    }
}
