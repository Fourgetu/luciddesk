//! Read-only discovery of Explorer's desktop icon view.
use std::ptr;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    UI::{
        Shell::{SHELLSTATEA, SHGetSetSettings, SSF_HIDEICONS},
        WindowsAndMessaging::{EnumWindows, FindWindowExW, GetShellWindow, IsWindowVisible},
    },
};

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
