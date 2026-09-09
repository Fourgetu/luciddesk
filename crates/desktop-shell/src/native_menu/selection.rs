//! Stable Shell identities for the research menu target and selection restoration.

// Diagnostics must never panic across a Windows callback when output is unavailable.
macro_rules! trace {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) || std::env::var_os("LUCIDPANE_MENU_TRACE").is_some() {
            use std::io::Write;
            let _ = writeln!(std::io::stderr().lock(), $($arg)*);
        }
    };
}
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    IFolderView2, IShellFolder, IShellItem, SHCreateItemFromParsingName, SHCreateItemWithParent,
    SICHINT_CANONICAL, SICHINT_TEST_FILESYSPATH_IF_NOT_EQUAL, SVGIO_ALLVIEW, SVSI_DESELECTOTHERS,
    SVSI_FOCUSED, SVSI_SELECT,
};
use windows::core::{HSTRING, Result};

pub fn item_at(folder: &IFolderView2, index: i32) -> Result<IShellItem> {
    unsafe {
        let parent: IShellFolder = folder.GetFolder()?;
        let pidl = folder.Item(index)?;
        let item = SHCreateItemWithParent(None, &parent, pidl);
        CoTaskMemFree(Some(pidl.cast()));
        item
    }
}

pub fn find(folder: &IFolderView2, target: &IShellItem) -> Result<Option<i32>> {
    unsafe {
        for index in 0..folder.ItemCount(SVGIO_ALLVIEW)? {
            // The merged desktop namespace and a filesystem parsing path can describe
            // the same file with different PIDLs. Let Shell compare their file paths too.
            if item_at(folder, index)?.Compare(
                target,
                (SICHINT_CANONICAL.0 | SICHINT_TEST_FILESYSPATH_IF_NOT_EQUAL.0).cast_unsigned(),
            )? == 0
            {
                return Ok(Some(index));
            }
        }
    }
    Ok(None)
}

pub fn resolve(folder: &IFolderView2, name: &str) -> Result<i32> {
    let target: IShellItem = unsafe { SHCreateItemFromParsingName(&HSTRING::from(name), None)? };
    find(folder, &target)?.ok_or_else(|| {
        windows::core::Error::new(
            windows::Win32::Foundation::E_INVALIDARG,
            "Requested Shell item is not in the desktop view; refusing a different target",
        )
    })
}

pub struct RestoreSelection {
    folder: IFolderView2,
    selected: Vec<(IShellItem, u32)>,
    armed: bool,
}

impl RestoreSelection {
    pub fn capture(folder: &IFolderView2) -> Result<Self> {
        let mut selected = Vec::new();
        unsafe {
            let focused = folder.GetFocusedItem()?;
            for index in 0..folder.ItemCount(SVGIO_ALLVIEW)? {
                let pidl = folder.Item(index)?;
                let flags = folder.GetSelectionState(pidl);
                CoTaskMemFree(Some(pidl.cast()));
                let mut flags = flags? & SVSI_SELECT.0.cast_unsigned();
                if index == focused {
                    flags |= SVSI_FOCUSED.0.cast_unsigned();
                }
                if flags != 0 {
                    selected.push((item_at(folder, index)?, flags));
                }
            }
        }
        Ok(Self {
            folder: folder.clone(),
            selected,
            armed: true,
        })
    }

    pub fn finish(mut self) -> Result<()> {
        self.armed = false;
        self.restore()
    }

    fn restore(&self) -> Result<()> {
        // Resolve again before changing selection: deletion/reordering invalidates indices.
        let mut resolved = Vec::new();
        for (item, flags) in &self.selected {
            if let Some(index) = find(&self.folder, item)? {
                resolved.push((index, *flags));
            }
        }
        unsafe {
            self.folder
                .SelectItem(-1, SVSI_DESELECTOTHERS.0.cast_unsigned())?;
            for &(index, flags) in &resolved {
                self.folder.SelectItem(index, flags)?;
            }
            let focused = self.folder.GetFocusedItem()?;
            for index in 0..self.folder.ItemCount(SVGIO_ALLVIEW)? {
                let pidl = self.folder.Item(index)?;
                let actual = self.folder.GetSelectionState(pidl);
                CoTaskMemFree(Some(pidl.cast()));
                let mut actual = actual? & SVSI_SELECT.0.cast_unsigned();
                if focused == index {
                    actual |= SVSI_FOCUSED.0.cast_unsigned();
                }
                let expected = resolved
                    .iter()
                    .find(|(i, _)| *i == index)
                    .map_or(0, |(_, f)| *f);
                if actual != expected {
                    return Err(windows::core::Error::new(
                        windows::Win32::Foundation::E_FAIL,
                        format!(
                            "Restored selection differs at index {index}: expected {expected}, actual {actual}"
                        ),
                    ));
                }
            }
        }
        trace!("desktop_selection_and_focus_verified=true");
        Ok(())
    }
}

impl Drop for RestoreSelection {
    fn drop(&mut self) {
        if self.armed {
            trace!("desktop_selection_restored={:?}", self.restore());
        }
    }
}
