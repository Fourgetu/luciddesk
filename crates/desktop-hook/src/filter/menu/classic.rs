//! Public Shell menu fallback; selection and command ownership stay isolated.
use super::{MenuContext, lifecycle::Invocation};
use std::cell::RefCell;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::{Interface, PCSTR, Result},
};

struct Popup(HMENU);
impl Drop for Popup {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.0);
        }
    }
}
struct Routing<'a>(&'a RefCell<Option<IContextMenu>>);
impl Drop for Routing<'_> {
    fn drop(&mut self) {
        self.0.borrow_mut().take();
    }
}

pub(super) fn show(
    hwnd: windows_sys::Win32::Foundation::HWND,
    menu: &IContextMenu,
    context: MenuContext,
    routing: &RefCell<Option<IContextMenu>>,
    invocation: &Invocation,
) -> Result<()> {
    unsafe {
        invocation.check()?;
        let popup = Popup(CreatePopupMenu()?);
        menu.QueryContextMenu(popup.0, 0, 1, 0x7fff, CMF_NORMAL | CMF_CANRENAME)
            .ok()?;
        invocation.check()?;
        *routing.borrow_mut() = Some(menu.clone());
        let _routing = Routing(routing);
        let command = TrackPopupMenuEx(
            popup.0,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            context.x,
            context.y,
            HWND(hwnd),
            None,
        )
        .0;
        invocation.check()?;
        if command != 0 {
            menu.InvokeCommand(&CMINVOKECOMMANDINFO {
                cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
                hwnd: HWND(hwnd),
                lpVerb: PCSTR((command as usize - 1) as _),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            })?;
        }
        Ok(())
    }
}
pub(super) fn message(menu: &IContextMenu, msg: u32, wp: usize, lp: isize) -> Option<isize> {
    if !matches!(
        msg,
        WM_INITMENUPOPUP | WM_MENUCHAR | WM_DRAWITEM | WM_MEASUREITEM
    ) {
        return None;
    }
    unsafe {
        if let Ok(menu) = menu.cast::<IContextMenu3>() {
            let mut result = LRESULT::default();
            menu.HandleMenuMsg2(msg, WPARAM(wp), LPARAM(lp), Some(&raw mut result))
                .ok()?;
            Some(result.0)
        } else {
            menu.cast::<IContextMenu2>()
                .ok()?
                .HandleMenuMsg(msg, WPARAM(wp), LPARAM(lp))
                .ok()?;
            Some(0)
        }
    }
}
