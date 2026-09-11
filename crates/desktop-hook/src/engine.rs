//! All state is confined to the existing view's UI thread. Reentrant messages pass through.
// Native message indices are checked for negative values and fixed array bounds before casts.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
use crate::protocol::{
    Area, DETACH, MAGIC, MOVE_ITEM, OK, QUERY, QUERY_AREA_COUNT, QUERY_AUTOARRANGE,
    QUERY_GENERATION, QUERY_ITEM_AREA, QUERY_ITEM_COUNT, REJECTED, Request, SET_AREAS, name_hash,
};
use std::cell::RefCell;
use std::mem::size_of;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, POINT};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN, GetModuleHandleExW,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};
use windows_sys::Win32::UI::Controls::{
    LVITEMW, LVM_DELETEALLITEMS, LVM_DELETEITEM, LVM_GETITEMCOUNT, LVM_GETITEMPOSITION,
    LVM_INSERTITEMW, LVM_SETITEMCOUNT, LVM_SETITEMPOSITION, LVM_SETITEMPOSITION32,
    LVM_SETITEMTEXTW, LVM_SORTITEMS, LVS_AUTOARRANGE,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetWindowLongW, GetWindowThreadProcessId, IsWindow, KillTimer, RemovePropW,
    SendMessageW, SetPropW, SetTimer, WM_COPYDATA, WM_DISPLAYCHANGE, WM_KEYDOWN, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_NCDESTROY, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN, WM_TIMER,
};

const SUBCLASS: usize = MAGIC;
const TIMER: usize = MAGIC + 1;
thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }

struct State {
    hwnd: HWND,
    owner: HWND,
    owner_process: HANDLE,
    managed: Vec<Area>,
    changes: u32,
    shell_changes: u32,
    geometry: crate::geometry::GeometrySession,
}

impl Drop for State {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.owner_process);
        }
    }
}

#[allow(clippy::too_many_lines)]
pub fn attach_geometry(hwnd: HWND, owner: HWND, magic: isize) {
    if magic != MAGIC as isize || hwnd.is_null() || owner.is_null() {
        return;
    }
    unsafe {
        SetPropW(
            hwnd,
            windows_sys::w!("LucidPane.Hook.Bootstrap"),
            1_usize as _,
        );
    }
    STATE.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return;
        };
        if slot.is_some() {
            return;
        }
        unsafe {
            let mut pid = 0;
            if GetWindowThreadProcessId(owner, &raw mut pid) == 0 {
                return;
            }
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if process.is_null() {
                return;
            }
            SetPropW(
                hwnd,
                windows_sys::w!("LucidPane.Hook.Bootstrap"),
                2_usize as _,
            );
            // A controller crash removes the Windows hook before our watchdog can run.
            // Keep callback code mapped until target-process exit; detach still removes every
            // callback/timer and geometry overrides. Never unload code with live stack frames.
            let mut module = null_mut();
            if GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                (crate::LucidPaneDesktopHook as *const ()).cast(),
                &raw mut module,
            ) == 0
            {
                CloseHandle(process);
                return;
            }
            SetPropW(
                hwnd,
                windows_sys::w!("LucidPane.Hook.Bootstrap"),
                3_usize as _,
            );
            let geometry = match crate::geometry::GeometrySession::attach(hwnd as isize) {
                Ok(session) => session,
                Err(error) => {
                    // Visible only to a diagnostic debugger; no modal UI inside Explorer.
                    eprintln!("Geometry attach rejected: {error}");
                    CloseHandle(process);
                    return;
                }
            };
            if SetWindowSubclass(hwnd, Some(subclass), SUBCLASS, 0) == 0 {
                CloseHandle(process);
                return;
            }
            SetPropW(
                hwnd,
                windows_sys::w!("LucidPane.Hook.Bootstrap"),
                4_usize as _,
            );
            if SetTimer(hwnd, TIMER, 1000, None) == 0 {
                RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS);
                CloseHandle(process);
                return;
            }
            *slot = Some(State {
                hwnd,
                owner,
                owner_process: process,
                managed: Vec::new(),
                changes: 0,
                shell_changes: 0,
                geometry,
            });
            SetPropW(
                hwnd,
                windows_sys::w!("LucidPane.Hook.Bootstrap"),
                5_usize as _,
            );
        }
    });
}

