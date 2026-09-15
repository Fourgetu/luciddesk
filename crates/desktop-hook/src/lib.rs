//! Explorer member filtering, with an explicitly enabled legacy geometry experiment.
#[cfg(feature = "legacy-geometry")]
mod client;
mod discovery;
#[cfg(feature = "legacy-geometry")]
mod engine;
pub mod filter;
#[cfg(all(feature = "legacy-geometry", target_arch = "x86_64"))]
pub mod geometry;
pub mod notifications;
#[cfg(feature = "legacy-geometry")]
mod pane_surface;
#[cfg(feature = "legacy-geometry")]
pub mod protocol;

#[cfg(feature = "legacy-geometry")]
pub use client::HookSession;
pub use discovery::{conflicting_desktop_extension, desktop_view};

#[cfg(feature = "legacy-geometry")]
use windows_sys::Win32::UI::WindowsAndMessaging::{CWPSTRUCT, CallNextHookEx};

/// Thread-scoped `WH_CALLWNDPROC` entry point, loaded by Windows into the target process.
/// No initialization, COM calls or patching happens under the loader lock.
///
/// # Safety
/// Only Windows may call this entry with a valid hook callback payload.
#[unsafe(no_mangle)]
#[cfg(feature = "legacy-geometry")]
pub unsafe extern "system" fn LucidPaneDesktopHook(code: i32, wp: usize, lp: isize) -> isize {
    if code >= 0 && lp != 0 {
        // Never unwind through a Windows callback. A failure leaves the original view active.
        let _ = std::panic::catch_unwind(|| {
            let message = unsafe { &*(lp as *const CWPSTRUCT) };
            if message.message == protocol::geometry_attach_message() {
                engine::attach_geometry(message.hwnd, message.wParam as _, message.lParam);
            }
        });
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wp, lp) }
}
