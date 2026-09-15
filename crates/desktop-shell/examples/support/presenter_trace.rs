//! Input diagnostics restricted to popups owned by the probe root.
use super::log;
macro_rules! println { ($($arg:tt)*) => { log(format_args!($($arg)*)) }; }
pub unsafe extern "system" fn inspect(
    hwnd: windows_sys::Win32::Foundation::HWND,
    _: u32,
    timer: usize,
    _: u32,
) {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let ticks = GetPropW(hwnd, windows_sys::w!("LP.TraceTicks")) as usize + 1;
        SetPropW(hwnd, windows_sys::w!("LP.TraceTicks"), ticks as _);
        if ticks >= 40 {
            KillTimer(hwnd, timer);
            RemovePropW(hwnd, windows_sys::w!("LP.TraceTicks"));
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(
            windows_sys::Win32::System::Threading::GetCurrentThreadId(),
            &raw mut info,
        );
        if ticks == 1 {
            println!(
                "input_state active={:?} focus={:?} capture={:?} foreground={:?}",
                info.hwndActive,
                info.hwndFocus,
                info.hwndCapture,
                GetForegroundWindow()
            );
        }
        if ticks == 1 {
            inspect_child(info.hwndFocus, 0);
        }
        windows_sys::Win32::UI::Shell::SetWindowSubclass(hwnd, Some(input_messages), 0x4c505449, 0);
        EnumChildWindows(hwnd, Some(inspect_child), 0);
        EnumWindows(Some(inspect_window), hwnd as isize);
    }
}
unsafe extern "system" fn inspect_window(
    hwnd: windows_sys::Win32::Foundation::HWND,
    owner: isize,
) -> i32 {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let mut pid = 0;
        let thread = GetWindowThreadProcessId(hwnd, &raw mut pid);
        if pid != std::process::id() || IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let mut class = [0u16; 128];
        let len = GetClassNameW(hwnd, class.as_mut_ptr(), 128);
        let class = String::from_utf16_lossy(&class[..len.max(0) as usize]);
        if class != "Microsoft.UI.Content.PopupWindowSiteBridge"
            || GetWindow(hwnd, GW_OWNER) as isize != owner
        {
            return 1;
        }
        let mut rect = windows_sys::Win32::Foundation::RECT::default();
        GetWindowRect(hwnd, &raw mut rect);
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return 1;
        }
        let fresh = GetPropW(hwnd, windows_sys::w!("LP.TracePopup")).is_null();
        if fresh {
            println!(
                "popup_input hwnd={hwnd:?} thread={thread} enabled={} style={:#x} exstyle={:#x} owner={:?} rect={},{},{},{}",
                windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(hwnd),
                GetWindowLongPtrW(hwnd, GWL_STYLE),
                GetWindowLongPtrW(hwnd, GWL_EXSTYLE),
                GetWindow(hwnd, GW_OWNER),
                rect.left,
                rect.top,
                rect.right,
                rect.bottom
            );
        }
        let point = windows_sys::Win32::Foundation::POINT {
            x: (rect.left + rect.right) / 2,
            y: rect.top + 30,
        };
        let hit = WindowFromPoint(point);
        if fresh {
            SetPropW(hwnd, windows_sys::w!("LP.TracePopup"), 1usize as _);
            let packed = ((point.y as u32 & 0xffff) << 16 | (point.x as u32 & 0xffff)) as isize;
            println!(
                "popup.hit center={},{} window={hit:?} nchit={}",
                point.x,
                point.y,
                SendMessageW(hwnd, WM_NCHITTEST, 0, packed)
            );
        }
        if thread == windows_sys::Win32::System::Threading::GetCurrentThreadId() {
            windows_sys::Win32::UI::Shell::SetWindowSubclass(
                hwnd,
                Some(input_messages),
                0x4c505449,
                0,
            );
            EnumChildWindows(hwnd, Some(inspect_child), 0);
        }
    }
    1
}
unsafe extern "system" fn inspect_child(
    hwnd: windows_sys::Win32::Foundation::HWND,
    _: isize,
) -> i32 {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        if hwnd.is_null() {
            return 1;
        }
        if GetPropW(hwnd, windows_sys::w!("LP.TraceChild")).is_null() {
            SetPropW(hwnd, windows_sys::w!("LP.TraceChild"), 1usize as _);
            let mut class = [0u16; 128];
            let len = GetClassNameW(hwnd, class.as_mut_ptr(), 128);
            let mut rect = windows_sys::Win32::Foundation::RECT::default();
            GetWindowRect(hwnd, &raw mut rect);
            println!(
                "input.child hwnd={hwnd:?} class={} parent={:?} visible={} enabled={} rect={},{},{},{}",
                String::from_utf16_lossy(&class[..len.max(0) as usize]),
                GetParent(hwnd),
                IsWindowVisible(hwnd),
                windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(hwnd),
                rect.left,
                rect.top,
                rect.right,
                rect.bottom
            );
        }
        windows_sys::Win32::UI::Shell::SetWindowSubclass(hwnd, Some(input_messages), 0x4c505449, 0);
    }
    1
}
unsafe extern "system" fn input_messages(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    id: usize,
    _: usize,
) -> isize {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        static INVOKE_MESSAGE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        if msg
            == *INVOKE_MESSAGE.get_or_init(|| {
                RegisterWindowMessageW(windows_sys::w!("FILE_EXPLORER_CONTEXTMENU_INVOKEMENUITEM"))
            })
        {
            println!("input.invoke_message hwnd={hwnd:?} command={wp:#x} lp={lp:#x}");
        }
        if matches!(
            msg,
            WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_POINTERDOWN
                | WM_POINTERUP
                | WM_MOUSEACTIVATE
                | WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_ACTIVATE
                | WM_CANCELMODE
                | WM_CAPTURECHANGED
                | WM_KEYDOWN
                | WM_COMMAND
        ) {
            println!("popup.message={msg:#x} hwnd={hwnd:?} wp={wp:#x}");
        }
        let result = windows_sys::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wp, lp);
        if matches!(msg, WM_NCHITTEST | WM_MOUSEACTIVATE) {
            let count = GetPropW(hwnd, windows_sys::w!("LP.TraceHitCount")) as usize;
            if count < 20 {
                println!("input.return message={msg:#x} hwnd={hwnd:?} result={result}");
                SetPropW(hwnd, windows_sys::w!("LP.TraceHitCount"), (count + 1) as _);
            }
        }
        if msg == WM_NCDESTROY {
            RemovePropW(hwnd, windows_sys::w!("LP.TracePopup"));
            RemovePropW(hwnd, windows_sys::w!("LP.TraceHitCount"));
            windows_sys::Win32::UI::Shell::RemoveWindowSubclass(hwnd, Some(input_messages), id);
        }
        result
    }
}
