//! Bounded remove/restore transaction, called only on Explorer's desktop STA.
use std::fmt::Write as _;
use windows::Win32::Foundation::{E_FAIL, POINT};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{Common::ITEMIDLIST, *};
use windows::core::{Interface, Result};
use windows_sys::Win32::UI::{Controls::*, WindowsAndMessaging::*};

struct Row {
    pidl: *mut ITEMIDLIST,
    position: POINT,
    selected: u32,
    focused: bool,
}
impl Drop for Row {
    fn drop(&mut self) {
        unsafe {
            CoTaskMemFree(Some(self.pidl.cast()));
        }
    }
}

fn snapshot(folder: &IFolderView2) -> Result<Vec<Row>> {
    unsafe {
        let mut rows = Vec::new();
        let focused = folder.GetFocusedItem().unwrap_or(-1);
        for index in 0..folder.ItemCount(SVGIO_ALLVIEW)? {
            let mut row = Row {
                pidl: folder.Item(index)?,
                position: POINT::default(),
                selected: 0,
                focused: index == focused,
            };
            row.position = folder.GetItemPosition(row.pidl)?;
            row.selected = folder.GetSelectionState(row.pidl)? & SVSI_SELECT.0 as u32;
            rows.push(row);
        }
        Ok(rows)
    }
}

fn same(a: *const ITEMIDLIST, b: *const ITEMIDLIST) -> bool {
    unsafe { ILIsEqual(a, b).as_bool() }
}

