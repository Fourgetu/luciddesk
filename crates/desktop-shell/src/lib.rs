pub use desktop_core::ShellIdentity;
mod file_command;
mod native_layout;
mod native_menu;
mod rename;
pub use file_command::{FileCommand, invoke_file_command, invoke_file_commands};
pub use native_layout::{
    NativeDesktopReader, NativeDesktopRevision, NativeDesktopSnapshot, native_desktop_snapshot,
};
pub use native_menu::peek_desktop_item;
pub use native_menu::{MenuInvocation, show_desktop_item_menu, show_desktop_items_menu};
pub use rename::rename_shell_identity;
use std::ffi::{OsStr, c_void};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::SystemTime;
use windows::Win32::Foundation::HWND as WindowsHwnd;
use windows::Win32::System::Com::CoTaskMemFree as CoTaskMemFreeCom;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::System::SystemServices::{
    SFGAO_CANDELETE, SFGAO_CANRENAME, SFGAO_FILESYSTEM, SFGAO_FOLDER, SFGAO_HIDDEN, SFGAO_LINK,
};
use windows::Win32::UI::Shell::{
    IShellItem, SHCONTF_FOLDERS, SHCONTF_NONFOLDERS, SHCreateItemWithParent, SHGetDesktopFolder,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH, SIGDN_NORMALDISPLAY,
};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FileIdInfo, GetFileInformationByHandleEx,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::Common::ITEMIDLIST;
use windows_sys::Win32::UI::Shell::{
    CSIDL_DESKTOP, FOLDERID_LocalAppData, SHCNE_ALLEVENTS, SHCNRF_InterruptLevel,
    SHCNRF_ShellLevel, SHChangeNotifyDeregister, SHChangeNotifyEntry, SHChangeNotifyRegister,
    SHELLSTATEA, SHGetKnownFolderPath, SHGetSetSettings, SHGetSpecialFolderLocation, SSF_HIDEICONS,
    ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, GetShellWindow, IsWindowVisible, SW_SHOWNORMAL,
};

/// Initializes COM as an OLE-capable STA for Shell UI, drag/drop, and file operations.
pub struct ShellApartment;

impl ShellApartment {
    /// Initializes OLE on the calling UI thread.
    ///
    /// # Errors
    ///
    /// Returns a COM error when the thread has already been initialized with an incompatible model.
    pub fn initialize_sta() -> Result<Self, ShellError> {
        unsafe { OleInitialize(None) }?;
        Ok(Self)
    }
}

impl Drop for ShellApartment {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ShellAttributes {
    pub folder: bool,
    pub file_system: bool,
    pub link: bool,
    pub hidden: bool,
    pub can_rename: bool,
    pub can_delete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopShellItem {
    pub identity: ShellIdentity,
    pub display_name: String,
    pub attributes: ShellAttributes,
    pub modified: Option<SystemTime>,
}

/// Resolves LocalAppData without assuming its physical location.
///
/// # Errors
///
/// Returns an error when Windows cannot resolve the requested folder.
pub fn local_app_data_path() -> Result<PathBuf, ShellError> {
    let mut raw_path = ptr::null_mut();
    let result = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_LocalAppData,
            0,
            ptr::null_mut(),
            &raw mut raw_path,
        )
    };
    if result < 0 {
        return Err(ShellError::Windows(result));
    }

    let path = unsafe {
        let mut length = 0;
        while *raw_path.add(length) != 0 {
            length += 1;
        }
        let value = std::ffi::OsString::from_wide(std::slice::from_raw_parts(raw_path, length));
        CoTaskMemFree(raw_path.cast::<c_void>());
        PathBuf::from(value)
    };
    Ok(path)
}

