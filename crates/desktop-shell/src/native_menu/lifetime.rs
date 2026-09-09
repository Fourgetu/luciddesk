//! Diagnostic `WinEvent` observer, restricted to the owning Explorer process.
//! This only records window show/hide and menu events; it does not send input.

// Diagnostics must never panic across a Windows callback when output is unavailable.
macro_rules! trace {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) || std::env::var_os("LUCIDPANE_MENU_TRACE").is_some() {
            use std::io::Write;
            let _ = writeln!(std::io::stderr().lock(), $($arg)*);
        }
    };
}
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowThreadProcessId, IsWindowVisible,
};
thread_local! { static POPUPS: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) }; }
thread_local! { static FIRST_VISIBLE: Cell<Option<Instant>> = const { Cell::new(None) }; }

type Hook = *mut c_void;
type Callback = unsafe extern "system" fn(Hook, u32, HWND, i32, i32, u32, u32);
#[link(name = "user32")]
unsafe extern "system" {
    fn SetWinEventHook(
        first: u32,
        last: u32,
        module: *mut c_void,
        callback: Option<Callback>,
        process: u32,
        thread: u32,
        flags: u32,
    ) -> Hook;
    fn UnhookWinEvent(hook: Hook) -> i32;
}

pub struct Observer {
    hooks: Vec<Hook>,
    process: u32,
    thread: u32,
}
impl Observer {
    pub fn new(process: u32, thread: u32) -> windows::core::Result<Self> {
        POPUPS.with(|popups| popups.borrow_mut().clear());
        FIRST_VISIBLE.set(None);
        trace!("menu_observer_process={process}, thread={thread}");
        let mut result = Self {
            hooks: Vec::new(),
            process,
            thread,
        };
        for (first, last) in [(4, 7), (0x8000, 0x8003)] {
            let hook = unsafe {
                SetWinEventHook(
                    first,
                    last,
                    std::ptr::null_mut(),
                    Some(on_event),
                    process,
                    thread,
                    2,
                )
            };
            if hook.is_null() {
                return Err(windows::core::Error::from_thread());
            }
            result.hooks.push(hook);
        }
        Ok(result)
    }

    pub fn first_visible(&self) -> Option<Instant> {
        FIRST_VISIBLE.get()
    }

    pub fn wait_for_close(&self) -> windows::core::Result<()> {
        debug_assert_eq!(
            self.hooks.len(),
            2,
            "The observer must stay registered while waiting"
        );
        let start = Instant::now();
        let mut seen = false;
        let mut hidden_since = None;
        loop {
            if !pump_messages() {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_ABORT,
                ));
            }
            let (observed, visible) = POPUPS.with(|popups| {
                let popups = popups.borrow();
                (
                    !popups.is_empty(),
                    popups.iter().any(|&hwnd| self.is_live_popup(hwnd)),
                )
            });
            seen |= observed;
            if seen && !visible {
                let hidden = hidden_since.get_or_insert_with(Instant::now);
                if hidden.elapsed() >= Duration::from_millis(200) {
                    trace!(
                        "native_popup_closed_after_ms={}",
                        start.elapsed().as_millis()
                    );
                    return Ok(());
                }
            } else {
                hidden_since = None;
            }
            if !seen && start.elapsed() > Duration::from_secs(5) {
                trace!("native_popup_not_observed=true");
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_FAIL,
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn is_live_popup(&self, hwnd: HWND) -> bool {
        unsafe {
            let mut process = 0;
            let thread = GetWindowThreadProcessId(hwnd, &raw mut process);
            if process != self.process || thread != self.thread || IsWindowVisible(hwnd) == 0 {
                return false;
            }
            let mut name = [0u16; 256];
            let count = GetClassNameW(hwnd, name.as_mut_ptr(), 256);
            usize::try_from(count)
                .is_ok_and(|count| is_menu_class(&String::from_utf16_lossy(&name[..count])))
        }
    }
}

fn is_menu_class(name: &str) -> bool {
    matches!(
        name,
        "Microsoft.UI.Content.PopupWindowSiteBridge" | "#32768"
    )
}

fn pump_messages() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage, TranslateMessage, WM_QUIT,
    };
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&raw mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            if message.message == WM_QUIT {
                PostQuitMessage(0);
                return false;
            }
            TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
    }
    true
}
impl Drop for Observer {
    fn drop(&mut self) {
        for &hook in &self.hooks {
            unsafe {
                UnhookWinEvent(hook);
            }
        }
    }
}
unsafe extern "system" fn on_event(
    _hook: Hook,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    thread: u32,
    time: u32,
) {
    if hwnd.is_null() || (event >= 0x8000 && (object != 0 || child != 0)) {
        return;
    }
    let mut name = [0u16; 256];
    let count = unsafe { GetClassNameW(hwnd, name.as_mut_ptr(), 256) };
    if let Ok(count) = usize::try_from(count) {
        let name = String::from_utf16_lossy(&name[..count]);
        // Classic menus may follow the compact popup via "Show more options".
        // Track their SHOW / MENUPOPUPSTART events through the same close barrier.
        if matches!(event, 0x8002 | 6) && is_menu_class(&name) {
            if FIRST_VISIBLE.get().is_none() {
                FIRST_VISIBLE.set(Some(Instant::now()));
            }
            POPUPS.with(|popups| {
                let mut popups = popups.borrow_mut();
                if !popups.contains(&hwnd) {
                    popups.push(hwnd);
                }
            });
        }
        trace!(
            "menu_window_event=0x{event:x}, hwnd={hwnd:?}, class={name}, thread={thread}, time={time}"
        );
    }
}
