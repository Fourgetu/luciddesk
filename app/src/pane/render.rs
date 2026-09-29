//! Cached Canvas content. Live windows draw directly into the GPU swap chain.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
use windows_canvas::{ColorF, Ellipse, Rect, RoundedRect, Vector2};

use super::native_graphics::canvas_result;
use super::{GroupModel, assets, canvas, layout::HEADER};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use windows_canvas::ID2D1DeviceContext;

use windows::core::Result;
type ImageBitmap = (Arc<assets::Pixels>, windows_canvas::Bitmap, (u32, u32));

struct TitleLayout {
    text: String,
    natural_width: f32,
    width_pixels: f32,
    scale: f32,
    layout: windows_canvas::TextLayout,
}

pub struct Renderer {
    family: String,
    #[cfg(test)]
    offscreen_device: Option<windows_canvas::GpuDevice>,
    labels: windows_canvas::TextFormat,
    title: windows_canvas::TextFormat,
    title_layout: Option<TitleLayout>,
    details: windows_canvas::TextFormat,
    tab_title: windows_canvas::TextFormat,
    column_label_widths: [f32; 4],
    sort_icons: windows_canvas::TextFormat,
    menu_shortcut: windows_canvas::TextFormat,
    icons: windows_canvas::TextFormat,
    navigation_icons: windows_canvas::TextFormat,
    target: Option<(u32, u32, Option<canvas::Offscreen>, ID2D1DeviceContext)>,
    images: HashMap<usize, ImageBitmap>,
    states: HashMap<(u32, u32, u32, i32), windows_canvas::Bitmap>,
}

impl Renderer {
    #[cfg(test)]
    pub fn flyout(
        &mut self,
        width: u32,
        height: u32,
        scale: f32,
        rows: &[super::menu::Entry],
        selected: Option<usize>,
        native: bool,
        dark: bool,
    ) -> Result<Vec<u8>> {
        self.prepare(width, height, scale)?;
        let (_, _, bitmap, target) = self.target.as_ref().unwrap();
        let highlights: Vec<_> = (0..rows.len())
            .map(|i| if selected == Some(i) { 1.0 } else { 0.0 })
            .collect();
        self.paint_flyout(
            target,
            width,
            height,
            scale,
            rows,
            &highlights,
            native,
            dark,
        )?;
        bitmap.as_ref().unwrap().pixels()
    }

