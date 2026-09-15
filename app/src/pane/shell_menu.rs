//! Explorer hosts the compact menu against an isolated, validated Shell selection.
use windows_sys::Win32::Foundation::{HWND, POINT};

pub fn show_many(owner: HWND, host: isize, point: POINT, keyboard: bool) -> Result<(), String> {
    desktop_shell::show_isolated_item_menu(
        windows::Win32::Foundation::HWND(owner),
        windows::Win32::Foundation::HWND(host as _),
        windows::Win32::Foundation::POINT {
            x: point.x,
            y: point.y,
        },
        if keyboard {
            desktop_shell::MenuInvocation::Keyboard
        } else {
            desktop_shell::MenuInvocation::Mouse
        },
    )
    .map_err(|error| format!("无法打开 Explorer 图标菜单：{error}"))
}
