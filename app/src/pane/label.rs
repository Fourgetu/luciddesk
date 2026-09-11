//! Shared DirectWrite layout for labels, selection geometry and drag previews.
use super::native_graphics::canvas_result;
use super::{assets::Pixels, canvas};
use windows_canvas::ColorF;

use windows::core::Result;
use windows_canvas::{TextAlignment, TextFormat, TextLayout, WordWrapping};

pub struct Label {
    pub pixels: Pixels,
    #[cfg(test)]
    pub text_height: u32,
    pub padding: u32,
}

pub fn layout(text: &str, width: u32, dpi: u32, lines: u32) -> Result<(TextLayout, f32)> {
    use std::{cell::RefCell, collections::HashMap};
    thread_local! {
        static CACHE: RefCell<HashMap<(String, u32, u32, u32), (TextLayout, f32)>> = RefCell::new(HashMap::new());
    }
    let key = (text.to_owned(), width, dpi, lines);
    CACHE.with(|cache| {
        if let Some(value) = cache.borrow().get(&key) {
            return Ok(value.clone());
        }
        let scale = dpi.max(48) as f32 / 96.0;
        let (_, size) = super::assets::font();
        let format = canvas_result(TextFormat::new(super::assets::UI_FONT, size))?
            .with_alignment(TextAlignment::Center)
            .with_word_wrapping(WordWrapping::Wrap);
        canvas::ellipsis(&format)?;
        let probe = canvas_result(TextLayout::new("A", &format, 1000.0, 1000.0))?;
        let max_height = probe.metrics().height * lines as f32;
        let layout = canvas_result(TextLayout::new(
            text,
            &format,
            (width as f32 / scale - 4.0).max(1.0),
            max_height,
        ))?;
        let height = (layout.metrics().height.min(max_height) * scale).ceil();
        let mut cache = cache.borrow_mut();
        if cache.len() >= 1024 {
            cache.clear();
        }
        cache.insert(key, (layout.clone(), height));
        Ok((layout, height))
    })
}

// Drag images still need pixels, but their glyphs use the same DirectWrite
// layout as live panes. Transparent drag bitmaps always use grayscale.
pub fn raster(text: &str, width: u32, dpi: u32, max_lines: u32) -> Option<Label> {
    if !(8..=4096).contains(&width) || !(48..=768).contains(&dpi) || !(1..=8).contains(&max_lines) {
        return None;
    }
    let (layout, height) = layout(text, width, dpi, max_lines).ok()?;
    let scale = dpi as f32 / 96.0;
    let padding = (2.0 * scale).ceil() as u32;
    let text_height = height as u32;
    let height = text_height + padding * 2;
    let device = windows_canvas::GpuDevice::new().ok()?;
    let bitmap = canvas::Offscreen::new(&device, width, height).ok()?;
    canvas::draw(&bitmap.target, scale, |frame| {
        frame.clear(ColorF {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        });
        let ink = canvas_result(frame.create_solid_brush(ColorF::WHITE))?;
        frame.clipped_layout(&layout, 2.0, padding as f32 / scale, &ink);
        frame.finish()
    })
    .ok()?;
    let data = bitmap.pixels().ok()?;
    let mask: Vec<_> = data.chunks_exact(4).map(|pixel| pixel[3]).collect();
    Some(Label {
        pixels: Pixels {
            width,
            height,
            data: compose_shadow(&mask, width as usize, height as usize, padding as usize),
        },
        #[cfg(test)]
        text_height,
        padding,
    })
}

