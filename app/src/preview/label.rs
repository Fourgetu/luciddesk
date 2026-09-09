//! Desktop labels use the system icon LOGFONT and GDI wrapping metrics.
#![allow(
    clippy::wildcard_imports,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
use super::assets::Pixels;
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::Gdi::*,
    UI::{HiDpi::SystemParametersInfoForDpi, WindowsAndMessaging::SPI_GETICONTITLELOGFONT},
};

pub struct Label {
    pub pixels: Pixels,
    pub text_height: u32,
    pub padding: u32,
}
struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    font: HFONT,
    old_bitmap: HGDIOBJ,
    old_font: HGDIOBJ,
}
impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old_bitmap);
            SelectObject(self.dc, self.old_font);
            DeleteObject(self.bitmap);
            DeleteObject(self.font);
            DeleteDC(self.dc);
        }
    }
}

#[allow(clippy::too_many_lines)] // Keep the GDI resource lifetime and bitmap transfer together.
pub fn raster(text: &str, width: u32, dpi: u32, max_lines: u32) -> Option<Label> {
    if !(8..=4096).contains(&width) || !(48..=768).contains(&dpi) || !(1..=8).contains(&max_lines) {
        return None;
    }
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return None;
        }
        let mut canvas = Canvas {
            dc,
            bitmap: std::ptr::null_mut(),
            font: std::ptr::null_mut(),
            old_bitmap: std::ptr::null_mut(),
            old_font: std::ptr::null_mut(),
        };
        let mut font = LOGFONTW::default();
        if SystemParametersInfoForDpi(
            SPI_GETICONTITLELOGFONT,
            size_of::<LOGFONTW>() as u32,
            (&raw mut font).cast(),
            0,
            dpi,
        ) == 0
        {
            return None;
        }
        font.lfQuality = ANTIALIASED_QUALITY;
        canvas.font = CreateFontIndirectW(&raw const font);
        if canvas.font.is_null() {
            return None;
        }
        canvas.old_font = SelectObject(dc, canvas.font);
        let mut metrics = TEXTMETRICW::default();
        if GetTextMetricsW(dc, &raw mut metrics) == 0 {
            return None;
        }
        let padding = (dpi as f32 / 96.0 * 2.0).ceil() as u32;
        if padding * 2 >= width || metrics.tmHeight <= 0 {
            return None;
        }
        let mut text: Vec<_> = text.encode_utf16().collect();
        let flags = DT_CENTER | DT_WORDBREAK | DT_NOPREFIX | DT_END_ELLIPSIS;
        let mut bounds = RECT {
            left: padding as i32,
            top: padding as i32,
            right: (width - padding) as i32,
            bottom: 4096,
        };
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            text.len() as i32,
            &raw mut bounds,
            flags | DT_CALCRECT,
        );
        let text_height = (bounds.bottom - bounds.top)
            .max(metrics.tmHeight)
            .min(metrics.tmHeight * max_lines as i32) as u32;
        let height = text_height + padding * 2;
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        canvas.bitmap = CreateDIBSection(
            dc,
            &raw const info,
            DIB_RGB_COLORS,
            &raw mut bits,
            std::ptr::null_mut(),
            0,
        );
        if canvas.bitmap.is_null() {
            return None;
        }
        canvas.old_bitmap = SelectObject(dc, canvas.bitmap);
        std::ptr::write_bytes(bits, 0, (width * height * 4) as usize);
        SetBkMode(dc, TRANSPARENT.cast_signed());
        SetTextColor(dc, 0x00ff_ffff);
        bounds = RECT {
            left: padding as i32,
            top: padding as i32,
            right: (width - padding) as i32,
            bottom: (padding + text_height) as i32,
        };
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            text.len() as i32,
            &raw mut bounds,
            flags,
        );
        GdiFlush();
        let data = std::slice::from_raw_parts(bits.cast::<u8>(), (width * height * 4) as usize);
        let mask: Vec<_> = data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[0].max(p[1]).max(p[2]))
            .collect();
        Some(Label {
            pixels: Pixels {
                width,
                height,
                data: compose_shadow(&mask, width as usize, height as usize, padding as usize),
            },
            text_height,
            padding,
        })
    }
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
        let shade = if at >= width { shadow[at - width] } else { 0.0 };
        let alpha = (ink + shade * 1.5 * (1.0 - ink / 255.0)).clamp(0.0, 255.0);
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
            assert_eq!(wrapped.text_height, single.text_height * 2);
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