#[allow(clippy::too_many_lines)]
unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _id: usize,
    _data: usize,
) -> isize {
    let result = std::panic::catch_unwind(|| {
        STATE.with(|slot| {
            let Ok(mut slot) = slot.try_borrow_mut() else {
                return None;
            };
            let state = slot.as_mut()?;
            if state.hwnd != hwnd {
                return None;
            }
            // Signal user intent, not selection notifications: Shell menus also
            // select a hidden desktop item programmatically. No synchronous pane
            // calls, item scans or mouse-move traffic on the desktop input path.
            if matches!(
                msg,
                WM_LBUTTONDOWN
                    | WM_LBUTTONDBLCLK
                    | WM_RBUTTONDOWN
                    | WM_RBUTTONDBLCLK
                    | WM_MBUTTONDOWN
            ) || (msg == WM_KEYDOWN && lp & (1 << 30) == 0)
            {
                if !state.geometry.menu_active() {
                    unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                            state.owner,
                            crate::protocol::DESKTOP_INPUT_MESSAGE,
                            hwnd as usize,
                            windows_sys::Win32::UI::WindowsAndMessaging::GetMessageTime() as isize,
                        );
                    }
                }
            }
            if msg == crate::protocol::clear_selection_message() && wp as HWND == state.owner {
                // A queued request may arrive after the user has returned to the
                // desktop. Never clear their newer desktop selection in that case.
                unsafe {
                    let mut owner_pid = 0;
                    let mut foreground_pid = 0;
                    GetWindowThreadProcessId(state.owner, &raw mut owner_pid);
                    GetWindowThreadProcessId(
                        windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow(),
                        &raw mut foreground_pid,
                    );
                    if owner_pid != 0 && owner_pid == foreground_pid {
                        state.geometry.clear_desktop_selection();
                    }
                }
                return Some(0);
            }
            if msg == crate::protocol::geometry_attach_message() {
                return Some(if state.owner == wp as HWND {
                    OK
                } else {
                    REJECTED
                });
            }
            if msg == WM_NCDESTROY {
                uninstall(state);
                *slot = None;
                return None;
            }
            if msg == WM_DISPLAYCHANGE {
                // Old monitor rectangles are unsafe after topology changes. Restore and let the
                // controller report the lost session rather than parking items off screen.
                uninstall(state);
                *slot = None;
                return None;
            }
            if msg == WM_TIMER && wp == TIMER {
                if unsafe { WaitForSingleObject(state.owner_process, 0) } != 0x102
                    || unsafe { IsWindow(state.owner) } == 0
                {
                    uninstall(state);
                    *slot = None;
                }
                return Some(0);
            }
            if msg == WM_COPYDATA && wp as HWND == state.owner && lp != 0 {
                let data = unsafe { &*(lp as *const COPYDATASTRUCT) };
                if data.dwData == crate::protocol::TEXTURE_MAGIC {
                    let geometry = &state.geometry;
                    let offset = size_of::<crate::protocol::TextureHeader>();
                    if data.lpData.is_null() || (data.cbData as usize) < offset {
                        return Some(REJECTED);
                    }
                    let header = unsafe {
                        std::ptr::read_unaligned(
                            data.lpData.cast::<crate::protocol::TextureHeader>(),
                        )
                    };
                    let Some(count) = header.byte_count() else {
                        return Some(REJECTED);
                    };
                    if count + offset != data.cbData as usize {
                        return Some(REJECTED);
                    }
                    let pixels = unsafe {
                        std::slice::from_raw_parts(data.lpData.cast::<u8>().add(offset), count)
                    };
                    return Some(if geometry.set_texture(header, pixels).is_ok() {
                        OK
                    } else {
                        REJECTED
                    });
                }
                if data.dwData == crate::protocol::LAYOUT_MAGIC {
                    if data.cbData as usize != size_of::<crate::protocol::LayoutBatch>()
                        || data.lpData.is_null()
                    {
                        return Some(REJECTED);
                    }
                    let batch = unsafe {
                        std::ptr::read_unaligned(data.lpData.cast::<crate::protocol::LayoutBatch>())
                    };
                    return Some(apply_layout(state, &batch));
                }
                if data.dwData != MAGIC {
                    return None;
                }
                if data.cbData as usize != size_of::<Request>() || data.lpData.is_null() {
                    return Some(REJECTED);
                }
                let request = unsafe { std::ptr::read_unaligned(data.lpData.cast::<Request>()) };
                if !request.valid() {
                    return Some(REJECTED);
                }
                if request.command == DETACH {
                    uninstall(state);
                    *slot = None;
                    return Some(OK);
                }
                return Some(dispatch(state, &request));
            }
            if matches!(
                msg,
                LVM_INSERTITEMW
                    | LVM_DELETEITEM
                    | LVM_DELETEALLITEMS
                    | LVM_SETITEMTEXTW
                    | LVM_SORTITEMS
                    | windows_sys::Win32::UI::Controls::LVM_SORTITEMSEX
                    | LVM_SETITEMCOUNT
                    | LVM_SETITEMPOSITION
                    | LVM_SETITEMPOSITION32
                    | windows_sys::Win32::UI::Controls::LVM_ARRANGE
            ) {
                state.changes = state.changes.wrapping_add(1);
                state.shell_changes = state.shell_changes.wrapping_add(1);
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                        state.owner,
                        crate::protocol::SCENE_DIRTY_MESSAGE,
                        0,
                        0,
                    );
                }
            }
            None
        })
    });
    match result {
        Ok(Some(value)) => value,
        // This is also the reentrancy path used by our own ListView calls.
        _ => unsafe { DefSubclassProc(hwnd, msg, wp, lp) },
    }
}

