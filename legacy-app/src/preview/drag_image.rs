//! A nonactivating, per-pixel-alpha overlay follows the pointer across windows.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::wildcard_imports
)]
use super::assets::Pixels;
use windows_sys::Win32::{
    Foundation::{HWND, POINT, SIZE},
    Graphics::Gdi::*,
    UI::WindowsAndMessaging::*,
};

pub struct DragImage {
    hwnd: HWND,
    hotspot: POINT,
}

/// Keep the drag preview in cell coordinates so dragging by the label does not jump.
pub fn item_pixels(
    image: &Pixels,
    name: &str,
    grid: super::layout::Grid,
    scale: f32,
    managed: bool,
) -> Option<Pixels> {
    if image.width == 0 || image.height == 0 {
        return None;
    }
    let width = (grid.cell_width * scale).round().max(1.0) as u32;
    let label = super::label::raster(name, width, (96.0 * scale).round() as u32, 2)?;
    let label_y = ((grid.icon_size + if managed { crate::preview::layout::LABEL_OFFSET } else { 9.0 }) * scale)
        .round() as u32 - label.padding;
    let height = label_y + label.pixels.height;
    let mut data = vec![0; (width * height * 4) as usize];
    let ratio = grid.icon_size * scale / image.width.max(image.height) as f32;
    let iw = (image.width as f32 * ratio).round().max(1.0) as u32;
    let ih = (image.height as f32 * ratio).round().max(1.0) as u32;
    let left = (width - iw) / 2;
    let top = ((if managed { 2.0 } else { 4.0 }) * scale
        + (grid.icon_size * scale - ih as f32) / 2.0).round() as u32;
    let icon = resize(image, iw, ih);
    for y in 0..ih {
        let start = (((top + y) * width + left) * 4) as usize;
        let source = (y * iw * 4) as usize;
        data[start..start + (iw * 4) as usize]
            .copy_from_slice(&icon[source..source + (iw * 4) as usize]);
    }
    for (source, destination) in label.pixels.data.chunks_exact(4)
        .zip(data[(label_y * width * 4) as usize..].chunks_exact_mut(4))
    {
        let remaining = 255 - u16::from(source[3]);
        for c in 0..4 {
            destination[c] = (u16::from(source[c])
                + (u16::from(destination[c]) * remaining + 127) / 255) as u8;
        }
    }
    Some(Pixels { width, height, data })
}

// Interpolate premultiplied BGRA without losing edge alpha.
fn resize(source: &Pixels, width: u32, height: u32) -> Vec<u8> {
    let mut output = vec![0; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let sx = ((x as f32 + 0.5) * source.width as f32 / width as f32 - 0.5)
                .clamp(0.0, (source.width - 1) as f32);
            let sy = ((y as f32 + 0.5) * source.height as f32 / height as f32 - 0.5)
                .clamp(0.0, (source.height - 1) as f32);
            let (x0, y0) = (sx as u32, sy as u32);
            let (x1, y1) = (
                (x0 + 1).min(source.width - 1),
                (y0 + 1).min(source.height - 1),
            );
            let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
            for c in 0..4 {
                let sample =
                    |x, y| f32::from(source.data[((y * source.width + x) * 4 + c) as usize]);
                let top = sample(x0, y0) * (1.0 - fx) + sample(x1, y0) * fx;
                let bottom = sample(x0, y1) * (1.0 - fx) + sample(x1, y1) * fx;
                output[((y * width + x) * 4 + c) as usize] =
                    (top * (1.0 - fy) + bottom * fy).round() as u8;
            }
        }
    }
    output
}

impl DragImage {
    pub fn new(
        owner: HWND,
        pixels: &Pixels,
        point: POINT,
        hotspot: POINT,
        size: SIZE,
    ) -> Option<Self> {
        if pixels.width == 0 || pixels.height == 0 || size.cx <= 0 || size.cy <= 0 {
            return None;
        }
        let data = resize(pixels, size.cx as u32, size.cy as u32);
        unsafe {
            let class: Vec<_> = "STATIC\0".encode_utf16().collect();
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                size.cx,
                size.cy,
                owner,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return None;
            }
            let overlay = Self { hwnd, hotspot };
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            if dc.is_null() {
                return None;
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size.cx,
                    biHeight: -size.cy,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                dc,
                &raw const info,
                DIB_RGB_COLORS,
                &raw mut bits,
                std::ptr::null_mut(),
                0,
            );
            if bitmap.is_null() {
                DeleteDC(dc);
                return None;
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), bits.cast(), data.len());
            let old = SelectObject(dc, bitmap);
            let destination = POINT {
                x: point.x - hotspot.x,
                y: point.y - hotspot.y,
            };
            let origin = POINT::default();
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 210,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let ok = UpdateLayeredWindow(
                hwnd,
                std::ptr::null_mut(),
                &raw const destination,
                &raw const size,
                dc,
                &raw const origin,
                0,
                &raw const blend,
                ULW_ALPHA,
            );
            SelectObject(dc, old);
            DeleteObject(bitmap);
            DeleteDC(dc);
            if ok == 0 {
                return None;
            }
            // Present alpha pixels before making the window visible to avoid a black flash.
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            Some(overlay)
        }
    }
    pub fn move_to(&self, point: POINT) {
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                point.x - self.hotspot.x,
                point.y - self.hotspot.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
    }
}
impl Drop for DragImage {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drag_preview_includes_label_at_each_dpi() {
        for scale in [1.0, 1.5, 2.0] {
            for managed in [true, false] {
                let grid = super::super::layout::Grid::system(
                    400.0, 300.0, 48.0, (88.0, 96.0), true,
                );
                for (width, height) in [(16, 16), (32, 16), (16, 32)] {
                    let source = Pixels {
                        width, height,
                        data: [60, 120, 240, 255].repeat((width * height) as usize),
                    };
                    let preview = item_pixels(&source, "微信 Music", grid, scale, managed)
                        .expect("icon and label preview");
                    let label_start = ((grid.icon_size + crate::preview::layout::LABEL_OFFSET) * scale).ceil() as u32;
                    assert!(preview.data[(label_start * preview.width * 4) as usize..]
                        .chunks_exact(4).any(|p| p[0] > 200 && p[1] > 200 && p[2] > 200),
                        "the dragged name must remain visible below the icon");
                    assert!(preview.data.chunks_exact(4).any(|p| p == [60, 120, 240, 255]));
                    assert_eq!(&preview.data[..4], &[0, 0, 0, 0]);
                    assert!(preview.data.chunks_exact(4)
                        .all(|p| p[..3].iter().all(|c| *c <= p[3])));
                }
            }
        }
    }

    #[test]
    fn scaling_preserves_transparent_pixels_and_premultiplied_edges() {
        let source = Pixels {
            width: 2,
            height: 1,
            data: vec![0, 0, 0, 0, 60, 120, 240, 255],
        };
        let output = resize(&source, 4, 2);
        assert_eq!(&output[..4], &[0, 0, 0, 0]);
        assert_eq!(&output[12..16], &[60, 120, 240, 255]);
        assert_eq!(&output[4..8], &[15, 30, 60, 64]);
        assert_eq!(&output[..16], &output[16..]);
    }
}
