//! Live desktop view filtering experiment. Default mode only queries interfaces.
//! No file is deleted or moved. Mutation modes must restore the view before exit.
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, IServiceProvider};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    CSIDL_DESKTOP, IFolderFilterSite, IFolderView2, IShellBrowser, IShellFolderView, IShellWindows,
    SID_STopLevelBrowser, SVGIO_ALLVIEW, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
};
use windows::core::{Interface, w};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _apartment = desktop_shell::ShellApartment::initialize_sta()?;
    unsafe {
        let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        let mut desktop_hwnd = 0;
        let dispatch = shell.FindWindowSW(
            &VARIANT::from(CSIDL_DESKTOP.cast_signed()),
            &VARIANT::default(),
            SWC_DESKTOP,
            &raw mut desktop_hwnd,
            SWFO_NEEDDISPATCH,
        )?;
        let provider: IServiceProvider = dispatch.cast()?;
        let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
        let view = browser.QueryActiveShellView()?;
        if std::env::args().any(|arg| arg == "--refresh") { view.Refresh()?; }
        let folder: IFolderView2 = view.cast()?;
        let hwnd = view.GetWindow()?;
        let list = windows_sys::Win32::UI::WindowsAndMessaging::FindWindowExW(
            hwnd.0,
            std::ptr::null_mut(),
            w!("SysListView32").as_ptr(),
            std::ptr::null(),
        );
        if list.is_null() {
            return Err("Desktop ListView not found".into());
        }
        if std::env::args().any(|arg| arg == "--in-process") {
            run_in_process(list)?;
            return Ok(());
        }
        println!("shell_count={}", folder.ItemCount(SVGIO_ALLVIEW)?);
        println!(
            "list_count={}",
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                list,
                windows_sys::Win32::UI::Controls::LVM_GETITEMCOUNT,
                0,
                0
            )
        );
        println!("folder_flags={:#x}", folder.GetCurrentFolderFlags()?);
        println!(
            "list_style={:#x}",
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongW(
                list,
                windows_sys::Win32::UI::WindowsAndMessaging::GWL_STYLE
            )
        );
        println!(
            "view_IShellFolderView={:?}",
            view.cast::<IShellFolderView>().map(|_| ())
        );
        println!(
            "view_IFolderFilterSite={:?}",
            view.cast::<IFolderFilterSite>().map(|_| ())
        );
        println!(
            "browser_IFolderFilterSite={:?}",
            browser.cast::<IFolderFilterSite>().map(|_| ())
        );
        if let Ok(legacy) = view.cast::<IShellFolderView>() {
            println!("legacy_count={:?}", legacy.GetObjectCount());
        }
    }
    Ok(())
}

fn run_in_process(
    list: windows_sys::Win32::Foundation::HWND,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows_sys::Win32::System::LibraryLoader::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let source = std::env::current_exe()?.with_file_name("desktop_filter_probe_hook.dll");
        let bytes = std::fs::read(&source)?;
        let hash = bytes.iter().fold(0xcbf29ce484222325_u64, |h,b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3));
        let path = source.with_file_name(format!("desktop-filter-probe-{hash:x}.dll"));
        if !path.exists() { std::fs::write(&path, bytes)?; }
        let wide: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let module = LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        );
        if module.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let Some(proc) = GetProcAddress(module, windows_sys::s!("DesktopFilterProbeHook")) else {
            windows_sys::Win32::Foundation::FreeLibrary(module);
            return Err("Missing probe export".into());
        };
        let callback = std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "system" fn(i32, usize, isize) -> isize,
        >(proc);
        let mut pid = 0;
        let thread = GetWindowThreadProcessId(list, &raw mut pid);
        if std::env::args().any(|arg| arg == "--isolated-menu" || arg == "--identity-menu") {
            println!("allow_explorer_foreground={}", AllowSetForegroundWindow(pid));
        }
        let hook = SetWindowsHookExW(WH_GETMESSAGE, Some(callback), module, thread);
        if hook.is_null() {
            windows_sys::Win32::Foundation::FreeLibrary(module);
            return Err(std::io::Error::last_os_error().into());
        }
        let message = RegisterWindowMessageW(windows_sys::w!("LucidDesk.DesktopFilterProbe.v1"));
        let log = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/desktop-filter-inprocess.log"
        );
        if let Err(error) = std::fs::remove_file(log)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            UnhookWindowsHookEx(hook);
            windows_sys::Win32::Foundation::FreeLibrary(module);
            return Err(error.into());
        }
        let command = if std::env::args().any(|arg| arg == "--visible-shell-original-loop") {
            AllowSetForegroundWindow(pid);
            8
        } else if std::env::args().any(|arg| arg == "--visible-shell-desktop-sta") {
            AllowSetForegroundWindow(pid);
            7
        } else if std::env::args().any(|arg| arg == "--visible-shell") {
            AllowSetForegroundWindow(pid);
            6
        } else if std::env::args().any(|arg| arg == "--presenter-menu") {
            AllowSetForegroundWindow(pid);
            5
        } else if std::env::args().any(|arg| arg == "--identity-menu") {
            4
        } else if std::env::args().any(|arg| arg == "--isolated-menu") {
            3
        } else if std::env::args().any(|arg| arg == "--remove-refresh") {
            2
        } else if std::env::args().any(|arg| arg == "--remove-restore") {
            1
        } else {
            0
        };
        let status = PostMessageW(list, message, 0x4c504650, command);
        let mut report = None;
        if status != 0 {
            for _ in 0..200 {
                std::thread::sleep(std::time::Duration::from_millis(50));
                if let Ok(text) = std::fs::read_to_string(log)
                    && text.ends_with("\nDONE\n")
                {
                    report = Some(text);
                    break;
                }
            }
        }
        UnhookWindowsHookEx(hook);
        windows_sys::Win32::Foundation::FreeLibrary(module);
        if status == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let report = report.ok_or("Probe did not finish within 10 seconds")?;
        println!("{report}");
        if report.starts_with("error=")
            || report.starts_with("panic caught")
            || report.contains("transaction_error=")
        {
            return Err(
                "In-process probe failed; see report above and recovery log if present".into(),
            );
        }
    }
    Ok(())
}
