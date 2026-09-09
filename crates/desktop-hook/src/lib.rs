//! Experimental UI-thread Hook transport and standard-list work-area backend.
//! Explorer's virtual desktop is rejected until a compatible layout backend is implemented.
mod client;
mod engine;
mod pane_surface;
#[cfg(target_arch = "x86_64")]
pub mod geometry;
pub mod protocol;

pub use client::{HookSession, conflicting_desktop_extension, desktop_view, validate_layout_view};

use windows_sys::Win32::UI::WindowsAndMessaging::{CWPSTRUCT, CallNextHookEx};

/// Thread-scoped `WH_CALLWNDPROC` entry point, loaded by Windows into the target process.
/// No initialization, COM calls or patching happens under the loader lock.
///
/// # Safety
/// Only Windows may call this entry with a valid hook callback payload.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LucidPaneDesktopHook(code: i32, wp: usize, lp: isize) -> isize {
    if code >= 0 && lp != 0 {
        // Never unwind through a Windows callback. A failure leaves the original view active.
        let _ = std::panic::catch_unwind(|| {
            let message = unsafe { &*(lp as *const CWPSTRUCT) };
            if message.message == protocol::attach_message() {
                engine::attach(message.hwnd, message.wParam as _, message.lParam);
            } else if message.message == protocol::geometry_attach_message() {
                engine::attach_geometry(message.hwnd, message.wParam as _, message.lParam);
            }
        });
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wp, lp) }
}
