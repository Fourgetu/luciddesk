//! The executable, windows and notification area share one embedded icon.
use std::{cell::RefCell, collections::BTreeMap, ptr::null};
use windows_sys::Win32::{
    Foundation::HWND,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
        WindowsAndMessaging::*,
    },
};

const APP_ICON: u16 = 1;

pub(crate) struct Icon(pub HICON);

impl Drop for Icon {
    fn drop(&mut self) {
        unsafe { DestroyIcon(self.0) };
    }
}

pub(crate) fn load(width: i32, height: i32) -> Result<Icon, String> {
    let icon = unsafe {
        LoadImageW(
            GetModuleHandleW(null()),
            usize::from(APP_ICON) as _,
            IMAGE_ICON,
            width,
            height,
            0,
        )
    };
    if icon.is_null() {
        Err(format!(
            "无法加载应用图标：{}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(Icon(icon))
    }
}

thread_local! {
    // Window icon handles must outlive the windows. The UI thread owns both;
    // keep one owned handle per size until its message loop has shut down.
    static WINDOW_ICONS: RefCell<BTreeMap<(i32, i32), Icon>> = const { RefCell::new(BTreeMap::new()) };
}

pub(crate) fn apply(hwnd: HWND) -> Result<(), String> {
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    WINDOW_ICONS.with(|cache| {
        let mut cache = cache.borrow_mut();
        for (kind, cx, cy) in [
            (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
            (ICON_BIG, SM_CXICON, SM_CYICON),
        ] {
            let size = unsafe {
                (
                    GetSystemMetricsForDpi(cx, dpi),
                    GetSystemMetricsForDpi(cy, dpi),
                )
            };
            if let std::collections::btree_map::Entry::Vacant(entry) = cache.entry(size) {
                entry.insert(load(size.0, size.1)?);
            }
            unsafe { SendMessageW(hwnd, WM_SETICON, kind as usize, cache[&size].0 as isize) };
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW};

    #[test]
    fn window_icons_are_attached_and_reused() {
        let window = windows_window::Window::new("LucidPane icon test")
            .create()
            .unwrap();
        let hwnd = window.hwnd().cast();
        apply(hwnd).unwrap();
        let small = unsafe { SendMessageW(hwnd, WM_GETICON, ICON_SMALL as usize, 0) };
        let large = unsafe { SendMessageW(hwnd, WM_GETICON, ICON_BIG as usize, 0) };
        assert_ne!(small, 0);
        assert_ne!(large, 0);
        assert_ne!(small, large);
        apply(hwnd).unwrap();
        assert_eq!(small, unsafe {
            SendMessageW(hwnd, WM_GETICON, ICON_SMALL as usize, 0)
        });
        assert_eq!(large, unsafe {
            SendMessageW(hwnd, WM_GETICON, ICON_BIG as usize, 0)
        });
    }

    #[test]
    fn embedded_icon_loads_at_shell_sizes() {
        for size in [16, 20, 24, 32, 40, 48, 64, 96, 128, 256] {
            let icon = load(size, size).unwrap();
            let mut info = ICONINFO::default();
            assert_ne!(unsafe { GetIconInfo(icon.0, &raw mut info) }, 0);
            let mut bitmap = BITMAP::default();
            let result = unsafe {
                GetObjectW(
                    info.hbmColor,
                    size_of::<BITMAP>() as i32,
                    (&raw mut bitmap).cast(),
                )
            };
            unsafe {
                DeleteObject(info.hbmColor);
                DeleteObject(info.hbmMask);
            }
            assert_ne!(result, 0);
            assert_eq!((bitmap.bmWidth, bitmap.bmHeight), (size, size));
        }
    }
}
