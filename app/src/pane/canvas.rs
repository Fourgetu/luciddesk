//! Canvas draw-pass lifetime, clipping and the few native operations absent from Canvas 0.100.
use super::native_graphics::{canvas_result, native_interface};
use windows::Win32::Graphics::Direct2D::{
    Common::D2D_RECT_F, D2D1_ANTIALIAS_MODE_ALIASED, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
    ID2D1DeviceContext, ID2D1Image,
};
use windows::core::Result;
use windows_canvas as c;

/// Canvas does not expose ellipsis trimming; keep this native operation at the boundary.
pub fn ellipsis(format: &c::TextFormat) -> Result<()> {
    ellipsis_delimiter(format, 0)
}

/// Center the visible glyphs, rather than the font's asymmetric line box.
pub fn text_ink_center_offset(layout: &c::TextLayout) -> Result<f32> {
    use windows::Win32::Graphics::DirectWrite::IDWriteTextLayout;
    let native: IDWriteTextLayout = native_interface(layout.raw())?;
    let ink = unsafe { native.GetOverhangMetrics()? };
    Ok((ink.top - ink.bottom) * 0.5)
}

/// Preserve the final path component or extension when trimming long labels.
pub fn ellipsis_delimiter(format: &c::TextFormat, delimiter: u32) -> Result<()> {
    apply_fallback(format)?;
    use windows::Win32::Graphics::DirectWrite::*;
    unsafe {
        let native: IDWriteTextFormat = native_interface(format.raw())?;
        let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let sign = factory.CreateEllipsisTrimmingSign(&native)?;
        native.SetTrimming(
            &DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter,
                delimiterCount: u32::from(delimiter != 0),
                ..Default::default()
            },
            &sign,
        )
    }
}

/// Prefer the language default for missing text glyphs, then Windows fallback.
/// The format owns the mapping; no COM objects survive in thread-local caches.
pub fn apply_fallback(format: &c::TextFormat) -> Result<()> {
    use windows::Win32::Graphics::DirectWrite::*;
    use windows::core::PCWSTR;
    unsafe {
        let factory: IDWriteFactory2 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let builder = factory.CreateFontFallbackBuilder()?;
        let wide: Vec<u16> = crate::i18n::default_font().encode_utf16().chain(Some(0)).collect();
        // Private-use icons keep their dedicated font mapping.
        builder.AddMapping(&[
            DWRITE_UNICODE_RANGE { first: 0, last: 0xd7ff },
            DWRITE_UNICODE_RANGE { first: 0xf900, last: 0xeffff },
        ], &[wide.as_ptr()], None, PCWSTR::null(), PCWSTR::null(), 1.0)?;
        builder.AddMappings(&factory.GetSystemFontFallback()?)?;
        let native: IDWriteTextFormat1 = native_interface(format.raw())?;
        native.SetFontFallback(&builder.CreateFontFallback()?)
    }
}

pub struct DrawPass<'a> {
    session: c::DrawingSession<'a>,
    native: ID2D1DeviceContext,
    active: bool,
    clips: std::cell::Cell<usize>,
}

