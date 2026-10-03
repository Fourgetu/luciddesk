//! Coalesced worker notifications. The mutex also fences HWND destruction.
use std::sync::{Arc, Mutex};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

pub(super) const READY: u32 = WM_APP + 0x4c2;

#[derive(Clone, Default)]
pub(super) struct Wake(Arc<Mutex<(isize, bool, bool)>>);

pub(super) struct NotifyOnDrop(Wake);
impl Drop for NotifyOnDrop {
    fn drop(&mut self) { self.0.notify(); }
}

impl Wake {
    pub fn on_drop(&self) -> NotifyOnDrop { NotifyOnDrop(self.clone()) }
    pub fn bind(&self, hwnd: isize) {
        *self.0.lock().unwrap() = (hwnd, false, false);
        self.notify();
    }

    pub fn unbind(&self) {
        *self.0.lock().unwrap() = (0, false, false);
    }

    pub fn received(&self) -> bool {
        let mut state = self.0.lock().unwrap();
        state.1 = false;
        std::mem::take(&mut state.2)
    }

    fn layout_changed(&self) {
        self.0.lock().unwrap().2 = true;
        self.notify();
    }

    /// Observe lifecycle and per-window DPI without borrowing PaneApp during
    /// synchronous destruction or layout callbacks.
    pub fn watch_window(&self, hwnd: isize) -> Result<(), String> {
        use windows_sys::Win32::UI::Shell::SetWindowSubclass;
        let data = Box::into_raw(Box::new(self.clone()));
        if unsafe { SetWindowSubclass(hwnd as _, Some(window_event), READY as usize, data as usize) } == 0 {
            unsafe { drop(Box::from_raw(data)); }
            return Err(crate::i18n::text("ui-could-not-register-panel-notifications").into());
        }
        Ok(())
    }

    /// Called only on the UI thread, outside any PaneApp borrow.
    pub fn refresh_hotkeys(&self) -> bool {
        let hwnd = self.0.lock().unwrap().0;
        hwnd != 0 && unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(hwnd as _, super::runtime::REFRESH_HOTKEYS, 0, 0)
        } == 1
    }

    pub fn notify(&self) {
        let mut state = self.0.lock().unwrap();
        if state.0 != 0 && !state.1 {
            state.1 = unsafe { PostMessageW(state.0 as _, READY, 0, 0) } != 0;
        }
    }
}

unsafe extern "system" fn window_event(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    id: usize,
    data: usize,
) -> isize {
    use windows_sys::Win32::UI::{Shell::*, WindowsAndMessaging::*};
    let wake = unsafe { &*(data as *const Wake) };
    if msg == WM_DPICHANGED {
        wake.layout_changed();
    } else if msg == WM_NCDESTROY {
        wake.notify();
        unsafe {
            RemoveWindowSubclass(hwnd, Some(window_event), id);
            drop(Box::from_raw(data as *mut Wake));
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    #[test]
    fn worker_exit_wakes_after_disconnect_even_during_unwinding() {
        let wake = Wake::default();
        let window = windows_window::Window::new("Worker exit test").style(WS_POPUP).create().unwrap();
        let hwnd = window.hwnd().cast();
        let take = || unsafe {
            let mut message = MSG::default();
            PeekMessageW(&raw mut message, hwnd, READY, READY, PM_REMOVE) != 0
        };
        wake.bind(window.hwnd() as isize);
        assert!(take());
        wake.received();
        for panic in [false, true] {
            let ready = wake.clone();
            let (sender, receiver) = std::sync::mpsc::channel::<()>();
            let result = std::thread::spawn(move || {
                let _exit = ready.on_drop();
                let _sender = sender;
                assert!(!panic, "injected worker panic");
            }).join();
            assert_eq!(result.is_err(), panic);
            assert!(take());
            assert_eq!(receiver.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected));
            wake.received();
        }
        wake.unbind();
    }

    #[test]
    fn pane_dpi_and_destruction_notify_without_borrowing_application_state() {
        let wake = Wake::default();
        let supervisor = windows_window::Window::new("Lifecycle supervisor")
            .style(WS_POPUP)
            .on_message(|_, msg, _, _| (msg == WM_DESTROY).then_some(0))
            .create().unwrap();
        let pane = windows_window::Window::new("Lifecycle pane")
            .style(WS_POPUP)
            .on_message(|_, msg, _, _| matches!(msg, WM_DESTROY | WM_DPICHANGED).then_some(0))
            .create().unwrap();
        wake.bind(supervisor.hwnd() as isize);
        wake.watch_window(pane.hwnd() as isize).unwrap();
        let take = || unsafe {
            let mut message = MSG::default();
            PeekMessageW(&raw mut message, supervisor.hwnd().cast(), READY, READY, PM_REMOVE) != 0
        };
        assert!(take());
        assert!(!wake.received());
        unsafe { SendMessageW(pane.hwnd().cast(), WM_DPICHANGED, 0, 0); }
        assert!(take());
        assert!(wake.received());
        drop(pane);
        assert!(take());
        assert!(!wake.received());
        wake.unbind();
    }

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
