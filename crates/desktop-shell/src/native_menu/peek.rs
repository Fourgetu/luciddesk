//! Use Peek's resident Shell entry point, preserving display names and virtual items.
use super::*;
use std::time::{Duration, Instant};
use windows::Win32::{Foundation::E_FAIL, UI::Shell::SIGDN_NORMALDISPLAY};
use windows::core::Error;
use windows_sys::Win32::{
    Foundation::{CloseHandle as CloseRawHandle, HANDLE},
    System::Threading::{
        EVENT_MODIFY_STATE, OpenEventW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW, SetEvent,
    },
    UI::WindowsAndMessaging::*,
};

struct EventHandle(HANDLE);
impl Drop for EventHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseRawHandle(self.0) };
    }
}

fn peek_has_item(name: &str) -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &raw mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut path = [0u16; 32768];
        let mut length = path.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &raw mut length);
        CloseRawHandle(process);
        if ok == 0
            || !String::from_utf16_lossy(&path[..length as usize])
                .to_ascii_lowercase()
                .ends_with("\\powertoys.peek.ui.exe")
        {
            return false;
        }
        let mut title = [0u16; 1024];
        let length = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
        let title = String::from_utf16_lossy(&title[..length.max(0) as usize]);
        // Peek sets its title after the synchronous Shell-item query. Activation
        // alone happens before that query on first launch and is not sufficient.
        title == name || title.starts_with(&format!("{name} - "))
    }
}

/// Preview a desktop namespace member through PowerToys' running Peek service.
/// Caller must permit temporary hidden desktop selection and release model borrows.
/// # Errors
/// Fails if Peek is unavailable, the item cannot be resolved, or handoff times out.
pub fn peek_desktop_item(owner: HWND, identity: &ShellIdentity) -> Result<()> {
    let _active = ActiveMenu::acquire()?;
    unsafe {
        let signal = EventHandle(OpenEventW(
            EVENT_MODIFY_STATE,
            0,
            windows_sys::w!("Local\\ShowPeekEvent"),
        ));
        if signal.0.is_null() {
            return Err(Error::new(E_FAIL, "请启动 PowerToys 并启用 Peek（速览）"));
        }
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
        let folder: IFolderView2 = view.cast()?;
        let index = selection::resolve(&folder, &identity.activation_name().to_string_lossy())?;
        let item = selection::item_at(&folder, index)?;
        let name = crate::namespace::shell_item_name(&item, SIGDN_NORMALDISPLAY)
            .map_err(|e| Error::new(E_FAIL, e.to_string()))?;
        let hwnd = view.GetWindow()?.0;
        let restore = selection::RestoreSelection::deselect_on_close(&folder);
        let _focus = ReturnFocus {
            owner,
            desktop: HWND(GetAncestor(hwnd, GA_ROOT)),
        };
        folder.SelectItem(
            index,
            (SVSI_SELECT.0 | SVSI_FOCUSED.0 | SVSI_DESELECTOTHERS.0) as u32,
        )?;
        // Refuse to wake Peek with a stale/different desktop selection.
        let selected: windows::Win32::UI::Shell::IShellItemArray = folder.Items(SVGIO_SELECTION)?;
        if selected.GetCount()? != 1 || !selection::matches(&selected.GetItemAt(0)?, &item)? {
            return Err(Error::new(E_FAIL, "无法将选中项目交给 Peek"));
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &raw mut pid);
        if AllowSetForegroundWindow(pid) == 0
            || SetForegroundWindow(GetAncestor(hwnd, GA_ROOT)) == 0
        {
            return Err(Error::new(E_FAIL, "无法切换到桌面预览入口"));
        }
        view.UIActivate(SVUIA_ACTIVATE_FOCUS.0 as u32)?;
        if SetEvent(signal.0) == 0 {
            return Err(Error::from_thread());
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if peek_has_item(&name) {
                return restore.finish();
            }
            let mut message = MSG::default();
            while PeekMessageW(&raw mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT {
                    PostQuitMessage(message.wParam as i32);
                    return Err(Error::new(E_FAIL, "预览已取消"));
                }
                TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
            MsgWaitForMultipleObjectsEx(0, std::ptr::null(), 25, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
        Err(Error::new(
            E_FAIL,
            "Peek 未读取到选中项目，请确认 PowerToys 已运行且 Peek 已启用",
        ))
    }
}
