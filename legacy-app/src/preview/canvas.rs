//! Canvas drawing with explicit error reporting and the existing DIP layout types.
use super::composition::{canvas_result, native_interface};
use canvas_core::Interface as _;
use windows::Win32::Graphics::{
    Direct2D::{
        Common::{D2D_RECT_F, D2D1_COLOR_F},
        D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ELLIPSE, D2D1_ROUNDED_RECT,
        D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, ID2D1DeviceContext, ID2D1Image, ID2D1RenderTarget,
    },
    DirectWrite::IDWriteTextFormat,
};
use windows::core::{Interface, Result};
use windows_canvas as c;

pub struct Format {
    canvas: c::TextFormat,
    native: IDWriteTextFormat,
}

impl std::ops::Deref for Format {
    type Target = IDWriteTextFormat;
    fn deref(&self) -> &Self::Target {
        &self.native
    }
}
impl Format {
    pub fn new(family: &str, size: f32, weight: i32) -> Result<Self> {
        let canvas = canvas_result(c::TextFormat::with_weight(
            family,
            size,
            c::FontWeight(weight),
        ))?;
        let native = native_interface(canvas.raw())?;
        Ok(Self { canvas, native })
    }
}

pub struct Brush {
    canvas: c::Brush,
    color: c::ColorF,
}
impl Brush {
    pub fn set_opacity(&self, opacity: f32) {
        self.canvas.set_color(c::ColorF {
            a: self.color.a * opacity,
            ..self.color
        });
    }
}
fn rect(r: &D2D_RECT_F) -> c::Rect {
    c::Rect::new(r.left, r.top, r.right, r.bottom)
}
fn rounded(r: &D2D1_ROUNDED_RECT) -> c::RoundedRect {
    c::RoundedRect {
        rect: rect(&r.rect),
        radius_x: r.radiusX,
        radius_y: r.radiusY,
    }
}
fn point(p: windows_numerics::Vector2) -> c::Vector2 {
    c::Vector2 { x: p.X, y: p.Y }
}
fn ellipse(e: &D2D1_ELLIPSE) -> c::Ellipse {
    c::Ellipse {
        center: point(e.point),
        radius_x: e.radiusX,
        radius_y: e.radiusY,
    }
}

pub struct Frame<'a> {
    session: c::DrawingSession<'a>,
    native: ID2D1DeviceContext,
    active: bool,
    clips: std::cell::Cell<usize>,
}

