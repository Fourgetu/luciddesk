use desktop_core::ShellIdentity;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ, HPALETTE};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, IWICImagingFactory, WICBitmapUsePremultipliedAlpha,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::{
    IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK,
};
use windows::core::{Interface, PCWSTR};

#[derive(Clone, Debug)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

pub fn load(identity: &ShellIdentity, size: i32) -> windows::core::Result<Pixels> {
    let name = match identity {
        ShellIdentity::FileSystem { path, .. } => path.to_string_lossy().into_owned(),
        ShellIdentity::Namespace { parsing_name } => parsing_name.clone(),
    };
    let name: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None)?;
        if item
            .GetAttributes(windows::Win32::System::SystemServices::SFGAO_LINK)
            .is_ok_and(|a| a.0 != 0)
            && let Ok(pixels) = link_icon(&item, size)
        {
            return Ok(pixels);
        }
        let factory: IShellItemImageFactory = item.cast()?;
        let bitmap = factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_BIGGERSIZEOK)?;
        let result = (|| {
            let imaging: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let image = imaging.CreateBitmapFromHBITMAP(
                bitmap,
                HPALETTE::default(),
                WICBitmapUsePremultipliedAlpha,
            )?;
            let (mut width, mut height) = (0, 0);
            image.GetSize(&raw mut width, &raw mut height)?;
            let mut data = vec![0; (width * height * 4) as usize];
            image.CopyPixels(std::ptr::null(), width * 4, &mut data)?;
            Ok(Pixels {
                width,
                height,
                data,
            })
        })();
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        result
    }
}

// Request the shortcut's own base icon, without an overlay. This preserves custom
// shortcut icons while hiding the arrow in LucidPane only.
#[allow(clippy::wildcard_imports)]
fn link_icon(item: &IShellItem, size: i32) -> windows::core::Result<Pixels> {
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::UI::{Controls::IImageList, Shell::*, WindowsAndMessaging::DestroyIcon};
    unsafe {
        let pidl = SHGetIDListFromObject(item)?;
        let mut info = SHFILEINFOW::default();
        let result = SHGetFileInfoW(
            PCWSTR(pidl.cast()),
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&raw mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_PIDL | SHGFI_SYSICONINDEX,
        );
        windows::Win32::System::Com::CoTaskMemFree(Some(pidl.cast()));
        if !info.hIcon.is_invalid() {
            let _ = DestroyIcon(info.hIcon);
        }
        if result == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let mut lists = Vec::new();
        for kind in [SHIL_SMALL, SHIL_LARGE, SHIL_EXTRALARGE, SHIL_JUMBO] {
            if let Ok(list) = SHGetImageList::<IImageList>(kind.cast_signed()) {
                let (mut w, mut h) = (0, 0);
                if list.GetIconSize(&raw mut w, &raw mut h).is_ok() {
                    lists.push((w, list));
                }
            }
        }
        lists.sort_by_key(|(w, _)| {
            if *w >= size {
                (*w - size, 0)
            } else {
                (size - *w, 1)
            }
        });
        let (_, list) = lists
            .first()
            .ok_or_else(windows::core::Error::from_thread)?;
        let icon = list.GetIcon(info.iIcon, 0)?;
        let pixels = (|| {
            let imaging: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let bitmap = imaging.CreateBitmapFromHICON(icon)?;
            let converter = imaging.CreateFormatConverter()?;
            converter.Initialize(
                &bitmap,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )?;
            let (mut width, mut height) = (0, 0);
            converter.GetSize(&raw mut width, &raw mut height)?;
            let mut data = vec![0; (width * height * 4) as usize];
            converter.CopyPixels(std::ptr::null(), width * 4, &mut data)?;
            Ok(Pixels {
                width,
                height,
                data,
            })
        })();
        let _ = DestroyIcon(icon);
        pixels
    }
}

pub fn font() -> (String, f32) {
    use windows_sys::Win32::Graphics::Gdi::LOGFONTW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETICONTITLELOGFONT, SystemParametersInfoW,
    };
    let mut font = LOGFONTW::default();
    if unsafe {
        SystemParametersInfoW(
            SPI_GETICONTITLELOGFONT,
            size_of::<LOGFONTW>() as u32,
            (&raw mut font).cast(),
            0,
        )
    } != 0
    {
        let end = font
            .lfFaceName
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(font.lfFaceName.len());
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForSystem() }.max(96);
        #[allow(clippy::cast_precision_loss)]
        let size = (font.lfHeight.unsigned_abs() as f32 * 96.0 / dpi as f32).max(11.0);
        (String::from_utf16_lossy(&font.lfFaceName[..end]), size)
    } else {
        ("Segoe UI".into(), 12.0)
    }
}
