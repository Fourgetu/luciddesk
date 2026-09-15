use std::collections::{BTreeMap, BTreeSet};
use windows::{
    Win32::{
        Foundation::POINT,
        System::Com::CoTaskMemFree,
        UI::Shell::{Common::ITEMIDLIST, *},
    },
    core::Result,
};

#[derive(Debug)]
pub struct ApplyError {
    pub error: windows::core::Error,
    pub before_write: bool,
}
impl ApplyError {
    pub fn code(&self) -> windows::core::HRESULT {
        self.error.code()
    }
}
impl From<windows::core::Error> for ApplyError {
    fn from(error: windows::core::Error) -> Self {
        Self {
            error,
            before_write: false,
        }
    }
}
impl From<windows::core::HRESULT> for ApplyError {
    fn from(error: windows::core::HRESULT) -> Self {
        windows::core::Error::from_hresult(error).into()
    }
}

pub struct Item {
    pub pidl: *mut ITEMIDLIST,
    pub name: String,
    pub position: POINT,
}
impl Drop for Item {
    fn drop(&mut self) {
        unsafe {
            CoTaskMemFree(Some(self.pidl.cast()));
        }
    }
}
pub fn key(name: &str) -> String {
    name.to_lowercase()
}
pub fn snapshot(folder: &IFolderView2) -> Result<Vec<Item>> {
    unsafe {
        let parent: IShellFolder = folder.GetFolder()?;
        let mut result = Vec::new();
        for index in 0..folder.ItemCount(SVGIO_ALLVIEW)? {
            let mut row = Item {
                pidl: folder.Item(index)?,
                name: String::new(),
                position: POINT::default(),
            };
            row.position = folder.GetItemPosition(row.pidl)?;
            let item: IShellItem = SHCreateItemWithParent(None, &parent, row.pidl)?;
            let raw = item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)?;
            let name = raw.to_string();
            CoTaskMemFree(Some(raw.0.cast()));
            row.name = key(&name?);
            result.push(row);
        }
        Ok(result)
    }
}

#[derive(Default)]
pub struct Membership {
    pub desired: BTreeSet<String>,
    hidden: BTreeMap<String, Item>,
    baseline: BTreeMap<String, POINT>,
    baseline_order: Vec<String>,
    user_layout_changed: bool,
}
impl Membership {
    pub fn replace_identity(&mut self, old: &str, new: &str) -> Result<()> {
        let old = key(old);
        let new = key(new);
        if !self.desired.contains(&old) {
            return Ok(());
        }
        let mut pidl = std::ptr::null_mut();
        let wide: Vec<u16> = new.encode_utf16().chain(Some(0)).collect();
        unsafe {
            SHParseDisplayName(
                windows::core::PCWSTR(wide.as_ptr()),
                None,
                &raw mut pidl,
                0,
                None,
            )?;
        }
        let position = self
            .baseline
            .get(&old)
            .copied()
            .or_else(|| self.hidden.get(&old).map(|row| row.position))
            .unwrap_or_default();
        self.hidden.remove(&old);
        self.hidden.insert(
            new.clone(),
            Item {
                pidl,
                name: new.clone(),
                position,
            },
        );
        self.desired.remove(&old);
        self.desired.insert(new.clone());
        if let Some(position) = self.baseline.remove(&old) {
            self.baseline.insert(new.clone(), position);
        }
        for name in &mut self.baseline_order {
            if *name == old {
                *name = new.clone();
            }
        }
        Ok(())
    }
    pub fn apply(
        &mut self,
        folder: &IFolderView2,
        legacy: &IShellFolderView,
        paused: bool,
    ) -> std::result::Result<(), ApplyError> {
        if self.desired.is_empty() && self.hidden.is_empty() && self.baseline.is_empty() {
            return Ok(());
        }
        let mut current = snapshot(folder).map_err(|error| ApplyError {
            error,
            before_write: true,
        })?;
        let layout: BTreeMap<_, _> = current
            .iter()
            .map(|row| (row.name.clone(), row.position))
            .collect();
        if self.baseline.is_empty() && !self.desired.is_empty() && !paused {
            self.baseline_order = current.iter().map(|row| row.name.clone()).collect();
            self.baseline = layout;
            self.user_layout_changed = false;
        }
        let release: Vec<_> = self
            .hidden
            .keys()
            .filter(|name| paused || !self.desired.contains(*name))
            .cloned()
            .collect();
        let mut added = false;
        for name in release {
            let row = &self.hidden[&name];
            if !current.iter().any(|item| item.name == name) && exists(&row.name) {
                unsafe {
                    legacy.AddObject(row.pidl)?;
                }
                added = true;
            }
            self.hidden.remove(&name);
        }
        if added {
            current = snapshot(folder)?;
        }
        // Restore the initial layout only while the user has not moved/sorted it.
        // Native changes during the session take precedence over our old baseline.
        if self.hidden.is_empty()
            && (paused || self.desired.is_empty())
            && !self.baseline.is_empty()
            && !self.user_layout_changed
        {
            // With auto-arrange enabled, the batch order influences insertion.
            // Reapply in the original view order, not the order AddObject produced.
            let positioned: Vec<_> = self
                .baseline_order
                .iter()
                .filter_map(|name| {
                    let row = current.iter().find(|row| &row.name == name)?;
                    self.baseline
                        .get(name)
                        .map(|position| (row.pidl.cast_const(), *position))
                })
                .collect();
            if !positioned.is_empty() {
                let ids: Vec<_> = positioned.iter().map(|(id, _)| *id).collect();
                let points: Vec<_> = positioned.iter().map(|(_, point)| *point).collect();
                unsafe {
                    folder.SelectAndPositionItems(
                        ids.len() as u32,
                        ids.as_ptr(),
                        Some(points.as_ptr()),
                        SVSI_POSITIONITEM.0 as u32,
                    )?;
                }
            }
        }
        if !paused {
            // Remove by PIDL, never by the indices invalidated by earlier removals.
            for row in current
                .into_iter()
                .rev()
                .filter(|row| self.desired.contains(&row.name))
            {
                unsafe {
                    legacy.RemoveObject(Some(row.pidl))?;
                }
                self.hidden.entry(row.name.clone()).or_insert(row);
            }
        }
        if !paused && self.desired.is_empty() {
            self.baseline.clear();
            self.baseline_order.clear();
            self.user_layout_changed = false;
        }
        Ok(())
    }
    pub fn restore(&mut self, folder: &IFolderView2, legacy: &IShellFolderView) -> Result<()> {
        self.desired.clear();
        self.apply(folder, legacy, false).map_err(|e| e.error)
    }
    pub fn user_changed_layout(&mut self) {
        self.user_layout_changed = true;
    }
}