/// Enumerates the Windows Desktop Shell Namespace, including virtual Shell items.
///
/// Unlike reading the user and public desktop directories separately, this follows the
/// Shell's unified Desktop Folder and can return items such as Recycle Bin or This PC.
/// The calling thread must own a [`ShellApartment`].
///
/// # Errors
///
/// Returns a COM or Shell error when the Desktop Folder cannot be enumerated.
pub fn enumerate_desktop_namespace(owner: isize) -> Result<Vec<DesktopShellItem>, ShellError> {
    let desktop = unsafe { SHGetDesktopFolder() }?;
    let mut enumerator = None;
    let flags = u32::try_from(SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0).unwrap_or_default();
    let result = unsafe {
        desktop.EnumObjects(
            WindowsHwnd(owner as *mut c_void),
            flags,
            &raw mut enumerator,
        )
    };
    result.ok()?;
    let Some(enumerator) = enumerator else {
        return Ok(Vec::new());
    };

    let mut items = Vec::new();
    loop {
        let mut child = [ptr::null_mut()];
        let mut fetched = 0_u32;
        let result = unsafe { enumerator.Next(&mut child, Some(&raw mut fetched)) };
        if fetched == 0 {
            if result.is_err() {
                return Err(windows::core::Error::from(result).into());
            }
            break;
        }
        let child = Pidl::new(child[0]);
        let Ok(shell_item): Result<IShellItem, _> =
            (unsafe { SHCreateItemWithParent(None, &desktop, child.as_ptr()) })
        else {
            continue;
        };
        if let Ok(item) = desktop_shell_item(&shell_item) {
            items.push(item);
        }
    }

    items.sort_by(|left, right| {
        right
            .attributes
            .folder
            .cmp(&left.attributes.folder)
            .then_with(|| {
                left.display_name
                    .to_lowercase()
                    .cmp(&right.display_name.to_lowercase())
            })
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    Ok(items)
}

fn desktop_shell_item(item: &IShellItem) -> Result<DesktopShellItem, ShellError> {
    let display_name = shell_item_name(item, SIGDN_NORMALDISPLAY)?;
    let identity = shell_item_name(item, SIGDN_FILESYSPATH)
        .ok()
        .filter(|value| !value.is_empty())
        .map_or_else(
            || {
                shell_item_name(item, SIGDN_DESKTOPABSOLUTEPARSING)
                    .map(|parsing_name| ShellIdentity::Namespace { parsing_name })
            },
            |path| {
                let path = PathBuf::from(path);
                let (volume_id, file_id) = stable_file_identity(&path)
                    .map_or((None, None), |(volume_id, file_id)| {
                        (Some(volume_id), Some(file_id))
                    });
                Ok(ShellIdentity::FileSystem {
                    path,
                    volume_id,
                    file_id,
                })
            },
        )?;
    let attribute_mask = SFGAO_FOLDER
        | SFGAO_FILESYSTEM
        | SFGAO_LINK
        | SFGAO_HIDDEN
        | SFGAO_CANRENAME
        | SFGAO_CANDELETE;
    let raw_attributes =
        unsafe { item.GetAttributes(attribute_mask) }.map_or(0, |attributes| attributes.0);
    let metadata = identity
        .file_system_path()
        .and_then(|path| fs::metadata(path).ok());
    let path_is_link = identity.file_system_path().is_some_and(|path| {
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
    });
    let attributes = ShellAttributes {
        folder: raw_attributes & SFGAO_FOLDER.0 != 0
            || metadata.as_ref().is_some_and(fs::Metadata::is_dir),
        file_system: raw_attributes & SFGAO_FILESYSTEM.0 != 0
            || identity.file_system_path().is_some(),
        link: raw_attributes & SFGAO_LINK.0 != 0 || path_is_link,
        hidden: raw_attributes & SFGAO_HIDDEN.0 != 0,
        can_rename: raw_attributes & SFGAO_CANRENAME.0 != 0,
        can_delete: raw_attributes & SFGAO_CANDELETE.0 != 0,
    };
    let modified = metadata.and_then(|metadata| metadata.modified().ok());
    Ok(DesktopShellItem {
        identity,
        display_name,
        attributes,
        modified,
    })
}

fn stable_file_identity(path: &Path) -> Option<(u64, u128)> {
    let file = OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .ok()?;
    let mut info = FILE_ID_INFO::default();
    let succeeded = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&raw mut info).cast(),
            u32::try_from(size_of::<FILE_ID_INFO>()).ok()?,
        )
    };
    (succeeded != 0).then(|| {
        (
            info.VolumeSerialNumber,
            u128::from_le_bytes(info.FileId.Identifier),
        )
    })
}