fn apply_layout(state: &mut State, batch: &crate::protocol::LayoutBatch) -> isize {
    let geometry = &state.geometry;
    if !batch.valid() {
        return REJECTED;
    }
    let original_areas = std::mem::replace(
        &mut state.managed,
        batch.areas.areas[..batch.areas.count as usize].to_vec(),
    );
    geometry.begin_positions();
    for position in &batch.items[..batch.count as usize] {
        let mut request = Request::new(MOVE_ITEM);
        request.item = position.item;
        request.x = position.x;
        request.y = position.y;
        request.name_hash = position.name_hash;
        if dispatch(state, &request) != OK {
            state.managed = original_areas;
            return REJECTED;
        }
    }
    let members = batch.items[..batch.count as usize]
        .iter()
        .filter(|p| p.reserved > 0)
        .map(|p| (p.item, p.reserved as usize - 1))
        .collect();
    let result = if state
        .geometry
        .commit_scene(&batch.panes[..batch.pane_count as usize], members)
        .is_ok()
    {
        state
            .geometry
            .identities(&batch.items[..batch.count as usize], state.owner as isize);
        state.changes = state.changes.wrapping_add(1);
        if batch.flush == 1 {
            unsafe {
                windows_sys::Win32::Graphics::Gdi::UpdateWindow(state.hwnd);
            }
        }
        OK
    } else {
        REJECTED
    };
    if result != OK {
        state.managed = original_areas;
    }
    result
}

