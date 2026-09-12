//! Cached Canvas content. Live windows draw directly into the GPU swap chain.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
use windows_canvas::{ColorF, Ellipse, Rect, RoundedRect, Vector2};

use super::native_graphics::canvas_result;
use super::{GroupModel, assets, canvas, layout::HEADER};
use std::collections::HashMap;
use std::sync::Arc;

use windows_canvas::ID2D1DeviceContext;

use windows::core::Result;
type ImageBitmap = (Arc<assets::Pixels>, windows_canvas::Bitmap, (u32, u32));

pub struct Renderer {
    #[cfg(test)]
    offscreen_device: Option<windows_canvas::GpuDevice>,
    labels: windows_canvas::TextFormat,
    title: windows_canvas::TextFormat,
    icons: windows_canvas::TextFormat,
    target: Option<(u32, u32, Option<canvas::Offscreen>, ID2D1DeviceContext)>,
    images: HashMap<String, ImageBitmap>,
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
        self.paint_flyout(target, width, height, scale, rows, selected, native, dark)?;
        bitmap.as_ref().unwrap().pixels()
    }

    pub fn paint_flyout(
        &self,
        target: &ID2D1DeviceContext,
        width: u32,
        height: u32,
        scale: f32,
        rows: &[super::menu::Entry],
        selected: Option<usize>,
        native: bool,
        dark: bool,
    ) -> Result<()> {
        canvas::draw(target, scale, |target| {
            let ink = if dark { 0.95 } else { 0.10 };
            let text = canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 1.0)))?;
            let subtle =
                canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 0.13)))?;
            let hover =
                canvas_result(target.create_solid_brush(ColorF::new(0.8, 0.88, 1.0, 0.14)))?;
            target.clear(ColorF::new(
                if dark { 0.09 } else { 0.96 },
                if dark { 0.10 } else { 0.96 },
                if dark { 0.12 } else { 0.96 },
                if native { 0.0 } else { 1.0 },
            ));
            let width = width as f32 / scale;
            target.draw_rounded_rect(
                &RoundedRect {
                    rect: Rect::from_xywh(0.5, 0.5, width - 1.0, height as f32 / scale - 1.0),
                    radius_x: 8.0,
                    radius_y: 8.0,
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
                if selected == Some(index) {
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
                for (label, left, available) in [
                    (row.icon, 12.0, 20.0),
                    (row.label, 38.0, width - 68.0),
                    (row.trailing, width - 28.0, 20.0),
                ] {
                    target.clipped_text(
                        label,
                        &self.title,
                        &Rect::from_xywh(left, top, available, super::menu::ROW_HEIGHT),
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
            .with_word_wrapping(WordWrapping::Wrap);
        canvas::ellipsis(&labels)?;
        let title = canvas_result(TextFormat::new(&family, size + 1.0))?
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .with_word_wrapping(WordWrapping::NoWrap);
        Ok(Self {
            #[cfg(test)]
            offscreen_device: None,
            labels,
            title,
            icons: canvas_result(TextFormat::new("Segoe Fluent Icons", 12.0))?
                .with_alignment(windows_canvas::TextAlignment::Center)
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
        let live: std::collections::HashSet<_> = model
            .items
            .iter()
            .filter_map(|item| item.image.as_ref().map(Arc::as_ptr))
            .collect();
        self.images
            .retain(|_, (source, _, _)| live.contains(&Arc::as_ptr(source)));
        self.draw(width, height, scale, model)
    }

    #[allow(clippy::too_many_lines)]
    fn draw(&mut self, width: u32, height: u32, scale: f32, model: &GroupModel) -> Result<()> {
        {
            let (_, _, _, target) = self.target.as_ref().unwrap();
            canvas::draw(target, scale, |target| {
                let (w, h) = (width as f32 / scale, height as f32 / scale);
                let opacity = if model.native_material { 0.0 } else { 1.0 };
                let base = if model.dark { 0.085 } else { 0.96 };
                let ink = if model.dark { 1.0 } else { 0.10 };
                let background = canvas_result(
                    target.create_solid_brush(ColorF::new(base, base, base, opacity)),
                )?;
                let outline =
                    canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 0.16)))?;
                let white =
                    canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 1.0)))?;
                let dim =
                    canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 0.58)))?;
                let selection =
                    canvas_result(target.create_solid_brush(ColorF::new(0.55, 0.75, 1.0, 0.25)))?;
                let hover =
                    canvas_result(target.create_solid_brush(ColorF::new(0.7, 0.85, 1.0, 0.12)))?;
                let inactive =
                    canvas_result(target.create_solid_brush(ColorF::new(0.75, 0.8, 0.85, 0.16)))?;
                target.clear(ColorF::new(0.0, 0.0, 0.0, 0.0));
                let rounded = RoundedRect {
                    rect: Rect::from_xywh(0.5, 0.5, w - 1.0, h - 1.0),
                    radius_x: f32::from(model.options.corner_radius),
                    radius_y: f32::from(model.options.corner_radius),
                };
                {
                    target.fill_rounded_rect(&rounded, &background);
                    if model.options.border {
                        target.draw_rounded_rect(&rounded, &outline, 1.0);
                    }
                    let title = &model.title;
                    target.clipped_text(
                        title,
                        &self.title,
                        &Rect::from_xywh(
                            14.0,
                            0.0,
                            (w - super::layout::HEADER_BUTTONS_WIDTH - 16.0).max(0.0),
                            HEADER,
                        ),
                        &white,
                    );
                    for button in 0..3 {
                        let x = super::layout::header_button_x(w, button);
                        let hovered = model.hovered_button == Some(button);
                        let glyph = canvas_result(target.create_solid_brush(ColorF::new(
                            ink,
                            ink,
                            ink,
                            if hovered { 0.95 } else { 0.72 },
                        )))?;
                        if hovered {
                            let fill = canvas_result(
                                target.create_solid_brush(ColorF::new(ink, ink, ink, 0.07)),
                            )?;
                            target.fill_rounded_rect(
                                &RoundedRect {
                                    rect: Rect::from_xywh(x + 1.0, 6.0, 26.0, 26.0),
                                    radius_x: 5.0,
                                    radius_y: 5.0,
                                },
                                &fill,
                            );
                        }
                        let center = x + 14.0;
                        if button == 0 {
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
                                        y: 19.0 + (pair[0].1 - 19.0) * 1.25,
                                    },
                                    Vector2 {
                                        x: center + (pair[1].0 - center) * 1.25,
                                        y: 19.0 + (pair[1].1 - 19.0) * 1.25,
                                    },
                                    &glyph,
                                    1.5,
                                );
                            }
                        } else if button == 1 {
                            for offset in [-4.5, 0.0, 4.5] {
                                target.fill_ellipse(
                                    &Ellipse {
                                        center: Vector2 {
                                            x: center + offset,
                                            y: 19.0,
                                        },
                                        radius_x: 1.1,
                                        radius_y: 1.1,
                                    },
                                    &glyph,
                                );
                            }
                        } else {
                            target.clipped_text(
                                if model.locked { "\u{e72e}" } else { "\u{e785}" },
                                &self.icons,
                                &Rect::from_xywh(x, 5.0, 28.0, 28.0),
                                &glyph,
                            );
                        }
                    }
                }
                if h > HEADER + 1.0 {
                    target.push_clip(&{
                        Rect::from_xywh(6.0, HEADER, w - 12.0, (h - HEADER - 6.0).max(0.0))
                    });
                    let grid = model.grid(w, h);
                    for (index, item) in model.items.iter().enumerate() {
                        let (x, y) = model.cell(grid, index);
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
                            let key = item.identity.persistent_key();
                            let ratio =
                                grid.icon_size * scale / image.width.max(image.height) as f32;
                            let size = (
                                (image.width as f32 * ratio).round().max(1.0) as u32,
                                (image.height as f32 * ratio).round().max(1.0) as u32,
                            );
                            if self.images.get(&key).is_none_or(|(source, _, old_size)| {
                                !Arc::ptr_eq(source, image) || *old_size != size
                            }) {
                                let pixels = assets::resample(image, size.0, size.1)?;
                                let bitmap = canvas_result(target.create_bitmap(
                                    &pixels.data,
                                    pixels.width,
                                    pixels.height,
                                ))?;
                                self.images
                                    .insert(key.clone(), (Arc::clone(image), bitmap, size));
                                if std::env::var_os("LUCIDPANE_ICON_TRACE").is_some()
                                    && key
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
                            let left = ((x + (grid.cell_width - iw) / 2.0) * scale).round() / scale;
                            let top =
                                ((y + 2.0 + (grid.icon_size - ih) / 2.0) * scale).round() / scale;
                            target.draw_bitmap(
                                &self.images[&key].1,
                                &Rect::from_xywh(left, top, iw, ih),
                                1.0,
                            );
                        }

                        if model.renaming.as_ref() == Some(&item.identity) {
                            continue;
                        }
                        let (layout, _) = super::label::layout(
                            &item.label,
                            (grid.cell_width * scale).round() as u32,
                            (96.0 * scale).round() as u32,
                            2,
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
                                "正在读取文件夹…"
                            } else {
                                "此文件夹为空"
                            }
                        } else if model.loading {
                            "正在读取桌面项目…"
                        } else {
                            "将图标拖入此分组"
                        };
                        target.clipped_text(
                            text,
                            &self.labels,
                            &Rect::from_xywh(20.0, HEADER + 48.0, w - 40.0, 40.0),
                            &dim,
                        );
                    }
                    target.pop_clip();
                    let max = grid.max_scroll(model.items.len());
                    // Intermediate fold heights can overflow even when the expanded pane fits.
                    if max > 0 && !model.collapsed && model.reveal >= 1.0 {
                        let track = (h - HEADER - 24.0).max(10.0);
                        let thumb = (track / (max + 1) as f32).max(16.0).min(track);
                        target.fill_rounded_rect(
                            &RoundedRect {
                                rect: Rect::from_xywh(
                                    w - 6.0,
                                    HEADER
                                        + 12.0
                                        + (track - thumb) * model.scroll as f32 / max as f32,
                                    2.0,
                                    thumb,
                                ),
                                radius_x: 1.0,
                                radius_y: 1.0,
                            },
                            &dim,
                        );
                    }
                }
                target.finish()
            })
        }
    }
}

