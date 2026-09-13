//! Coalesced worker notifications. The mutex also fences HWND destruction.
use std::sync::{Arc, Mutex};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

pub(super) const READY: u32 = WM_APP + 0x4c2;

#[derive(Clone, Default)]
pub(super) struct Wake(Arc<Mutex<(isize, bool)>>);

impl Wake {
    pub fn bind(&self, hwnd: isize) {
        *self.0.lock().unwrap() = (hwnd, false);
        self.notify();
    }

    pub fn unbind(&self) {
        *self.0.lock().unwrap() = (0, false);
    }

    pub fn received(&self) {
        self.0.lock().unwrap().1 = false;
    }

    pub fn notify(&self) {
        let mut state = self.0.lock().unwrap();
        if state.0 != 0 && !state.1 {
            state.1 = unsafe { PostMessageW(state.0 as _, READY, 0, 0) } != 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    #[test]
    fn worker_bursts_coalesce_and_notifications_stop_after_unbind() {
        let wake = Wake::default();
        let window = windows_window::Window::new("Wake test")
            .size(1, 1)
            .style(WS_POPUP)
            .create()
            .unwrap();
        let hwnd = window.hwnd().cast();
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        let take = || unsafe {
            let mut msg = MSG::default();
            PeekMessageW(&raw mut msg, hwnd, READY, READY, PM_REMOVE) != 0
        };
        wake.bind(window.hwnd() as isize);
        let worker = wake.clone();
        std::thread::spawn(move || {
            for _ in 0..1000 {
                worker.notify();
            }
        })
        .join()
        .unwrap();
        assert!(take());
        assert!(!take());
        wake.received();
        wake.notify();
        assert!(take());
        wake.unbind();
        wake.notify();
        assert!(!take());
    }
}
