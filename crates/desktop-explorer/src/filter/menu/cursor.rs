//! Clear Shell's temporary busy pointer only in our foreground menu STA.
//! Never replaces a system cursor or changes another application's pointer.
use windows_sys::Win32::{System::Threading::GetCurrentThreadId, UI::WindowsAndMessaging::*};

pub(super) fn normal_pointer() {
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.is_null()
            || GetWindowThreadProcessId(foreground, std::ptr::null_mut()) != GetCurrentThreadId()
        {
            return;
        }
        let mut class = [0u16; 64];
        let len = GetClassNameW(foreground, class.as_mut_ptr(), class.len() as i32);
        if len <= 0
            || class[..len as usize]
                != "LucidDesk.IsolatedShellHost.v1"
                    .encode_utf16()
                    .collect::<Vec<_>>()
        {
            return;
        }
        let current = GetCursor();
        if current == LoadCursorW(std::ptr::null_mut(), IDC_WAIT)
            || current == LoadCursorW(std::ptr::null_mut(), IDC_APPSTARTING)
        {
            SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_ARROW));
            #[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
            {
                let key = windows_sys::w!("LucidDesk.Menu.BusyCursorCleared");
                let count = GetPropW(foreground, key) as usize;
                SetPropW(foreground, key, count.saturating_add(1) as _);
            }
        }
    }
}