    pub fn paint_flyout(
        &self,
        target: &ID2D1DeviceContext,
        width: u32,
        height: u32,
        scale: f32,
        rows: &[super::menu::Entry],
        highlights: &[f32],
        native: bool,
        dark: bool,
    ) -> Result<()> {
        canvas::draw(target, scale, |target| {
            let ink = if dark { 0.95 } else { 0.10 };
            let text = canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 1.0)))?;
            let subtle =
                canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 0.13)))?;
            target.clear(ColorF::new(0.0, 0.0, 0.0, 0.0));
            let width = width as f32 / scale;
            if !native {
                let background = canvas_result(target.create_solid_brush(ColorF::new(
                    if dark { 0.09 } else { 0.96 },
                    if dark { 0.10 } else { 0.96 },
                    if dark { 0.12 } else { 0.96 },
                    1.0,
                )))?;
                target.fill_rounded_rect(&RoundedRect {
                    rect: Rect::from_xywh(0.0, 0.0, width, height as f32 / scale),
                    radius_x: super::menu::CORNER_RADIUS + 0.5,
                    radius_y: super::menu::CORNER_RADIUS + 0.5,
                }, &background);
            }
            target.draw_rounded_rect(
                &RoundedRect {
                    rect: Rect::from_xywh(0.5, 0.5, width - 1.0, height as f32 / scale - 1.0),
                    radius_x: super::menu::CORNER_RADIUS,
                    radius_y: super::menu::CORNER_RADIUS,
                },
                &subtle,
                1.0,
            );
            for (index, row) in rows.iter().enumerate() {
                let top = super::menu::row_top(rows, index);
                if row.id == 0 {
                    target.fill_rect(
                        &Rect::from_xywh(12.0, top + 4.0, width - 24.0, 1.0),
                        &subtle,
                    );
                    continue;
                }
                let progress = highlights.get(index).copied().unwrap_or(0.0);
                if progress > 0.0 {
                    let hover = canvas_result(target.create_solid_brush(ColorF::new(
                        ink,
                        ink,
                        ink,
                        (if dark { 0.09 } else { 0.05 }) * progress,
                    )))?;
                    target.fill_rounded_rect(
                        &RoundedRect {
                            rect: Rect::from_xywh(
                                4.0,
                                top + 2.0,
                                width - 8.0,
                                super::menu::ROW_HEIGHT - 4.0,
                            ),
                            radius_x: 5.0,
                            radius_y: 5.0,
                        },
                        &hover,
                    );
                }
                let trailing_width = if !row.children.is_empty() { 24.0 } else if row.trailing.is_empty() { 0.0 } else { 72.0 };
                for (label, left, available) in [
                    (row.icon, 12.0, 20.0),
                    (row.label, 38.0, width - 50.0 - trailing_width),
                ] {
                    if label.chars().next().is_some_and(|ch| ('\u{e000}'..='\u{f8ff}').contains(&ch)) {
                        target.clipped_icon(label, &self.icons,
                            &Rect::from_xywh(left, top, available, super::menu::ROW_HEIGHT), &text);
                        continue;
                    }
                    target.clipped_text(
                        label,
                        &self.title,
                        &Rect::from_xywh(left, top, available, super::menu::ROW_HEIGHT),
                        &text,
                    );
                }
                if !row.children.is_empty() {
                    target.clipped_icon("\u{e76c}", &self.icons,
                        &Rect::from_xywh(width - 28.0, top, 16.0, super::menu::ROW_HEIGHT), &text);
                } else if trailing_width > 0.0 {
                    target.clipped_text(
                        row.trailing,
                        &self.menu_shortcut,
                        &Rect::from_xywh(width - 76.0, top, 64.0, super::menu::ROW_HEIGHT),
                        &text,
                    );
                }
            }
            target.finish()
        })
    }
    pub fn new() -> Result<Self> {
        use windows_canvas::{ParagraphAlignment, TextAlignment, TextFormat, WordWrapping};
        let (family, size) = assets::font();
        let labels = canvas_result(TextFormat::new(&family, size))?
            .with_alignment(TextAlignment::Center)
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .with_word_wrapping(WordWrapping::Wrap);
        canvas::ellipsis(&labels)?;
        let title = canvas_result(TextFormat::new(&family, size + 1.0))?
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .with_word_wrapping(WordWrapping::NoWrap);
        canvas::ellipsis(&title)?;
        let details = canvas_result(TextFormat::new(&family, 12.0))?
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .with_word_wrapping(WordWrapping::NoWrap);
        canvas::ellipsis(&details)?;
        let tab_title = canvas_result(TextFormat::new(&family, 12.0))?
            .with_alignment(TextAlignment::Center)
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .with_word_wrapping(WordWrapping::NoWrap);
        canvas::ellipsis(&tab_title)?;
        let mut column_label_widths = [0.0; 4];
        for (index, name) in [crate::i18n::text("ui-name"), crate::i18n::text("ui-type"), crate::i18n::text("ui-modified"), crate::i18n::text("ui-size")].iter().enumerate() {
            column_label_widths[index] = canvas_result(windows_canvas::TextLayout::new(
                name, &details, 256.0, super::layout::LIST_HEADER,
            ))?.metrics().width_including_trailing_whitespace;
        }
        Ok(Self {
            family: family.clone(),
            #[cfg(test)]
            offscreen_device: None,
            labels,
            title,
            title_layout: None,
            details,
            tab_title,
            column_label_widths,
            sort_icons: canvas_result(TextFormat::new(super::fonts::icon_family(), 10.0))?
                .with_alignment(TextAlignment::Center)
                .with_paragraph_alignment(ParagraphAlignment::Center)
                .with_word_wrapping(WordWrapping::NoWrap),
            menu_shortcut: canvas_result(TextFormat::new(&family, 12.0))?
                .with_alignment(TextAlignment::Trailing)
                .with_paragraph_alignment(ParagraphAlignment::Center)
                .with_word_wrapping(WordWrapping::NoWrap),
            icons: canvas_result(TextFormat::new(super::fonts::icon_family(), 12.0))?
                .with_alignment(windows_canvas::TextAlignment::Center)
                .with_paragraph_alignment(ParagraphAlignment::Center)
                .with_word_wrapping(WordWrapping::NoWrap),
            navigation_icons: canvas_result(TextFormat::new(super::fonts::icon_family(), 12.0))?
                .with_alignment(TextAlignment::Center)
                .with_paragraph_alignment(ParagraphAlignment::Center)
                .with_word_wrapping(WordWrapping::NoWrap),
            target: None,
            images: HashMap::new(),
            states: HashMap::new(),
        })
    }

    #[cfg(test)]
    fn prepare(&mut self, width: u32, height: u32, _scale: f32) -> Result<()> {
        if self
            .target
            .as_ref()
            .is_none_or(|(w, h, bitmap, _)| *w != width || *h != height || bitmap.is_none())
        {
            if self.offscreen_device.is_none() {
                self.offscreen_device = Some(super::native_graphics::gpu_device()?);
            }
            let bitmap =
                canvas::Offscreen::new(self.offscreen_device.as_ref().unwrap(), width, height)?;
            let target = bitmap.target.clone();
            self.target = Some((width, height, Some(bitmap), target));
            self.images.clear();
            self.states.clear();
        }
        Ok(())
    }

    /// CPU readback is reserved for small flyouts, export and regression tests.
    #[cfg(test)]
    pub fn pixels(
        &mut self,
        width: u32,
        height: u32,
        scale: f32,
        model: &GroupModel,
    ) -> Result<Vec<u8>> {
        self.prepare(width, height, scale)?;
        self.draw(width, height, scale, model)?;
        self.target.as_ref().unwrap().2.as_ref().unwrap().pixels()
    }

    pub fn paint(
        &mut self,
        target: &ID2D1DeviceContext,
        width: u32,
        height: u32,
        scale: f32,
        model: &GroupModel,
    ) -> Result<()> {
        if self
            .target
            .as_ref()
            .is_none_or(|(_, _, bitmap, old)| bitmap.is_some() || old != target)
        {
            self.images.clear();
            self.states.clear();
        }
        self.target = Some((width, height, None, target.clone()));
        let height_dip = height as f32 / scale;
        let grid = model.grid(width as f32 / scale, height_dip);
        // Keep the viewport plus one row for smooth scrolling, not every icon
        // ever visited in a large mapped folder. Source identity still detects
        // replaced icons without re-uploading unchanged visible textures.
        let candidates = grid.visible_indices(model.scroll, HEADER - grid.cell_height, height_dip + grid.cell_height, model.items.len());
        let live: HashSet<_> = model
            .items
            .iter()
            .enumerate()
            .skip(candidates.start)
            .take(candidates.len())
            .filter(|(index, _)| {
                let (_, y) = model.cell(grid, *index);
                height_dip > HEADER + 1.0
                    && y + grid.cell_height * 2.0 > HEADER
                    && y < height_dip + grid.cell_height
            })
            .filter_map(|(_, item)| {
                item.image
                    .as_ref()
                    .map(|image| Arc::as_ptr(image) as usize)
            })
            .collect();
        self.images
            .retain(|key, _| live.contains(key));
        self.draw(width, height, scale, model)
    }

    fn layout_title(
        &mut self,
        text: &str,
        available: f32,
        scale: f32,
    ) -> Result<windows_canvas::TextLayout> {
        if self
            .title_layout
            .as_ref()
            .is_none_or(|cached| cached.text != text)
        {
            let layout = canvas_result(windows_canvas::TextLayout::new(
                text,
                &self.title,
                1_000_000.0,
                HEADER,
            ))?;
            self.title_layout = Some(TitleLayout {
                text: text.into(),
                natural_width: layout.metrics().width_including_trailing_whitespace,
                width_pixels: 0.0,
                scale,
                layout,
            });
        }
        let cached = self.title_layout.as_mut().unwrap();
        // Draw this same layout instead of reshaping with DrawText and a
        // fractional rectangle; leave one physical pixel around a full title.
        let available_pixels = (available * scale).floor().max(1.0);
        let pixels = available_pixels.min((cached.natural_width * scale).ceil() + 1.0);
        // Shrink immediately, but require four spare physical pixels before
        // growing again so size jitter cannot toggle a final glyph and ellipsis.
        // Use uncapped space here so even a fully visible title can recover.
        if cached.width_pixels == 0.0
            || cached.scale != scale
            || pixels < cached.width_pixels
            || available_pixels >= cached.width_pixels + 4.0
        {
            cached.width_pixels = pixels;
            cached.scale = scale;
        }
        cached
            .layout
            .set_max_size(cached.width_pixels / scale, HEADER);
        Ok(cached.layout.clone())
    }

    #[allow(clippy::too_many_lines)]
    fn draw(&mut self, width: u32, height: u32, scale: f32, model: &GroupModel) -> Result<()> {
        if self.family != super::fonts::family() {
            let fresh = Self::new()?;
            self.family = fresh.family;
            self.labels = fresh.labels;
            self.title = fresh.title;
            self.tab_title = fresh.tab_title;
            self.details = fresh.details;
            self.menu_shortcut = fresh.menu_shortcut;
            self.column_label_widths = fresh.column_label_widths;
            self.title_layout = None;
            self.states.clear();
        }

        let mut live_states = HashSet::new();
        let w = width as f32 / scale;
        let (mut title_left, mut title_space) = super::layout::title_area(w);
        if model.folder.is_some() {
            title_space = (title_space - (74.0 - title_left).max(0.0)).max(1.0);
            title_left = title_left.max(74.0);
        }
        let show_icon = model.folder.is_some() && title_space >= 42.0;
        let icon_width = if model.folder.is_some() { 24.0 } else { 0.0 };
        let title = self.layout_title(&model.title, (title_space - icon_width).max(1.0), scale)?;
        let group_left =
            ((title_left + (title_space - title.max_size().0 - icon_width).max(0.0) / 2.0) * scale)
                .floor()
                / scale;
        {
            let (_, _, _, target) = self.target.as_ref().unwrap();
            canvas::draw(target, scale, |target| {
                let (w, h) = (width as f32 / scale, height as f32 / scale);
                let contrast = super::theme::panel_contrast(
                    model.backdrop,
                    model.dark,
                    model.options.text,
                    model.native_material,
                );
                let opacity = if model.native_material {
                    if model.options.text_protection {
                        contrast.scrim
                    } else {
                        0.0
                    }
                } else {
                    1.0
                };
                let base = contrast.base();
                let ink = contrast.ink();
                let background = canvas_result(
                    target.create_solid_brush(
                        if !model.native_material { super::theme::mica_fallback(model.backdrop, model.dark) } else { None }
                            .unwrap_or(ColorF::new(base, base, base, opacity))),
                )?;
                let outline = canvas_result(
                    target
                        .create_solid_brush(super::theme::panel_border(model.dark, model.backdrop)),
                )?;
                let white =
                    canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 1.0)))?;
                let dim =
                    canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 0.85)))?;
                let selection =
                    canvas_result(target.create_solid_brush(ColorF::new(0.55, 0.75, 1.0, 0.25)))?;
                let hover =
                    canvas_result(target.create_solid_brush(ColorF::new(0.7, 0.85, 1.0, 0.12)))?;
                let inactive =
                    canvas_result(target.create_solid_brush(ColorF::new(0.75, 0.8, 0.85, 0.16)))?;
                target.clear(ColorF::new(0.0, 0.0, 0.0, 0.0));
                let rounded = RoundedRect {
                    rect: Rect::from_xywh(0.5, 0.5, w - 1.0, h - 1.0),
                    radius_x: model.options.corner_radius,
                    radius_y: model.options.corner_radius,
                };
                {
                    target.fill_rounded_rect(&rounded, &background);
                    if model.options.border {
                        // One physical pixel, fully inside the client bounds at every DPI.
                        let stroke = 1.0 / scale;
                        let inset = stroke * 0.5;
                        let radius = (model.options.corner_radius - inset).max(0.0);
                        target.draw_rounded_rect(&RoundedRect {
                            rect: Rect::from_xywh(inset, inset, w - stroke, h - stroke),
                            radius_x: radius,
                            radius_y: radius,
                        }, &outline, stroke);
                    }
                    if show_icon && model.tabs.len() < 2 && model.merge_preview.is_empty() {
                        target.clipped_icon(
                            "\u{e8b7}",
                            &self.icons,
                            &Rect::from_xywh(group_left, 0.0, 18.0, HEADER),
                            &white,
                        );
                    }
                    if model.tabs.len() < 2 && model.merge_preview.is_empty() {
                        target.clipped_layout(&title, group_left + icon_width, canvas::text_ink_center_offset(&title)?, &white);
                    }
                    for button in 0..if !model.merge_preview.is_empty() { 0 } else if model.folder.is_some() { 4 } else { 2 } {
                        let top = super::layout::HEADER_INSET;
                        let button_height = HEADER - top * 2.0;
                        let center_y = HEADER / 2.0;
                        if model.tabs.len() > 1 && button < 2 { continue; }
                        let x = model.header_button_x(w, button);
                        let enabled = model.header_button_enabled(button);
                        let hovered = enabled && model.hovered_button == Some(button);
                        let glyph = canvas_result(target.create_solid_brush(ColorF::new(
                            ink,
                            ink,
                            ink,
                            if !enabled { 0.3 } else if hovered { 1.0 } else { 0.85 },
                        )))?;
                        if hovered {
                            let fill = canvas_result(
                                target.create_solid_brush(ColorF::new(ink, ink, ink, 0.07)),
                            )?;
                            target.fill_rounded_rect(
                                &RoundedRect {
                                    rect: Rect::from_xywh(x + 1.0, top, 26.0, button_height),
                                    radius_x: 5.0,
                                    radius_y: 5.0,
                                },
                                &fill,
                            );
                        }
                        let center = x + 14.0;
                        if button >= 2 {
                            target.clipped_icon(
                                if button == 2 { "\u{e72b}" } else { "\u{e80f}" },
                                &self.navigation_icons,
                                &Rect::from_xywh(x, top, 28.0, button_height),
                                &glyph,
                            );
                        } else if button == 0 {
                            let amount = model.reveal.clamp(0.0, 1.0);
                            let points = [
                                (center - 2.0 - 2.0 * amount, 15.0 + 2.0 * amount),
                                (center + 2.0 - 2.0 * amount, 19.0 + 2.0 * amount),
                                (center - 2.0 + 6.0 * amount, 23.0 - 6.0 * amount),
                            ];
                            for pair in points.windows(2) {
                                target.draw_line(
                                    Vector2 {
                                        x: center + (pair[0].0 - center) * 1.25,
                                        y: center_y + (pair[0].1 - 19.0) * 1.25,
                                    },
                                    Vector2 {
                                        x: center + (pair[1].0 - center) * 1.25,
                                        y: center_y + (pair[1].1 - 19.0) * 1.25,
                                    },
                                    &glyph,
                                    1.5,
                                );
                            }
                        } else {
                            for offset in [-4.5, 0.0, 4.5] {
                                target.fill_ellipse(
                                    &Ellipse {
                                        center: Vector2 {
                                            x: center + offset,
                                            y: center_y,
                                        },
                                        radius_x: 1.1,
                                        radius_y: 1.1,
                                    },
                                    &glyph,
                                );
                            }
                        }
                    }
                }
                let mut chrome = super::theme::material_chrome(model.backdrop, model.dark);
                if !matches!(model.backdrop.base(), desktop_core::Backdrop::Acrylic | desktop_core::Backdrop::Mica | desktop_core::Backdrop::MicaAlt) {
                    chrome.tab_active = ColorF::new(ink, ink, ink, 0.14);
                    chrome.tab_inactive = ColorF::new(ink, ink, ink, 0.04);
                    chrome.tab_hover = ColorF::new(ink, ink, ink, 0.09);
                    chrome.tab_incoming = ColorF::new(ink, ink, ink, 0.09);
                }
                for (id, bounds) in super::tabs::strip(model, w).into_iter().filter(|_| model.merge_preview.is_empty()) {
                    let rect = Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height);
                    let fill = canvas_result(target.create_solid_brush(
                        if id == model.active_tab { chrome.tab_active } else if model.hovered_tab == Some(id) { chrome.tab_hover } else { chrome.tab_inactive }))?;
                    target.fill_rounded_rect(&RoundedRect { rect, radius_x: model.options.corner_radius, radius_y: model.options.corner_radius }, &fill);
                    if id == model.active_tab || model.hovered_tab == Some(id) {
                        let edge = canvas_result(target.create_solid_brush(chrome.card_border))?;
                        target.draw_rounded_rect(&RoundedRect { rect, radius_x: model.options.corner_radius, radius_y: model.options.corner_radius }, &edge, 1.0);
                    }
                    let text = model.tabs.iter().find(|(tab, _)| *tab == id).map_or("", |(_, title)| title.as_str());
                    target.clipped_text(text, &self.tab_title, &Rect::from_xywh(rect.left + 10.0, rect.top, (bounds.width - 20.0).max(1.0), bounds.height),
                        if id == model.active_tab { &white } else { &dim });
                }
                for (text, incoming, active, bounds) in super::tabs::merge_strip(model, w) {
                    let rect = Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height);
                    let fill = canvas_result(target.create_solid_brush(
                        if incoming { chrome.tab_incoming } else if active { chrome.tab_active } else { chrome.tab_inactive }))?;
                    target.fill_rounded_rect(&RoundedRect { rect, radius_x: 5.0, radius_y: 5.0 }, &fill);
                    if incoming {
                        let edge = canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 0.24 + chrome.border.a)))?;
                        // Short neutral dashes keep provisional tabs distinct without an accent color.
                        let mut x = rect.left + 5.0;
                        while x < rect.right - 5.0 {
                            for y in [rect.top + 0.5, rect.bottom - 0.5] {
                                target.draw_line(Vector2::new(x, y), Vector2::new((x + 4.0).min(rect.right - 5.0), y), &edge, 1.0);
                            }
                            x += 7.0;
                        }
                        let mut y = rect.top + 5.0;
                        while y < rect.bottom - 5.0 {
                            for x in [rect.left + 0.5, rect.right - 0.5] {
                                target.draw_line(Vector2::new(x, y), Vector2::new(x, (y + 4.0).min(rect.bottom - 5.0)), &edge, 1.0);
                            }
                            y += 7.0;
                        }
                    }
                    target.clipped_text(&text, &self.tab_title, &Rect::from_xywh(rect.left + 10.0, rect.top,
                        (bounds.width - 20.0).max(1.0), bounds.height), if active || incoming { &white } else { &dim });
                }
                if super::header_divider::enabled() && !model.collapsed && h > model.content_header() + 1.0 {
                    let divider = canvas_result(target.create_solid_brush(super::theme::panel_divider(model.dark, model.backdrop)))?;
                    let inset = super::layout::HEADER_INSET;
                    let divider_y = ((model.content_header() - inset + 4.0) * scale).round() / scale;
                    target.fill_rect(&Rect::from_xywh(inset, divider_y, (w - inset * 2.0).max(0.0), 1.0 / scale), &divider);
                }
                if h > model.content_header() + 1.0 {
                    target.push_clip(&{
                        Rect::from_xywh(6.0, model.content_header(), w - 12.0, (h - model.content_header() - 6.0).max(0.0))
                    });
                    let grid = model.grid(w, h);
                    let list = model.is_list();
                    let columns = model.list_columns(grid.cell_width);
                    if list && model.folder.is_some() {
                        for column in 1..4 {
                            if model.folder_visible_columns & (1 << column) == 0 { continue; }
                            let boundary = columns[column];
                            target.fill_rect(
                                &Rect::from_xywh(
                                    super::layout::PADDING + boundary,
                                    grid.content_top - super::layout::LIST_HEADER + 6.0,
                                    1.0, super::layout::LIST_HEADER - 12.0,
                                ),
                                &hover,
                            );
                        }
                        for (column, name) in [crate::i18n::text("ui-name"), crate::i18n::text("ui-type"), crate::i18n::text("ui-modified"), crate::i18n::text("ui-size")].iter().enumerate()
                        {
                            if column > 0 && model.folder_visible_columns & (1 << column) == 0 { continue; }
                            let inset = if column == 0 { 0.0 } else { 8.0 };
                            let left = super::layout::PADDING + columns[column] + inset;
                            let top = grid.content_top - super::layout::LIST_HEADER;
                            let available = (columns[column + 1] - columns[column] - inset - 8.0).max(1.0);
                            let sorted = model.folder_sort.0 as usize == column && available >= 30.0;
                            let text_width = if sorted { available - 18.0 } else { available };
                            target.clipped_text(
                                name,
                                &self.details,
                                &Rect::from_xywh(
                                    left, top, text_width,
                                    super::layout::LIST_HEADER,
                                ),
                                &dim,
                            );
                            if sorted {
                                target.clipped_icon(
                                    if model.folder_sort.1 { "\u{e70d}" } else { "\u{e70e}" },
                                    &self.sort_icons,
                                    &Rect::from_xywh(
                                        left + self.column_label_widths[column].min(text_width) + 4.0,
                                        top, 14.0, super::layout::LIST_HEADER,
                                    ),
                                    &dim,
                                );
                            }
                        }
                        target.fill_rect(
                            &Rect::from_xywh(
                                super::layout::PADDING,
                                grid.content_top - 1.0,
                                grid.cell_width,
                                1.0,
                            ),
                            &hover,
                        );
                    }
                    for index in grid.visible_indices(model.scroll, HEADER, h, model.items.len()) {
                        let item = &model.items[index];
                        let (x, y) = model.cell(grid, index);
                        if list && y < grid.content_top {
                            continue;
                        }
                        if y + grid.cell_height <= { HEADER } || y >= h {
                            continue;
                        }
                        let bounds = model.selection_bounds(grid, index, scale);
                        let selection_height = bounds.height;
                        let selection_width = bounds.width;
                        let selection_x = bounds.x;
                        if model.selection.contains(&index) || model.hovered_item == Some(index) {
                            let state = if model.selection.contains(&index) {
                                if !model.focused {
                                    5
                                } else if model.hovered_item == Some(index) {
                                    6
                                } else {
                                    3
                                }
                            } else {
                                2
                            };
                            let key = (
                                (selection_width * scale).round() as u32,
                                (selection_height * scale).round() as u32,
                                (96.0 * scale).round() as u32,
                                state,
                            );
                            live_states.insert(key);
                            if !self.states.contains_key(&key)
                                && let Some(pixels) =
                                    super::theme::selection(key.0, key.1, key.2, key.3)
                            {
                                let bitmap = canvas_result(target.create_bitmap(
                                    &pixels.data,
                                    key.0,
                                    key.1,
                                ))?;
                                if self.states.len() >= 64 {
                                    if let Some(old) = self.states.keys().next().copied() {
                                        self.states.remove(&old);
                                    }
                                }
                                self.states.insert(key, bitmap);
                            }
                            if let Some(bitmap) = self.states.get(&key) {
                                target.draw_bitmap(
                                    bitmap,
                                    &Rect::from_xywh(
                                        selection_x,
                                        y,
                                        selection_width,
                                        selection_height,
                                    ),
                                    1.0,
                                );
                            } else {
                                target.fill_rounded_rect(
                                    &RoundedRect {
                                        rect: Rect::from_xywh(
                                            selection_x,
                                            y,
                                            selection_width,
                                            selection_height,
                                        ),
                                        radius_x: 0.0,
                                        radius_y: 0.0,
                                    },
                                    if model.selection.contains(&index) {
                                        if model.focused { &selection } else { &inactive }
                                    } else {
                                        &hover
                                    },
                                );
                            }
                        }
                        if let Some(image) = &item.image {
                            let key = Arc::as_ptr(image) as usize;
                            let ratio =
                                grid.icon_size * scale / image.width.max(image.height) as f32;
                            let size = (
                                (image.width as f32 * ratio).round().max(1.0) as u32,
                                (image.height as f32 * ratio).round().max(1.0) as u32,
                            );
                            if self.images.get(&key).is_none_or(|(source, _, old_size)| {
                                !Arc::ptr_eq(source, image) || *old_size != size
                            }) {
                                let pixels = super::scaled_icons::resample(image, size.0, size.1)?;
                                let bitmap = canvas_result(target.create_bitmap(
                                    &pixels.data,
                                    pixels.width,
                                    pixels.height,
                                ))?;
                                self.images
                                    .insert(key, (Arc::clone(image), bitmap, size));
                                if std::env::var_os("LUCIDPANE_ICON_TRACE").is_some()
                                    && item.identity.persistent_key()
                                        .to_ascii_lowercase()
                                        .contains("645ff040-5081-101b-9f08-00aa002f954e")
                                {
                                    eprintln!(
                                        "recycle-render-upload hash={:x} size={:?}",
                                        image.data.iter().fold(0u64, |h, b| h
                                            .wrapping_mul(31)
                                            .wrapping_add(u64::from(*b))),
                                        size
                                    );
                                }
                            }
                            let (iw, ih) = (size.0 as f32 / scale, size.1 as f32 / scale);
                            let left = ((if list {
                                x + 4.0
                            } else {
                                x + (grid.cell_width - iw) / 2.0
                            }) * scale)
                                .round()
                                / scale;
                            let top = ((if list {
                                y + (grid.cell_height - ih) / 2.0
                            } else {
                                y + 2.0 + (grid.icon_size - ih) / 2.0
                            }) * scale)
                                .round()
                                / scale;
                            target.draw_bitmap(
                                &self.images[&key].1,
                                &Rect::from_xywh(left, top, iw, ih),
                                1.0,
                            );
                        } else {
                            // Enumeration is published before Shell image extraction.
                            // Keep a visible placeholder while loading or after a failure.
                            target.clipped_icon(
                                if item.details.folder { "\u{e8b7}" } else { "\u{e8a5}" },
                                &self.icons,
                                &Rect::from_xywh(
                                    if list { x + 4.0 } else { x + (grid.cell_width - grid.icon_size) / 2.0 },
                                    y, grid.icon_size,
                                    if list { grid.cell_height } else { grid.icon_size },
                                ),
                                &white,
                            );
                        }

                        if list {
                            let size = super::folder::size_text(item.details.size, item.details.folder);
                            for (column, text) in
                                [&item.label, &item.details.kind, &item.details.modified, &size]
                                    .iter()
                                    .enumerate()
                            {
                                if column > 0 && model.folder.is_none() {
                                    continue;
                                }
                                if column > 0 && model.folder_visible_columns & (1 << column) == 0 { continue; }
                                if column == 0 && model.renaming.as_ref() == Some(&item.identity) {
                                    continue;
                                }
                                let inset = if column == 0 { 0.0 } else { 8.0 };
                                target.clipped_text(
                                    text,
                                    if column == 3 { &self.menu_shortcut } else { &self.details },
                                    &Rect::from_xywh(
                                        x + columns[column] + inset,
                                        y,
                                        (columns[column + 1] - columns[column] - inset - 8.0).max(1.0),
                                        grid.cell_height,
                                    ),
                                    if column == 0 { &white } else { &dim },
                                );
                            }
                            continue;
                        }
                        if model.renaming.as_ref() == Some(&item.identity) {
                            continue;
                        }
                        let (layout, _) = super::label::layout_scaled(
                            &item.label,
                            (grid.cell_width * scale).round() as u32,
                            (96.0 * scale).round() as u32,
                            2,
                            grid.text_scale,
                        )?;
                        target.clipped_layout(
                            &layout,
                            (x * scale).round() / scale + 2.0,
                            ((y + grid.icon_size + crate::pane::layout::LABEL_OFFSET) * scale)
                                .round()
                                / scale,
                            &white,
                        );
                        continue;
                    }
                    if model.items.is_empty() {
                        let text = if let Some(status) = &model.folder_status {
                            status.as_str()
                        } else if model.folder.is_some() {
                            if model.loading {
                                crate::i18n::text("ui-loading-folder")
                            } else {
                                crate::i18n::text("ui-this-folder-is-empty")
                            }
                        } else if model.loading {
                            crate::i18n::text("ui-loading-desktop-items")
                        } else {
                            crate::i18n::text("ui-drag-icons-into-this-group")
                        };
                        target.clipped_text(
                            text,
                            &self.labels,
                            &Rect::from_xywh(20.0, 0.0, (w - 40.0).max(1.0), h),
                            &dim,
                        );
                    }
                    target.pop_clip();
                    if let Some(bar) = super::scrollbar::Bar::for_model(model, w, h) {
                        let expansion = model.scrollbar.expansion.clamp(0.0, 1.0);
                        let center = bar.left + super::scrollbar::Bar::WIDTH / 2.0;
                        if expansion > 0.0 {
                            let track_width = 2.0 + 4.0 * expansion;
                            let track = canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 0.06 * expansion)))?;
                            target.fill_rounded_rect(
                                &RoundedRect {
                                    rect: Rect::from_xywh(center - track_width / 2.0, bar.top, track_width, bar.height),
                                    radius_x: track_width / 2.0, radius_y: track_width / 2.0,
                                }, &track,
                            );
                        }
                        let width = 2.0 + 2.0 * expansion;
                        let thumb = canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 0.85 + 0.15 * expansion)))?;
                        target.fill_rounded_rect(
                            &RoundedRect {
                                rect: Rect::from_xywh(
                                    center - width / 2.0,
                                    bar.thumb_top, width, bar.thumb_height,
                                ),
                                radius_x: width / 2.0,
                                radius_y: width / 2.0,
                            },
                            &thumb,
                        );
                    }
                }
                let result = target.finish();
                // Retain only backgrounds used by this frame, including after resize,
                // DPI changes, deselection, or collapsing the content.
                self.states.retain(|key, _| live_states.contains(key));
                result
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn title_ellipsis_stays_stable_when_width_jitters_at_last_character() {
        use windows::Win32::Graphics::DirectWrite::{DWRITE_LINE_METRICS, IDWriteTextLayout};
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let trimmed = |title: &windows_canvas::TextLayout| {
            let native: IDWriteTextLayout =
                super::super::native_graphics::native_interface(title.raw()).unwrap();
            let mut lines = [DWRITE_LINE_METRICS::default(); 1];
            let mut count = 0;
            unsafe {
                native
                    .GetLineMetrics(Some(&mut lines), &raw mut count)
                    .unwrap();
            }
            lines[0].isTrimmed.as_bool()
        };
        let mut renderer = Renderer::new().unwrap();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for text in ["新建分组", "Project 项目文件夹与资料", "tinyMediaManager"] {
                let mut boundary = None;
                for width in (8..=600).rev() {
                    let title = renderer
                        .layout_title(text, (width as f32 + 0.01) / scale, scale)
                        .unwrap();
                    if trimmed(&title) {
                        boundary = Some(width);
                        break;
                    }
                }
                let boundary = boundary.expect("title must cross its actual trimming boundary");
                for offset in [1, 0, 2, 1, 0, 1, 2, 0] {
                    let title = renderer
                        .layout_title(text, ((boundary + offset) as f32 + 0.01) / scale, scale)
                        .unwrap();
                    assert!(
                        trimmed(&title),
                        "ellipsis toggled at scale {scale}, offset {offset}"
                    );
                }
                let title = renderer
                    .layout_title(text, (boundary as f32 + 4.01) / scale, scale)
                    .unwrap();
                assert!(
                    !trimmed(&title),
                    "full title must return when enough space is available"
                );
            }
        }
    }

    #[test]
    fn title_trimming_does_not_reverse_while_shrinking() {
        use windows::Win32::Graphics::DirectWrite::{DWRITE_LINE_METRICS, IDWriteTextLayout};
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut renderer = Renderer::new().unwrap();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for text in ["新建分组", "Project 项目文件夹与资料", "tinyMediaManager"] {
                let mut was_trimmed = false;
                let mut previous_end = u32::MAX;
                for width in (8..=600).rev() {
                    let title = renderer
                        .layout_title(text, width as f32 / scale, scale)
                        .unwrap();
                    let native: IDWriteTextLayout =
                        super::super::native_graphics::native_interface(title.raw()).unwrap();
                    let mut lines = [DWRITE_LINE_METRICS::default(); 1];
                    let mut count = 0;
                    unsafe {
                        native
                            .GetLineMetrics(Some(&mut lines), &raw mut count)
                            .unwrap();
                    }
                    let trimmed = lines[0].isTrimmed.as_bool();
                    assert!(
                        !was_trimmed || trimmed,
                        "ellipsis reverted at {width}px, scale {scale}"
                    );
                    let end = title
                        .hit_test_point(Vector2::new(title.max_size().0 - 0.25, HEADER / 2.0))
                        .text_position;
                    assert!(
                        end <= previous_end,
                        "shrinking revealed characters at {width}px"
                    );
                    was_trimmed = trimmed;
                    previous_end = end;
                }
                assert!(was_trimmed);
            }
        }
    }
    #[test]
    fn list_view_columns_render_and_share_scrolled_hit_geometry() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        model.folder = Some(std::path::PathBuf::from(r"C:\Documents"));
        assert!(!model.header_button_enabled(2));
        assert!(!model.header_button_enabled(3));
        model.folder_navigation = [true, true];
        assert!(model.header_button_enabled(2));
        assert!(model.header_button_enabled(3));
        model.list_view = true;
        model.items[0].label = "项目进度报告.txt".into();
        model.items[0].details = super::super::ItemDetails {
            kind: "文本文档".into(),
            modified: "2026/09/12 16:30".into(),
            size: Some(1536),
            ..Default::default()
        };
        model.items = vec![model.items[0].clone(); 20];
        let mut renderer = Renderer::new().unwrap();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            model.scroll = 3;
            let grid = model.grid(480.0, 300.0);
            let (x, y) = model.cell(grid, 3);
            assert_eq!(grid.columns, 1);
            assert_eq!(model.hit(grid, x + 4.0, y + 10.0, scale), Some(3));
            assert_eq!(
                model.hit(grid, x + grid.cell_width - 4.0, y + 10.0, scale),
                Some(3)
            );
            assert_eq!(model.hit(grid, x + 20.0, y - 2.0, scale), None);
            assert_eq!(
                model.hit(grid, x + 20.0, y + grid.cell_height + 2.0, scale),
                Some(4)
            );
            let width = (480.0 * scale) as u32;
            let height = (300.0 * scale) as u32;
            let pixels = renderer.pixels(width, height, scale, &model).unwrap();
            let mut blank = model.clone();
            let bar = super::super::scrollbar::Bar::for_model(&model, 480.0, 300.0).unwrap();
            let mut hovered = model.clone();
            hovered.scrollbar.hovered = true;
            hovered.scrollbar.expansion = 1.0;
            let active = renderer.pixels(width, height, scale, &hovered).unwrap();
            assert!(((bar.top * scale) as u32..((bar.top + bar.height) * scale) as u32).any(|row| {
                ((bar.left * scale) as u32..((bar.left + super::super::scrollbar::Bar::WIDTH) * scale) as u32).any(|col| {
                    let at = ((row * width + col) * 4) as usize;
                    pixels[at..at + 4] != active[at..at + 4]
                })
            }), "scrollbar hover must be visible at scale {scale}");
            for item in &mut blank.items {
                item.label.clear();
                item.details = Default::default();
            }
            let empty = renderer.pixels(width, height, scale, &blank).unwrap();
            let columns = super::super::layout::list_columns(grid.cell_width);
            for column in 0..4 {
                let changed = (((y + 2.0) * scale) as u32
                    ..((y + grid.cell_height - 2.0) * scale) as u32)
                    .any(|row| {
                        (((x + columns[column]) * scale) as u32
                            ..((x + columns[column + 1] - 8.0) * scale) as u32)
                            .any(|col| {
                                let at = ((row * width + col) * 4) as usize;
                                pixels[at..at + 4] != empty[at..at + 4]
                            })
                    });
                assert!(
                    changed,
                    "column {column} must contain rendered text at scale {scale}"
                );
            }
        }
    }
    #[test]
    fn desktop_list_renders_full_width_names_and_row_drag_preview() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        model.list_view = true;
        model.items[0].label = "普通分组中较长的文件名称 — desktop document.txt".into();
        model.items = vec![model.items[0].clone(); 20];
        model.scroll = 3;
        let mut renderer = Renderer::new().unwrap();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let grid = model.grid(480.0, 300.0);
            let (x, y) = model.cell(grid, 3);
            assert_eq!(model.hit(grid, x + grid.cell_width - 2.0, y + 10.0, scale), Some(3));
            assert_eq!(model.hit(grid, x + 20.0, y - 2.0, scale), None);
            let pixels = renderer.pixels((480.0 * scale) as u32, (300.0 * scale) as u32, scale, &model).unwrap();
            let mut blank = model.clone();
            for item in &mut blank.items { item.label.clear(); }
            let empty = renderer.pixels((480.0 * scale) as u32, (300.0 * scale) as u32, scale, &blank).unwrap();
            assert_ne!(pixels, empty);
            let icon = assets::Pixels { width: 1, height: 1, data: vec![255; 4] };
            let preview = super::super::drag_drop::image::list_item_pixels(&icon, &model.items[0].label, grid, scale).unwrap();
            assert_eq!(preview.height, (grid.cell_height * scale).round() as u32);
            assert!(preview.data.chunks_exact(4).any(|p| p[3] > 0));
        }
    }

    #[test]
    fn grid_scale_changes_icon_and_text_size_together() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        let mut renderer = Renderer::new().unwrap();
        for dpi_scale in [1.0, 1.25, 1.5, 2.0] {
            let mut previous_text_height = 0.0;
            for percent in [50.0, 100.0, 150.0, 200.0] {
                model.options.grid_scale = percent;
                let grid = model.grid(600.0, 500.0);
                assert_eq!(grid.icon_size, model.icon_size * percent / 100.0);
                let (_, text_height) = super::super::label::layout_scaled(
                    "Desktop", (grid.cell_width * dpi_scale).round() as u32,
                    (96.0 * dpi_scale).round() as u32, 2, grid.text_scale,
                ).unwrap();
                assert!(text_height > previous_text_height, "glyph height must scale at {percent}%");
                previous_text_height = text_height;
                renderer.pixels((600.0 * dpi_scale) as u32, (500.0 * dpi_scale) as u32, dpi_scale, &model).unwrap();
                let icon = assets::Pixels { width: 1, height: 1, data: vec![255; 4] };
                let preview = super::super::drag_drop::image::item_pixels(&icon, "Desktop", grid, dpi_scale).unwrap();
                assert!(preview.height as f32 > grid.icon_size * dpi_scale);
            }
        }
    }
    use super::*;

    #[test]
    fn icon_pixels_remain_sharp_at_fractional_dpi_and_invalidate_size_cache() {
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        let mut data = vec![0; 48 * 48 * 4];
        for y in 4..44usize {
            for x in 4..44usize {
                let value = if (x / 3 + y / 3) % 2 == 0 { 255 } else { 0 };
                data[(y * 48 + x) * 4..(y * 48 + x + 1) * 4]
                    .copy_from_slice(&[value, value, value, 255]);
            }
        }
        model.items[0].image = Some(Arc::new(assets::Pixels {
            width: 48,
            height: 48,
            data,
        }));
        model.clear_selection();
        model.hovered_item = None;
        let mut renderer = Renderer::new().unwrap();
        for scale in [1.0, 1.25, 1.5, 2.0, 1.0] {
            let pixels = renderer.pixels(600, 420, scale, &model).unwrap();
            let grid = model.grid(600.0 / scale, 420.0 / scale);
            let (x, y) = model.cell(grid, 0);
            let size = (grid.icon_size * scale).round() as u32;
            let expected =
                assets::resample(model.items[0].image.as_ref().unwrap(), size, size).unwrap();
            let left =
                ((x + (grid.cell_width - size as f32 / scale) / 2.0) * scale).round() as usize;
            let top =
                ((y + 2.0 + (grid.icon_size - size as f32 / scale) / 2.0) * scale).round() as usize;
            for iy in 0..size as usize {
                for ix in 0..size as usize {
                    for c in 0..4 {
                        let actual = pixels[((top + iy) * 600 + left + ix) * 4 + c];
                        let expected = expected.data[(iy * size as usize + ix) * 4 + c];
                        assert!(
                            actual.abs_diff(expected) <= 1,
                            "icon was filtered a second time at DPI {scale}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn canvas_flyout_retains_transparency_and_hover_after_resize() {
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut renderer = Renderer::new().unwrap();
        let entries = [super::super::menu::Entry {
            id: 1,
            label: "返回上个文件夹",
            icon: "",
            trailing: "Alt + ←",
            children: Vec::new(),
        }];
        for (width, height, scale) in [
            (240, 48, 1.0),
            (360, 72, 1.5),
            (480, 96, 2.0),
            (240, 48, 1.0),
        ] {
            let pixels = renderer
                .flyout(width, height, scale, &entries, None, true, true)
                .unwrap();
            let selected = renderer
                .flyout(width, height, scale, &entries, Some(0), true, true)
                .unwrap();
            let at = (((12.0 * scale) as u32 * width + (12.0 * scale) as u32) * 4) as usize;
            assert_eq!(pixels[at + 3], 0);
            assert!(selected[at + 3] > 0 && selected[at + 3] < 255);
            let opaque = renderer
                .flyout(width, height, scale, &entries, None, false, false)
                .unwrap();
            assert_eq!(opaque[at + 3], 255);
            for (x, y) in [(0, 0), (width - 1, 0), (0, height - 1), (width - 1, height - 1)] {
                assert_eq!(opaque[((y * width + x) * 4 + 3) as usize], 0);
            }
        }
    }
    use crate::pane::{Item, assets::Pixels};
    use desktop_core::ShellIdentity;
    use std::sync::Arc;

    #[test]
    fn tab_strip_renders_at_supported_dpi_and_widths() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        model.tabs = ["工作", "项目资料", "下载与归档"].into_iter().enumerate()
            .map(|(at, title)| (desktop_core::PanelId::new(at as u64 + 1), title.into())).collect();
        model.active_tab = desktop_core::PanelId::new(2);
        let mut renderer = Renderer::new().unwrap();
        for scale in [1.0, 1.5, 2.0] {
            for width in [260, 420, 800] {
                for dark in [false, true] {
                    model.dark = dark;
                    let w = (width as f32 * scale) as u32;
                    let h = (300.0 * scale) as u32;
                    let pixels = renderer.pixels(w, h, scale, &model).unwrap();
                    assert_eq!(pixels.len(), (w * h * 4) as usize);
                    if let Some((id, _)) = super::super::tabs::strip(&model, width as f32).into_iter().find(|(id, _)| *id != model.active_tab) {
                        let mut hovered = model.clone();
                        hovered.hovered_tab = Some(id);
                        let hover_pixels = renderer.pixels(w, h, scale, &hovered).unwrap();
                        assert_ne!(pixels, hover_pixels);
                        let body = ((HEADER * scale).ceil() as u32 * w * 4) as usize;
                        assert_eq!(&pixels[body..], &hover_pixels[body..], "tab hover must not change the material or pane body");
                    }
                    let mut title_changed = model.clone();
                    title_changed.title = "这个旧标题不应绘制".into();
                    assert_eq!(pixels, renderer.pixels(w, h, scale, &title_changed).unwrap());
                    let mut blank = model.clone();
                    for (_, title) in &mut blank.tabs { title.clear(); }
                    assert_ne!(pixels, renderer.pixels(w, h, scale, &blank).unwrap());
                    if std::env::var_os("LUCIDPANE_TEST_EXPORT_SNAPSHOTS").is_some() && scale == 1.0 && width == 420 && dark {
                        let mut bmp = vec![0u8; 54];
                        bmp[..2].copy_from_slice(b"BM");
                        bmp[2..6].copy_from_slice(&(54u32 + pixels.len() as u32).to_le_bytes());
                        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
                        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
                        bmp[18..22].copy_from_slice(&(w as i32).to_le_bytes());
                        bmp[22..26].copy_from_slice(&(-(h as i32)).to_le_bytes());
                        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
                        bmp[28..30].copy_from_slice(&32u16.to_le_bytes());
                        bmp.extend_from_slice(&pixels);
                        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/tabs-preview.bmp");
                        std::fs::write(output, bmp).unwrap();
                    }
                }
            }
        }
    }

    fn sample_model() -> GroupModel {
        GroupModel {
            merge_preview: Vec::new(),
        merge_occluded: false,
            tabs: Vec::new(),
            active_tab: desktop_core::PanelId::new(0),
            folder_sort: (0, false),
            folder_columns: None,
            folder_visible_columns: 15,
        folder_navigation: [false; 2],
            list_view: false,
            folder: None,
            folder_status: None,
            options: desktop_core::PaneOptions::default(),
            theme: desktop_core::PanelTheme::Dark,
            dark: true,

            hovered_item: None,
            scrollbar: Default::default(),
            focused: true,
            auto_hide: false,
            locked: false,
            reveal: 1.0,
            hovered_tab: None,
        hovered_button: None,
            pressed_button: None,
            backdrop: desktop_core::Backdrop::Acrylic,
            native_material: true,
            title: "透明度验证".into(),
            items: vec![Item {
                details: Default::default(),
                identity: ShellIdentity::Namespace {
                    parsing_name: "test:opaque-icon".into(),
                },
                label: "Test".into(),
                image: Some(Arc::new(Pixels {
                    width: 16,
                    height: 16,
                    data: [80, 100, 200, 255].repeat(16 * 16),
                })),
            }],
            icon_size: 48.0,
            selected: None,
            selection: Default::default(),
            selection_anchor: None,
            renaming: None,
            scroll: 0,
            collapsed: false,
            loading: false,
        }
    }

    #[test]
    fn pane_border_and_corners_can_be_disabled_independently() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut renderer = Renderer::new().unwrap();
        let mut model = sample_model();
        model.items.clear();
        model.title.clear();
        model.native_material = true;
        let bordered = renderer.pixels(320, 200, 1.0, &model).unwrap();
        model.options.border = false;
        let borderless = renderer.pixels(320, 200, 1.0, &model).unwrap();
        assert!(bordered[160 * 4 + 3] > borderless[160 * 4 + 3]);
        model.native_material = false;
        let rounded = renderer.pixels(320, 200, 1.0, &model).unwrap();
        model.options.corner_radius = 0.0;
        let square = renderer.pixels(320, 200, 1.0, &model).unwrap();
        let corner = (320 + 1) * 4 + 3;
        assert!(square[corner] > rounded[corner]);
    }

    #[test]
    fn item_hit_stops_at_visible_highlight_across_dpi_and_scroll() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();

        model.items.push(sample_model().items.remove(0));
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for scroll in [0, 1] {
                model.scroll = scroll;
                for text in ["Termius", "A longer name that wraps onto two lines"] {
                    model.items[scroll].label = text.into();
                    let grid = model.grid(112.0, 250.0);
                    let (x, y) = grid.cell(scroll, scroll);
                    let label = super::super::label::raster(
                        text,
                        (grid.cell_width * scale).round() as u32,
                        (96.0 * scale).round() as u32,
                        2,
                    )
                    .unwrap();
                    let bottom = y
                        + (grid.icon_size
                            + crate::pane::layout::LABEL_OFFSET
                            + 1.0
                            + label.text_height as f32 / scale)
                            .min(grid.cell_height - 2.0);
                    let center = x + grid.cell_width / 2.0;
                    assert_eq!(
                        model.hit(grid, center, bottom - 1.0 / scale, scale),
                        Some(scroll)
                    );
                    assert_eq!(model.hit(grid, center, bottom, scale), None);
                    assert_eq!(
                        model.hit(grid, center, y + grid.cell_height - 1.0, scale),
                        None
                    );
                    assert_eq!(model.hit(grid, x - 1.0, y + 10.0, scale), None);
                    assert_eq!(model.hit(grid, x + grid.cell_width, y + 10.0, scale), None);
                }
            }
        }
    }

    #[test]
    fn light_and_dark_text_contrast_without_changing_icon_pixels() {
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        model.native_material = false;
        let mut renderer = Renderer::new().unwrap();
        for dark in [true, false] {
            model.dark = dark;
            let pixels = renderer.pixels(400, 240, 1.0, &model).unwrap();
            let at = |x: usize, y: usize| &pixels[(y * 400 + x) * 4..][..4];
            assert_eq!(at(59, 78), [80, 100, 200, 255]);
            assert_eq!(at(380, 200)[0] < 128, dark);
            let ink_present = (5..32).any(|y| {
                (106..294).any(|x| {
                    let p = at(x, y)[0];
                    if dark { p > 180 } else { p < 100 }
                })
            });
            assert!(ink_present, "Title must contrast with its background");
        }
    }

    #[test]
    fn transparent_panel_protection_preserves_icons_and_rounded_edges() {
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        model.options.text_protection = true;
        model.backdrop = desktop_core::Backdrop::Solid {
            color: 0xffffff,
            opacity: 0.0,
        };
        let mut renderer = Renderer::new().unwrap();
        for mode in [
            desktop_core::PanelText::Light,
            desktop_core::PanelText::Dark,
        ] {
            model.options.text = mode;
            let pixels = renderer.pixels(400, 240, 1.0, &model).unwrap();
            let at = |x: usize, y: usize| &pixels[(y * 400 + x) * 4..][..4];
            assert_eq!(at(59, 78), [80, 100, 200, 255]);
            assert!(at(380, 200)[3] > 0 && at(380, 200)[3] < 255);
            assert_eq!(at(0, 0)[3], 0);
            assert_eq!(at(380, 200)[0] == 0, mode == desktop_core::PanelText::Light);
            model.options.text_protection = false;
            let unprotected = renderer.pixels(400, 240, 1.0, &model).unwrap();
            assert_eq!(unprotected[(200 * 400 + 380) * 4 + 3], 0);
            assert_eq!(
                &unprotected[(78 * 400 + 59) * 4..][..4],
                [80, 100, 200, 255]
            );
            model.options.text_protection = true;
        }
    }

    #[test]
    fn background_alpha_does_not_dim_icons_at_multiple_scales() {
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        let mut renderer = Renderer::new().unwrap();
        for scale in [1.0, 1.5, 2.0] {
            let width = (400.0 * scale) as u32;
            let pixels = renderer
                .pixels(width, (240.0 * scale) as u32, scale, &model)
                .unwrap();
            let at = |x: f32, y: f32| -> &[u8] {
                let index = (((y * scale) as u32 * width + (x * scale) as u32) * 4) as usize;
                &pixels[index..index + 4]
            };
            assert_eq!(
                at(59.0, 78.0),
                [80, 100, 200, 255],
                "icon retains its original opaque colors"
            );
            assert!(
                at(380.0, 200.0)[3] == 0,
                "content leaves the native backdrop unobstructed"
            );
            assert_eq!(at(0.0, 0.0)[3], 0, "outside rounded corner is transparent");
            model.native_material = false;
            let fallback = renderer
                .pixels(width, (240.0 * scale) as u32, scale, &model)
                .unwrap();
            let background =
                (((200.0 * scale) as u32 * width + (380.0 * scale) as u32) * 4) as usize;
            assert_eq!(
                fallback[background + 3],
                255,
                "unsupported materials have an opaque fallback"
            );
            model.native_material = true;
        }
    }

    #[test]
    fn viewport_layout_matches_full_measurement_across_sizes_and_scroll_positions() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        let template = model.items[0].clone();
        for count in [1, 37, 257] {
            model.items = (0..count).map(|index| Item {
                label: ["short", "long filename with wrapping content", "项目文件名称与测试资料"][index % 3].into(),
                ..template.clone()
            }).collect();
            for percent in [50.0, 100.0, 200.0] {
                model.options.grid_scale = percent;
                for width in [180.0, 420.0] {
                    for height in [44.0, 80.0, 240.0, 600.0] {
                        for scroll in [0, 2, count - 1] {
                            model.scroll = scroll;
                            let grid = model.grid(width, height);
                            let rows = model.row_contents(grid);
                            let available = height - super::super::layout::HEADER - super::super::layout::PADDING;
                            let old_visible = super::super::layout::fitting_rows(&rows[scroll.min(rows.len()-1)..], grid.cell_height, available);
                            let old_limit = (0..rows.len()).find(|start| super::super::layout::fitting_rows(&rows[*start..], grid.cell_height, available) >= rows.len()-start).unwrap_or(rows.len()-1);
                            assert_eq!(grid.visible_rows, old_visible);
                            assert_eq!(grid.scroll_limit, Some(old_limit));
                            let expected: Vec<_> = (0..count).filter(|index| {
                                let (_, y) = model.cell(grid, *index);
                                y + grid.cell_height > HEADER && y < height
                            }).collect();
                            let actual: Vec<_> = grid.visible_indices(scroll, HEADER, height, count).filter(|index| {
                                let (_, y) = model.cell(grid, *index);
                                y + grid.cell_height > HEADER && y < height
                            }).collect();
                            assert_eq!(actual, expected);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn large_directory_layout_timing() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        let template = model.items[0].clone();
        model.items = (0..10000).map(|index| Item {
            label: format!("Document {index} with a longer file name.txt"), ..template.clone()
        }).collect();
        let grid = model.grid(600.0, 400.0);
        let start = std::time::Instant::now();
        let rows = model.row_contents(grid);
        let old = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..100 { std::hint::black_box(model.grid(600.0, 400.0)); }
        eprintln!("10,000-file layout: full row measurement {:?}; viewport layout average {:?}", old, start.elapsed()/100);
        assert_eq!(rows.len(), model.items.len().div_ceil(grid.columns));
    }

    #[test]
    fn selection_backgrounds_release_old_sizes_and_deselected_textures() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        model.list_view = true;
        model.selection.insert(0);
        let device = windows_canvas::GpuDevice::new_warp().unwrap();
        let mut renderer = Renderer::new().unwrap();
        let bitmap = canvas::Offscreen::new(&device, 640, 240).unwrap();
        for width in [320, 480, 640, 400] {
            renderer.paint(&bitmap.target, width, 240, 1.0, &model).unwrap();
            assert_eq!(renderer.states.len(), 1);
        }
        model.selection.clear();
        model.hovered_item = None;
        renderer.paint(&bitmap.target, 400, 240, 1.0, &model).unwrap();
        assert!(renderer.states.is_empty());
    }

    #[test]
    fn same_size_upload_borrows_original_pixel_buffer() {
        let source = assets::Pixels { width: 32, height: 32, data: vec![255; 32 * 32 * 4] };
        let pixels = assets::resample(&source, 32, 32).unwrap();
        assert!(std::ptr::eq(pixels.data.as_ptr(), source.data.as_ptr()));
    }

    #[test]
    fn identical_visible_icons_share_one_gpu_upload_and_changed_pixels_replace_it() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        let template = model.items[0].clone();
        model.items = (0..8).map(|index| Item {
            identity: ShellIdentity::Namespace { parsing_name: format!("shared:{index}") },
            ..template.clone()
        }).collect();
        let device = windows_canvas::GpuDevice::new_warp().unwrap();
        let mut renderer = Renderer::new().unwrap();
        for scale in [1.0, 1.5, 2.0] {
            let bitmap = canvas::Offscreen::new(&device, (600.0 * scale) as u32, (400.0 * scale) as u32).unwrap();
            renderer.paint(&bitmap.target, (600.0 * scale) as u32, (400.0 * scale) as u32, scale, &model).unwrap();
            assert_eq!(renderer.images.len(), 1, "shared pixels should upload only once");
        }
        let bitmap = canvas::Offscreen::new(&device, 600, 400).unwrap();
        Arc::make_mut(model.items[0].image.as_mut().unwrap()).data[0] ^= 1;
        renderer.paint(&bitmap.target, 600, 400, 1.0, &model).unwrap();
        assert_eq!(renderer.images.len(), 2);
        model.items.remove(0);
        renderer.paint(&bitmap.target, 600, 400, 1.0, &model).unwrap();
        assert_eq!(renderer.images.len(), 1);
    }

    #[test]
    fn scrolling_releases_offscreen_icon_textures() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        let image = model.items[0].image.clone();
        model.items = (0..1000)
            .map(|index| Item {
                identity: ShellIdentity::Namespace {
                    parsing_name: format!("test:icon-{index}"),
                },
                label: format!("Icon {index}"),
                image: image.as_ref().map(|image| Arc::new((**image).clone())),
                details: Default::default(),
            })
            .collect();
        let device = windows_canvas::GpuDevice::new().unwrap();
        let bitmap = canvas::Offscreen::new(&device, 400, 240).unwrap();
        let mut renderer = Renderer::new().unwrap();
        for row in (0..150).step_by(5) {
            model.scroll = row;
            renderer
                .paint(&bitmap.target, 400, 240, 1.0, &model)
                .unwrap();
            assert!(!renderer.images.is_empty());
            assert!(
                renderer.images.len() <= 32,
                "offscreen uploads accumulated: {}",
                renderer.images.len()
            );
        }
        model.scroll = 0;
        renderer
            .paint(&bitmap.target, 400, 240, 1.0, &model)
            .unwrap();
        assert!(
            renderer
                .images
                .contains_key(&(Arc::as_ptr(model.items[0].image.as_ref().unwrap()) as usize))
        );
    }

    #[test]
    fn gpu_frames_preserve_colors_alpha_and_cached_images_across_resize() {
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let model = sample_model();
        // Exercise the actual swap-chain path, including buffer rotation and resize.
        // The test window stays hidden and never takes over Explorer.
        let window = windows_window::Window::new("LucidDesk GPU regression")
            .size(400, 240)
            .style(windows_sys::Win32::UI::WindowsAndMessaging::WS_POPUP)
            .ex_style(windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_NOREDIRECTIONBITMAP)
            .create()
            .unwrap();
        {
            let mut surface = super::super::composition::Surface::new(
                windows::Win32::Foundation::HWND(window.hwnd().cast()),
            )
            .unwrap();
            let mut gpu = Renderer::new().unwrap();
            for (width, height) in [(400, 240), (400, 240), (500, 300), (400, 240)] {
                let target = surface.begin_frame(width, height).unwrap();
                gpu.paint(&target, width, height, 1.0, &model).unwrap();
                assert!(
                    gpu.target.as_ref().unwrap().2.is_none(),
                    "live rendering has no CPU frame bitmap"
                );
                assert_eq!(gpu.images.len(), 1);
                let pixels = surface.readback().unwrap();
                let grid = model.grid(width as f32, height as f32);
                let (x, y) = grid.cell(0, 0);
                let at =
                    (((y + 20.0) as u32 * width + (x + grid.cell_width / 2.0) as u32) * 4) as usize;
                assert_eq!(&pixels[at..at + 4], &[80, 100, 200, 255]);
                assert_eq!(
                    pixels[((height - 20) * width * 4 + (width - 20) * 4 + 3) as usize],
                    0
                );
                surface.end_frame().unwrap();
            }
            // A Shell item can keep its identity while its artwork changes, e.g.
            // the Recycle Bin. Verify replacement reaches the real GPU texture.
            let mut updated = sample_model();
            for color in [[25, 180, 70, 255], [80, 100, 200, 255]] {
                let old = updated.items[0].image.as_ref().unwrap();
                updated.items[0].image = Some(Arc::new(assets::Pixels {
                    width: old.width,
                    height: old.height,
                    data: color.repeat((old.width * old.height) as usize),
                }));
                let target = surface.begin_frame(400, 240).unwrap();
                gpu.paint(&target, 400, 240, 1.0, &updated).unwrap();
                let pixels = surface.readback().unwrap();
                let grid = updated.grid(400.0, 240.0);
                let (x, y) = grid.cell(0, 0);
                let at =
                    (((y + 20.0) as u32 * 400 + (x + grid.cell_width / 2.0) as u32) * 4) as usize;
                assert_eq!(&pixels[at..at + 4], &color);
                assert_eq!(gpu.images.len(), 1);
                surface.end_frame().unwrap();
            }
            // An inventory or another pane can still own the pixels after this
            // pane loses the item. Its GPU upload must nevertheless be released.
            let retained_source = updated.items[0].image.clone().unwrap();
            updated.items.clear();
            let target = surface.begin_frame(400, 240).unwrap();
            gpu.paint(&target, 400, 240, 1.0, &updated).unwrap();
            assert!(gpu.images.is_empty());
            assert!(!retained_source.data.is_empty());
            surface.end_frame().unwrap();
            // Alternate the test-only CPU upload path with native drawing to catch lingering buffer
            // references and context state that would make ResizeBuffers fail.
            for (width, height) in [(160, 120), (400, 240)] {
                assert!(surface.present(width, height, &[0; 4]).is_err());
                let pixels = [32, 64, 128, 128].repeat((width * height) as usize);
                surface.present(width, height, &pixels).unwrap();
                let target = surface.begin_frame(width, height).unwrap();
                gpu.paint(&target, width, height, 1.5, &model).unwrap();
                let pixels = surface.readback().unwrap();
                assert_eq!(pixels.len(), (width * height * 4) as usize);
                assert_eq!(
                    pixels[((height - 1) * width * 4 + (width - 1) * 4 + 3) as usize],
                    0
                );
                surface.end_frame().unwrap();
            }
        }
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(window.hwnd().cast());
        }
    }
}
