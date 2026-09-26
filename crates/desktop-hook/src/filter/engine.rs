//! Membership and all COM calls stay on Explorer's desktop STA.
use super::*;
use std::{
    cell::{Cell, RefCell},
    ptr::null_mut,
};
use windows::{
    Win32::{
        System::{
            Com::{CLSCTX_ALL, CoCreateInstance, IServiceProvider},
            Variant::VARIANT,
        },
        UI::Shell::*,
    },
    core::{Interface, Result},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HWND},
    System::{DataExchange::COPYDATASTRUCT, LibraryLoader::*, Threading::*},
    UI::{
        Controls::*,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
    },
};

const TIMER: usize = MAGIC;
thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }
// COM calls can synchronously re-enter the subclass while STATE is borrowed.
// Keep the redraw gate separate so Explorer cannot unfreeze the restored items.
thread_local! { static FROZEN_VIEW: Cell<HWND> = const { Cell::new(null_mut()) }; }
struct State {
    hwnd: HWND,
    owner: HWND,
    process: HANDLE,
    view: IShellView,
    folder: IFolderView2,
    legacy: IShellFolderView,
    membership: items::Membership,
    menu_host: Option<menu::worker::Worker>,
    menu_prepared: bool,
    pending: Option<wire::Request>,
    paused: bool,
    redraw_paused: bool,
    updating: bool,
    update_sequence: u32,
    queued: bool,
    failed: bool,
    read_retry: retry::ReadRetry,
    drag_start: Option<windows_sys::Win32::Foundation::POINT>,
    user_menu: bool,
}
impl Drop for State {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.process);
        }
    }
}

pub fn attach(hwnd: HWND, owner: HWND) {
    let result = attach_inner(hwnd, owner);
    if let Err(error) = result {
        unsafe {
            SetPropW(hwnd, ERROR, error.code().0 as u32 as usize as _);
        }
    }
}
fn attach_inner(hwnd: HWND, owner: HWND) -> Result<()> {
    unsafe {
        if IsWindow(owner) == 0
            || GetWindowThreadProcessId(hwnd, null_mut()) != GetCurrentThreadId()
        {
            return Err(windows::core::Error::from_thread());
        }
        if STATE.with(|slot| slot.borrow().is_some()) {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_UNEXPECTED,
            ));
        }
        let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        let mut raw = 0;
        let dispatch = shell.FindWindowSW(
            &VARIANT::from(CSIDL_DESKTOP.cast_signed()),
            &VARIANT::default(),
            SWC_DESKTOP,
            &raw mut raw,
            SWFO_NEEDDISPATCH,
        )?;
        let provider: IServiceProvider = dispatch.cast()?;
        let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
        let view = browser.QueryActiveShellView()?;
        if GetParent(hwnd) != view.GetWindow()?.0 {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_INVALIDARG,
            ));
        }
        let folder: IFolderView2 = view.cast()?;
        let legacy: IShellFolderView = view.cast()?;
        if legacy.GetObjectCount()? as i32 != folder.ItemCount(SVGIO_ALLVIEW)? {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_FAIL,
            ));
        }
        let mut pid = 0;
        GetWindowThreadProcessId(owner, &raw mut pid);
        let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if process.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        let mut module = null_mut();
        if GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            (LucidPaneFilterHook as *const ()).cast(),
            &raw mut module,
        ) == 0
        {
            CloseHandle(process);
            return Err(windows::core::Error::from_thread());
        }
        let state = State {
            hwnd,
            owner,
            process,
            view,
            folder,
            legacy,
            membership: Default::default(),
            menu_host: None,
            menu_prepared: false,
            pending: None,
            paused: false,
            redraw_paused: false,
            updating: false,
            update_sequence: 0,
            queued: false,
            failed: false,
            read_retry: Default::default(),
            drag_start: None,
            user_menu: false,
        };
        if SetWindowSubclass(hwnd, Some(subclass), MAGIC, 0) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        if SetWindowSubclass(GetParent(hwnd), Some(subclass), MAGIC, 0) == 0 {
            RemoveWindowSubclass(hwnd, Some(subclass), MAGIC);
            return Err(windows::core::Error::from_thread());
        }
        if SetTimer(hwnd, TIMER, 1000, None) == 0 {
            RemoveWindowSubclass(GetParent(hwnd), Some(subclass), MAGIC);
            RemoveWindowSubclass(hwnd, Some(subclass), MAGIC);
            return Err(windows::core::Error::from_thread());
        }
        RemovePropW(hwnd, ERROR);
        RemovePropW(hwnd, ACK);
        STATE.with(|slot| *slot.borrow_mut() = Some(state));
        SetPropW(hwnd, OWNER, owner);
    }
    Ok(())
}