#[allow(clippy::too_many_lines)]
fn dispatch(state: &mut State, request: &Request) -> isize {
    unsafe {
        match request.command {
            QUERY => OK,
            crate::protocol::CLEAR_DESKTOP_SELECTION => {
                state.geometry.clear_desktop_selection();
                OK
            }
            crate::protocol::MENU_SELECTION_BEGIN | crate::protocol::MENU_SELECTION_END => {
                state
                    .geometry
                    .menu_selection(request.command == crate::protocol::MENU_SELECTION_BEGIN);
                if request.command == crate::protocol::MENU_SELECTION_END
                    && state.geometry.take_rename_request()
                {
                    crate::protocol::RENAME_REQUESTED
                } else {
                    OK
                }
            }
            crate::protocol::QUERY_SHELL_GENERATION => state.shell_changes as isize,
            crate::protocol::QUERY_MOVE_REQUESTS => crate::geometry::move_requests(),
            crate::protocol::QUERY_DROP_PROXY => isize::from(state.geometry.has_drop_proxy()),
            crate::protocol::QUERY_ORIGINAL_POSITION => {
                match state.geometry.original_position(request.item) {
                    Ok(p) if request.x == 0 => p.x as isize,
                    Ok(p) if request.x == 1 => p.y as isize,
                    _ => REJECTED,
                }
            }
            crate::protocol::BEGIN_POSITIONS => {
                state.geometry.begin_positions();
                OK
            }
            crate::protocol::COMMIT_POSITIONS => {
                if state.geometry.commit_positions().is_ok() {
                    state.changes = state.changes.wrapping_add(1);
                    if request.x == 1 {
                        windows_sys::Win32::Graphics::Gdi::UpdateWindow(state.hwnd);
                    }
                    OK
                } else {
                    REJECTED
                }
            }
            crate::protocol::CLEAR_POSITIONS => {
                return if state.geometry.set_positions(&[]).is_ok() {
                    OK
                } else {
                    REJECTED
                };
            }
            crate::protocol::QUERY_HIT => {
                let mut hit = windows_sys::Win32::UI::Controls::LVHITTESTINFO {
                    pt: POINT {
                        x: request.x,
                        y: request.y,
                    },
                    ..std::mem::zeroed()
                };
                SendMessageW(
                    state.hwnd,
                    windows_sys::Win32::UI::Controls::LVM_HITTEST,
                    0,
                    (&raw mut hit) as isize,
                ) + 1
            }
            crate::protocol::QUERY_INSERTION_TARGET => state
                .geometry
                .insertion_target(POINT {
                    x: request.x,
                    y: request.y,
                })
                .map_or(0, |(item, after)| {
                    ((item as isize + 1) * 2) + isize::from(after)
                }),
            crate::protocol::QUERY_ICON_RECT => {
                if request.item < 0 {
                    return REJECTED;
                }
                let mut rect = windows_sys::Win32::Foundation::RECT {
                    left: windows_sys::Win32::UI::Controls::LVIR_ICON as i32,
                    ..std::mem::zeroed()
                };
                if SendMessageW(
                    state.hwnd,
                    windows_sys::Win32::UI::Controls::LVM_GETITEMRECT,
                    request.item as usize,
                    (&raw mut rect) as isize,
                ) == 0
                {
                    return REJECTED;
                }
                match request.x {
                    0 => rect.left as isize,
                    1 => rect.top as isize,
                    2 => rect.right as isize,
                    3 => rect.bottom as isize,
                    _ => REJECTED,
                }
            }
            QUERY_GENERATION => state.changes as isize,
            QUERY_AUTOARRANGE => {
                isize::from(GetWindowLongW(state.hwnd, GWL_STYLE) & LVS_AUTOARRANGE as i32 != 0)
            }
            QUERY_ITEM_COUNT => SendMessageW(state.hwnd, LVM_GETITEMCOUNT, 0, 0),
            QUERY_ITEM_AREA => {
                let mut actual = POINT::default();
                if request.item < 0
                    || SendMessageW(
                        state.hwnd,
                        LVM_GETITEMPOSITION,
                        request.item as usize,
                        (&raw mut actual) as isize,
                    ) == 0
                {
                    return REJECTED;
                }
                state
                    .managed
                    .iter()
                    .position(|a| a.contains(actual.x, actual.y))
                    .map_or(REJECTED, |i| i as isize)
            }
            QUERY_AREA_COUNT => state.managed.len() as isize,
            SET_AREAS => {
                state.managed = request.areas[..request.count as usize].to_vec();
                state.changes = state.changes.wrapping_add(1);
                OK
            }
            MOVE_ITEM => {
                let count = SendMessageW(state.hwnd, LVM_GETITEMCOUNT, 0, 0);
                if request.item as isize >= count
                    || !state
                        .managed
                        .iter()
                        .any(|a| a.contains(request.x, request.y))
                {
                    return REJECTED;
                }
                if request.name_hash != 0 {
                    let mut text = [0_u16; 1024];
                    let mut item = LVITEMW {
                        mask: windows_sys::Win32::UI::Controls::LVIF_TEXT,
                        iItem: request.item,
                        pszText: text.as_mut_ptr(),
                        cchTextMax: text.len() as i32,
                        ..std::mem::zeroed()
                    };
                    let len = {
                        if SendMessageW(
                            state.hwnd,
                            windows_sys::Win32::UI::Controls::LVM_GETITEMW,
                            0,
                            (&raw mut item) as isize,
                        ) == 0
                        {
                            return REJECTED;
                        }
                        // Shell may return its own text pointer for callback data. Copy while
                        // still on the view thread, bounded by our protocol's label capacity.
                        if item.pszText.is_null() {
                            return REJECTED;
                        }
                        let mut len = 0;
                        while len < text.len() - 1 && *item.pszText.add(len) != 0 {
                            len += 1;
                        }
                        if item.pszText != text.as_mut_ptr() {
                            std::ptr::copy(item.pszText, text.as_mut_ptr(), len);
                        }
                        len as isize
                    };
                    if len <= 0
                        || len as usize >= text.len() - 1
                        || name_hash(text[..len as usize].iter().copied()) != request.name_hash
                    {
                        return REJECTED;
                    }
                }
                let desired = POINT {
                    x: request.x,
                    y: request.y,
                };

                return if state.geometry.set_position(request.item, desired).is_ok() {
                    state.changes = state.changes.wrapping_add(1);
                    OK
                } else {
                    REJECTED
                };
            }
            _ => REJECTED,
        }
    }
}

fn uninstall(state: &mut State) {
    unsafe {
        KillTimer(state.hwnd, TIMER);
        RemoveWindowSubclass(state.hwnd, Some(subclass), SUBCLASS);
        RemovePropW(state.hwnd, windows_sys::w!("LucidPane.Hook.Bootstrap"));
    }
    // The caller drops State immediately, restoring the geometry session.
}
