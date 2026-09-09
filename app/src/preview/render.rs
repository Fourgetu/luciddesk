//! Cached Direct2D content. Live windows draw directly into the GPU swap chain.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
use super::{GroupModel, assets, layout::HEADER};
use std::collections::HashMap;
use std::sync::Arc;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap,
    ID2D1Factory, ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP, DWriteCreateFactory, IDWriteFactory,
    IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmap, IWICImagingFactory,
    WICBitmapCacheOnLoad,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::core::{PCWSTR, Result};
type LabelBitmap = (ID2D1Bitmap, u32, u32, u32);
type ImageBitmap = (Arc<assets::Pixels>, ID2D1Bitmap);

pub struct Renderer {
    factory: ID2D1Factory,
    imaging: IWICImagingFactory,
    labels: IDWriteTextFormat,
    title: IDWriteTextFormat,
    target: Option<(u32, u32, Option<IWICBitmap>, ID2D1RenderTarget)>,
    images: HashMap<String, ImageBitmap>,
    states: HashMap<(u32, u32, u32, i32), ID2D1Bitmap>,
    native_labels: HashMap<(String, u32, u32), LabelBitmap>,
}

fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}
fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

impl Renderer {
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
        unsafe {
            let (_, _, bitmap, target) = self.target.as_ref().unwrap();
            target.SetDpi(96.0 * scale, 96.0 * scale);
            let ink = if dark { 0.95 } else { 0.10 };
            let text = target.CreateSolidColorBrush(&color(ink, ink, ink, 1.0), None)?;
            let subtle = target.CreateSolidColorBrush(&color(ink, ink, ink, 0.13), None)?;
            let hover = target.CreateSolidColorBrush(&color(0.8, 0.88, 1.0, 0.14), None)?;
            target.BeginDraw();
            target.Clear(Some(&color(
                if dark { 0.09 } else { 0.96 },
                if dark { 0.10 } else { 0.96 },
                if dark { 0.12 } else { 0.96 },
                if native { 0.0 } else { 1.0 },
            )));
            let pixel_width = width;
            let width = width as f32 / scale;
            target.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: rect(0.5, 0.5, width - 1.0, height as f32 / scale - 1.0),
                    radiusX: 8.0,
                    radiusY: 8.0,
                },
                &subtle,
                1.0,
                None,
            );
            for (index, row) in rows.iter().enumerate() {
                let top = super::menu::row_top(rows, index);
                if row.id == 0 {
                    target.FillRectangle(&rect(12.0, top + 4.0, width - 24.0, 1.0), &subtle);
                    continue;
                }
                if selected == Some(index) {
                    target.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: rect(4.0, top + 2.0, width - 8.0, super::menu::ROW_HEIGHT - 4.0),
                            radiusX: 5.0,
                            radiusY: 5.0,
                        },
                        &hover,
                    );
                }
                for (label, left, available) in [
                    (row.icon, 12.0, 20.0),
                    (row.label, 38.0, width - 68.0),
                    (row.trailing, width - 28.0, 20.0),
                ] {
                    let label: Vec<u16> = label.encode_utf16().collect();
                    target.DrawText(
                        &label,
                        &self.title,
                        &rect(left, top, available, super::menu::ROW_HEIGHT),
                        &text,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                }
            }
            target.EndDraw(None, None)?;
            let mut pixels = vec![0; (pixel_width * height * 4) as usize];
            bitmap
                .as_ref()
                .unwrap()
                .CopyPixels(std::ptr::null(), pixel_width * 4, &mut pixels)?;
            Ok(pixels)
        }
    }
    pub fn new() -> Result<Self> {
        unsafe {
            let factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let imaging = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let (family, size) = assets::font();
            let family: Vec<_> = family.encode_utf16().chain(Some(0)).collect();
            let labels = write.CreateTextFormat(
                PCWSTR(family.as_ptr()),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                windows::core::w!("zh-CN"),
            )?;
            labels.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            labels.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            let ellipsis = write.CreateEllipsisTrimmingSign(&labels)?;
            labels.SetTrimming(
                &DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    ..Default::default()
                },
                &ellipsis,
            )?;
            let title = write.CreateTextFormat(
                PCWSTR(family.as_ptr()),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size + 1.0,
                windows::core::w!("zh-CN"),
            )?;
            title.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            title.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            Ok(Self {
                factory,
                imaging,
                labels,
                title,
                target: None,
                images: HashMap::new(),
                states: HashMap::new(),
                native_labels: HashMap::new(),
            })
        }
    }

    fn prepare(&mut self, width: u32, height: u32, scale: f32) -> Result<()> {
        unsafe {
            if self
                .target
                .as_ref()
                .is_none_or(|(w, h, bitmap, _)| *w != width || *h != height || bitmap.is_none())
            {
                let bitmap = self.imaging.CreateBitmap(
                    width,
                    height,
                    &GUID_WICPixelFormat32bppPBGRA,
                    WICBitmapCacheOnLoad,
                )?;
                let target = self.factory.CreateWicBitmapRenderTarget(
                    &bitmap,
                    &D2D1_RENDER_TARGET_PROPERTIES {
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        dpiX: 96.0 * scale,
                        dpiY: 96.0 * scale,
                        ..Default::default()
                    },
                )?;
                target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                self.target = Some((width, height, Some(bitmap), target));
                self.images.clear();
                self.states.clear();
                self.native_labels.clear();
            }
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
        let mut pixels = vec![0; (width * height * 4) as usize];
        unsafe {
            self.target
                .as_ref()
                .unwrap()
                .2
                .as_ref()
                .unwrap()
                .CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
        }
        Ok(pixels)
    }

    pub fn paint(
        &mut self,
        target: &ID2D1RenderTarget,
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
            self.native_labels.clear();
        }
        self.target = Some((width, height, None, target.clone()));
        // Models and the inventory own live sources. Drop obsolete GPU images after a reload.
        self.images
            .retain(|_, (source, _)| Arc::strong_count(source) > 1);
        self.draw(width, height, scale, model)
    }

    #[allow(clippy::too_many_lines)]
    fn draw(&mut self, width: u32, height: u32, scale: f32, model: &GroupModel) -> Result<()> {
        unsafe {
            let (_, _, _, target) = self.target.as_ref().unwrap();
            if model.desktop && std::env::var("LUCIDPANE_INSPECT").as_deref() == Ok("native") {
                target.BeginDraw();
                target.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
                return target.EndDraw(None, None);
            }
            target.SetDpi(96.0 * scale, 96.0 * scale);
            let (w, h) = (width as f32 / scale, height as f32 / scale);
            let opacity = if model.native_material { 0.0 } else { 1.0 };
            let base = if model.dark { 0.085 } else { 0.96 };
            let ink = if model.dark { 1.0 } else { 0.10 };
            let background = target.CreateSolidColorBrush(&color(base, base, base, opacity), None)?;
            let outline = target.CreateSolidColorBrush(&color(ink, ink, ink, 0.16), None)?;
            let white = target.CreateSolidColorBrush(&color(ink, ink, ink, 1.0), None)?;
            let dim = target.CreateSolidColorBrush(&color(ink, ink, ink, 0.58), None)?;
            let shadow = target.CreateSolidColorBrush(&color(0.0, 0.0, 0.0, if model.dark { 0.8 } else { 0.0 }), None)?;
            let selection = target.CreateSolidColorBrush(&color(0.55, 0.75, 1.0, 0.25), None)?;
            let hover = target.CreateSolidColorBrush(&color(0.7, 0.85, 1.0, 0.12), None)?;
            let inactive = target.CreateSolidColorBrush(&color(0.75, 0.8, 0.85, 0.16), None)?;
            target.BeginDraw();
            target.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
            let rounded = D2D1_ROUNDED_RECT {
                rect: rect(0.5, 0.5, w - 1.0, h - 1.0),
                radiusX: 7.0,
                radiusY: 7.0,
            };
            if !model.desktop {
                target.FillRoundedRectangle(&raw const rounded, &background);
                target.DrawRoundedRectangle(&raw const rounded, &outline, 1.0, None);
                let title: Vec<_> = model.title.encode_utf16().collect();
                target.DrawText(
                    &title,
                    &self.title,
                    &rect(14.0, 0.0, w - 86.0, HEADER),
                    &white,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
                for button in 0..2 {
                    let x = w - 70.0 + button as f32 * 32.0;
                    if model.hovered_button == Some(button) {
                        target.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: rect(x, 5.0, 28.0, 28.0),
                                radiusX: 6.0,
                                radiusY: 6.0,
                            },
                            &selection,
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
                            target.DrawLine(
                                windows_numerics::Vector2 {
                                    X: pair[0].0,
                                    Y: pair[0].1,
                                },
                                windows_numerics::Vector2 {
                                    X: pair[1].0,
                                    Y: pair[1].1,
                                },
                                &white,
                                1.5,
                                None,
                            );
                        }
                    } else {
                        for offset in [-4.0, 0.0, 4.0] {
                            target.FillEllipse(
                                &windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
                                    point: windows_numerics::Vector2 {
                                        X: center + offset,
                                        Y: 19.0,
                                    },
                                    radiusX: 1.1,
                                    radiusY: 1.1,
                                },
                                &white,
                            );
                        }
                    }
                }
            }
            if model.desktop || h > HEADER + 1.0 {
                target.PushAxisAlignedClip(
                    &if model.desktop {
                        rect(0.0, 0.0, w, h)
                    } else {
                        rect(6.0, HEADER, w - 12.0, (h - HEADER - 6.0).max(0.0))
                    },
                    D2D1_ANTIALIAS_MODE_ALIASED,
                );
                let grid = model.grid(w, h);
                for (index, item) in model.items.iter().enumerate() {
                    let (x, y) = model.cell(grid, index);
                    if y + grid.cell_height <= if model.desktop { 0.0 } else { HEADER } || y >= h {
                        continue;
                    }
                    let label_key = (
                        item.label.clone(),
                        (grid.cell_width * scale).round() as u32,
                        (96.0 * scale).round() as u32,
                    );
                    if model.managed
                        && !self.native_labels.contains_key(&label_key)
                        && let Some(label) =
                            super::label::raster(&item.label, label_key.1, label_key.2, 2)
                    {
                        let pixels = &label.pixels;
                        let bitmap = target.CreateBitmap(
                            D2D_SIZE_U {
                                width: pixels.width,
                                height: pixels.height,
                            },
                            Some(pixels.data.as_ptr().cast()),
                            pixels.width * 4,
                            &D2D1_BITMAP_PROPERTIES {
                                pixelFormat: D2D1_PIXEL_FORMAT {
                                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                                },
                                dpiX: 96.0,
                                dpiY: 96.0,
                            },
                        )?;
                        self.native_labels.insert(
                            label_key.clone(),
                            (bitmap, label.text_height, label.padding, pixels.height),
                        );
                    }
                    let selection_height = if model.managed {
                        super::theme::selection_height(
                            grid.icon_size,
                            self.native_labels
                                .get(&label_key)
                                .map_or(16.0, |l| l.1 as f32 / scale),
                            grid.cell_height,
                        )
                    } else {
                        grid.cell_height - 3.0
                    };
                    let selection_width = grid.cell_width - if model.managed { 0.0 } else { 4.0 };
                    let selection_x = x + if model.managed { 0.0 } else { 2.0 };
                    if model.selected == Some(index) || model.hovered_item == Some(index) {
                        let state = if model.selected == Some(index) {
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
                        if model.managed
                            && !self.states.contains_key(&key)
                            && let Some(pixels) =
                                super::theme::selection(key.0, key.1, key.2, key.3)
                        {
                            let bitmap = target.CreateBitmap(
                                D2D_SIZE_U {
                                    width: key.0,
                                    height: key.1,
                                },
                                Some(pixels.data.as_ptr().cast()),
                                key.0 * 4,
                                &D2D1_BITMAP_PROPERTIES {
                                    pixelFormat: D2D1_PIXEL_FORMAT {
                                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                                    },
                                    dpiX: 96.0,
                                    dpiY: 96.0,
                                },
                            )?;
                            self.states.insert(key, bitmap);
                        }
                        if model.managed
                            && let Some(bitmap) = self.states.get(&key)
                        {
                            target.DrawBitmap(
                                bitmap,
                                Some(&rect(selection_x, y, selection_width, selection_height)),
                                1.0,
                                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                                None,
                            );
                        } else {
                            target.FillRoundedRectangle(
                                &D2D1_ROUNDED_RECT {
                                    rect: rect(selection_x, y, selection_width, selection_height),
                                    radiusX: if model.managed { 0.0 } else { 3.0 },
                                    radiusY: if model.managed { 0.0 } else { 3.0 },
                                },
                                if model.selected == Some(index) {
                                    if model.focused { &selection } else { &inactive }
                                } else {
                                    &hover
                                },
                            );
                        }
                    }
                    if let Some(image) = &item.image {
                        let key = item.identity.persistent_key();
                        if self
                            .images
                            .get(&key)
                            .is_none_or(|(source, _)| !Arc::ptr_eq(source, image))
                        {
                            let bitmap = target.CreateBitmap(
                                D2D_SIZE_U {
                                    width: image.width,
                                    height: image.height,
                                },
                                Some(image.data.as_ptr().cast()),
                                image.width * 4,
                                &D2D1_BITMAP_PROPERTIES {
                                    pixelFormat: D2D1_PIXEL_FORMAT {
                                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                                    },
                                    dpiX: 96.0,
                                    dpiY: 96.0,
                                },
                            )?;
                            self.images.insert(key.clone(), (Arc::clone(image), bitmap));
                        }
                        let ratio = grid.icon_size / image.width.max(image.height) as f32;
                        let (iw, ih) = (image.width as f32 * ratio, image.height as f32 * ratio);
                        target.DrawBitmap(
                            &self.images[&key].1,
                            Some(&rect(
                                x + (grid.cell_width - iw) / 2.0,
                                y + if model.managed { 2.0 } else { 4.0 }
                                    + (grid.icon_size - ih) / 2.0,
                                iw,
                                ih,
                            )),
                            1.0,
                            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                            None,
                        );
                    }
                    if model.renaming.as_ref() == Some(&item.identity) { continue; }
                    if model.managed && model.dark
                        && let Some((bitmap, _, padding, height)) =
                            self.native_labels.get(&label_key)
                    {
                        target.DrawBitmap(
                            bitmap,
                            Some(&rect(
                                (x * scale).round() / scale,
                                ((y + grid.icon_size + 4.0) * scale).round() / scale
                                    - *padding as f32 / scale,
                                label_key.1 as f32 / scale,
                                *height as f32 / scale,
                            )),
                            1.0,
                            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                            None,
                        );
                        continue;
                    }
                    let text: Vec<_> = item.label.encode_utf16().collect();
                    let label = rect(
                        x + if model.managed { 0.0 } else { 3.0 },
                        y + grid.icon_size + if model.managed { 4.0 } else { 9.0 },
                        grid.cell_width - if model.managed { 0.0 } else { 6.0 },
                        if model.managed {
                            (self.labels.GetFontSize() * 2.6)
                                .min(grid.cell_height - grid.icon_size - 4.0)
                        } else {
                            (grid.cell_height - grid.icon_size - 6.0).max(16.0)
                        },
                    );
                    target.DrawText(
                        &text,
                        &self.labels,
                        &D2D_RECT_F {
                            top: label.top + 1.0,
                            bottom: label.bottom + 1.0,
                            ..label
                        },
                        &shadow,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                    target.DrawText(
                        &text,
                        &self.labels,
                        &raw const label,
                        &white,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                }
                if model.items.is_empty() && !model.desktop {
                    let text: Vec<_> = (if model.loading {
                        "正在读取桌面项目…"
                    } else {
                        "将图标拖入此分组"
                    })
                    .encode_utf16()
                    .collect();
                    target.DrawText(
                        &text,
                        &self.labels,
                        &rect(20.0, HEADER + 48.0, w - 40.0, 40.0),
                        &dim,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                }
                target.PopAxisAlignedClip();
                let max = grid.max_scroll(model.items.len());
                if max > 0 && !model.desktop {
                    let track = (h - HEADER - 24.0).max(10.0);
                    let thumb = (track / (max + 1) as f32).max(16.0).min(track);
                    target.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: rect(
                                w - 6.0,
                                HEADER + 12.0 + (track - thumb) * model.scroll as f32 / max as f32,
                                2.0,
                                thumb,
                            ),
                            radiusX: 1.0,
                            radiusY: 1.0,
                        },
                        &dim,
                    );
                }
            }
            target.EndDraw(None, None)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::{Item, assets::Pixels};
    use desktop_core::ShellIdentity;
    use std::sync::Arc;

    fn sample_model() -> GroupModel {
        GroupModel {
            theme: desktop_core::PanelTheme::Dark, dark: true,
            desktop: false,
            managed: false,
            spacing: (88.0, 96.0),
            hovered_item: None,
            focused: true,
            auto_hide: false,
            reveal: 1.0,
            hovered_button: None,
            backdrop: desktop_core::Backdrop::Acrylic,
            native_material: true,
            title: "透明度验证".into(),
            items: vec![Item {
                position: desktop_core::PointDip::default(),
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
            renaming: None,
            scroll: 0,
            collapsed: false,
            loading: false,
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
            let ink_present = (5..32).any(|y| (14..120).any(|x| {
                let p = at(x, y)[0]; if dark { p > 180 } else { p < 100 }
            }));
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
    #[ignore = "manual 4K rendering benchmark; timings are not a CI assertion"]
    fn benchmark_4k_desktop_submission() {
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut model = sample_model();
        model.managed = true;
        model.desktop = true;
        model.spacing = (75.0, 98.0);
        let seed = model.items[0].clone();
        model.items = (0..200)
            .map(|index| Item {
                identity: ShellIdentity::Namespace {
                    parsing_name: format!("test:{index}"),
                },
                label: format!("桌面项目 {index}"),
                position: desktop_core::PointDip::new(
                    (index / 14) as f32 * 75.0,
                    (index % 14) as f32 * 98.0,
                ),
                image: seed.image.clone(),
            })
            .collect();
        let window = windows_window::Window::new("LucidPane hidden benchmark")
            .size(3840, 2160)
            .style(windows_sys::Win32::UI::WindowsAndMessaging::WS_POPUP)
            .ex_style(windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_NOREDIRECTIONBITMAP)
            .create()
            .unwrap();
        {
            let mut surface = super::super::composition::Surface::new(
                windows::Win32::Foundation::HWND(window.hwnd().cast()),
            )
            .unwrap();
            for direct in [false, true] {
                let mut renderer = Renderer::new().unwrap();
                let mut samples = Vec::new();
                for index in 0..28 {
                    model.hovered_item = Some(index);
                    let start = std::time::Instant::now();
                    if direct {
                        let target = surface.begin_frame(3840, 2160).unwrap();
                        renderer.paint(&target, 3840, 2160, 1.5, &model).unwrap();
                        surface.end_frame().unwrap();
                    } else {
                        let pixels = renderer.pixels(3840, 2160, 1.5, &model).unwrap();
                        surface.present(3840, 2160, &pixels).unwrap();
                    }
                    if index >= 4 {
                        samples.push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "4K / 200 icons / warmed / hidden swap chain / {}: median {:.2} ms, p95 {:.2} ms",
                    if direct {
                        "GPU direct"
                    } else {
                        "CPU bitmap + upload"
                    },
                    samples[12],
                    samples[22]
                );
            }
        }
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(window.hwnd().cast());
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
        }
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(window.hwnd().cast());
        }
    }
}
