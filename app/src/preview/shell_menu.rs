//! Item menus are hosted by Explorer so Windows 11 owns the compact UI and commands.
use desktop_core::ShellIdentity;
use windows_sys::Win32::Foundation::{HWND, POINT};

pub fn show(_owner: HWND, identity: &ShellIdentity, point: POINT) -> Result<(), String> {
    desktop_shell::show_desktop_item_menu(
        identity,
        windows::Win32::Foundation::POINT {
            x: point.x,
            y: point.y,
        },
    )
    .map_err(|error| format!("???? Explorer ???????{error}"))
}