fn exists(name: &str) -> bool {
    if name.starts_with("::{") {
        return true;
    }
    // No file content is opened. A deleted/renamed file must never be resurrected
    // as a stale Shell item when its membership is released.
    let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    unsafe {
        windows_sys::Win32::Storage::FileSystem::GetFileAttributesW(wide.as_ptr())
            != windows_sys::Win32::Storage::FileSystem::INVALID_FILE_ATTRIBUTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renamed_membership_uses_new_pidl_and_keeps_original_restore_position() {
        use windows::Win32::System::Com::{
            COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
        };
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        }
        let root = std::env::temp_dir().join(format!(
            "lucidpane-membership-rename-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let before = root.join("before.txt");
        let after = root.join("after.txt");
        std::fs::write(&before, b"owned rename regression").unwrap();
        {
            let old = key(&before.to_string_lossy());
            let new = key(&after.to_string_lossy());
            let mut membership = Membership::default();
            membership.desired.insert(old.clone());
            membership
                .baseline
                .insert(old.clone(), POINT { x: 120, y: 240 });
            membership.baseline_order.push(old.clone());
            std::fs::rename(&before, &after).unwrap();
            membership.replace_identity(&old, &new).unwrap();
            assert!(!membership.desired.contains(&old));
            assert!(membership.desired.contains(&new));
            assert_eq!(membership.baseline_order, vec![new.clone()]);
            assert_eq!(membership.baseline[&new].x, 120);
            assert_eq!(membership.hidden[&new].position.y, 240);
            assert!(!membership.hidden[&new].pidl.is_null());
            let item: IShellItem =
                unsafe { SHCreateItemFromIDList(membership.hidden[&new].pidl).unwrap() };
            let raw = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH).unwrap() };
            let actual = unsafe { raw.to_string().unwrap() };
            unsafe {
                CoTaskMemFree(Some(raw.0.cast()));
            }
            assert_eq!(key(&actual), new);
            assert!(
                membership
                    .replace_identity(&new, &root.join("missing.txt").to_string_lossy())
                    .is_err()
            );
            assert!(membership.desired.contains(&new));
        }
        std::fs::remove_file(after).unwrap();
        std::fs::remove_dir(root).unwrap();
        unsafe {
            CoUninitialize();
        }
    }
}
