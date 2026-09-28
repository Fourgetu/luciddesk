//! Display enumeration and application error reporting.
mod monitors;
pub use monitors::{MonitorDescriptor, PixelRect, enumerate_monitors};

pub fn show_error(error: &str) {
    let text: Vec<u16> = error.encode_utf16().chain(Some(0)).collect();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            windows_sys::w!("LucidDesk"),
            windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONWARNING,
        );
    }
}