#[cfg(test)]
mod tests {
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
            label: "设置",
            icon: "",
            trailing: "",
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
            assert!(opaque.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        }
    }
    use crate::pane::{Item, assets::Pixels};
    use desktop_core::ShellIdentity;
    use std::sync::Arc;

    fn sample_model() -> GroupModel {
        GroupModel {
            folder: None,
            folder_status: None,
            options: desktop_core::PaneOptions::default(),
            theme: desktop_core::PanelTheme::Dark,
            dark: true,

            spacing: (88.0, 96.0),
            hovered_item: None,
            focused: true,
            auto_hide: false,
            locked: false,
            reveal: 1.0,
            hovered_button: None,
            pressed_button: None,
            backdrop: desktop_core::Backdrop::Acrylic,
            native_material: true,
            title: "透明度验证".into(),
            items: vec![Item {
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
        model.options.corner_radius = 0;
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
                (14..120).any(|x| {
                    let p = at(x, y)[0];
                    if dark { p > 180 } else { p < 100 }
                })
            });
            assert!(ink_present, "Title must contrast with its background");
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
    fn gpu_frames_preserve_colors_alpha_and_cached_images_across_resize() {
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let model = sample_model();
        // Exercise the actual swap-chain path, including buffer rotation and resize.
        // The test window stays hidden and never takes over Explorer.
        let window = windows_window::Window::new("LucidPane GPU regression")
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
            // Menus upload CPU-rendered pixels through the same Canvas surface.
            // Alternate that path with native drawing to catch lingering buffer
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
