//! Explorer-hosted desktop item menus. Call on the UI STA in response to user input.
mod input;
mod lifetime;
pub use input::MenuInvocation;
mod peek;
mod performance;
mod selection;
pub use peek::peek_desktop_item;
pub(crate) use selection::update_hints;

use desktop_core::ShellIdentity;
use std::cell::Cell;
use windows::Win32::Foundation::{ERROR_BUSY, HWND, POINT};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, IServiceProvider};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    CMF_CANRENAME, CMF_ITEMMENU, CSIDL_DESKTOP, IContextMenu, IContextMenuSite, IFolderView2,
    IShellBrowser, IShellWindows, SID_STopLevelBrowser, SVGIO_SELECTION, SVSI_DESELECTOTHERS,
    SVSI_FOCUSED, SVSI_SELECT, SVUIA_ACTIVATE_FOCUS, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
};
use windows::core::{Interface, Result};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GA_ROOT, GetAncestor, GetForegroundWindow, GetWindowThreadProcessId,
    IsWindowVisible, SetForegroundWindow,
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

struct ReturnFocus {
    owner: HWND,
    desktop: HWND,
}
impl Drop for ReturnFocus {
    fn drop(&mut self) {
        unsafe {
            focus_trace("return-before", self.owner, self.desktop, None);
            // A command may have opened Properties or another app, or the user
            // may have switched away. Never steal focus from that destination.
            if !self.owner.0.is_null()
                && IsWindowVisible(self.owner.0) != 0
                && GetForegroundWindow() == self.desktop.0
            {
                let accepted = SetForegroundWindow(self.owner.0);
                focus_trace("return-attempt", self.owner, self.desktop, Some(accepted));
            }
            focus_trace("return-after", self.owner, self.desktop, None);
        }
    }
}

// Opt-in diagnostics contain window classes/handles only, never window titles
// or filenames. Keep filesystem work out of the Explorer hook and paint paths.
fn focus_trace(stage: &str, owner: HWND, desktop: HWND, accepted: Option<i32>) {
    if std::env::var_os("LUCIDPANE_MENU_TRACE").is_none() {
        return;
    }
    use std::io::Write;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GUITHREADINFO, GetClassNameW, GetGUIThreadInfo,
    };
    let Some(base) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let path = std::path::PathBuf::from(base)
        .join("LucidPane")
        .join("menu-focus.log");
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    unsafe {
        let foreground = GetForegroundWindow();
        let mut name = [0u16; 128];
        let len = GetClassNameW(foreground, name.as_mut_ptr(), 128).max(0) as usize;
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let got = GetGUIThreadInfo(0, &raw mut info);
        let _ = writeln!(
            file,
            "{:?} {stage} owner={:?} desktop={:?} foreground={foreground:?} class={} accepted={accepted:?} gui={got} active={:?} focus={:?}",
            std::time::SystemTime::now(),
            owner.0,
            desktop.0,
            String::from_utf16_lossy(&name[..len]),
            info.hwndActive,
            info.hwndFocus
        );
    }
}

/// Opens the real Explorer menu for a desktop item at physical screen coordinates.
/// Pumps UI messages until dismissal; callers must release model borrows first.
/// A missing desktop item is an error, never a request for a background menu.
///
/// # Errors
/// Returns an error when the Shell host, target, focus handoff or popup observation
/// is unavailable, another menu is active, or selection restoration fails.
pub fn show_desktop_item_menu(
    owner: HWND,
    identity: &ShellIdentity,
    point: POINT,
    invocation: MenuInvocation,
) -> Result<()> {
    show_desktop_items_menu(owner, std::slice::from_ref(identity), point, invocation)
}

/// Opens the Explorer menu for the complete desktop selection.
/// # Errors
/// Returns errors resolving any selected item or showing the Shell menu.
pub fn show_desktop_items_menu(
    owner: HWND,
    identities: &[ShellIdentity],
    point: POINT,
    invocation: MenuInvocation,
) -> Result<()> {
    if identities.is_empty() {
        return Ok(());
    }
    let _active = ActiveMenu::acquire()?;
    let mut timings = performance::Timings::new();
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
        timings.mark("explorer-connected");
        let indices: Vec<_> = identities
            .iter()
            .map(|identity| {
                selection::resolve(&folder, &identity.activation_name().to_string_lossy())
            })
            .collect::<Result<_>>()?;
        timings.mark("target-resolved");
        let site: IContextMenuSite = view.cast()?;
        let hwnd = view.GetWindow()?.0;
        let mut pid = 0;
        let thread = GetWindowThreadProcessId(hwnd, &raw mut pid);
        if pid == 0 || thread == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let observer = lifetime::Observer::new(pid, thread)?;
        timings.mark("observer-ready");
        let restore = if owner.0.is_null() {
            selection::RestoreSelection::capture(&folder)?
        } else {
            selection::RestoreSelection::deselect_on_close(&folder)
        };
        // Drop before RestoreSelection, including on error paths, so restoring
        // the user's selection does not briefly paint an active desktop highlight.
        let return_focus = ReturnFocus {
            owner,
            desktop: HWND(GetAncestor(hwnd, GA_ROOT)),
        };
        focus_trace("target-before", owner, return_focus.desktop, None);
        // Prepare the hidden menu target before activating the desktop, instead
        // of activating its old selection and immediately replacing it.
        for (at, index) in indices.into_iter().enumerate() {
            let flags = SVSI_SELECT.0
                | if at == 0 {
                    SVSI_FOCUSED.0 | SVSI_DESELECTOTHERS.0
                } else {
                    0
                };
            folder.SelectItem(index, flags.cast_unsigned())?;
        }
        timings.mark("target-selected");
        focus_trace("target-after", owner, return_focus.desktop, None);
        if AllowSetForegroundWindow(pid) == 0
            || SetForegroundWindow(GetAncestor(hwnd, GA_ROOT)) == 0
        {
            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "Explorer menu focus handoff failed",
            ));
        }
        view.UIActivate(SVUIA_ACTIVATE_FOCUS.0.cast_unsigned())?;
        timings.mark("focus-ready");
        focus_trace("menu-activated", owner, return_focus.desktop, None);
        timings.mark("context-ready");
        let result = match invocation {
            MenuInvocation::Mouse => input::open_mouse(HWND(hwnd), point),
            MenuInvocation::Keyboard => {
                let menu: IContextMenu = view.GetItemObject(SVGIO_SELECTION)?;
                site.DoContextMenuPopup(&menu, CMF_ITEMMENU | CMF_CANRENAME, point)
            }
        }
        .and_then(|()| observer.wait_for_close());
        if let Some(visible) = observer.first_visible() {
            timings.at("popup-first-observed", visible);
        }
        let desktop = return_focus.desktop;
        focus_trace("menu-closed", owner, desktop, None);
        drop(return_focus);
        let restored = restore.finish();
        focus_trace("selection-restored", owner, desktop, None);
        result.and(restored)
    }
}
