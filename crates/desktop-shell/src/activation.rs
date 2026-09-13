//! Shell activation and decoding of incoming drag identities.
use crate::{ShellError, ShellIdentity, namespace::shell_item_name};
use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::PathBuf, ptr};
use windows::Win32::UI::Shell::SIGDN_FILESYSPATH;
use windows_sys::Win32::{
    Foundation::HWND,
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};

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