pub fn draw<T>(
    target: &c::ID2D1DeviceContext,
    scale: f32,
    paint: impl FnOnce(DrawPass<'_>) -> Result<T>,
) -> Result<T> {
    let native: ID2D1DeviceContext = native_interface(target)?;
    unsafe {
        native.SetDpi(96.0 * scale, 96.0 * scale);
        native.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        native.BeginDraw();
    }
    let session = c::DrawingSession::from_borrowed_context(target, c::Matrix3x2::identity());
    paint(DrawPass {
        session,
        native,
        active: true,
        clips: std::cell::Cell::new(0),
    })
}

impl<'a> std::ops::Deref for DrawPass<'a> {
    type Target = c::DrawingSession<'a>;
    fn deref(&self) -> &Self::Target {
        &self.session
    }
}

impl DrawPass<'_> {
    /// Center the visible glyph, not the font's line box. MDL2's em-square
    /// placement differs from the UI text font and from Segoe Fluent Icons.
    pub fn clipped_icon(&self, text: &str, format: &c::TextFormat, bounds: &c::Rect, brush: &c::Brush) {
        let Ok(layout) = c::TextLayout::new(text, format,
            bounds.right - bounds.left, bounds.bottom - bounds.top) else {
            self.clipped_text(text, format, bounds, brush);
            return;
        };
        let offset = text_ink_center_offset(&layout).unwrap_or(0.0);
        self.push_clip(bounds);
        self.session.draw_text_layout(c::Vector2::new(bounds.left, bounds.top + offset), &layout, brush);
        self.pop_clip();
    }

    pub fn clipped_text(
        &self,
        text: &str,
        format: &c::TextFormat,
        bounds: &c::Rect,
        brush: &c::Brush,
    ) {
        self.push_clip(bounds);
        self.session.draw_text(text, format, bounds, brush);
        self.pop_clip();
    }
    pub fn clipped_layout(&self, layout: &c::TextLayout, x: f32, y: f32, brush: &c::Brush) {
        let metrics = layout.metrics();
        self.push_clip(&c::Rect::from_xywh(
            x,
            y,
            metrics.layout_width,
            metrics.layout_height,
        ));
        self.session
            .draw_text_layout(c::Vector2::new(x, y), layout, brush);
        self.pop_clip();
    }
    pub fn push_clip(&self, r: &c::Rect) {
        unsafe {
            self.native.PushAxisAlignedClip(
                &D2D_RECT_F {
                    left: r.left,
                    top: r.top,
                    right: r.right,
                    bottom: r.bottom,
                },
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
        };
        self.clips.set(self.clips.get() + 1);
    }
    pub fn pop_clip(&self) {
        assert!(self.clips.get() > 0);
        unsafe { self.native.PopAxisAlignedClip() };
        self.clips.set(self.clips.get() - 1);
    }
    pub fn finish(mut self) -> Result<()> {
        while self.clips.get() > 0 {
            self.pop_clip();
        }
        self.active = false;
        unsafe { self.native.EndDraw(None, None) }
    }
}
impl Drop for DrawPass<'_> {
    fn drop(&mut self) {
        if self.active {
            while self.clips.get() > 0 {
                self.pop_clip();
            }
            unsafe {
                let _ = self.native.EndDraw(None, None);
            }
        }
    }
}

