//! Shared identity resolution and selection for new and reused menu hosts.
use windows::{
    Win32::UI::Shell::{
        IFolderView2, IShellItem, IShellItemArray, SHCreateItemFromParsingName, SICHINT_CANONICAL,
        SICHINT_TEST_FILESYSPATH_IF_NOT_EQUAL, SVSI_DESELECTOTHERS, SVSI_FOCUSED, SVSI_SELECT,
    },
    core::{HSTRING, Result},
};

/// Request-local objects, created and released on the menu STA. Never send
/// these COM interfaces through the worker channel or cache them across requests.
pub(super) struct ResolvedTargets {
    pub names: Vec<String>,
    pub items: Vec<IShellItem>,
}
impl ResolvedTargets {
    pub fn resolve(names: &[String]) -> Result<Self> {
        let items = names
            .iter()
            .map(|name| unsafe { SHCreateItemFromParsingName(&HSTRING::from(name), None) })
            .collect::<Result<_>>()?;
        Ok(Self {
            names: names.to_vec(),
            items,
        })
    }
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

/// Wait only while populating the isolated collection; verify identities as well
/// as count because replacing one item with another can keep the same count.
pub(super) fn wait_for_targets(folder: &IFolderView2, items: &[IShellItem]) -> Result<()> {
    use windows::Win32::{Foundation::E_FAIL, UI::Shell::SVGIO_ALLVIEW};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    unsafe {
        loop {
            let matches = (|| -> Result<bool> {
                if folder.ItemCount(SVGIO_ALLVIEW)? as usize != items.len() { return Ok(false); }
                let live: IShellItemArray = folder.Items(SVGIO_ALLVIEW)?;
                Ok(live.GetCount()? as usize == items.len() && contains_all(&live, items)?)
            })();
            match matches {
                Ok(true) => return Ok(()),
                Ok(false) => {},
                // RemoveAll/AddItem updates the view asynchronously. Its old
                // count can briefly outlive the corresponding item array.
                Err(error) if error.code() == E_FAIL || error.code() == windows::Win32::Foundation::E_BOUNDS => {},
                Err(error) => return Err(error),
            }
            if std::time::Instant::now() >= deadline {
                return Err(windows::core::Error::new(E_FAIL, "独立菜单项目加载超时"));
            }
            let mut message = MSG::default();
            for _ in 0..32 {
                if PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) == 0 { break; }
                if message.message == WM_QUIT {
                    PostQuitMessage(message.wParam as i32);
                    return Err(E_FAIL.into());
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            MsgWaitForMultipleObjectsEx(0, std::ptr::null(), 5, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
}
