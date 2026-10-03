//! Full Explorer host control. Crop only its visible region; retain its entire
//! Shell service/command/input chain. Owns and closes only the window it creates.
use std::{
    cell::RefCell,
    path::PathBuf,
    ptr::null_mut,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        System::{
            Com::{CLSCTX_LOCAL_SERVER, CoCreateInstance, IServiceProvider},
            Variant::VARIANT,
        },
        UI::Shell::*,
    },
    core::Interface,
};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};
#[path = "support/full_shell_collection.rs"]
mod collection;

struct ExplorerHost {
    browser: IWebBrowser2,
    root: HWND,
    view: HWND,
    cropped: bool,
    last_rect: Option<(i32, i32, i32, i32)>,
    frame: HWND,
    previous_owner: HWND,
    folded: bool,
    expanded_height: i32,
}
impl ExplorerHost {
    unsafe fn fit_frame(&mut self) -> windows::core::Result<()> {
        unsafe {
            if self.frame.is_null() {
                return self.update_region();
            }
            if self.folded || IsIconic(self.frame) != 0 || IsWindowVisible(self.frame) == 0 {
                if IsWindowVisible(self.root) != 0 {
                    ShowWindow(self.root, SW_HIDE);
                    println!("pane_content_visible=false");
                }
                return Ok(());
            }
            if !self.cropped {
                return self.update_region();
            }
            let mut client = RECT::default();
            let mut origin = POINT::default();
            let mut outer = RECT::default();
            let mut view = RECT::default();
            if GetClientRect(self.frame, &raw mut client) == 0
                || ClientToScreen(self.frame, &raw mut origin) == 0
                || GetWindowRect(self.root, &raw mut outer) == 0
                || GetWindowRect(self.view, &raw mut view) == 0
            {
                return Err(windows::core::Error::from_thread());
            }
            let scale = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(self.frame) as f64 / 96.0;
            let pad = (8.0 * scale).round() as i32;
            let header = (52.0 * scale).round() as i32;
            let width = (client.right - 2 * pad).max(100);
            let height = (client.bottom - header - pad).max(80);
            let x = origin.x + pad - (view.left - outer.left);
            let y = origin.y + header - (view.top - outer.top);
            let root_width = (outer.right - outer.left) + width - (view.right - view.left);
            let root_height = (outer.bottom - outer.top) + height - (view.bottom - view.top);
            if (
                outer.left,
                outer.top,
                outer.right - outer.left,
                outer.bottom - outer.top,
            ) != (x, y, root_width, root_height)
            {
                if SetWindowPos(
                    self.root,
                    null_mut(),
                    x,
                    y,
                    root_width,
                    root_height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                ) == 0
                {
                    return Err(windows::core::Error::from_thread());
                }
                println!(
                    "pane_layout target={},{},{},{} root={x},{y},{root_width},{root_height}",
                    origin.x + pad,
                    origin.y + header,
                    width,
                    height
                );
            }
            self.update_region()?;
            if IsWindowVisible(self.root) == 0 {
                ShowWindow(self.root, SW_SHOWNOACTIVATE);
                println!("pane_content_visible=true");
            }
            Ok(())
        }
    }
    unsafe fn update_region(&mut self) -> windows::core::Result<()> {
        unsafe {
            if !self.cropped {
                if self.last_rect.take().is_some() {
                    SetWindowRgn(self.root, null_mut(), 1);
                }
                return Ok(());
            }
            let mut outer = RECT::default();
            let mut content = RECT::default();
            if GetWindowRect(self.root, &raw mut outer) == 0
                || GetWindowRect(self.view, &raw mut content) == 0
            {
                return Err(windows::core::Error::from_thread());
            }
            let rect = (
                content.left - outer.left,
                content.top - outer.top,
                content.right - outer.left,
                content.bottom - outer.top,
            );
            if rect.2 <= rect.0 || rect.3 <= rect.1 {
                return Ok(());
            }
            if self.last_rect == Some(rect) {
                return Ok(());
            }
            let region = CreateRectRgn(rect.0, rect.1, rect.2, rect.3);
            if region.is_null() {
                return Err(windows::core::Error::from_thread());
            }
            // SetWindowRgn owns the region only on success.
            if SetWindowRgn(self.root, region, 1) == 0 {
                DeleteObject(region);
                return Err(windows::core::Error::from_thread());
            }
            self.last_rect = Some(rect);
            println!("cropped=true content_rect={rect:?}");
            Ok(())
        }
    }
}
impl Drop for ExplorerHost {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.root) != 0 {
                if !self.frame.is_null() {
                    SetWindowLongPtrW(self.root, GWLP_HWNDPARENT, self.previous_owner as isize);
                }
                SetWindowRgn(self.root, null_mut(), 1);
            }
            // Never enumerate or close other Explorer windows.
            println!("owned_explorer_quit_requested={:?}", self.browser.Quit());
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lp as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<ExplorerHost>;
        match msg {
            WM_COMMAND if !state.is_null() => {
                let Ok(mut host) = (*state).try_borrow_mut() else {
                    return 0;
                };
                match wp & 0xffff {
                    1 => host.cropped = true,
                    2 => host.cropped = false,
                    3 => {
                        PostMessageW(hwnd, WM_CLOSE, 0, 0);
                        return 0;
                    }
                    4 => {
                        let mut rect = RECT::default();
                        GetWindowRect(hwnd, &raw mut rect);
                        host.folded = !host.folded;
                        if host.folded {
                            host.expanded_height = rect.bottom - rect.top;
                        }
                        let height = if host.folded {
                            100
                        } else {
                            host.expanded_height
                        };
                        SetWindowPos(
                            hwnd,
                            null_mut(),
                            0,
                            0,
                            rect.right - rect.left,
                            height,
                            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                    _ => return DefWindowProcW(hwnd, msg, wp, lp),
                }
                if let Err(error) = host.fit_frame() {
                    eprintln!("region_error={error:?}");
                }
                0
            }
            WM_TIMER | 0x8051 if !state.is_null() => {
                let Ok(mut host) = (*state).try_borrow_mut() else {
                    return 0;
                };
                if IsWindow(host.root) == 0 {
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                } else if let Err(error) = host.fit_frame() {
                    eprintln!("region_error={error:?}");
                }
                0
            }
            WM_CLOSE => {
                KillTimer(hwnd, 1);
                PostQuitMessage(0);
                0
            }
            WM_WINDOWPOSCHANGED => {
                PostMessageW(hwnd, 0x8051, 0, 0);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_DPICHANGED => {
                let rect = &*(lp as *const RECT);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                0
            }
            WM_DESTROY => {
                KillTimer(hwnd, 1);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn fixture() -> std::io::Result<PathBuf> {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/full-shell-pane-probe")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&folder)?;
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(folder.join("Rename probe.txt"))?
        .write_all(b"Disposable full Shell host test file.\r\n")?;
    folder.canonicalize()
}

unsafe fn create_browser(folder: &str) -> Result<IWebBrowser2, Box<dyn std::error::Error>> {
    unsafe {
        match CoCreateInstance::<_, IWebBrowser2>(&ShellBrowserWindow, None, CLSCTX_LOCAL_SERVER) {
            Ok(browser) => return Ok(browser),
            Err(error) => println!(
                "shell_browser_coclass_unavailable={error:?}; launching owned fixture window"
            ),
        }
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)?;
        let mut previous = std::collections::HashSet::new();
        for i in 0..windows.Count()? {
            if let Ok(item) = windows.Item(&VARIANT::from(i)) {
                if let Ok(browser) = item.cast::<IWebBrowser2>() {
                    if let Ok(hwnd) = browser.HWND() {
                        previous.insert(hwnd.0);
                    }
                }
            }
        }
        let mut launcher = std::process::Command::new("C:/Windows/explorer.exe")
            .arg(format!("/n,{folder}"))
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(12);
        while Instant::now() < deadline {
            let _ = launcher.try_wait();
            for i in 0..windows.Count()? {
                let Ok(item) = windows.Item(&VARIANT::from(i)) else {
                    continue;
                };
                let Ok(browser) = item.cast::<IWebBrowser2>() else {
                    continue;
                };
                let Ok(hwnd) = browser.HWND() else { continue };
                if previous.contains(&hwnd.0) {
                    continue;
                }
                let Ok(provider) = browser.cast::<IServiceProvider>() else {
                    continue;
                };
                let Ok(shell) = provider.QueryService::<IShellBrowser>(&SID_STopLevelBrowser)
                else {
                    continue;
                };
                let Ok(view) = shell.QueryActiveShellView() else {
                    continue;
                };
                let Ok(view) = view.cast::<IFolderView2>() else {
                    continue;
                };
                let Ok(item) = view.GetFolder::<IShellItem>() else {
                    continue;
                };
                let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) else {
                    continue;
                };
                let text = name.to_string();
                windows::Win32::System::Com::CoTaskMemFree(Some(name.0.cast()));
                if text.is_ok_and(|name| name.eq_ignore_ascii_case(folder)) {
                    return Ok(browser);
                }
            }
            let mut msg = MSG::default();
            while PeekMessageW(&raw mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err("Could not identify a new Explorer window for the unique fixture; no existing window was changed".into())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        let _sta = luciddesk_shell::ShellApartment::initialize_sta()?;
        let folder = fixture()?;
        let folder = folder.to_string_lossy();
        let folder = folder.strip_prefix(r"\\?\").unwrap_or(&folder);
        let browser = create_browser(folder)?;
        let pane = std::env::args().any(|arg| arg == "--pane");
        let mut host = ExplorerHost {
            browser,
            root: null_mut(),
            view: null_mut(),
            cropped: true,
            last_rect: None,
            frame: null_mut(),
            previous_owner: null_mut(),
            folded: false,
            expanded_height: 520,
        };
        host.browser.SetLeft(360)?;
        host.browser.SetTop(240)?;
        host.browser.SetWidth(1000)?;
        host.browser.SetHeight(680)?;
        let empty = VARIANT::default();
        host.browser.Navigate2(
            &VARIANT::from(folder),
            Some(&empty),
            Some(&empty),
            Some(&empty),
            Some(&empty),
        )?;
        host.browser.SetVisible(true.into())?;
        let provider: IServiceProvider = host.browser.cast()?;
        let shell: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(view) = shell.QueryActiveShellView() {
                if let Ok(current) = view.cast::<IFolderView2>() {
                    if let Ok(item) = current.GetFolder::<IShellItem>() {
                        if let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                            let name_text = name.to_string();
                            windows::Win32::System::Com::CoTaskMemFree(Some(name.0.cast()));
                            if name_text?.eq_ignore_ascii_case(folder)
                                && current.ItemCount(SVGIO_ALLVIEW)? == 1
                            {
                                if pane {
                                    current.SetViewModeAndIconSize(FVM_ICON, 48)?;
                                }
                                host.view = view.GetWindow()?.0;
                                host.root = host.browser.HWND()?.0 as HWND;
                                break;
                            }
                        }
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err("Owned Explorer did not reach the one-file fixture".into());
            }
            let mut msg = MSG::default();
            while PeekMessageW(&raw mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut pid = 0;
        if std::env::args().any(|arg| arg == "--collection") {
            let view = collection::open(&shell, std::path::Path::new(folder))?;
            host.view = view.GetWindow()?.0;
        }
        if std::env::args().any(|arg| arg == "--results") {
            let view = collection::open_results(&shell, std::path::Path::new(folder))?;
            host.view = view.GetWindow()?.0;
        }
        let thread = GetWindowThreadProcessId(host.view, &raw mut pid);
        println!(
            "ready=true root={:?} view={:?} explorer_pid={pid} thread={thread} fixture={folder}",
            host.root, host.view
        );
        host.update_region()?;

        let instance = GetModuleHandleW(std::ptr::null());
        let class = windows_sys::w!("LucidDesk.FullShellPaneControl.v1");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let state = RefCell::new(host);
        let control = CreateWindowExW(
            0,
            class,
            if pane {
                windows_sys::w!("LucidDesk Shell 内容联动原型")
            } else {
                windows_sys::w!("LucidDesk 完整 Shell 对照：右键下方文件测试")
            },
            if pane {
                WS_OVERLAPPEDWINDOW
            } else {
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU
            },
            600,
            180,
            650,
            if pane { 520 } else { 95 },
            null_mut(),
            null_mut(),
            instance,
            (&raw const state).cast(),
        );
        if control.is_null() {
            return Err(windows::core::Error::from_thread().into());
        }
        let buttons = if pane {
            vec![(4, "折叠 / 展开", 12), (3, "关闭此测试", 220)]
        } else {
            vec![
                (1, "只显示文件视图", 12),
                (2, "显示完整资源管理器", 208),
                (3, "关闭此测试", 422),
            ]
        };
        for (id, label, x) in buttons {
            let label: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
            CreateWindowExW(
                0,
                windows_sys::w!("BUTTON"),
                label.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                x,
                12,
                190,
                32,
                control,
                id as HMENU,
                instance,
                std::ptr::null(),
            );
        }
        if pane {
            let mut host = state.borrow_mut();
            SetLastError(0);
            let previous = SetWindowLongPtrW(host.root, GWLP_HWNDPARENT, control as isize);
            if previous == 0 && GetLastError() != 0 {
                DestroyWindow(control);
                return Err(windows::core::Error::from_thread().into());
            }
            host.previous_owner = previous as HWND;
            host.frame = control;
            println!("pane_frame={control:?} explorer_owner_attached=true");
        }
        ShowWindow(control, SW_SHOWNORMAL);
        ShowWindow(control, SW_SHOW);
        SetTimer(control, 1, 300, None);
        let mut msg = MSG::default();
        while GetMessageW(&raw mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        drop(state.into_inner());
        DestroyWindow(control);
        Ok(())
    }
}
