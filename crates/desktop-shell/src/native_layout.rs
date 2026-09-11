//! Native desktop layout access. Explorer retains all rendering and input ownership.
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, IServiceProvider};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    CSIDL_DESKTOP, IFolderView2, IShellBrowser, IShellWindows, SID_STopLevelBrowser, SVGIO_ALLVIEW,
    SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
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
    capture_desktop_snapshot(false)
}

/// Read-only inventory for a worker STA. Yields between batches so Explorer can
/// service mouse input while the controller performs its periodic audit.
/// # Errors
/// Returns an error when Explorer's desktop COM view cannot be reached.
pub fn native_desktop_snapshot_background() -> Result<NativeDesktopSnapshot, String> {
    capture_desktop_snapshot(true)
}

fn capture_desktop_snapshot(paced: bool) -> Result<NativeDesktopSnapshot, String> {
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
                if paced && index > 0 && index % 8 == 0 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                let pidl = Pidl(folder.Item(index)?);
                let position = folder.GetItemPosition(pidl.0)?;
                let item: windows::Win32::UI::Shell::IShellItem =
                    windows::Win32::UI::Shell::SHCreateItemWithParent(None, &parent, pidl.0)?;
                if let Ok(mut entry) = super::desktop_shell_item(&item) {
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
        let snapshot = capture().map_err(|error| format!("读取原生桌面布局失败：{error}"))?;
        super::native_menu::update_hints(snapshot.items.iter().zip(&snapshot.view_indices).map(
            |((item, _, _), &index)| {
                let name = match &item.identity {
                    desktop_core::ShellIdentity::FileSystem { path, .. } => {
                        path.to_string_lossy().into_owned()
                    }
                    desktop_core::ShellIdentity::Namespace { parsing_name } => parsing_name.clone(),
                };
                (name, index)
            },
        ));
        Ok(snapshot)
    }
}
