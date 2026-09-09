//! Native desktop layout access. Explorer retains all rendering and input ownership.
use desktop_core::RectDip;
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, IServiceProvider};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    CSIDL_DESKTOP, FWF_AUTOARRANGE, IFolderView2, IShellBrowser, IShellWindows,
    SID_STopLevelBrowser, SVGIO_ALLVIEW, SVSI_POSITIONITEM, SWC_DESKTOP, SWFO_NEEDDISPATCH,
    ShellWindows,
};
use windows::core::Interface;

use super::Pidl;

/// Read-only capture of Explorer's visible items and icon metrics. Never changes folder flags.
#[derive(Clone, Debug)]
pub struct NativeDesktopSnapshot {
    pub icon_size: i32,
    pub spacing: (i32, i32),
    pub dpi: u32,
    pub items: Vec<(super::DesktopShellItem, i32, i32)>,
    /// Original view indices, aligned with items even if an individual Shell item was skipped.
    pub view_indices: Vec<i32>,
}

/// Captures the actual desktop view, including its visibility preferences and display names.
/// # Errors
/// Returns an error when Explorer's desktop COM view cannot be reached.
pub fn native_desktop_snapshot() -> Result<NativeDesktopSnapshot, String> {
    unsafe {
        let capture = || -> windows::core::Result<NativeDesktopSnapshot> {
            let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
            let mut desktop_hwnd = 0;
            let dispatch = shell.FindWindowSW(
                &VARIANT::from(CSIDL_DESKTOP.cast_signed()),
                &VARIANT::default(),
                SWC_DESKTOP,
                &raw mut desktop_hwnd,
                SWFO_NEEDDISPATCH,
            )?;
            let provider: IServiceProvider = dispatch.cast()?;
            let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
            let view = browser.QueryActiveShellView()?;
            let folder: IFolderView2 = view.cast()?;
            let parent: windows::Win32::UI::Shell::IShellFolder = folder.GetFolder()?;
            let hwnd = view.GetWindow()?;
            let mut origin = windows_sys::Win32::Foundation::POINT::default();
            windows_sys::Win32::Graphics::Gdi::ClientToScreen(hwnd.0, &raw mut origin);
            let mut icon_size = 48;
            let mut mode = windows::Win32::UI::Shell::FOLDERVIEWMODE::default();
            folder.GetViewModeAndIconSize(&raw mut mode, &raw mut icon_size)?;
            let mut spacing = POINT::default();
            folder.GetSpacing(&raw mut spacing)?;
            let mut items = Vec::new();
            let mut view_indices = Vec::new();
            for index in 0..folder.ItemCount(SVGIO_ALLVIEW)? {
                let pidl = Pidl(folder.Item(index)?);
                let position = folder.GetItemPosition(pidl.0)?;
                let item: windows::Win32::UI::Shell::IShellItem =
                    windows::Win32::UI::Shell::SHCreateItemWithParent(None, &parent, pidl.0)?;
                if let Ok(mut entry) = super::desktop_shell_item(&item, false) {
                    // Known-folder desktop objects can expose a filesystem path but have a
                    // different icon and verbs from the underlying directory (e.g. User Files).
                    if let Ok(parsing_name) = super::shell_item_name(
                        &item,
                        windows::Win32::UI::Shell::SIGDN_DESKTOPABSOLUTEPARSING,
                    ) && parsing_name.starts_with("::{")
                    {
                        entry.identity = desktop_core::ShellIdentity::Namespace { parsing_name };
                    }
                    items.push((entry, position.x + origin.x, position.y + origin.y));
                    view_indices.push(index);
                }
            }
            Ok(NativeDesktopSnapshot {
                icon_size,
                spacing: (spacing.x, spacing.y),
                dpi: windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd.0).max(96),
                items,
                view_indices,
            })
        };
        capture().map_err(|error| format!("读取原生桌面布局失败：{error}"))
    }
}

/// Moves native desktop items whose origins are inside a screen-pixel rectangle.
/// Coordinates use pixels, matching the current window host's geometry convention.
/// Only an explicit group move calls this; startup never writes Explorer's layout or settings.
///
/// # Errors
/// Returns an error if Explorer is unavailable, auto-arrange prevents positioning, or Shell
/// positioning fails. Auto-arrange and snap-to-grid preferences are never changed.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn move_native_desktop_items(bounds: RectDip, dx: i32, dy: i32) -> Result<(), String> {
    if dx == 0 && dy == 0 {
        return Ok(());
    }
    unsafe { move_items(bounds, dx, dy) }.map_err(|error| format!("无法移动原生桌面图标：{error}"))
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
unsafe fn move_items(bounds: RectDip, dx: i32, dy: i32) -> windows::core::Result<()> {
    unsafe {
        let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        let mut desktop_hwnd = 0;
        let dispatch = shell.FindWindowSW(
            &VARIANT::from(CSIDL_DESKTOP.cast_signed()),
            &VARIANT::default(),
            SWC_DESKTOP,
            &raw mut desktop_hwnd,
            SWFO_NEEDDISPATCH,
        )?;
        let provider: IServiceProvider = dispatch.cast()?;
        let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
        let view = browser.QueryActiveShellView()?;
        let folder: IFolderView2 = view.cast()?;
        let hwnd = view.GetWindow()?;
        let mut origin = windows_sys::Win32::Foundation::POINT::default();
        if windows_sys::Win32::Graphics::Gdi::ClientToScreen(hwnd.0, &raw mut origin) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let mut items = Vec::new();
        let mut positions = Vec::new();
        for index in 0..folder.ItemCount(SVGIO_ALLVIEW)? {
            let pidl = Pidl(folder.Item(index)?);
            let position = folder.GetItemPosition(pidl.0)?;
            let x = (position.x + origin.x) as f32;
            let y = (position.y + origin.y) as f32;
            if x >= bounds.x
                && x < bounds.x + bounds.width
                && y >= bounds.y
                && y < bounds.y + bounds.height
            {
                items.push(pidl);
                positions.push(POINT {
                    x: position.x + dx,
                    y: position.y + dy,
                });
            }
        }
        if items.is_empty() {
            return Ok(());
        }
        if folder.GetCurrentFolderFlags()? & FWF_AUTOARRANGE.0.cast_unsigned() != 0 {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x8000_4005_u32.cast_signed()),
                "桌面已启用自动排列。可先在分组菜单取消“移动框内图标”，或在桌面菜单关闭自动排列。",
            ));
        }
        let pointers: Vec<_> = items.iter().map(|pidl| pidl.0.cast_const()).collect();
        folder.SelectAndPositionItems(
            u32::try_from(items.len()).unwrap_or(u32::MAX),
            pointers.as_ptr(),
            Some(positions.as_ptr()),
            SVSI_POSITIONITEM.0.cast_unsigned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_group_reads_live_explorer_without_moving_icons_or_hiding_it() {
        let _apartment = crate::ShellApartment::initialize_sta().unwrap();
        let hidden = crate::desktop_icons_hidden();
        // An empty rectangle can never contain an item. This exercises COM discovery and
        // coordinate reads against real Explorer without writing any positions or preferences.
        move_native_desktop_items(
            RectDip {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            1,
            1,
        )
        .unwrap();
        assert_eq!(crate::desktop_icons_hidden(), hidden);
    }
}
