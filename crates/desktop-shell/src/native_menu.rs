//! Explorer-hosted desktop item menus. Call on the UI STA in response to user input.
mod lifetime;
mod selection;

use desktop_core::ShellIdentity;
use std::cell::Cell;
use windows::Win32::Foundation::{ERROR_BUSY, POINT};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, IServiceProvider};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    CMF_ITEMMENU, CSIDL_DESKTOP, IContextMenu, IContextMenuSite, IFolderView2, IShellBrowser,
    IShellWindows, SID_STopLevelBrowser, SVGIO_SELECTION, SVSI_DESELECTOTHERS, SVSI_FOCUSED,
    SVSI_SELECT, SVUIA_ACTIVATE_FOCUS, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
};
use windows::core::{Interface, Result};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GA_ROOT, GetAncestor, GetWindowThreadProcessId, SetForegroundWindow,
};

thread_local! { static ACTIVE: Cell<bool> = const { Cell::new(false) }; }
struct ActiveMenu;
impl ActiveMenu {
    fn acquire() -> Result<Self> {
        if ACTIVE.with(|active| active.replace(true)) {
            return Err(windows::core::Error::from_hresult(
                windows::core::HRESULT::from_win32(ERROR_BUSY.0),
            ));
        }
        Ok(Self)
    }
}
impl Drop for ActiveMenu {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.set(false));
    }
}

/// Opens the real Explorer menu for a desktop item at physical screen coordinates.
/// Pumps UI messages until dismissal; callers must release model borrows first.
/// A missing desktop item is an error, never a request for a background menu.
///
/// # Errors
/// Returns an error when the Shell host, target, focus handoff or popup observation
/// is unavailable, another menu is active, or selection restoration fails.
pub fn show_desktop_item_menu(identity: &ShellIdentity, point: POINT) -> Result<()> {
    let _active = ActiveMenu::acquire()?;
    let name = match identity {
        ShellIdentity::FileSystem { path, .. } => path.to_string_lossy().into_owned(),
        ShellIdentity::Namespace { parsing_name } => parsing_name.clone(),
    };
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
        let folder: IFolderView2 = view.cast()?;
        let index = selection::resolve(&folder, &name)?;
        let site: IContextMenuSite = view.cast()?;
        let hwnd = view.GetWindow()?.0;
        let mut pid = 0;
        let thread = GetWindowThreadProcessId(hwnd, &raw mut pid);
        if pid == 0 || thread == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let observer = lifetime::Observer::new(pid, thread)?;
        let restore = selection::RestoreSelection::capture(&folder)?;
        if AllowSetForegroundWindow(pid) == 0
            || SetForegroundWindow(GetAncestor(hwnd, GA_ROOT)) == 0
        {
            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "Explorer menu focus handoff failed",
            ));
        }
        view.UIActivate(SVUIA_ACTIVATE_FOCUS.0.cast_unsigned())?;
        folder.SelectItem(
            index,
            (SVSI_SELECT.0 | SVSI_FOCUSED.0 | SVSI_DESELECTOTHERS.0).cast_unsigned(),
        )?;
        let menu: IContextMenu = view.GetItemObject(SVGIO_SELECTION)?;
        let result = site
            .DoContextMenuPopup(&menu, CMF_ITEMMENU, point)
            .and_then(|()| observer.wait_for_close());
        let restored = restore.finish();
        result.and(restored)
    }
}