fn pump(milliseconds: u64) {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_millis(milliseconds) {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    PostQuitMessage(msg.wParam as i32);
                    return;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

struct Restore {
    view: IShellView,
    folder: IFolderView2,
    legacy: IShellFolderView,
    before: Vec<Row>,
    target: usize,
    armed: bool,
}
impl Restore {
    fn restore(&mut self, log: &mut String) -> Result<()> {
        unsafe {
            let current = snapshot(&self.folder)?;
            let target = self.before[self.target].pidl;
            if !current.iter().any(|row| same(row.pidl, target)) {
                let result = self.legacy.AddObject(target);
                let _ = writeln!(log, "restore_add={result:?}");
                if result.is_err() {
                    self.view.Refresh()?;
                }
                pump(250);
            }
            let current = snapshot(&self.folder)?;
            let moved = self
                .before
                .iter()
                .filter(|old| {
                    current.iter().any(|row| {
                        same(row.pidl, old.pidl)
                            && (row.position.x != old.position.x
                                || row.position.y != old.position.y)
                    })
                })
                .count();
            if moved != 0 {
                let ids: Vec<_> = self
                    .before
                    .iter()
                    .map(|row| row.pidl.cast_const())
                    .collect();
                let positions: Vec<_> = self.before.iter().map(|row| row.position).collect();
                let result = self.folder.SelectAndPositionItems(
                    ids.len() as u32,
                    ids.as_ptr(),
                    Some(positions.as_ptr()),
                    SVSI_POSITIONITEM.0 as u32,
                );
                let _ = writeln!(log, "restore_positions={result:?}");
                result?;
                pump(150);
            }
            // Clear transient probe selection, then restore by identity, never by stale index.
            self.folder.SelectItem(-1, SVSI_DESELECTOTHERS.0 as u32)?;
            let current = snapshot(&self.folder)?;
            for old in &self.before {
                let flags = old.selected
                    | if old.focused {
                        SVSI_FOCUSED.0 as u32
                    } else {
                        0
                    };
                if flags != 0
                    && let Some(index) = current.iter().position(|row| same(row.pidl, old.pidl))
                {
                    self.folder.SelectItem(index as i32, flags)?;
                }
            }
            let after = snapshot(&self.folder)?;
            let ids_ok = self.before.len() == after.len()
                && self
                    .before
                    .iter()
                    .all(|old| after.iter().any(|row| same(row.pidl, old.pidl)));
            let positions_ok = self.before.iter().all(|old| {
                after
                    .iter()
                    .any(|row| same(row.pidl, old.pidl) && row.position == old.position)
            });
            let selection_ok = self.before.iter().all(|old| {
                after.iter().any(|row| {
                    same(row.pidl, old.pidl)
                        && row.selected == old.selected
                        && row.focused == old.focused
                })
            });
            let _ = writeln!(
                log,
                "restored_count={} identities_ok={ids_ok} positions_ok={positions_ok} selection_ok={selection_ok}",
                after.len()
            );
            if !ids_ok || !positions_ok || !selection_ok {
                return Err(windows::core::Error::new(E_FAIL, "Restoration mismatch"));
            }
            self.armed = false;
        }
        Ok(())
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        if self.armed {
            let mut log = String::new();
            let result = self.restore(&mut log);
            let _ = writeln!(log, "emergency_restore={result:?}");
            let _ = std::fs::write(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../target/desktop-filter-recovery.log"
                ),
                log,
            );
        }
    }
}

pub fn run(
    view: &IShellView,
    folder: &IFolderView2,
    refresh: bool,
    log: &mut String,
) -> Result<()> {
    unsafe {
        let legacy: IShellFolderView = view.cast()?;
        let before = snapshot(folder)?;
        if before.len() < 3 {
            return Err(windows::core::Error::new(
                E_FAIL,
                "At least 3 items required",
            ));
        }
        let target = before.len() / 2;
        let parent: IShellFolder = folder.GetFolder()?;
        let item: IShellItem = SHCreateItemWithParent(None, &parent, before[target].pidl)?;
        let raw = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = raw.to_string();
        CoTaskMemFree(Some(raw.0.cast()));
        let path = path?;
        if !std::path::Path::new(&path).exists() {
            return Err(windows::core::Error::new(E_FAIL, "Target file missing"));
        }
        let hwnd = view.GetWindow()?;
        let list = FindWindowExW(
            hwnd.0,
            std::ptr::null_mut(),
            windows_sys::w!("SysListView32"),
            std::ptr::null(),
        );
        if list.is_null() {
            return Err(windows::core::Error::new(E_FAIL, "ListView missing"));
        }
        let _ = writeln!(
            log,
            "target_index={target} target={path} before={} flags={:#x}",
            before.len(),
            folder.GetCurrentFolderFlags()?
        );
        let mut guard = Restore {
            view: view.clone(),
            folder: folder.clone(),
            legacy,
            before,
            target,
            armed: false,
        };
        // Save PIDLs and coordinates before mutation for independent recovery/debugging.
        let mut baseline = String::new();
        for row in &guard.before {
            let bytes = std::slice::from_raw_parts(
                row.pidl.cast::<u8>(),
                ILGetSize(Some(row.pidl)) as usize,
            );
            let _ = writeln!(
                baseline,
                "{} {} {} {} {}",
                row.position.x,
                row.position.y,
                row.selected,
                row.focused,
                bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
        }
        std::fs::write(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../target/desktop-filter-baseline.txt"
            ),
            baseline,
        )
        .map_err(|error| windows::core::Error::new(E_FAIL, error.to_string()))?;
        let test_result = (|| -> Result<()> {
            guard.armed = true;
            let removed = guard.legacy.RemoveObject(Some(guard.before[target].pidl));
            let _ = writeln!(log, "remove_result={removed:?}");
            removed?;
            pump(300);
            let after = snapshot(folder)?;
            let native_count = SendMessageW(list, LVM_GETITEMCOUNT, 0, 0);
            let absent = !after
                .iter()
                .any(|row| same(row.pidl, guard.before[target].pidl));
            let expected = guard.before.len() - 1 == after.len()
                && guard
                    .before
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != target)
                    .all(|(_, old)| after.iter().any(|row| same(row.pidl, old.pidl)));
            let moved = after
                .iter()
                .filter(|row| {
                    guard
                        .before
                        .iter()
                        .any(|old| same(row.pidl, old.pidl) && row.position != old.position)
                })
                .count();
            let _ = writeln!(
                log,
                "after_remove shell={} list={native_count} target_absent={absent} remaining_identities_ok={expected} file_exists={} moved_remaining={moved}",
                after.len(),
                std::path::Path::new(&path).exists()
            );
            if !absent || !expected || native_count != after.len() as isize {
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "Removal did not produce expected view",
                ));
            }
            // Native control selection must identify the same Shell item after index compaction.
            let index = target.min(after.len() - 1);
            let state = LVITEMW {
                state: LVIS_SELECTED | LVIS_FOCUSED,
                stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                ..Default::default()
            };
            SendMessageW(list, LVM_SETITEMSTATE, index, (&raw const state) as isize);
            let selected = folder.GetSelectionState(after[index].pidl)? & SVSI_SELECT.0 as u32 != 0;
            let mut rect = windows_sys::Win32::Foundation::RECT {
                left: LVIR_ICON as i32,
                ..Default::default()
            };
            let rect_ok = SendMessageW(list, LVM_GETITEMRECT, index, (&raw mut rect) as isize) != 0;
            let mut hit = LVHITTESTINFO {
                pt: windows_sys::Win32::Foundation::POINT {
                    x: (rect.left + rect.right) / 2,
                    y: (rect.top + rect.bottom) / 2,
                },
                ..Default::default()
            };
            let hit_index = if rect_ok {
                SendMessageW(list, LVM_HITTEST, 0, (&raw mut hit) as isize)
            } else {
                -1
            };
            let _ = writeln!(
                log,
                "native_selection_index={index} shell_selection_matches={selected} icon_hit_index={hit_index}"
            );
            if !selected || hit_index != index as isize {
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "Native selection or hit-test identity mismatch",
                ));
            }
            if refresh {
                let result = view.Refresh();
                let _ = writeln!(log, "refresh={result:?}");
                result?;
                pump(1500);
                let after = snapshot(folder)?;
                let _ = writeln!(
                    log,
                    "after_refresh shell={} list={} target_returned={}",
                    after.len(),
                    SendMessageW(list, LVM_GETITEMCOUNT, 0, 0),
                    after
                        .iter()
                        .any(|row| same(row.pidl, guard.before[target].pidl))
                );
            }
            Ok(())
        })();
        let restore_result = guard.restore(log);
        let _ = writeln!(
            log,
            "test_result={test_result:?} restore_result={restore_result:?}"
        );
        restore_result?;
        test_result
    }
}
