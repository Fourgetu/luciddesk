//! Desktop view discovery and conflicting extension detection.
#![allow(clippy::cast_possible_truncation)]
use std::{
    mem::size_of,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{EnumWindows, FindWindowExW, FindWindowW, GetWindowThreadProcessId},
};

/// Finds the real desktop `ListView`; never creates or hides a replacement icon layer.
/// # Errors
/// Returns an error when the current Shell desktop cannot be found.
pub fn desktop_view() -> Result<isize, String> {
    unsafe {
        let progman = FindWindowW(windows_sys::w!("Progman"), null());
        let defview = FindWindowExW(
            progman,
            null_mut(),
            windows_sys::w!("SHELLDLL_DefView"),
            null(),
        );
        if !defview.is_null() {
            let view = FindWindowExW(
                defview,
                null_mut(),
                windows_sys::w!("SysListView32"),
                null(),
            );
            if !view.is_null() {
                return Ok(view as isize);
            }
        }
        let mut result: HWND = null_mut();
        EnumWindows(Some(find_view), (&raw mut result) as isize);
        if result.is_null() {
            Err("找不到 Explorer 原生桌面图标视图".into())
        } else {
            Ok(result as isize)
        }
    }
}

unsafe extern "system" fn find_view(hwnd: HWND, lp: isize) -> i32 {
    let defview = unsafe {
        FindWindowExW(
            hwnd,
            null_mut(),
            windows_sys::w!("SHELLDLL_DefView"),
            null(),
        )
    };
    if !defview.is_null() {
        let view = unsafe {
            FindWindowExW(
                defview,
                null_mut(),
                windows_sys::w!("SysListView32"),
                null(),
            )
        };
        if !view.is_null() {
            unsafe {
                *(lp as *mut HWND) = view;
            }
            return 0;
        }
    }
    1
}

/// Detect another active desktop organizer before allowing work-area changes.
#[must_use]
pub fn conflicting_desktop_extension() -> bool {
    // Detection is intentionally limited to known loaded Explorer modules, not window titles.
    // The app may also use this to display a concrete conflict instead of fighting its layout.
    let Ok(view) = desktop_view() else {
        return false;
    };
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(view as HWND, &raw mut pid);
    }
    loaded_fences(pid) && fences_running()
}

fn fences_running() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return true;
        }
        let mut item = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..std::mem::zeroed()
        };
        let mut current = Process32FirstW(snapshot, &raw mut item);
        let mut found = false;
        while current != 0 {
            let end = item
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(item.szExeFile.len());
            if String::from_utf16_lossy(&item.szExeFile[..end]).eq_ignore_ascii_case("Fences.exe") {
                found = true;
                break;
            }
            current = Process32NextW(snapshot, &raw mut item);
        }
        CloseHandle(snapshot);
        found
    }
}

fn loaded_fences(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, TH32CS_SNAPMODULE,
        TH32CS_SNAPMODULE32,
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry = MODULEENTRY32W {
            dwSize: size_of::<MODULEENTRY32W>() as u32,
            ..std::mem::zeroed()
        };
        let mut found = false;
        let mut available = Module32FirstW(snapshot, &raw mut entry);
        while available != 0 {
            let end = entry
                .szModule
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szModule.len());
            let name = String::from_utf16_lossy(&entry.szModule[..end]).to_lowercase();
            if name == "desktopdock64.dll" {
                found = true;
                break;
            }
            available = Module32NextW(snapshot, &raw mut entry);
        }
        CloseHandle(snapshot);
        found
    }
}