impl State {
    fn queue(&mut self) {
        if !self.queued {
            self.queued = unsafe { PostMessageW(self.hwnd, work_message(), 0, 0) != 0 };
        }
    }
    fn redraw(&mut self, enabled: bool) {
        set_redraw(self.hwnd, &mut self.redraw_paused, enabled);
    }
    fn restore(&mut self) {
        if self.membership.restore(&self.folder, &self.legacy).is_err() {
            // A refresh asks the real data source to rebuild membership. It does
            // not invent missing filesystem items or depend on our cached PIDLs.
            let _ = unsafe { self.view.Refresh() };
        }
        self.redraw(true);
    }
    fn apply_request(
        &mut self,
        request: &wire::Request,
    ) -> std::result::Result<(), items::ApplyError> {
        match request.op {
            wire::REPLACE_IDENTITY => {
                if !self.updating {
                    return Err(windows::Win32::Foundation::E_UNEXPECTED.into());
                }
                self.membership
                    .replace_identity(&request.names[0], &request.names[1])?;
                // Metadata handoff is atomic; the subsequent SET/UPDATE_END
                // applies membership with its own read/write error boundary.
                return Ok(());
            }
            wire::UPDATE_BEGIN => {
                if self.updating || self.paused || self.menu_prepared {
                    return Err(windows::Win32::Foundation::E_UNEXPECTED.into());
                }
                self.update_sequence = request.sequence;
                if unsafe { GetPropW(self.hwnd, UPDATE_RELEASE) as usize }
                    == request.sequence as usize
                {
                    unsafe {
                        SetPropW(self.hwnd, UPDATE_RELEASED, request.sequence as usize as _);
                    }
                    return Err(windows::Win32::Foundation::E_ABORT.into());
                }
                self.updating = true;
                self.redraw(false);
                return Ok(());
            }
            wire::UPDATE_END => {
                if !self.updating {
                    return Ok(());
                }
                self.updating = false;
                let result = self
                    .membership
                    .apply(&self.folder, &self.legacy, self.paused);
                self.redraw(true);
                unsafe {
                    SetPropW(
                        self.hwnd,
                        UPDATE_RELEASED,
                        self.update_sequence as usize as _,
                    );
                }
                return result;
            }
            wire::MENU_PREPARE => {
                unsafe {
                    let context = request
                        .menu
                        .ok_or(windows::Win32::Foundation::E_INVALIDARG)?;
                    let mut requested_pid = 0;
                    let mut owner_pid = 0;
                    GetWindowThreadProcessId(context.owner as _, &raw mut requested_pid);
                    GetWindowThreadProcessId(self.owner, &raw mut owner_pid);
                    if requested_pid == 0 || requested_pid != owner_pid || self.menu_prepared {
                        return Err(windows::Win32::Foundation::E_INVALIDARG.into());
                    }
                    RemovePropW(self.hwnd, RENAME);
                    RemovePropW(self.hwnd, MENU_HOST);
                    if !self
                        .menu_host
                        .as_ref()
                        .is_some_and(menu::worker::Worker::is_alive)
                    {
                        self.menu_host = Some(menu::worker::Worker::create(self.hwnd)?);
                    }
                    let hwnd = self
                        .menu_host
                        .as_ref()
                        .ok_or(windows::Win32::Foundation::E_FAIL)?
                        .prepare(&request.names, context)?;
                    self.menu_prepared = true;
                    SetPropW(self.hwnd, MENU_HOST, hwnd as _);
                }
                return Ok(());
            }
            wire::MENU_FINISH | wire::MENU_CANCEL => {
                self.menu_prepared = false;
                unsafe {
                    RemovePropW(self.hwnd, MENU_HOST);
                }
                if request.op == wire::MENU_CANCEL {
                    // Cancellation may follow a stuck WinUI popup. Retire its
                    // STA as well as the presenter; never reuse that UI state.
                    // Worker drop signals shutdown without joining Explorer.
                    if let Some(host) = self.menu_host.take() {
                        host.finish(true)?;
                    }
                } else if let Some(host) = &self.menu_host {
                    host.finish(false)?;
                }
                return Ok(());
            }
            wire::SET => {
                self.membership.desired =
                    request.names.iter().map(|name| items::key(name)).collect();
            }
            wire::CLEAR_SELECTION => {
                unsafe {
                    let value = LVITEMW {
                        state: 0,
                        stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                        ..Default::default()
                    };
                    SendMessageW(
                        self.hwnd,
                        LVM_SETITEMSTATE,
                        usize::MAX,
                        (&raw const value) as isize,
                    );
                }
                return Ok(());
            }
            wire::PAUSE => {
                unsafe {
                    RemovePropW(self.hwnd, RENAME);
                }
                self.redraw(false);
                self.paused = true;
            }
            wire::RESUME => {
                self.paused = false;
            }
            _ => {}
        }
        let result = self
            .membership
            .apply(&self.folder, &self.legacy, self.paused);
        if !self.paused && !self.updating {
            self.redraw(true);
        }
        result
    }
    fn apply_result(
        &mut self,
        result: std::result::Result<(), items::ApplyError>,
    ) -> Option<items::ApplyError> {
        match result {
            Ok(()) => {
                self.read_retry.recovered();
                None
            }
            Err(error) => {
                if !self
                    .read_retry
                    .failed(error.before_write, std::time::Instant::now())
                {
                    self.restore();
                    self.failed = true;
                    unsafe {
                        SetPropW(self.hwnd, ERROR, error.code().0 as u32 as usize as _);
                    }
                }
                Some(error)
            }
        }
    }
    fn cleanup(&mut self) {
        self.menu_host.take();
        self.restore();
        remove_registration(self.hwnd);
    }
}

