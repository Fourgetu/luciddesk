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
