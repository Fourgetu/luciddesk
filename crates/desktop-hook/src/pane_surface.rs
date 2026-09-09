//! Backgrounds only. Explorer continues to render every icon, label and selection.
#![allow(
    clippy::wildcard_imports,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
use crate::protocol::{PANE_HEADER, PaneAppearance, TextureHeader};
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::*;

pub fn pane_region(pane: &PaneAppearance) -> HRGN {
    let b = pane.bounds;
    unsafe {
        CreateRoundRectRgn(
            b.left,
            b.top,
            b.right + 1,
            b.bottom + 1,
            pane.radius * 2,
            pane.radius * 2,
        )
    }
}

pub fn inside(pane: &PaneAppearance, x: i32, y: i32) -> bool {
    if !pane.bounds.contains(x, y) {
        return false;
    }
    let b = pane.bounds;
    let r = pane.radius;
    if r == 0 {
        return true;
    }
    let cx = x.clamp(b.left + r, b.right - r - 1);
    let cy = y.clamp(b.top + r, b.bottom - r - 1);
    (x - cx).pow(2) + (y - cy).pow(2) <= r * r
}

struct Bitmap {
    dc: HDC,
    handle: HBITMAP,
    previous: HGDIOBJ,
    header: TextureHeader,
}
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.handle);
            DeleteDC(self.dc);
        }
    }
}

pub struct Surface {
    texture: Option<Bitmap>,
    tint: Bitmap,
    font: HFONT,
}
impl Surface {
    pub fn new() -> Result<Self, String> {
        let header = TextureHeader {
            version: crate::protocol::VERSION,
            width: 1,
            height: 1,
            bounds: crate::protocol::Area {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            },
        };
        let tint = Self::bitmap(header, &[32, 27, 24, 255])?;
        let font = unsafe {
            CreateFontW(
                -20,
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                u32::from(DEFAULT_CHARSET),
                u32::from(OUT_DEFAULT_PRECIS),
                u32::from(CLIP_DEFAULT_PRECIS),
                u32::from(CLEARTYPE_QUALITY),
                u32::from(DEFAULT_PITCH),
                windows_sys::w!("Segoe UI"),
            )
        };
        if font.is_null() {
            return Err("无法创建分组标题字体".into());
        }
        Ok(Self {
            texture: None,
            tint,
            font,
        })
    }
    fn bitmap(header: TextureHeader, pixels: &[u8]) -> Result<Bitmap, String> {
        if header.byte_count() != Some(pixels.len()) {
            return Err("无效的材质像素".into());
        }
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                return Err("无法创建材质 DC".into());
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: header.width as i32,
                    biHeight: -(header.height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(
                dc,
                &raw const info,
                DIB_RGB_COLORS,
                &raw mut bits,
                null_mut(),
                0,
            );
            if bitmap.is_null() {
                DeleteDC(dc);
                return Err("无法创建材质位图".into());
            }
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast(), pixels.len());
            let previous = SelectObject(dc, bitmap);
            Ok(Bitmap {
                dc,
                handle: bitmap,
                previous,
                header,
            })
        }
    }
    pub fn texture(&mut self, header: TextureHeader, pixels: &[u8]) -> Result<(), String> {
        self.texture = Some(Self::bitmap(header, pixels)?);
        Ok(())
    }
    #[allow(clippy::too_many_lines)]
    pub fn paint(&self, dc: HDC, panes: &[PaneAppearance]) {
        unsafe {
            for pane in panes {
                let b = pane.bounds;
                let bounds = RECT {
                    left: b.left,
                    top: b.top,
                    right: b.right,
                    bottom: b.bottom,
                };
                if RectVisible(dc, &raw const bounds) == 0 {
                    continue;
                }
                let saved = SaveDC(dc);
                if saved == 0 {
                    continue;
                }
                let region = pane_region(pane);
                if region.is_null() {
                    RestoreDC(dc, saved);
                    continue;
                }
                ExtSelectClipRgn(dc, region, RGN_AND);
                if pane.material==2 {
                    // Isolated native-backdrop experiment: expose the parent DWM backdrop.
                    FillRect(dc,&raw const bounds,GetStockObject(BLACK_BRUSH));
                    DeleteObject(region);RestoreDC(dc,saved);continue;
                }
                let brush = CreateSolidBrush(0x0020_1b18);
                FillRect(dc, &raw const bounds, brush);
                DeleteObject(brush);
                if let Some(texture) = &self.texture {
                    let h = texture.header;
                    let s = h.bounds;
                    SetStretchBltMode(dc, HALFTONE);
                    SetBrushOrgEx(dc, 0, 0, null_mut());
                    let sx = (b.left - s.left) * h.width as i32 / (s.right - s.left);
                    let sy = (b.top - s.top) * h.height as i32 / (s.bottom - s.top);
                    let sw = ((b.right - b.left) * h.width as i32 / (s.right - s.left)).max(1);
                    let sh = ((b.bottom - b.top) * h.height as i32 / (s.bottom - s.top)).max(1);
                    StretchBlt(
                        dc,
                        b.left,
                        b.top,
                        b.right - b.left,
                        b.bottom - b.top,
                        texture.dc,
                        sx,
                        sy,
                        sw,
                        sh,
                        SRCCOPY,
                    );
                }
                AlphaBlend(
                    dc,
                    b.left,
                    b.top,
                    b.right - b.left,
                    b.bottom - b.top,
                    self.tint.dc,
                    0,
                    0,
                    1,
                    1,
                    BLENDFUNCTION {
                        BlendOp: AC_SRC_OVER as u8,
                        BlendFlags: 0,
                        SourceConstantAlpha: if pane.material == 0 { 152 } else { 224 },
                        AlphaFormat: 0,
                    },
                );
                let edge = CreateSolidBrush(0x0060_5a52);
                FrameRgn(dc, region, edge, 1, 1);
                DeleteObject(edge);
                SelectObject(dc, self.font);
                SetBkMode(dc, TRANSPARENT as i32);
                SetTextColor(dc, 0x00f5_f3f2);
                let mut title = RECT {
                    left: b.left + 16,
                    top: b.top,
                    right: b.right - 52,
                    bottom: b.top + PANE_HEADER,
                };
                let len = pane.title.iter().position(|&c| c == 0).unwrap_or(95) as i32;
                DrawTextW(
                    dc,
                    pane.title.as_ptr(),
                    len,
                    &raw mut title,
                    DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                );
                let mut button = RECT {
                    left: b.right - 40,
                    top: b.top,
                    right: b.right - 12,
                    bottom: b.top + PANE_HEADER,
                };
                DrawTextW(
                    dc,
                    windows_sys::w!("⋯"),
                    1,
                    &raw mut button,
                    DT_SINGLELINE | DT_VCENTER,
                );
                DeleteObject(region);
                RestoreDC(dc, saved);
            }
        }
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.font);
        }
    }
}