fn compose_shadow(mask: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let mut shadow: Vec<f32> = mask.iter().map(|p| f32::from(*p)).collect();
    for _ in 0..radius {
        let old = shadow.clone();
        for y in 0..height {
            for x in 0..width {
                let sample = |dx: isize, dy: isize| {
                    let (nx, ny) = (x as isize + dx, y as isize + dy);
                    if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
                        0.0
                    } else {
                        old[ny as usize * width + nx as usize]
                    }
                };
                shadow[y * width + x] = (sample(0, 0) * 4.0
                    + sample(-1, 0) * 2.0
                    + sample(1, 0) * 2.0
                    + sample(0, -1) * 2.0
                    + sample(0, 1) * 2.0
                    + sample(-1, -1)
                    + sample(1, -1)
                    + sample(-1, 1)
                    + sample(1, 1))
                    / 16.0;
            }
        }
    }
    let mut pixels = vec![0; mask.len() * 4];
    for (at, p) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let ink = f32::from(mask[at]);
        // A faint, centered halo preserves wallpaper contrast without a
        // displaced second outline below the single antialiased glyph mask.
        let shade = shadow[at] * 0.4;
        let alpha = (ink + shade * (1.0 - ink / 255.0)).clamp(0.0, 255.0);
        *p = [ink as u8, ink as u8, ink as u8, alpha.round() as u8];
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_font_wraps_at_each_dpi_without_an_opaque_background() {
        for dpi in [96, 144, 192] {
            let width = 75 * dpi / 96;
            let single = raster("同花顺", width, dpi, 2).expect("system icon font");
            let wrapped = raster("桌面图标文字换行测试", width, dpi, 2).expect("wrapped label");
            assert!(wrapped.text_height > single.text_height);
            // DirectWrite retains fractional line heights; rounding two lines
            // together may differ by one pixel from twice a rounded line.
            assert!(wrapped.text_height.abs_diff(single.text_height * 2) <= 1);
            for label in [single, wrapped] {
                assert_eq!(
                    label.pixels.data.len(),
                    (width * label.pixels.height * 4) as usize
                );
                let pixels = label.pixels.data.as_chunks::<4>().0;
                assert!(pixels.iter().any(|p| p[3] == 0));
                assert!(pixels.iter().any(|p| p[0] > 200));
                assert!(
                    pixels
                        .iter()
                        .all(|p| p[0] <= p[3] && p[0] == p[1] && p[1] == p[2])
                );
            }
        }
    }

    #[test]
    fn soft_shadow_is_centered_and_does_not_duplicate_glyph_edges() {
        let mut mask = vec![0; 81];
        mask[40] = 255;
        let pixels = compose_shadow(&mask, 9, 9, 2);
        for y in 0..9 {
            for x in 0..9 {
                let at = y * 9 + x;
                assert_eq!(pixels[at * 4], mask[at], "glyph coverage is unchanged");
                assert_eq!(pixels[at * 4 + 3], pixels[((8 - y) * 9 + x) * 4 + 3]);
                assert_eq!(pixels[at * 4 + 3], pixels[(y * 9 + 8 - x) * 4 + 3]);
                if at != 40 {
                    assert!(pixels[at * 4 + 3] <= 102);
                }
            }
        }
    }

    #[test]
    fn shadow_retains_opaque_glyph_and_transparent_background() {
        let mut mask = vec![0; 81];
        mask[40] = 255;
        let pixels = compose_shadow(&mask, 9, 9, 2);
        assert_eq!(&pixels[160..164], &[255, 255, 255, 255]);
        assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[0] == 0 && p[3] > 0)
        );
        assert!(pixels.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3]));
    }
}

// Measurement is cached separately from GPU uploads; resizing/hit testing never
// repeatedly rasterizes labels. The same wrapping routine supplies paint metrics.
pub fn content_height(text: &str, width: u32) -> f32 {
    content_height_at_dpi(text, width, 96)
}

pub fn content_height_at_dpi(text: &str, width: u32, dpi: u32) -> f32 {
    use std::{cell::RefCell, collections::BTreeMap};
    thread_local! {
        static HEIGHTS: RefCell<BTreeMap<(String, u32, u32), f32>> = RefCell::new(BTreeMap::new());
    }
    HEIGHTS.with(|cache| {
        let key = (text.to_owned(), width, dpi);
        if let Some(height) = cache.borrow().get(&key) {
            return *height;
        }
        let height =
            layout(text, width, dpi, 2).map_or(16.0 * dpi as f32 / 96.0, |(_, height)| height);
        let mut cache = cache.borrow_mut();
        if cache.len() >= 1024 {
            cache.clear();
        }
        cache.insert(key, height);
        height
    })
}
