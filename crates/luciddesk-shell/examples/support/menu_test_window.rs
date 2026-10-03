//! Foreground test harness. Right-click requests the Explorer-hosted menu.
use windows::Win32::UI::Shell::IShellView;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, GetWindowThreadProcessId, MSG,
    PostQuitMessage, RegisterClassW, SetWindowLongPtrW, TranslateMessage, WM_CLOSE, WM_DESTROY,
    WM_NCCREATE, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

struct State {
    view: IShellView,
    busy: std::cell::Cell<bool>,
}

pub fn run(view: &IShellView) -> windows::core::Result<()> {
    unsafe {
        if windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        ) == 0
        {
            return Err(windows::core::Error::from_thread());
        }
    }
    let state = State {
        view: view.clone(),
        busy: std::cell::Cell::new(false),
    };
    unsafe {
        let class = windows_sys::w!("LucidDeskMenuProbe");
        let definition = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&raw const definition) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let window = CreateWindowExW(
            0,
            class,
            windows_sys::w!("LucidDesk native menu test - right-click inside"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            600,
            300,
            650,
            600,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            (&raw const state).cast(),
        );
        if window.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        println!(
            "foreground_test_window_ready=true, pid={}",
            std::process::id()
        );
        let mut message = MSG::default();
        while GetMessageW(&raw mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        match message {
            WM_RBUTTONUP => {
                let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
                if let Some(state) = state.as_ref()
                    && !state.busy.replace(true)
                {
                    let mut point = windows_sys::Win32::Foundation::POINT::default();
                    if windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&raw mut point)
                        == 0
                    {
                        state.busy.set(false);
                        return 0;
                    }
                    if let Ok(shell_window) = state.view.GetWindow() {
                        let mut pid = 0;
                        GetWindowThreadProcessId(shell_window.0, &raw mut pid);
                        println!(
                            "allow_Explorer_foreground={}",
                            AllowSetForegroundWindow(pid)
                        );
                        if std::env::args().any(|arg| arg == "--activate-desktop") {
                            use windows_sys::Win32::UI::WindowsAndMessaging::{
                                GA_ROOT, GetAncestor, SetForegroundWindow,
                            };
                            let root = GetAncestor(shell_window.0, GA_ROOT);
                            println!("activate_Explorer_root={}", SetForegroundWindow(root));
                            println!(
                                "activate_Explorer_view={:?}",
                                state.view.UIActivate(
                                    windows::Win32::UI::Shell::SVUIA_ACTIVATE_FOCUS
                                        .0
                                        .cast_unsigned()
                                )
                            );
                        }
                    }
                    println!(
                        "foreground_popup_result={:?}",
                        super::show_at(
                            &state.view,
                            windows::Win32::Foundation::POINT {
                                x: point.x,
                                y: point.y
                            }
                        )
                    );
                    state.busy.set(false);
                }
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
