//! Visible, persistent Shell content hosted by a standalone Pane-sized window.
//! Uses disposable workspace files; does not attach to or change the desktop.
//! Run with --compact to probe the unpublished Win11 presenter separately.
#[path = "support/shell_pane_presenter.rs"]
mod presenter;
macro_rules! println { ($($arg:tt)*) => { presenter::log(format_args!($($arg)*)) }; }

use std::{cell::RefCell, path::PathBuf, ptr::null_mut};
use windows::{
    Win32::{
        Foundation::{HWND, RECT},
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
            Ole::IObjectWithSite,
        },
        UI::Shell::*,
    },
    core::{HSTRING, IUnknown, Interface, Result},
};
use windows_sys::Win32::{System::LibraryLoader::*, UI::WindowsAndMessaging::*};

thread_local! {
    static BROWSER: RefCell<Option<IExplorerBrowser>> = const { RefCell::new(None) };
    static RETAINED_HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

unsafe extern "system" fn close_retained_host(
    hwnd: windows_sys::Win32::Foundation::HWND,
    _: u32,
    timer: usize,
    _: u32,
) {
    unsafe {
        KillTimer(hwnd, timer);
    }
    // Release outside the RefCell borrow: destroying the Shell view reenters UI.
    let host = RETAINED_HOST.with(|slot| slot.borrow_mut().take());
    drop(host);
}

struct Host {
    browser: IExplorerBrowser,
    hwnd: windows_sys::Win32::Foundation::HWND,
    presenter: Option<presenter::PresenterSite>,
}

impl Drop for Host {
    fn drop(&mut self) {
        BROWSER.with(|browser| browser.borrow_mut().take());
        unsafe {
            if let Some(presenter) = &self.presenter {
                presenter.close();
            }
            if let Ok(site) = self.browser.cast::<IObjectWithSite>() {
                let _ = site.SetSite(None::<&IUnknown>);
            }
            let _ = self.browser.Destroy();
            DestroyWindow(self.hwnd);
        }
        println!("host_destroyed=true");
    }
}

unsafe extern "system" fn window_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wp: usize,
    lp: isize,
) -> isize {
    unsafe {
        match message {
            WM_SIZE => {
                let browser = BROWSER.with(|slot| slot.borrow().clone());
                if let Some(browser) = browser {
                    let mut rect = windows_sys::Win32::Foundation::RECT::default();
                    GetClientRect(hwnd, &raw mut rect);
                    let _ = browser.SetRect(
                        None,
                        RECT {
                            left: 0,
                            top: 0,
                            right: rect.right,
                            bottom: rect.bottom,
                        },
                    );
                }
                0
            }
            WM_CLOSE => {
                if RETAINED_HOST.with(|slot| slot.borrow().is_some()) {
                    SetTimer(hwnd, 0x4c50434c, 1, Some(close_retained_host));
                    return 0;
                }
                // Destroy the presenter and view after leaving their message loop.
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    }
}

fn fixture() -> std::io::Result<Vec<PathBuf>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/shell-pane-probe")
        .join(std::process::id().to_string());
    let mut paths = Vec::new();
    for (folder, name) in [("A", "Rename test A.txt"), ("B", "Rename test B.txt")] {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(name);
        // Never overwrite a previous test, even if Windows reuses a process ID.
        if !path.exists() {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(b"Disposable LucidDesk Shell Pane prototype test file.\r\n")?;
        }
        paths.push(path.canonicalize()?);
    }
    Ok(paths)
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let compact = std::env::args().any(|arg| arg == "--compact");
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    let _sta = luciddesk_shell::ShellApartment::initialize_sta()?;
    let files = fixture()?;
    unsafe {
        run(
            &files,
            compact,
            std::env::args().any(|arg| arg == "--folder"),
            i32::from(std::env::args().any(|arg| arg == "--desktop-presenter")),
            false,
        )?;
    }
    Ok(())
}

pub fn run_in_explorer() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let _sta = luciddesk_shell::ShellApartment::initialize_sta()?;
    let files = fixture()?;
    unsafe {
        run(&files, true, true, 0, false)?;
    }
    Ok(())
}

pub fn start_on_explorer_loop() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let _sta = luciddesk_shell::ShellApartment::initialize_sta()?;
    if RETAINED_HOST.with(|slot| slot.borrow().is_some()) {
        return Err("This probe already has a retained host".into());
    }
    let files = fixture()?;
    unsafe {
        run(&files, true, true, 0, true)?;
    }
    Ok(())
}