pub fn draw<T>(
    target: &ID2D1RenderTarget,
    scale: f32,
    paint: impl FnOnce(Frame<'_>) -> Result<T>,
) -> Result<T> {
    let native: ID2D1DeviceContext = target.cast()?;
    let raw = native.as_raw();
    // QueryInterface owns a fresh reference; target remains alive throughout.
    let context = canvas_result(unsafe {
        canvas_core::IUnknown::from_raw_borrowed(&raw)
            .unwrap()
            .cast()
    })?;
    unsafe {
        native.SetDpi(96.0 * scale, 96.0 * scale);
        native.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        native.BeginDraw();
    }
    let session = c::DrawingSession::from_borrowed_context(&context, c::Matrix3x2::identity());
    paint(Frame {
        session,
        native,
        active: true,
        clips: std::cell::Cell::new(0),
    })
}

impl Frame<'_> {
    fn session(&self) -> &c::DrawingSession<'_> {
        &self.session
    }
    pub fn brush(&self, color: &D2D1_COLOR_F) -> Result<Brush> {
        let color = c::ColorF::new(color.r, color.g, color.b, color.a);
        Ok(Brush {
            canvas: canvas_result(self.session().create_solid_brush(color))?,
            color,
        })
    }
    pub fn text_layout(&self, layout: &windows::Win32::Graphics::DirectWrite::IDWriteTextLayout,
        x: f32, y: f32, color: D2D1_COLOR_F) -> Result<()> {
        unsafe {
            let brush = self.native.CreateSolidColorBrush(&color, None)?;
            self.native.DrawTextLayout(windows_numerics::Vector2 { X: x, Y: y }, layout, &brush,
                windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_CLIP);
        }
        Ok(())
    }
    pub fn clear(&self, color: &D2D1_COLOR_F) {
        self.session()
            .clear(c::ColorF::new(color.r, color.g, color.b, color.a));
    }
    pub fn fill_rect(&self, r: &D2D_RECT_F, brush: &Brush) {
        self.session().fill_rect(&rect(r), &brush.canvas);
    }
    pub fn fill_rounded_rect(&self, r: &D2D1_ROUNDED_RECT, brush: &Brush) {
        self.session().fill_rounded_rect(&rounded(r), &brush.canvas);
    }
    pub fn draw_rounded_rect(&self, r: &D2D1_ROUNDED_RECT, brush: &Brush, width: f32) {
        self.session()
            .draw_rounded_rect(&rounded(r), &brush.canvas, width);
    }
    pub fn fill_ellipse(&self, e: &D2D1_ELLIPSE, brush: &Brush) {
        self.session().fill_ellipse(&ellipse(e), &brush.canvas);
    }
    pub fn draw_ellipse(&self, e: &D2D1_ELLIPSE, brush: &Brush, width: f32) {
        self.session()
            .draw_ellipse(&ellipse(e), &brush.canvas, width);
    }
    pub fn draw_line(
        &self,
        a: windows_numerics::Vector2,
        b: windows_numerics::Vector2,
        brush: &Brush,
        width: f32,
    ) {
        self.session()
            .draw_line(point(a), point(b), &brush.canvas, width);
    }
    pub fn text(&self, text: &str, format: &Format, r: &D2D_RECT_F, brush: &Brush) {
        // Canvas 0.100 uses DRAW_TEXT_OPTIONS_NONE; preserve the old CLIP behavior.
        self.push_clip(r);
        self.session()
            .draw_text(text, &format.canvas, &rect(r), &brush.canvas);
        self.pop_clip();
    }
    pub fn bitmap(&self, pixels: &[u8], width: u32, height: u32) -> Result<c::Bitmap> {
        canvas_result(self.session().create_bitmap(pixels, width, height))
    }
    pub fn draw_bitmap(&self, bitmap: &c::Bitmap, r: &D2D_RECT_F, opacity: f32) {
        self.session().draw_bitmap(bitmap, &rect(r), opacity);
    }
    pub fn push_clip(&self, r: &D2D_RECT_F) {
        unsafe {
            self.native
                .PushAxisAlignedClip(r, D2D1_ANTIALIAS_MODE_ALIASED);
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
impl Drop for Frame<'_> {
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

/// CPU readback for menus and tests; all drawing still uses the Canvas GPU context.
pub struct Offscreen {
    canvas: c::RenderTarget,
    context: ID2D1DeviceContext,
    image: ID2D1Image,
    pub target: ID2D1RenderTarget,
}
impl Offscreen {
    pub fn new(device: &c::GpuDevice, width: u32, height: u32) -> Result<Self> {
        let canvas = canvas_result(device.create_render_target(width, height))?;
        let mut captured = None;
        canvas_result(canvas.draw(|session| {
            captured = Some((|| -> Result<_> {
                let context: ID2D1DeviceContext = native_interface(session.raw())?;
                let image = unsafe { context.GetTarget()? };
                Ok((context, image))
            })());
            Ok(())
        }))?;
        let (context, image) = captured.expect("Canvas invokes the draw closure synchronously")?;
        unsafe {
            context.SetTarget(&image);
        }
        let target = context.cast()?;
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
    use windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING_NO_WRAP;

    #[test]
    fn text_uses_grayscale_on_both_opaque_and_transparent_surfaces() {
        let device = windows_canvas::GpuDevice::new().unwrap();
        let target = Offscreen::new(&device, 220, 50).unwrap();
        let format = Format::new(super::super::assets::UI_FONT, 17.0, 400).unwrap();
        for alpha in [0.0, 1.0] {
            draw(&target.target, 1.0, |frame| {
                frame.clear(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: alpha });
                let ink = frame.brush(&WHITE)?;
                frame.text("Grayscale ABC 123", &format, &bounds(4.0, 4.0, 216.0, 46.0), &ink);
                frame.finish()
            }).unwrap();
            let pixels = target.pixels().unwrap();
            assert!(pixels.chunks_exact(4).any(|p| p[0] > 0));
            assert!(pixels.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2] && p[0] <= p[3]));
        }
    }

    fn bounds(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
        D2D_RECT_F {
            left,
            top,
            right,
            bottom,
        }
    }
    const WHITE: D2D1_COLOR_F = D2D1_COLOR_F {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    #[test]
    fn failed_bitmap_upload_unwinds_clip_and_draw_before_next_frame() {
        let device = c::GpuDevice::new_warp().unwrap();
        let surface = Offscreen::new(&device, 64, 64).unwrap();
        let failed = draw(&surface.target, 1.0, |frame| {
            frame.push_clip(&bounds(0.0, 0.0, 8.0, 8.0));
            frame.bitmap(&[0; 4], 4, 4)?;
            frame.finish()
        });
        assert!(failed.is_err());
        draw(&surface.target, 1.0, |frame| {
            frame.clear(&D2D1_COLOR_F::default());
            let brush = frame.brush(&WHITE)?;
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
        let format = Format::new("Microsoft YaHei UI", 24.0, 400).unwrap();
        unsafe {
            format
                .SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)
                .unwrap();
        }
        for (scale, edge) in [(1.0, 32usize), (1.5, 48), (2.0, 64)] {
            let surface = Offscreen::new(&device, 192, 128).unwrap();
            draw(&surface.target, scale, |frame| {
                frame.clear(&D2D1_COLOR_F::default());
                let brush = frame.brush(&WHITE)?;
                frame.text(
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
