//! Native desktop layout access. Explorer retains all rendering and input ownership.
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::UI::Shell::{IFolderView2, IShellWindows, SVGIO_ALLVIEW, ShellWindows};
use windows::core::Interface;

use crate::namespace::Pidl;

/// Explorer-owned item IDs and view metrics, without opening desktop files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeDesktopRevision {
    icon_size: i32,
    spacing: (i32, i32),
    dpi: u32,
    item_ids: Vec<Vec<u8>>,
}

/// Read-only capture of Explorer's visible items and icon metrics. Never changes folder flags.
#[derive(Clone, Debug)]
pub struct NativeDesktopSnapshot {
    pub revision: NativeDesktopRevision,
    pub icon_size: i32,
    pub spacing: (i32, i32),
    pub dpi: u32,
    pub items: Vec<(super::DesktopShellItem, i32, i32)>,
    /// Original view indices, aligned with items even if an individual Shell item was skipped.
    pub view_indices: Vec<i32>,
}

impl NativeDesktopSnapshot {
    /// Reject partial metadata captures before reconciling persisted membership.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.items.len() == self.revision.item_ids.len()
            && self.items.len() == self.view_indices.len()
    }
}

/// Captures the actual desktop view, including its visibility preferences and display names.
/// # Errors
/// Returns an error when Explorer's desktop COM view cannot be reached.
pub fn native_desktop_snapshot() -> Result<NativeDesktopSnapshot, String> {
    capture_desktop_snapshot()
}

/// Read only Explorer's in-memory item IDs on a worker STA. Metadata is resolved
/// by the full snapshot only after a revision change has been detected.
#[derive(Default)]
pub struct NativeDesktopReader {
    shell: Option<IShellWindows>,
}

impl NativeDesktopReader {
    /// Must be used and dropped within the caller's initialized COM apartment.
    /// Failed queries discard the cached connection so the next check reconnects.
    ///
    /// # Errors
    /// Returns an error when Explorer's desktop COM view cannot be reached.
    pub fn revision(&mut self) -> Result<NativeDesktopRevision, String> {
        let result = self.capture_revision();
        if result.is_err() {
            self.shell = None;
        }
        result.map_err(|error| format!("读取桌面项目标识失败：{error}"))
    }

    fn capture_revision(&mut self) -> windows::core::Result<NativeDesktopRevision> {
        if self.shell.is_none() {
            self.shell = Some(unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)? });
        }
        capture_revision(self.shell.as_ref().unwrap())
    }
}

fn capture_revision(shell: &IShellWindows) -> windows::core::Result<NativeDesktopRevision> {
    let (folder, hwnd) = desktop_folder(shell)?;
    let mut revision = revision_header(&folder, hwnd)?;
    for index in 0..unsafe { folder.ItemCount(SVGIO_ALLVIEW)? } {
        if index > 0 && index % 8 == 0 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let pidl = Pidl::new(unsafe { folder.Item(index)? });
        revision.item_ids.push(item_id(&pidl));
    }
    Ok(revision)
}

fn revision_header(
    folder: &IFolderView2,
    hwnd: windows::Win32::Foundation::HWND,
) -> windows::core::Result<NativeDesktopRevision> {
    unsafe {
        let mut icon_size = 48;
        let mut mode = windows::Win32::UI::Shell::FOLDERVIEWMODE::default();
        folder.GetViewModeAndIconSize(&raw mut mode, &raw mut icon_size)?;
        let mut spacing = POINT::default();
        folder.GetSpacing(&raw mut spacing)?;
        Ok(NativeDesktopRevision {
            icon_size,
            spacing: (spacing.x, spacing.y),
            dpi: windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd.0).max(96),
            item_ids: Vec::new(),
        })
    }
}

fn item_id(pidl: &Pidl) -> Vec<u8> {
    if pidl.as_ptr().is_null() {
        return Vec::new();
    }
    unsafe {
        let size = windows::Win32::UI::Shell::ILGetSize(Some(pidl.as_ptr())) as usize;
        std::slice::from_raw_parts(pidl.as_ptr().cast::<u8>(), size).to_vec()
    }
}

fn desktop_folder(
    shell: &IShellWindows,
) -> windows::core::Result<(IFolderView2, windows::Win32::Foundation::HWND)> {
    unsafe {
        let view = crate::desktop::shell_view(shell)?;
        let folder: IFolderView2 = view.cast()?;
        let hwnd = view.GetWindow()?;
        Ok((folder, hwnd))
    }
}

fn capture_desktop_snapshot() -> Result<NativeDesktopSnapshot, String> {
    unsafe {
        let capture = || -> windows::core::Result<NativeDesktopSnapshot> {
            let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
            let (folder, hwnd) = desktop_folder(&shell)?;
            let parent: windows::Win32::UI::Shell::IShellFolder = folder.GetFolder()?;
            let mut origin = windows_sys::Win32::Foundation::POINT::default();
            windows_sys::Win32::Graphics::Gdi::ClientToScreen(hwnd.0, &raw mut origin);
            let mut revision = revision_header(&folder, hwnd)?;
            let mut items = Vec::new();
            let mut view_indices = Vec::new();
            for index in 0..folder.ItemCount(SVGIO_ALLVIEW)? {
                let pidl = Pidl::new(folder.Item(index)?);
                revision.item_ids.push(item_id(&pidl));
                let position = folder.GetItemPosition(pidl.as_ptr())?;
                let item: windows::Win32::UI::Shell::IShellItem =
                    windows::Win32::UI::Shell::SHCreateItemWithParent(
                        None,
                        &parent,
                        pidl.as_ptr(),
                    )?;
                if let Ok(mut entry) = crate::namespace::desktop_shell_item(&item) {
                    // Known-folder desktop objects can expose a filesystem path but have a
                    // different icon and verbs from the underlying directory (e.g. User Files).
                    if let Ok(parsing_name) = crate::namespace::shell_item_name(
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
                icon_size: revision.icon_size,
                spacing: revision.spacing,
                dpi: revision.dpi,
                revision,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Reads live Explorer; run in an interactive session with the desktop unchanged"]
    fn background_revision_matches_full_snapshot() {
        let _sta = crate::ShellApartment::initialize_sta().unwrap();
        let snapshot = native_desktop_snapshot().unwrap();
        let revision = std::thread::spawn(|| {
            let _sta = crate::ShellApartment::initialize_sta().unwrap();
            let mut reader = NativeDesktopReader::default();
            let first = reader.revision().unwrap();
            assert_eq!(reader.revision().unwrap(), first);
            // Simulate invalidating a stale connection without restarting Explorer.
            reader.shell = None;
            assert_eq!(reader.revision().unwrap(), first);
            first
        })
        .join()
        .unwrap();
        assert_eq!(revision, snapshot.revision);
    }
}
