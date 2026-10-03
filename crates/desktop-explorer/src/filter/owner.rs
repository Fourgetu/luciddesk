//! Callbacks only post a wakeup; all Shell work stays on the desktop STA.
use std::{cell::Cell, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HANDLE, HWND, INVALID_HANDLE_VALUE},
    System::Threading::*,
    UI::{Accessibility::*, WindowsAndMessaging::*},
};

thread_local! {
    static WINDOWS: Cell<(HWND, HWND)> = const { Cell::new((null_mut(), null_mut())) };
}
pub(super) struct OwnerWatch {
    wait: HANDLE,
    events: HWINEVENTHOOK,
}
impl OwnerWatch {
    pub fn new(process: HANDLE, owner: HWND, view: HWND) -> windows::core::Result<Self> {
        let mut watch = Self {
            wait: null_mut(),
            events: null_mut(),
        };
        unsafe {
            if RegisterWaitForSingleObject(
                &raw mut watch.wait,
                process,
                Some(exited),
                view.cast(),
                INFINITE,
                WT_EXECUTEONLYONCE,
            ) == 0
            {
                return Err(windows::core::Error::from_thread());
            }
            let mut pid = 0;
            let thread = GetWindowThreadProcessId(owner, &raw mut pid);
            if thread == 0 || pid == 0 {
                return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_ABORT));
            }
            watch.events = SetWinEventHook(
                EVENT_OBJECT_DESTROY,
                EVENT_OBJECT_DESTROY,
                null_mut(),
                Some(destroyed),
                pid,
                thread,
                WINEVENT_OUTOFCONTEXT,
            );
            if watch.events.is_null() {
                return Err(windows::core::Error::from_thread());
            }
        }
        WINDOWS.with(|windows| windows.set((owner, view)));
        Ok(watch)
    }
}
impl Drop for OwnerWatch {
    fn drop(&mut self) {
        WINDOWS.with(|windows| windows.set((null_mut(), null_mut())));
        unsafe {
            if !self.events.is_null() {
                UnhookWinEvent(self.events);
            }
            // The callback never waits on the STA. Join it before closing the
            // process handle or allowing a view HWND to be reused.
            if !self.wait.is_null() {
                UnregisterWaitEx(self.wait, INVALID_HANDLE_VALUE);
            }
        }
    }
}
unsafe extern "system" fn exited(context: *mut core::ffi::c_void, _: bool) {
    let _callback = super::library::Callback::enter();
    unsafe {
        PostMessageW(context.cast(), super::work_message(), 0, 0);
    }
}
unsafe extern "system" fn destroyed(
    _: HWINEVENTHOOK,
    _: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    let _callback = super::library::Callback::enter();
    if object == 0 && child == 0 {
        WINDOWS.with(|windows| {
            let (owner, view) = windows.get();
            if hwnd == owner && !view.is_null() {
                unsafe {
                    PostMessageW(view, super::work_message(), 0, 0);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::CloseHandle;

    unsafe fn window() -> HWND {
        unsafe {
            CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                windows_sys::w!("hook owner test"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        }
    }
    fn received(view: HWND, duration: Duration) -> bool {
        let deadline = Instant::now() + duration;
        loop {
            let mut msg = MSG::default();
            unsafe {
                while PeekMessageW(&raw mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    if msg.hwnd == view && msg.message == super::super::work_message() {
                        return true;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn signaled_owner_wakes_once_and_unregister_cancels_pending_callback() {
        unsafe {
            let owner = window();
            let view = window();
            let signal = CreateEventW(null_mut(), 1, 0, null_mut());
            assert!(!owner.is_null() && !view.is_null() && !signal.is_null());
            let watch = OwnerWatch::new(signal, owner, view).unwrap();
            SetEvent(signal);
            assert!(received(view, Duration::from_secs(2)));
            assert!(
                !received(view, Duration::from_millis(50)),
                "one-shot wait must not spin on a signaled handle"
            );
            drop(watch);
            ResetEvent(signal);
            let watch = OwnerWatch::new(signal, owner, view).unwrap();
            drop(watch);
            SetEvent(signal);
            assert!(!received(view, Duration::from_millis(50)));
            CloseHandle(signal);
            DestroyWindow(owner);
            DestroyWindow(view);
        }
    }
    #[test]
    fn destroyed_controller_wakes_while_its_process_remains_alive() {
        unsafe {
            let owner = window();
            let view = window();
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, GetCurrentProcessId());
            assert!(!process.is_null());
            let watch = OwnerWatch::new(process, owner, view).unwrap();
            DestroyWindow(owner);
            assert!(received(view, Duration::from_secs(2)));
            drop(watch);
            CloseHandle(process);
            DestroyWindow(view);
        }
    }
}