fn shell_item_name(
    item: &IShellItem,
    kind: windows::Win32::UI::Shell::SIGDN,
) -> Result<String, ShellError> {
    let raw = unsafe { item.GetDisplayName(kind) }?;
    let value = unsafe { raw.to_string() };
    unsafe {
        CoTaskMemFreeCom(Some(raw.0.cast()));
    }
    value.map_err(Into::into)
}

struct Pidl(*mut windows::Win32::UI::Shell::Common::ITEMIDLIST);

impl Pidl {
    fn new(value: *mut windows::Win32::UI::Shell::Common::ITEMIDLIST) -> Self {
        Self(value)
    }

    const fn as_ptr(&self) -> *const windows::Win32::UI::Shell::Common::ITEMIDLIST {
        self.0
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe {
            CoTaskMemFreeCom(Some(self.0.cast()));
        }
    }
}

const HIDE_DESKTOP_ICONS_BIT: i32 = 1 << 12;

const SHELL_DLL_DEF_VIEW: &[u16] = &[
    b'S' as u16,
    b'H' as u16,
    b'E' as u16,
    b'L' as u16,
    b'L' as u16,
    b'D' as u16,
    b'L' as u16,
    b'L' as u16,
    b'_' as u16,
    b'D' as u16,
    b'e' as u16,
    b'f' as u16,
    b'V' as u16,
    b'i' as u16,
    b'e' as u16,
    b'w' as u16,
    0,
];
const SYS_LIST_VIEW: &[u16] = &[
    b'S' as u16,
    b'y' as u16,
    b's' as u16,
    b'L' as u16,
    b'i' as u16,
    b's' as u16,
    b't' as u16,
    b'V' as u16,
    b'i' as u16,
    b'e' as u16,
    b'w' as u16,
    b'3' as u16,
    b'2' as u16,
    0,
];
const FOLDER_VIEW: &[u16] = &[
    b'F' as u16,
    b'o' as u16,
    b'l' as u16,
    b'd' as u16,
    b'e' as u16,
    b'r' as u16,
    b'V' as u16,
    b'i' as u16,
    b'e' as u16,
    b'w' as u16,
    0,
];

/// Returns whether Explorer's native desktop icons are currently hidden.
#[must_use]
pub fn desktop_icons_hidden() -> bool {
    let mut state = SHELLSTATEA::default();
    unsafe {
        SHGetSetSettings(&raw mut state, SSF_HIDEICONS, 0);
    }
    let shell_state_hidden = state._bitfield1 & HIDE_DESKTOP_ICONS_BIT != 0;
    let view_hidden = desktop_list_view().is_some_and(|view| unsafe { IsWindowVisible(view) == 0 });
    shell_state_hidden || view_hidden
}

fn desktop_list_view() -> Option<HWND> {
    unsafe extern "system" fn enumerate_window(window: HWND, state: LPARAM) -> i32 {
        let result = unsafe { &mut *(state as *mut HWND) };
        if let Some(view) = unsafe { list_view_under(window) } {
            *result = view;
            0
        } else {
            1
        }
    }

    let shell = unsafe { GetShellWindow() };
    if !shell.is_null()
        && let Some(view) = unsafe { list_view_under(shell) }
    {
        return Some(view);
    }
    let mut result: HWND = ptr::null_mut();
    unsafe {
        EnumWindows(Some(enumerate_window), (&raw mut result) as LPARAM);
    }
    (!result.is_null()).then_some(result)
}

