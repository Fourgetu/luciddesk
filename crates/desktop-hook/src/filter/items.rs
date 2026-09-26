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

#[derive(PartialEq, Eq)]
struct RowRevision {
    id: Vec<u8>,
    position: (i32, i32),
}

fn changed_snapshot() -> windows::core::Error {
    windows::core::Error::new(
        windows::core::HRESULT::from_win32(windows::Win32::Foundation::ERROR_RETRY.0),
        "桌面过滤清单在读取期间变化，暂不写入",
    )
}

fn row_revision(row: &Item) -> Result<RowRevision> {
    if row.pidl.is_null() {
        return Err(changed_snapshot());
    }
    let bytes = unsafe {
        let size = ILGetSize(Some(row.pidl)) as usize;
        if size <= 2 {
            return Err(changed_snapshot());
        }
        std::slice::from_raw_parts(row.pidl.cast::<u8>(), size).to_vec()
    };
    Ok(RowRevision {
        id: bytes,
        position: (row.position.x, row.position.y),
    })
}

fn validate_snapshot(before: &[RowRevision], after: &[RowRevision]) -> Result<()> {
    let ids: BTreeSet<_> = before.iter().map(|row| &row.id).collect();
    if before != after || ids.len() != before.len() || before.iter().any(|row| row.id.is_empty()) {
        return Err(changed_snapshot());
    }
    Ok(())
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
            if row.pidl.is_null() {
                return Err(changed_snapshot());
            }
            row.position = folder.GetItemPosition(row.pidl)?;
            let item: IShellItem = SHCreateItemWithParent(None, &parent, row.pidl)?;
            let raw = item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)?;
            let name = raw.to_string();
            CoTaskMemFree(Some(raw.0.cast()));
            row.name = key(&name?);
            result.push(row);
        }
        // Names may invoke Shell extensions and reenter Explorer. Verify the
        // ordered PIDLs and positions again before using rows as a write plan
        // or restoration baseline. These reads never freeze or mutate the view.
        let before = result
            .iter()
            .map(row_revision)
            .collect::<Result<Vec<_>>>()?;
        let names: BTreeSet<_> = result.iter().map(|row| &row.name).collect();
        if names.len() != result.len() {
            return Err(changed_snapshot());
        }
        let count = folder.ItemCount(SVGIO_ALLVIEW)?;
        if usize::try_from(count).ok() != Some(result.len()) {
            return Err(changed_snapshot());
        }
        let mut after = Vec::with_capacity(result.len());
        for index in 0..count {
            let mut row = Item {
                pidl: folder.Item(index)?,
                name: String::new(),
                position: POINT::default(),
            };
            if row.pidl.is_null() {
                return Err(changed_snapshot());
            }
            row.position = folder.GetItemPosition(row.pidl)?;
            after.push(row_revision(&row)?);
        }
        if folder.ItemCount(SVGIO_ALLVIEW)? != count {
            return Err(changed_snapshot());
        }
        validate_snapshot(&before, &after)?;
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
    fn needs_position_restore(&self, current: &[Item], added: bool) -> bool {
        // New rows may need their original auto-arrange order even when their
        // current coordinates happen to match. Otherwise leave equal positions alone.
        added
            || current.iter().any(|row| {
                self.baseline
                    .get(&row.name)
                    .is_some_and(|point| point.x != row.position.x || point.y != row.position.y)
            })
    }
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
        self.apply_before_write(folder, legacy, paused, || {})
    }
    pub fn apply_before_write(
        &mut self,
        folder: &IFolderView2,
        legacy: &IShellFolderView,
        paused: bool,
        mut before_write: impl FnMut(),
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
                before_write();
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
            && self.needs_position_restore(&current, added)
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
                before_write();
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
                before_write();
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
    fn unchanged_layout_does_not_reposition_but_insertions_keep_original_order() {
        let mut membership = Membership::default();
        membership
            .baseline
            .insert("one".into(), POINT { x: 10, y: 20 });
        let mut rows = vec![Item {
            pidl: std::ptr::null_mut(),
            name: "one".into(),
            position: POINT { x: 10, y: 20 },
        }];
        assert!(!membership.needs_position_restore(&rows, false));
        assert!(membership.needs_position_restore(&rows, true));
        rows[0].position.x += 1;
        assert!(membership.needs_position_restore(&rows, false));
        rows[0].name = "unrelated".into();
        assert!(!membership.needs_position_restore(&rows, false));
    }
    #[test]
    #[ignore = "Read-only live Explorer snapshot; requires an interactive desktop"]
    fn live_filter_snapshot_reads_without_mutating_desktop() {
        use windows::{
            Win32::System::{
                Com::{
                    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                    CoUninitialize, IServiceProvider,
                },
                Variant::VARIANT,
            },
            core::Interface,
        };
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        }
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        unsafe {
            let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).unwrap();
            let mut raw = 0;
            let dispatch = shell
                .FindWindowSW(
                    &VARIANT::from(CSIDL_DESKTOP.cast_signed()),
                    &VARIANT::default(),
                    SWC_DESKTOP,
                    &raw mut raw,
                    SWFO_NEEDDISPATCH,
                )
                .unwrap();
            let provider: IServiceProvider = dispatch.cast().unwrap();
            let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser).unwrap();
            let folder: IFolderView2 = browser.QueryActiveShellView().unwrap().cast().unwrap();
            let rows = snapshot(&folder).unwrap();
            assert!(
                rows.iter()
                    .all(|row| !row.pidl.is_null() && !row.name.is_empty())
            );
        }
    }
    #[test]
    fn filter_snapshot_rejects_partial_reordered_replaced_and_moved_rows() {
        let rows = |ids: &[u8]| {
            ids.iter()
                .map(|id| RowRevision {
                    id: vec![*id],
                    position: (10, 20),
                })
                .collect::<Vec<_>>()
        };
        let before = rows(&[1, 2]);
        assert!(validate_snapshot(&before, &rows(&[1, 2])).is_ok());
        for ids in [&[1][..], &[1, 2, 3], &[2, 1], &[1, 3]] {
            assert!(validate_snapshot(&before, &rows(ids)).is_err());
        }
        let mut moved = rows(&[1, 2]);
        moved[1].position.0 += 1;
        assert!(validate_snapshot(&before, &moved).is_err());
        assert!(validate_snapshot(&rows(&[1, 1]), &rows(&[1, 1])).is_err());
        assert!(validate_snapshot(&[], &[]).is_ok());
        let invalid = vec![RowRevision {
            id: Vec::new(),
            position: (0, 0),
        }];
        assert!(validate_snapshot(&invalid, &invalid).is_err());
    }
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
