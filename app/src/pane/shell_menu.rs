//! Item menus are hosted by Explorer so Windows 11 owns the compact UI and commands.
use desktop_core::ShellIdentity;
use windows_sys::Win32::Foundation::{HWND, POINT};

pub fn show_many(
    owner: HWND,
    identities: &[ShellIdentity],
    point: POINT,
    keyboard: bool,
) -> Result<(), String> {
    desktop_shell::show_desktop_items_menu(
        windows::Win32::Foundation::HWND(owner),
        identities,
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