unsafe fn list_view_under(window: HWND) -> Option<HWND> {
    let definition = unsafe {
        FindWindowExW(
            window,
            ptr::null_mut(),
            SHELL_DLL_DEF_VIEW.as_ptr(),
            ptr::null(),
        )
    };
    if definition.is_null() {
        return None;
    }
    let named = unsafe {
        FindWindowExW(
            definition,
            ptr::null_mut(),
            SYS_LIST_VIEW.as_ptr(),
            FOLDER_VIEW.as_ptr(),
        )
    };
    if !named.is_null() {
        return Some(named);
    }
    let unnamed = unsafe {
        FindWindowExW(
            definition,
            ptr::null_mut(),
            SYS_LIST_VIEW.as_ptr(),
            ptr::null(),
        )
    };
    (!unnamed.is_null()).then_some(unnamed)
}

pub struct DesktopChangeSubscription {
    registration: u32,
    desktop_pidl: *mut ITEMIDLIST,
}

impl DesktopChangeSubscription {
    /// Registers a window message for recursive Desktop Shell Namespace changes.
    ///
    /// The receiver only needs the notification as an invalidation signal; item identity is
    /// resolved by a debounced namespace reconciliation.
    ///
    /// # Errors
    ///
    /// Returns an error when the Desktop PIDL or Shell registration cannot be created.
    pub fn register(owner: isize, message: u32) -> Result<Self, ShellError> {
        Self::register_folder(owner, message, CSIDL_DESKTOP.cast_signed())
    }

    /// Subscribe directly to Recycle Bin contents, independently of the desktop view.
    /// # Errors
    /// Returns an error if Shell cannot register the namespace notification.
    pub fn register_recycle_bin(owner: isize, message: u32) -> Result<Self, ShellError> {
        Self::register_folder(owner, message, 10) // CSIDL_BITBUCKET
    }

    fn register_folder(owner: isize, message: u32, folder: i32) -> Result<Self, ShellError> {
        let mut desktop_pidl = ptr::null_mut();
        let result =
            unsafe { SHGetSpecialFolderLocation(owner as HWND, folder, &raw mut desktop_pidl) };
        if result < 0 {
            return Err(ShellError::Windows(result));
        }
        let entry = SHChangeNotifyEntry {
            pidl: desktop_pidl,
            fRecursive: 1,
        };
        let registration = unsafe {
            SHChangeNotifyRegister(
                owner as HWND,
                SHCNRF_ShellLevel | SHCNRF_InterruptLevel,
                SHCNE_ALLEVENTS.cast_signed(),
                message,
                1,
                &raw const entry,
            )
        };
        if registration == 0 {
            unsafe { CoTaskMemFree(desktop_pidl.cast()) };
            return Err(ShellError::ChangeNotification);
        }
        Ok(Self {
            registration,
            desktop_pidl,
        })
    }
}

impl Drop for DesktopChangeSubscription {
    fn drop(&mut self) {
        unsafe {
            SHChangeNotifyDeregister(self.registration);
            CoTaskMemFree(self.desktop_pidl.cast());
        }
    }
}

/// Decode filesystem and virtual desktop items from an OLE drag without moving files.
/// # Errors
/// Rejects non-Shell data or an oversized drag.
pub fn drag_shell_identities(
    data: &windows::Win32::System::Com::IDataObject,
) -> windows::core::Result<Vec<ShellIdentity>> {
    use windows::Win32::UI::Shell::{
        IShellItemArray, SHCreateShellItemArrayFromDataObject, SIGDN_DESKTOPABSOLUTEPARSING,
    };
    unsafe {
        let items: IShellItemArray = SHCreateShellItemArrayFromDataObject(data)?;
        let count = items.GetCount()?;
        if count > 512 {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_INVALIDARG,
            ));
        }
        let mut identities = Vec::new();
        for index in 0..count {
            let shell_item = items.GetItemAt(index)?;
            // DragEnter runs during the cross-process preview handoff. Only
            // resolve names here: opening files for IDs and reading metadata
            // delays that handoff. The target matches these names against its
            // existing inventory and keeps the inventory's stable identities.
            let parsing_name =
                shell_item_name(&shell_item, SIGDN_DESKTOPABSOLUTEPARSING).map_err(|e| {
                    windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
                })?;
            let identity = if parsing_name.starts_with("::{") {
                ShellIdentity::Namespace { parsing_name }
            } else if let Ok(path) = shell_item_name(&shell_item, SIGDN_FILESYSPATH)
                && !path.is_empty()
            {
                ShellIdentity::FileSystem {
                    path: PathBuf::from(path),
                    volume_id: None,
                    file_id: None,
                }
            } else {
                ShellIdentity::Namespace { parsing_name }
            };
            identities.push(identity);
        }
        Ok(identities)
    }
}

