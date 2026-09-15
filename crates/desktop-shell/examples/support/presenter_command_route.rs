//! Complete the host-side command dispatch omitted by IExplorerBrowser.
//! This is confined to the opt-in private-presenter prototype.
use windows::core::{IUnknown, Interface, Result};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::{Shell::*, WindowsAndMessaging::*},
};

const SUBCLASS: usize = 0x4c504352;
pub struct CommandRoute {
    hwnd: HWND,
    _state: Box<State>,
}
struct State {
    message: u32,
    presenter: IUnknown,
}
impl CommandRoute {
    pub unsafe fn attach(hwnd: HWND, presenter: &IUnknown) -> Result<Self> {
        unsafe {
            let message =
                RegisterWindowMessageW(windows_sys::w!("FILE_EXPLORER_CONTEXTMENU_INVOKEMENUITEM"));
            if message == 0 {
                return Err(windows::core::Error::from_thread());
            }
            let state = Box::new(State {
                message,
                presenter: presenter.clone(),
            });
            if SetWindowSubclass(
                hwnd,
                Some(dispatch),
                SUBCLASS,
                (&*state as *const State) as usize,
            ) == 0
            {
                return Err(windows::core::Error::from_thread());
            }
            super::log(format_args!(
                "command_route.attached hwnd={hwnd:?} message={message:#x}"
            ));
            Ok(Self {
                hwnd,
                _state: state,
            })
        }
    }
}
impl Drop for CommandRoute {
    fn drop(&mut self) {
        unsafe {
            RemoveWindowSubclass(self.hwnd, Some(dispatch), SUBCLASS);
        }
    }
}
unsafe extern "system" fn dispatch(
    hwnd: HWND,
    message: u32,
    wp: usize,
    lp: isize,
    id: usize,
    data: usize,
) -> isize {
    unsafe {
        let state = &*(data as *const State);
        if message == state.message {
            // Native CDesktopBrowser dispatches this registered message to
            // IContextMenuPresenter::Invoke(UINT), vtable slot 8. The independent
            // view has no CDesktopBrowser to perform that final host step.
            // Clone before invoking: Shell callbacks may reenter or close us.
            let presenter = state.presenter.clone();
            type Invoke = unsafe extern "system" fn(*mut std::ffi::c_void, u32);
            let table = *presenter.as_raw().cast::<*const usize>();
            let invoke: Invoke = std::mem::transmute(*table.add(8));
            super::log(format_args!(
                "command_route.invoke hwnd={hwnd:?} command={wp:#x}"
            ));
            invoke(presenter.as_raw(), wp as u32);
            super::log(format_args!("command_route.return command={wp:#x}"));
            return 0;
        }
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(dispatch), id);
        }
        DefSubclassProc(hwnd, message, wp, lp)
    }
}