unsafe fn run(
    files: &[PathBuf],
    compact: bool,
    folder_mode: bool,
    host_kind: i32,
    retain: bool,
) -> Result<()> {
    unsafe {
        let mut instance = null_mut();
        if GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            (window_proc as *const ()).cast(),
            &raw mut instance,
        ) == 0
        {
            return Err(windows::core::Error::from_thread());
        }
        let class = windows_sys::w!("LucidDesk.ShellPanePrototype.v1");
        let definition = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&definition) == 0 {
            let mut existing = WNDCLASSW::default();
            if GetClassInfoW(instance, class, &raw mut existing) == 0
                || existing.lpfnWndProc.map(|proc| proc as usize)
                    != Some(window_proc as *const () as usize)
            {
                return Err(windows::core::Error::from_thread());
            }
        }
        let hwnd = CreateWindowExW(
            0,
            class,
            windows_sys::w!("LucidDesk Shell Pane prototype - native view"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            760,
            540,
            null_mut(),
            null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        let browser = match CoCreateInstance::<_, IExplorerBrowser>(
            &ExplorerBrowser,
            None,
            CLSCTX_INPROC_SERVER,
        ) {
            Ok(browser) => browser,
            Err(error) => {
                DestroyWindow(hwnd);
                return Err(error);
            }
        };
        let mut host = Host {
            browser,
            hwnd,
            presenter: None,
        };
        let mut rect = windows_sys::Win32::Foundation::RECT::default();
        GetClientRect(hwnd, &raw mut rect);
        host.browser.Initialize(
            HWND(hwnd),
            &RECT {
                left: 0,
                top: 0,
                right: rect.right,
                bottom: rect.bottom,
            },
            Some(&FOLDERSETTINGS {
                ViewMode: FVM_ICON.0 as u32,
                fFlags: FWF_AUTOARRANGE.0 as u32,
            }),
        )?;
        println!("browser_initialized=true");
        host.browser
            .SetOptions(EBO_NAVIGATEONCE | EBO_NOTRAVELLOG)?;
        if folder_mode {
            let name = files[0]
                .parent()
                .expect("Fixture has a parent")
                .to_string_lossy();
            let name = name.strip_prefix(r"\\?\").unwrap_or(&name);
            let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(name), None)?;
            host.browser.BrowseToObject(&item, SBSP_SAMEBROWSER)?;
            println!("source=filesystem_folder path={name}");
        } else {
            host.browser
                .FillFromObject(None::<&IUnknown>, EBF_NODROPTARGET)?;
            println!("source=mixed_results");
        }
        let folder: IFolderView2 = host.browser.GetCurrentView()?;
        let results: Option<IResultsFolder> = if folder_mode {
            None
        } else {
            Some(folder.GetFolder()?)
        };
        for file in files.iter().filter(|_| !folder_mode) {
            let name = file.to_string_lossy();
            let name = name.strip_prefix(r"\\?\").unwrap_or(&name);
            println!("parsing={name}");
            let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(name), None)?;
            results.as_ref().expect("Results mode").AddItem(&item)?;
            println!("fixture={}", file.display());
        }
        let view: IShellView = folder.cast()?;
        if compact {
            match presenter::PresenterSite::create(&view, HWND(hwnd), host_kind) {
                Ok(presenter) => {
                    let site: IObjectWithSite = host.browser.cast()?;
                    site.SetSite(&presenter.service())?;
                    host.presenter = Some(presenter);
                    println!("compact_presenter_initialized=true");
                    SetWindowTextW(
                        hwnd,
                        windows_sys::w!(
                            "LucidDesk Shell Pane prototype - compact experiment (F6: direct request)"
                        ),
                    );
                }
                Err(error) => {
                    println!("compact_presenter_initialized=false error={error:?}");
                    SetWindowTextW(
                        hwnd,
                        windows_sys::w!(
                            "LucidDesk Shell Pane prototype - compact unavailable; native default"
                        ),
                    );
                }
            }
        }
        BROWSER.with(|slot| *slot.borrow_mut() = Some(host.browser.clone()));
        ShowWindow(hwnd, SW_SHOWNORMAL);
        // A hidden launcher may override the first ShowWindow via STARTUPINFO.
        // This is an explicitly interactive prototype, so show its content window.
        ShowWindow(hwnd, SW_SHOW);
        let _ = view.UIActivate(SVUIA_ACTIVATE_FOCUS.0 as u32);
        println!(
            "ready=true pid={} hwnd={hwnd:?} view={:?} host_creations=1 thread={}",
            std::process::id(),
            view.GetWindow()?,
            windows_sys::Win32::System::Threading::GetCurrentThreadId()
        );
        if retain {
            SetWindowTextW(
                hwnd,
                windows_sys::w!("LucidDesk Shell Pane prototype - Explorer original loop"),
            );
            RETAINED_HOST.with(|slot| *slot.borrow_mut() = Some(host));
            println!("original_explorer_loop=true startup_returned=true");
            // Explorer already owns this STA. Return to its original message
            // loop without taking over dispatch or posting WM_QUIT on close.
            return Ok(());
        }
        let mut msg = MSG::default();
        loop {
            let received = GetMessageW(&raw mut msg, null_mut(), 0, 0);
            if received == -1 {
                return Err(windows::core::Error::from_thread());
            }
            if received == 0 {
                break;
            }
            if msg.message == WM_KEYDOWN && msg.wParam == 0x75 {
                // Compare normal mouse dispatch with an explicit request to the
                // SAME visible view; never select or restore desktop items.
                let selected = folder.ItemCount(SVGIO_SELECTION)?;
                if selected > 0 {
                    let mut point = windows_sys::Win32::Foundation::POINT { x: 40, y: 100 };
                    windows_sys::Win32::Graphics::Gdi::ClientToScreen(hwnd, &raw mut point);
                    let menu: IContextMenu = view.GetItemObject(SVGIO_SELECTION)?;
                    let site: IContextMenuSite = view.cast()?;
                    println!(
                        "direct_popup={:?}",
                        site.DoContextMenuPopup(
                            &menu,
                            CMF_ITEMMENU | CMF_CANRENAME,
                            windows::Win32::Foundation::POINT {
                                x: point.x,
                                y: point.y
                            }
                        )
                    );
                }
                continue;
            }
            let module = GetModuleHandleW(windows_sys::w!("Microsoft.UI.Windowing.Core.dll"));
            let consumed = if let Some(proc) =
                GetProcAddress(module, windows_sys::s!("ContentPreTranslateMessage"))
            {
                let translate: unsafe extern "system" fn(*const MSG) -> i32 =
                    std::mem::transmute(proc);
                translate(&msg) != 0
            } else {
                false
            };
            // A view may translate its own keyboard input only. In particular,
            // do not feed the independent XAML popup's mouse/activation messages
            // into the underlying folder view's accelerator handler.
            let view_hwnd = view.GetWindow()?.0;
            let view_keyboard = (WM_KEYFIRST..=WM_KEYLAST).contains(&msg.message)
                && (msg.hwnd == view_hwnd || IsChild(view_hwnd, msg.hwnd) != 0);
            // Both crates use the SDK MSG layout. Only S_OK means handled.
            let shell_handled = !consumed
                && view_keyboard
                && (view.vtable().TranslateAccelerator)(
                    view.as_raw(),
                    (&raw const msg).cast::<windows::Win32::UI::WindowsAndMessaging::MSG>(),
                )
                .0 == 0;
            if matches!(
                msg.message,
                WM_LBUTTONDOWN | WM_LBUTTONUP | WM_POINTERDOWN | WM_POINTERUP
            ) {
                println!(
                    "input message={:#x} hwnd={:?} winui_consumed={consumed} shell_consumed={shell_handled}",
                    msg.message, msg.hwnd
                );
            }
            if !consumed && !shell_handled {
                if matches!(msg.message, WM_RBUTTONUP | WM_CONTEXTMENU) {
                    println!("dispatch.enter message={:#x}", msg.message);
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
                if matches!(msg.message, WM_RBUTTONUP | WM_CONTEXTMENU) {
                    println!("dispatch.leave message={:#x}", msg.message);
                    if let Some(presenter) = &host.presenter {
                        presenter.inspect_input(HWND(hwnd));
                    }
                }
            }
        }
        println!("final_count={}", folder.ItemCount(SVGIO_ALLVIEW)?);
        drop(view);
        drop(results);
        drop(folder);
        drop(host);
    }
    Ok(())
}
