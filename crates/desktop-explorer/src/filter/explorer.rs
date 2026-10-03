//! Wake the client as soon as Explorer exits, before TaskbarCreated arrives.
use std::{ffi::c_void, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HWND, INVALID_HANDLE_VALUE, WAIT_TIMEOUT},
    System::Threading::*,
    UI::WindowsAndMessaging::PostMessageW,
};
pub(super) struct ExplorerWatch {
    process: HANDLE,
    wait: HANDLE,
}
impl ExplorerWatch {
    pub fn new(pid: u32, owner: HWND) -> Result<Self, String> {
        let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if process.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Self::from_handle(process, owner)
    }
    // Takes ownership of the synchronization handle, including on failure.
    fn from_handle(process: HANDLE, owner: HWND) -> Result<Self, String> {
        let mut watch = Self {
            process,
            wait: null_mut(),
        };
        unsafe {
            if RegisterWaitForSingleObject(
                &raw mut watch.wait,
                process,
                Some(exited),
                owner.cast(),
                INFINITE,
                WT_EXECUTEONLYONCE,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(watch)
    }
    pub fn alive(&self) -> bool {
        unsafe { WaitForSingleObject(self.process, 0) == WAIT_TIMEOUT }
    }
}
impl Drop for ExplorerWatch {
    fn drop(&mut self) {
        unsafe {
            if !self.wait.is_null() {
                UnregisterWaitEx(self.wait, INVALID_HANDLE_VALUE);
            }
            CloseHandle(self.process);
        }
    }
}
unsafe extern "system" fn exited(context: *mut c_void, _: bool) {
    unsafe {
        PostMessageW(
            context.cast(),
            crate::notifications::DESKTOP_EXIT_MESSAGE,
            0,
            0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    #[test]
    fn exit_signal_wakes_owner_once_without_a_poll_timer() {
        unsafe {
            let owner = CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                windows_sys::w!("exit wake test"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            assert!(!owner.is_null());
            let event = CreateEventW(null_mut(), 1, 0, null_mut());
            assert!(!event.is_null());
            let watch = ExplorerWatch::from_handle(event, owner).unwrap();
            assert!(watch.alive());
            SetEvent(event);
            assert!(!watch.alive());
            let mut message = MSG::default();
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut received = false;
            while Instant::now() < deadline {
                if PeekMessageW(
                    &raw mut message,
                    owner,
                    crate::notifications::DESKTOP_EXIT_MESSAGE,
                    crate::notifications::DESKTOP_EXIT_MESSAGE,
                    PM_REMOVE,
                ) != 0
                {
                    received = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(received);
            drop(watch); // joins callback before closing its handle
            assert_eq!(
                PeekMessageW(
                    &raw mut message,
                    owner,
                    crate::notifications::DESKTOP_EXIT_MESSAGE,
                    crate::notifications::DESKTOP_EXIT_MESSAGE,
                    PM_REMOVE
                ),
                0
            );
            DestroyWindow(owner);
        }
    }
}
