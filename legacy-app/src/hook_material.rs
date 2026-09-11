//! Wallpaper-derived materials for the shared Explorer icon view, decoded outside Explorer.
use desktop_hook::protocol::{Area, TextureHeader, VERSION};
use windows::{
    Win32::{
        Foundation::GENERIC_READ,
        Graphics::Imaging::*,
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
        UI::Shell::{
            DWPOS_CENTER, DWPOS_FIT, DWPOS_SPAN, DWPOS_STRETCH, DWPOS_TILE, DesktopWallpaper,
            IDesktopWallpaper,
        },
    },
    core::{PCWSTR, PWSTR},
};

unsafe fn owned_string(value: PWSTR) -> String {
    let result = unsafe { value.to_string() }.unwrap_or_default();
    unsafe {
        CoTaskMemFree(Some(value.0.cast()));
    }
    result
}

struct Image {
    width: usize,
    height: usize,
    original: (f64, f64),
    pixels: Vec<u8>,
}
fn decode(factory: &IWICImagingFactory, path: &str) -> windows::core::Result<Image> {
    unsafe {
        let path: Vec<_> = path.encode_utf16().chain(Some(0)).collect();
        let frame = factory
            .CreateDecoderFromFilename(
                PCWSTR(path.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnLoad,
            )?
            .GetFrame(0)?;
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&raw mut w, &raw mut h)?;
        if w == 0 || h == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let scale = (2048.0 / f64::from(w.max(h))).min(1.0);
        let width = (f64::from(w) * scale).round().max(1.0) as u32;
        let height = (f64::from(h) * scale).round().max(1.0) as u32;
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&frame, width, height, WICBitmapInterpolationModeFant)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let mut pixels = vec![0; (width * height * 4) as usize];
        converter.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
        Ok(Image {
            width: width as usize,
            height: height as usize,
            original: (f64::from(w), f64::from(h)),
            pixels,
        })
    }
}

pub fn capture(
    origin: windows_sys::Win32::Foundation::POINT,
) -> Result<(TextureHeader, Vec<u8>), String> {
    unsafe {
        let wallpaper: IDesktopWallpaper =
            CoCreateInstance(&DesktopWallpaper, None, windows::Win32::System::Com::CLSCTX_ALL)
                .map_err(|e| e.to_string())?;
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
        let position = wallpaper.GetPosition().map_err(|e| e.to_string())?;
        let color = wallpaper.GetBackgroundColor().map_err(|e| e.to_string())?.0;
        let mut monitors = Vec::new();
        for i in 0..wallpaper
            .GetMonitorDevicePathCount()
            .map_err(|e| e.to_string())?
        {
            let id = wallpaper
                .GetMonitorDevicePathAt(i)
                .map_err(|e| e.to_string())?;
            let rect = wallpaper.GetMonitorRECT(PCWSTR(id.0));
            let path = wallpaper.GetWallpaper(PCWSTR(id.0));
            CoTaskMemFree(Some(id.0.cast()));
            let rect = rect.map_err(|e| e.to_string())?;
            if rect.right <= rect.left || rect.bottom <= rect.top {
                continue;
            }
            let path = owned_string(path.map_err(|e| e.to_string())?);
            let image = if path.is_empty() {
                None
            } else {
                Some(decode(&factory, &path).map_err(|e| e.to_string())?)
            };
            monitors.push((rect, image));
        }
        let left = monitors
            .iter()
            .map(|(r, _)| r.left)
            .min()
            .ok_or("没有壁纸显示器")?;
        let top = monitors.iter().map(|(r, _)| r.top).min().unwrap();
        let right = monitors.iter().map(|(r, _)| r.right).max().unwrap();
        let bottom = monitors.iter().map(|(r, _)| r.bottom).max().unwrap();
        let factor = 4.0_f64.max(f64::from((right - left).max(bottom - top)) / 1600.0);
        let width = (f64::from(right - left) / factor).ceil() as usize;
        let height = (f64::from(bottom - top) / factor).ceil() as usize;
        let mut pixels = vec![0; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let offset = (y * width + x) * 4;
                pixels[offset..offset + 4].copy_from_slice(&[
                    (color >> 16) as u8,
                    (color >> 8) as u8,
                    color as u8,
                    255,
                ]);
                let sx = f64::from(left) + (x as f64 + 0.5) * factor;
                let sy = f64::from(top) + (y as f64 + 0.5) * factor;
                let Some((rect, Some(image))) = monitors.iter().find(|(r, _)| {
                    sx >= f64::from(r.left)
                        && sx < f64::from(r.right)
                        && sy >= f64::from(r.top)
                        && sy < f64::from(r.bottom)
                }) else {
                    continue;
                };
                let (l, t, mw, mh) = if position == DWPOS_SPAN {
                    (left, top, right - left, bottom - top)
                } else {
                    (
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                    )
                };
                let (iw, ih) = image.original;
                let (u, v) = if position == DWPOS_STRETCH {
                    (
                        (sx - f64::from(l)) / f64::from(mw),
                        (sy - f64::from(t)) / f64::from(mh),
                    )
                } else if position == DWPOS_TILE {
                    (
                        (sx - f64::from(l)).rem_euclid(iw) / iw,
                        (sy - f64::from(t)).rem_euclid(ih) / ih,
                    )
                } else {
                    let scale = if position == DWPOS_CENTER {
                        1.0
                    } else if position == DWPOS_FIT {
                        (f64::from(mw) / iw).min(f64::from(mh) / ih)
                    } else {
                        (f64::from(mw) / iw).max(f64::from(mh) / ih)
                    };
                    (
                        (sx - f64::from(l) + (iw * scale - f64::from(mw)) / 2.0) / (iw * scale),
                        (sy - f64::from(t) + (ih * scale - f64::from(mh)) / 2.0) / (ih * scale),
                    )
                };
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                let sample = ((v * image.height as f64) as usize * image.width
                    + (u * image.width as f64) as usize)
                    * 4;
                pixels[offset..offset + 3].copy_from_slice(&image.pixels[sample..sample + 3]);
            }
        }
        blur(&mut pixels, width, height, 10);
        Ok((
            TextureHeader {
                version: VERSION,
                width: width as u32,
                height: height as u32,
                bounds: Area {
                    left: left - origin.x,
                    top: top - origin.y,
                    right: right - origin.x,
                    bottom: bottom - origin.y,
                },
            },
            pixels,
        ))
    }
}