fn remove_registration(hwnd: HWND) {
    unsafe {
        KillTimer(hwnd, TIMER);
        RemoveWindowSubclass(GetParent(hwnd), Some(subclass), MAGIC);
        RemoveWindowSubclass(hwnd, Some(subclass), MAGIC);
        for property in [
            OWNER,
            ACK,
            ERROR,
            RENAME,
            MENU_HOST,
            MENU_ERROR,
            UPDATE_RELEASE,
            UPDATE_RELEASED,
            REQUEST_ERROR,
        ] {
            RemovePropW(hwnd, property);
        }
    }
}

fn set_redraw(hwnd: HWND, paused: &mut bool, enabled: bool) {
    unsafe {
        if enabled && *paused {
            FROZEN_VIEW.with(|view| view.set(null_mut()));
            SendMessageW(hwnd, WM_SETREDRAW, 1, 0);
            windows_sys::Win32::Graphics::Gdi::RedrawWindow(
                hwnd,
                std::ptr::null(),
                null_mut(),
                windows_sys::Win32::Graphics::Gdi::RDW_INVALIDATE
                    | windows_sys::Win32::Graphics::Gdi::RDW_ALLCHILDREN,
            );
            *paused = false;
        } else if !enabled && !*paused {
            FROZEN_VIEW.with(|view| view.set(hwnd));
            SendMessageW(hwnd, WM_SETREDRAW, 0, 0);
            *paused = true;
        }
    }
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    _: usize,
) -> isize {
    let frozen = FROZEN_VIEW.with(Cell::get);
    if !frozen.is_null() && msg == WM_NOTIFY && lp != 0 {
        let header = unsafe { &*(lp as *const NMHDR) };
        if header.hwndFrom == frozen && header.code == NM_CUSTOMDRAW {
            let draw = unsafe { &*(lp as *const NMCUSTOMDRAW) };
            if draw.dwDrawStage == CDDS_PREPAINT {
                return CDRF_SKIPDEFAULT as isize;
            }
        }
    }
    // AddObject, selection and menu activation can each enable redraw again.
    // Run before the RefCell guard, including during those nested COM calls.
    if frozen == hwnd {
        match msg {
            WM_SETREDRAW if wp != 0 => return 0,
            WM_PAINT => {
                unsafe {
                    windows_sys::Win32::Graphics::Gdi::ValidateRect(hwnd, std::ptr::null());
                }
                return 0;
            }
            WM_ERASEBKGND => return 1,
            WM_PRINTCLIENT => return 0,
            WM_NCDESTROY => FROZEN_VIEW.with(|view| view.set(null_mut())),
            _ => {}
        }
    }
    let handled = std::panic::catch_unwind(|| {
        STATE.with(|slot| {
            let Ok(mut slot) = slot.try_borrow_mut() else {
                return None;
            };
            let state = slot.as_mut()?;
            if hwnd != state.hwnd {
                if msg == WM_COMMAND && !state.paused && state.user_menu {
                    state.membership.user_changed_layout();
                    state.user_menu = false;
                }
                if msg == WM_NOTIFY && lp != 0 && state.paused {
                    let header = unsafe { &*(lp as *const NMHDR) };
                    if header.hwndFrom == state.hwnd
                        && matches!(header.code, LVN_BEGINLABELEDITA | LVN_BEGINLABELEDITW)
                    {
                        // Explorer's rename command belongs in the pane editor, not
                        // in the temporarily restored native view underneath it.
                        unsafe {
                            SetPropW(state.hwnd, RENAME, 1usize as _);
                        }
                        return Some(1);
                    }
                }
                return None;
            }
            if msg == WM_NCDESTROY {
                // The dying view cannot accept COM updates; its replacement enumerates
                // the original data source. No custom membership survives that rebuild.
                remove_registration(hwnd);
                slot.take();
                return None;
            }
            // Shell rename notifications may arrive after UPDATE_END. Process
            // queued membership changes before painting their newly inserted row.
            if msg == WM_PAINT
                && (state.queued || state.read_retry.pending())
                && !state.paused
                && !state.updating
                && !state.failed
            {
                if state.read_retry.waiting() {
                    unsafe {
                        windows_sys::Win32::Graphics::Gdi::ValidateRect(hwnd, std::ptr::null());
                    }
                    return Some(0);
                }
                // Reading an unchanged view must not invalidate the whole desktop.
                // Freeze only immediately before a native write, while retaining
                // the reentrant paint gate for AddObject/RemoveObject callbacks.
                // COM reads can also pump messages: keep newly inserted rows
                // from painting during enumeration without toggling WM_SETREDRAW.
                FROZEN_VIEW.with(|view| view.set(hwnd));
                let paused = &mut state.redraw_paused;
                let result = state.membership.apply_before_write(
                    &state.folder,
                    &state.legacy,
                    false,
                    || set_redraw(hwnd, paused, false),
                );
                state.redraw(true);
                FROZEN_VIEW.with(|view| view.set(null_mut()));
                if state.apply_result(result).is_some() && !state.failed {
                    unsafe {
                        windows_sys::Win32::Graphics::Gdi::ValidateRect(hwnd, std::ptr::null());
                    }
                    return Some(0);
                }
            }
            if msg == WM_COPYDATA && wp as HWND == state.owner && lp != 0 {
                let data = unsafe { &*(lp as *const COPYDATASTRUCT) };
                if data.dwData != MAGIC {
                    return None;
                }
                if data.lpData.is_null()
                    || data.cbData as usize > wire::MAX_BYTES
                    || state.pending.is_some()
                {
                    return Some(0);
                }
                let bytes = unsafe {
                    std::slice::from_raw_parts(data.lpData.cast::<u8>(), data.cbData as usize)
                };
                let Some(request) = wire::decode(bytes) else {
                    return Some(0);
                };
                state.pending = Some(request);
                state.queue();
                return Some(MAGIC as isize);
            }
            if msg == work_message() || (msg == WM_TIMER && wp == TIMER) {
                state.queued = false;
                if unsafe {
                    IsWindow(state.owner) == 0 || WaitForSingleObject(state.process, 0) == 0
                } {
                    state.cleanup();
                    slot.take();
                    return Some(0);
                }
                if state.updating
                    && unsafe { GetPropW(hwnd, UPDATE_RELEASE) as usize }
                        == state.update_sequence as usize
                {
                    // Do not restore all members on a cleanup/read error. Normal
                    // filtering will retry; presentation must always be released.
                    state.updating = false;
                    let result = state.membership.apply(&state.folder, &state.legacy, false);
                    state.apply_result(result);
                    state.redraw(true);
                    unsafe {
                        SetPropW(hwnd, UPDATE_RELEASED, state.update_sequence as usize as _);
                    }
                }
                let request = state.pending.take();
                if request.is_some() {
                    unsafe {
                        RemovePropW(hwnd, REQUEST_ERROR);
                    }
                }
                if request.is_none() && state.read_retry.waiting() {
                    return Some(0);
                }
                if request
                    .as_ref()
                    .is_some_and(|request| request.op == wire::DETACH)
                {
                    state.cleanup();
                    slot.take();
                    return Some(0);
                }
                if !state.failed {
                    if let Some(request) = &request {
                        if wire::is_menu_transaction(request.op) {
                            unsafe {
                                RemovePropW(hwnd, MENU_ERROR);
                            }
                            if let Err(error) = state.apply_request(request) {
                                unsafe {
                                    SetPropW(hwnd, MENU_ERROR, error.code().0 as u32 as usize as _);
                                }
                            }
                            unsafe {
                                SetPropW(hwnd, ACK, request.sequence as usize as _);
                            }
                            // A menu error must never restore desktop membership.
                            return Some(0);
                        }
                    }
                    let result = if let Some(request) = &request {
                        state.apply_request(request)
                    } else if state.paused {
                        Ok(())
                    } else {
                        state.membership.apply(&state.folder, &state.legacy, false)
                    };
                    if let Some(error) = state.apply_result(result) {
                        if request.is_some() {
                            unsafe {
                                SetPropW(hwnd, REQUEST_ERROR, error.code().0 as u32 as usize as _);
                            }
                        }
                    }
                }
                if let Some(request) = request {
                    unsafe {
                        SetPropW(hwnd, ACK, request.sequence as usize as _);
                    }
                }
                return Some(0);
            }
            if !state.paused
                && matches!(
                    msg,
                    WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_KEYDOWN
                )
            {
                unsafe {
                    PostMessageW(
                        state.owner,
                        crate::notifications::DESKTOP_INPUT_MESSAGE,
                        hwnd as usize,
                        GetMessageTime() as isize,
                    );
                }
            }
            if !state.paused && msg == WM_RBUTTONDOWN {
                state.user_menu = true;
            }
            if !state.paused && msg == WM_LBUTTONDOWN {
                let mut point = windows_sys::Win32::Foundation::POINT::default();
                unsafe {
                    GetCursorPos(&raw mut point);
                }
                state.drag_start = Some(point);
            }
            if !state.paused && matches!(msg, WM_LBUTTONUP | WM_CAPTURECHANGED) {
                if let Some(start) = state.drag_start.take() {
                    let mut end = windows_sys::Win32::Foundation::POINT::default();
                    unsafe {
                        GetCursorPos(&raw mut end);
                    }
                    if start.x.abs_diff(end.x) > 4 || start.y.abs_diff(end.y) > 4 {
                        state.membership.user_changed_layout();
                    }
                }
            }
            if matches!(msg, WM_DISPLAYCHANGE | WM_DPICHANGED) {
                state.membership.user_changed_layout();
            }
            if matches!(
                msg,
                LVM_SETITEMCOUNT
                    | LVM_INSERTITEMW
                    | LVM_DELETEITEM
                    | LVM_DELETEALLITEMS
                    | LVM_SORTITEMS
                    | LVM_SORTITEMSEX
                    | WM_DISPLAYCHANGE
                    | WM_SETTINGCHANGE
            ) {
                state.queue();
            }
            None
        })
    })
    .ok()
    .flatten();
    handled.unwrap_or_else(|| unsafe { DefSubclassProc(hwnd, msg, wp, lp) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_redraw_gate_survives_reentrant_calls_and_releases() {
        unsafe {
            // Use DefWindowProc's observable SysSetRedraw flag. Common controls
            // have their own redraw state and need not expose that flag.
            let class = windows_sys::w!("LucidPaneRedrawGateTest");
            let instance = GetModuleHandleW(std::ptr::null());
            let definition = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: instance,
                lpszClassName: class,
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&definition), 0);
            let hwnd = CreateWindowExW(
                0,
                class,
                windows_sys::w!(""),
                WS_POPUP | WS_VISIBLE,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                instance,
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            assert_ne!(SetWindowSubclass(hwnd, Some(subclass), MAGIC, 0), 0);
            windows_sys::Win32::Graphics::Gdi::ValidateRect(hwnd, std::ptr::null());
            let mut paused = false;
            set_redraw(hwnd, &mut paused, true);
            assert_eq!(
                windows_sys::Win32::Graphics::Gdi::GetUpdateRect(hwnd, std::ptr::null_mut(), 0),
                0,
                "a read-only membership check must not invalidate the desktop"
            );
            set_redraw(hwnd, &mut paused, false);
            let disabled = || !GetPropW(hwnd, windows_sys::w!("SysSetRedraw")).is_null();
            assert!(disabled());
            FROZEN_VIEW.with(|view| view.set(hwnd));
            // Match synchronous callbacks made while State::apply_request owns
            // the mutable state borrow, then the later menu message loop.
            STATE.with(|slot| {
                let _borrow = slot.borrow_mut();
                SendMessageW(hwnd, WM_SETREDRAW, 1, 0);
                assert!(disabled());
                let draw = NMCUSTOMDRAW {
                    hdr: NMHDR {
                        hwndFrom: hwnd,
                        code: NM_CUSTOMDRAW,
                        ..Default::default()
                    },
                    dwDrawStage: CDDS_PREPAINT,
                    ..Default::default()
                };
                assert_eq!(
                    SendMessageW(hwnd, WM_NOTIFY, 0, (&raw const draw) as isize),
                    CDRF_SKIPDEFAULT as isize
                );
            });
            SendMessageW(hwnd, WM_SETREDRAW, 1, 0);
            assert!(disabled());
            set_redraw(hwnd, &mut paused, true);
            assert!(!disabled());
            assert!(!paused);
            assert_ne!(
                windows_sys::Win32::Graphics::Gdi::GetUpdateRect(hwnd, std::ptr::null_mut(), 0),
                0,
                "membership writes still require a repaint when released"
            );
            RemoveWindowSubclass(hwnd, Some(subclass), MAGIC);
            DestroyWindow(hwnd);
            UnregisterClassW(class, instance);
        }
    }
}