/// Opens a filesystem or virtual Shell identity using the current Shell association.
///
/// # Errors
///
/// Returns an error when `ShellExecuteW` rejects the operation.
pub fn open_shell_identity(owner: isize, identity: &ShellIdentity) -> Result<(), ShellError> {
    open_shell_name(owner, identity.activation_name())
}

fn open_shell_name(owner: isize, target: &OsStr) -> Result<(), ShellError> {
    let operation = wide_null(OsStr::new("open"));
    let target = wide_null(target);
    let result = unsafe {
        ShellExecuteW(
            owner as HWND,
            operation.as_ptr(),
            target.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err(ShellError::Execute(result as isize));
    }
    Ok(())
}

fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[derive(Debug)]
pub enum ShellError {
    Io(io::Error),
    Com(windows::core::Error),
    Utf16(std::string::FromUtf16Error),
    Windows(i32),
    Execute(isize),
    ChangeNotification,
}

impl fmt::Display for ShellError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "folder I/O error: {error}"),
            Self::Com(error) => write!(formatter, "Windows Shell COM error: {error}"),
            Self::Utf16(error) => write!(formatter, "invalid UTF-16 from Windows Shell: {error}"),
            Self::Windows(code) => write!(formatter, "Windows Shell error 0x{code:08x}"),
            Self::Execute(code) => write!(formatter, "ShellExecuteW failed with code {code}"),
            Self::ChangeNotification => {
                write!(
                    formatter,
                    "failed to register Desktop Shell change notifications"
                )
            }
        }
    }
}

impl std::error::Error for ShellError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Com(error) => Some(error),
            Self::Utf16(error) => Some(error),
            Self::Windows(_) | Self::Execute(_) | Self::ChangeNotification => None,
        }
    }
}

impl From<io::Error> for ShellError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<windows::core::Error> for ShellError {
    fn from(value: windows::core::Error) -> Self {
        Self::Com(value)
    }
}

impl From<std::string::FromUtf16Error> for ShellError {
    fn from(value: std::string::FromUtf16Error) -> Self {
        Self::Utf16(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{ShellApartment, enumerate_desktop_namespace, stable_file_identity};
    use std::collections::HashSet;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn desktop_namespace_provides_stable_shell_identities() {
        let _apartment = ShellApartment::initialize_sta().unwrap();
        let items = enumerate_desktop_namespace(0).unwrap();
        assert!(!items.is_empty());

        let keys: HashSet<_> = items
            .iter()
            .map(|item| item.identity.persistent_key())
            .collect();
        assert_eq!(keys.len(), items.len());
        assert!(items.iter().all(|item| !item.display_name.is_empty()));
    }

    #[test]
    fn filesystem_identity_survives_a_rename() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("lucidpane-file-id-{unique}"));
        fs::create_dir_all(&root).unwrap();
        let before_path = root.join("before.txt");
        let after_path = root.join("after.txt");
        fs::write(&before_path, b"stable identity").unwrap();

        let before = stable_file_identity(&before_path).unwrap();
        fs::rename(&before_path, &after_path).unwrap();
        let after = stable_file_identity(&after_path).unwrap();

        assert_eq!(before, after);
        fs::remove_dir_all(root).unwrap();
    }
}