fn blur(pixels: &mut [u8], width: usize, height: usize, radius: usize) {
    let mut temp = vec![0; pixels.len()];
    for vertical in [false, true] {
        let (lines, len) = if vertical {
            (width, height)
        } else {
            (height, width)
        };
        let mut prefix = vec![[0_u32; 3]; len + 1];
        for line in 0..lines {
            prefix[0] = [0; 3];
            for i in 0..len {
                let p = if vertical {
                    (i * width + line) * 4
                } else {
                    (line * width + i) * 4
                };
                for c in 0..3 {
                    prefix[i + 1][c] = prefix[i][c] + u32::from(pixels[p + c]);
                }
            }
            for i in 0..len {
                let p = if vertical {
                    (i * width + line) * 4
                } else {
                    (line * width + i) * 4
                };
                let a = i.saturating_sub(radius);
                let b = (i + radius + 1).min(len);
                for c in 0..3 {
                    temp[p + c] = ((prefix[b][c] - prefix[a][c]) / (b - a) as u32) as u8;
                }
                temp[p + 3] = 255;
            }
        }
        pixels.copy_from_slice(&temp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "read-only live wallpaper decoding check"]
    fn captures_live_wallpaper_without_touching_desktop() {
        let _apartment=desktop_shell::ShellApartment::initialize_sta().unwrap();
        let started=std::time::Instant::now();
        let (header,pixels)=capture(windows_sys::Win32::Foundation::POINT::default()).unwrap();
        assert_eq!(header.byte_count(),Some(pixels.len()));
        assert!(pixels.as_chunks::<4>().0.iter().all(|p|p[3]==255));
        println!("Wallpaper material: {}x{}, {:?}",header.width,header.height,started.elapsed());
    }
    #[test]
    fn blur_preserves_uniform_color_and_spreads_detail() {
        let mut flat = [12, 34, 56, 255].repeat(25);
        let expected = flat.clone();
        blur(&mut flat, 5, 5, 2);
        assert_eq!(flat, expected);
        let mut impulse = vec![0; 5 * 5 * 4];
        impulse[(2 * 5 + 2) * 4] = 255;
        blur(&mut impulse, 5, 5, 1);
        assert!(impulse[(2 * 5 + 1) * 4] > 0);
        assert!(impulse[(2 * 5 + 2) * 4] < 255);
        assert_eq!(impulse[0], 0);
    }
}