/// CPU readback for Shell drag images and tests; live windows draw directly to GPU surfaces.
pub struct Offscreen {
    canvas: c::RenderTarget,
    context: ID2D1DeviceContext,
    image: ID2D1Image,
    pub target: c::ID2D1DeviceContext,
}
impl Offscreen {
    pub fn new(device: &c::GpuDevice, width: u32, height: u32) -> Result<Self> {
        let canvas = canvas_result(device.create_render_target(width, height))?;
        let mut captured = None;
        canvas_result(canvas.draw(|session| {
            captured = Some((|| -> Result<_> {
                let context: ID2D1DeviceContext = native_interface(session.raw())?;
                let image = unsafe { context.GetTarget()? };
                Ok((context, image, session.raw().clone()))
            })());
            Ok(())
        }))?;
        let (context, image, target) =
            captured.expect("Canvas invokes the draw closure synchronously")?;
        unsafe {
            context.SetTarget(&image);
        }
        Ok(Self {
            canvas,
            context,
            image,
            target,
        })
    }
    pub fn pixels(&self) -> Result<Vec<u8>> {
        unsafe {
            self.context.SetTarget(None::<&ID2D1Image>);
        }
        let result = canvas_result(self.canvas.read_pixels());
        unsafe {
            self.context.SetTarget(&self.image);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_uses_grayscale_on_both_opaque_and_transparent_surfaces() {
        let device = windows_canvas::GpuDevice::new().unwrap();
        let target = Offscreen::new(&device, 220, 50).unwrap();
        let format = c::TextFormat::new(super::super::assets::UI_FONT, 17.0).unwrap();
        for alpha in [0.0, 1.0] {
            draw(&target.target, 1.0, |frame| {
                frame.clear(c::ColorF {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: alpha,
                });
                let ink = canvas_result(frame.create_solid_brush(WHITE))?;
                frame.clipped_text(
                    "Grayscale ABC 123",
                    &format,
                    &bounds(4.0, 4.0, 216.0, 46.0),
                    &ink,
                );
                frame.finish()
            })
            .unwrap();
            let pixels = target.pixels().unwrap();
            assert!(pixels.chunks_exact(4).any(|p| p[0] > 0));
            assert!(
                pixels
                    .chunks_exact(4)
                    .all(|p| p[0] == p[1] && p[1] == p[2] && p[0] <= p[3])
            );
        }
    }

    fn bounds(left: f32, top: f32, right: f32, bottom: f32) -> c::Rect {
        c::Rect {
            left,
            top,
            right,
            bottom,
        }
    }
    const WHITE: c::ColorF = c::ColorF {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    #[test]
    fn icon_ink_stays_centered_for_mdl2_and_fluent_at_multiple_scales() {
        let device = c::GpuDevice::new_warp().unwrap();
        for family in ["Segoe MDL2 Assets", super::super::fonts::icon_family()] {
            let format = c::TextFormat::new(family, 20.0).unwrap()
                .with_alignment(c::TextAlignment::Center)
                .with_paragraph_alignment(c::ParagraphAlignment::Center);
            for scale in [1.0, 1.5, 2.0] {
                let size = (64.0 * scale) as u32;
                let surface = Offscreen::new(&device, size, size).unwrap();
                for glyph in ["\u{e790}", "\u{f0e2}", "\u{e721}", "\u{e70d}", "\u{e8bb}"] {
                    draw(&surface.target, scale, |frame| {
                        frame.clear(c::ColorF::new(0.0, 0.0, 0.0, 0.0));
                        let ink = canvas_result(frame.create_solid_brush(WHITE))?;
                        frame.clipped_icon(glyph, &format, &bounds(8.0, 8.0, 48.0, 48.0), &ink);
                        frame.finish()
                    }).unwrap();
                    let pixels = surface.pixels().unwrap();
                    let rows: Vec<_> = (0..size as usize).filter(|y|
                        pixels[y * size as usize * 4..(y + 1) * size as usize * 4]
                            .chunks_exact(4).any(|pixel| pixel[3] > 16)).collect();
                    let center = (*rows.first().unwrap() + *rows.last().unwrap() + 1) as f32 / 2.0;
                    assert!((center - 28.0 * scale).abs() <= 1.0,
                        "{family} {glyph} scale={scale} ink center={center}");
                }
            }
        }
    }

    #[test]
    fn failed_bitmap_upload_unwinds_clip_and_draw_before_next_frame() {
        let device = c::GpuDevice::new_warp().unwrap();
        let surface = Offscreen::new(&device, 64, 64).unwrap();
        let failed = draw(&surface.target, 1.0, |frame| {
            frame.push_clip(&bounds(0.0, 0.0, 8.0, 8.0));
            canvas_result(frame.create_bitmap(&[0; 4], 4, 4))?;
            frame.finish()
        });
        assert!(failed.is_err());
        draw(&surface.target, 1.0, |frame| {
            frame.clear(c::ColorF::default());
            let brush = canvas_result(frame.create_solid_brush(WHITE))?;
            frame.fill_rect(&bounds(0.0, 0.0, 64.0, 64.0), &brush);
            frame.finish()
        })
        .unwrap();
        assert!(
            surface
                .pixels()
                .unwrap()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [255; 4])
        );
    }

    #[test]
    fn text_remains_inside_its_bounds_at_multiple_dpi() {
        let device = c::GpuDevice::new_warp().unwrap();
        let format = c::TextFormat::new("Microsoft YaHei UI", 24.0).unwrap();
        let format = format.with_word_wrapping(c::WordWrapping::NoWrap);
        for (scale, edge) in [(1.0, 32usize), (1.5, 48), (2.0, 64)] {
            let surface = Offscreen::new(&device, 192, 128).unwrap();
            draw(&surface.target, scale, |frame| {
                frame.clear(c::ColorF::default());
                let brush = canvas_result(frame.create_solid_brush(WHITE))?;
                frame.clipped_text(
                    "中文名称 ABCDEFG",
                    &format,
                    &bounds(0.0, 0.0, 32.0, 32.0),
                    &brush,
                );
                frame.finish()
            })
            .unwrap();
            let pixels = surface.pixels().unwrap();
            assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
            for (index, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
                if index % 192 >= edge || index / 192 >= edge {
                    assert_eq!(pixel[3], 0, "text escaped the clipping rectangle");
                }
            }
        }
    }
}
