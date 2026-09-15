//! Shared identity resolution and selection for new and reused menu hosts.
use windows::{
    Win32::UI::Shell::{
        IFolderView2, IShellItem, IShellItemArray, SHCreateItemFromParsingName, SICHINT_CANONICAL,
        SICHINT_TEST_FILESYSPATH_IF_NOT_EQUAL, SVSI_DESELECTOTHERS, SVSI_FOCUSED, SVSI_SELECT,
    },
    core::{HSTRING, Result},
};

pub(super) fn resolve(names: &[String]) -> Result<Vec<IShellItem>> {
    names
        .iter()
        .map(|name| unsafe { SHCreateItemFromParsingName(&HSTRING::from(name), None) })
        .collect()
}

pub(super) fn select_all(folder: &IFolderView2, count: usize) -> Result<()> {
    for index in 0..count {
        unsafe {
            folder.SelectItem(
                index as i32,
                (SVSI_SELECT.0
                    | if index == 0 {
                        SVSI_DESELECTOTHERS.0 | SVSI_FOCUSED.0
                    } else {
                        0
                    }) as u32,
            )?;
        }
    }
    Ok(())
}

/// Callers check the array count first and choose whether a mismatch is an
/// error (new host) or a cache miss (reuse). Compare canonical Shell identities,
/// never display labels, so equal-named items cannot substitute for one another.
pub(super) fn contains_all(array: &IShellItemArray, items: &[IShellItem]) -> Result<bool> {
    unsafe {
        for item in items {
            let mut found = false;
            for index in 0..array.GetCount()? {
                found |= item.Compare(
                    &array.GetItemAt(index)?,
                    (SICHINT_CANONICAL.0 | SICHINT_TEST_FILESYSPATH_IF_NOT_EQUAL.0) as u32,
                )? == 0;
            }
            if !found {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
