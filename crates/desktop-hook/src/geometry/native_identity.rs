//! Cached, in-apartment desktop identity access. Never retain a COM proxy in paint.
use windows::{core::Interface, Win32::{
    System::{Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, IServiceProvider}, Variant::VARIANT},
    UI::Shell::{IShellWindows, ShellWindows, CSIDL_DESKTOP, SWC_DESKTOP, SWFO_NEEDDISPATCH,
        IShellBrowser, SID_STopLevelBrowser, IFolderView2, ILGetSize},
}};
use windows_sys::Win32::{Foundation::HWND, System::LibraryLoader::{
    GetModuleHandleExW, GetModuleFileNameW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
}, UI::WindowsAndMessaging::{IsChild, GetWindowThreadProcessId}};

#[derive(Clone)]
pub(super) struct View(IFolderView2);
impl View {
    pub(super) fn connect(view: HWND) -> windows::core::Result<Self> {
        unsafe {
            let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
            let mut hwnd = 0;
            let dispatch = shell.FindWindowSW(&VARIANT::from(CSIDL_DESKTOP.cast_signed()),
                &VARIANT::default(), SWC_DESKTOP, &raw mut hwnd, SWFO_NEEDDISPATCH)?;
            let provider: IServiceProvider = dispatch.cast()?;
            let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
            let shell_view = browser.QueryActiveShellView()?;
            let parent = shell_view.GetWindow()?;
            let folder: IFolderView2 = shell_view.cast()?;
            let mut module = std::ptr::null_mut();
            let method = folder.vtable().base__.Item as *const u16;
            let mut path = [0u16; 1024];
            let found = GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
                GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, method, &raw mut module);
            let len = if found != 0 { GetModuleFileNameW(module, path.as_mut_ptr(), 1024) } else { 0 };
            let path = String::from_utf16_lossy(&path[..len as usize]).to_ascii_lowercase();
            diagnostic(&format!("Item implementation={path}"));
            // Accept only the Shell implementation, never combase/RPC proxy stubs.
            if IsChild(parent.0, view) == 0 ||
                GetWindowThreadProcessId(view, std::ptr::null_mut()) != windows_sys::Win32::System::Threading::GetCurrentThreadId() ||
                !["\\shell32.dll", "\\windows.storage.dll", "\\explorerframe.dll"].iter().any(|suffix| path.ends_with(suffix)) {
                return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_NOINTERFACE));
            }
            Ok(Self(folder))
        }
    }

    pub(super) fn key(&self, index: i32) -> windows::core::Result<Vec<u8>> {
        unsafe {
            let pidl = self.0.Item(index)?;
            if pidl.is_null() { return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL)); }
            let size = ILGetSize(Some(pidl)) as usize;
            let key = std::slice::from_raw_parts(pidl.cast::<u8>(), size).to_vec();
            CoTaskMemFree(Some(pidl.cast()));
            Ok(key)
        }
    }
}

pub(super) fn diagnostic(message: &str) {
    use std::io::Write;
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true)
            .open(std::path::PathBuf::from(local).join("LucidPane/hook-identity.log")) {
            let _ = writeln!(file, "{:?} {message}", std::time::SystemTime::now());
        }
    }
}
