//! Selection checks on the disposable native control; never sends desktop input.
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    UI::{
        Controls::{
            LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_GETITEMSTATE, LVM_SETITEMSTATE,
            LVN_BEGINDRAG, NMHDR, NMLISTVIEW,
        },
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            GetParent, MSG, PM_REMOVE, PeekMessageW, PostMessageW, SendMessageW, WM_LBUTTONDOWN,
            WM_LBUTTONUP, WM_MOUSEFIRST, WM_MOUSELAST, WM_MOUSEMOVE, WM_NOTIFY,
        },
    },
};
static WRITES: AtomicUsize = AtomicUsize::new(0);
static DRAG_ITEM: AtomicUsize = AtomicUsize::new(usize::MAX);
type SetSelectionFlags = unsafe extern "system" fn(*mut std::ffi::c_void, u32, u32) -> i32;
pub struct ContentsSelection {
    object: *mut std::ffi::c_void,
    set: SetSelectionFlags,
    view: HWND,
    style: isize,
}
impl Drop for ContentsSelection {
    fn drop(&mut self) {
        unsafe {
            (self.set)(self.object, 1, 0);
            SendMessageW(
                self.view,
                windows_sys::Win32::UI::Controls::LVM_SETEXTENDEDLISTVIEWSTYLE,
                windows_sys::Win32::UI::Controls::LVS_EX_FULLROWSELECT as usize,
                self.style,
            );
            windows_sys::Win32::UI::Controls::SetWindowTheme(
                self.view,
                std::ptr::null(),
                std::ptr::null(),
            );
        }
    }
}
/// Disposable-view-only reproduction of Explorer's content-restricted selection.
/// Call after `GeometrySession::attach` has validated the exact comctl32 image.
pub fn restrict_to_contents(view: HWND) -> ContentsSelection {
    use windows_sys::Win32::{
        System::LibraryLoader::{
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            GetModuleHandleExW,
        },
        UI::{
            Controls::{LVM_SETEXTENDEDLISTVIEWSTYLE, LVS_EX_FULLROWSELECT, SetWindowTheme},
            WindowsAndMessaging::{
                GCLP_WNDPROC, GetClassLongPtrW, GetWindowLongPtrW, SendMessageW,
            },
        },
    };
    unsafe {
        let mut module = std::ptr::null_mut();
        assert_ne!(
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                GetClassLongPtrW(view, GCLP_WNDPROC) as _,
                &raw mut module
            ),
            0
        );
        let address = (module as usize + 0x0013_8b00) as *const u8;
        // CListView::SetSelectionFlags in the already validated image.
        assert_eq!(
            std::slice::from_raw_parts(address, 8),
            &[0x8b, 0xc2, 0x41, 0x23, 0xd0, 0xf7, 0xd0, 0x23]
        );
        let object = GetWindowLongPtrW(view, 0) as *mut std::ffi::c_void;
        assert!(!object.is_null());
        let set: SetSelectionFlags = std::mem::transmute(address);
        assert_eq!(set(object, 1, 1), 0);
        let style = SendMessageW(
            view,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            LVS_EX_FULLROWSELECT as usize,
            LVS_EX_FULLROWSELECT as isize,
        );
        assert_eq!(
            SetWindowTheme(view, windows_sys::w!("Explorer"), std::ptr::null()),
            0
        );
        let restricted: unsafe extern "system" fn(*mut std::ffi::c_void) -> bool =
            std::mem::transmute(module as usize + 0x001e_02c4);
        assert!(
            restricted(object),
            "Fixture did not enable Explorer's content selection branch"
        );
        ContentsSelection {
            object,
            set,
            view,
            style,
        }
    }
}
fn clear_queued_input(view: HWND) {
    let mut message = MSG::default();
    // DragDetect may finish as soon as movement crosses its threshold, leaving
    // our synthetic release queued. Do not let that terminate the next case.
    unsafe {
        while PeekMessageW(
            &raw mut message,
            view,
            WM_MOUSEFIRST,
            WM_MOUSELAST,
            PM_REMOVE,
        ) != 0
        {}
    }
}
unsafe extern "system" fn observer(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    _: usize,
) -> isize {
    if msg == LVM_SETITEMSTATE {
        WRITES.fetch_add(1, Ordering::Relaxed);
    }
    if msg == WM_NOTIFY && lp != 0 && unsafe { (*(lp as *const NMHDR)).code } == LVN_BEGINDRAG {
        DRAG_ITEM.store(
            usize::try_from(unsafe { (*(lp as *const NMLISTVIEW)).iItem }).unwrap_or(usize::MAX),
            Ordering::Relaxed,
        );
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

pub fn drag(view: HWND, item: usize, bounds: RECT) {
    clear_queued_input(view);
    unsafe {
        let parent = GetParent(view);
        assert_ne!(SetWindowSubclass(parent, Some(observer), 0x0053_454c, 0), 0);
        let clear = LVITEMW {
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..Default::default()
        };
        SendMessageW(
            view,
            LVM_SETITEMSTATE,
            usize::MAX,
            (&raw const clear) as isize,
        );
        DRAG_ITEM.store(usize::MAX, Ordering::Relaxed);
        let x = i32::midpoint(bounds.left, bounds.right);
        let y = i32::midpoint(bounds.top, bounds.bottom);
        let start = ((y << 16) | (x & 0xffff)) as isize;
        let end = (((y + 30) << 16) | ((x + 30) & 0xffff)) as isize;
        PostMessageW(view, WM_MOUSEMOVE, 1, end);
        PostMessageW(view, WM_LBUTTONUP, 0, end);
        SendMessageW(view, WM_LBUTTONDOWN, 1, start);
        SendMessageW(view, WM_LBUTTONUP, 0, end);
        RemoveWindowSubclass(parent, Some(observer), 0x0053_454c);
        assert_eq!(
            DRAG_ITEM.load(Ordering::Relaxed),
            item,
            "Native control did not start dragging an unselected icon on its first press"
        );
        println!("PASS: first press on unselected item={item} produces native LVN_BEGINDRAG");
    }
}

pub fn check(view: HWND, item: usize, bounds: RECT) {
    clear_queued_input(view);
    unsafe {
        assert_ne!(SetWindowSubclass(view, Some(observer), 0x0053_454c, 0), 0);
        let clear = LVITEMW {
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..Default::default()
        };
        SendMessageW(
            view,
            LVM_SETITEMSTATE,
            usize::MAX,
            (&raw const clear) as isize,
        );
        WRITES.store(0, Ordering::Relaxed);
        let x = i32::midpoint(bounds.left, bounds.right);
        let y = i32::midpoint(bounds.top, bounds.bottom);
        let point = (y << 16) | (x & 0xffff);
        // Queue release before the synchronous press so the native DragDetect loop
        // always has a terminating message, without manipulating the real cursor.
        PostMessageW(view, WM_LBUTTONUP, 0, point as isize);
        SendMessageW(view, WM_LBUTTONDOWN, 1, point as isize);
        SendMessageW(view, WM_LBUTTONUP, 0, point as isize);
        let selected = SendMessageW(
            view,
            LVM_GETITEMSTATE,
            item,
            (LVIS_SELECTED | LVIS_FOCUSED) as isize,
        );
        let writes = WRITES.load(Ordering::Relaxed);
        RemoveWindowSubclass(view, Some(observer), 0x0053_454c);
        assert_eq!(
            selected,
            (LVIS_SELECTED | LVIS_FOCUSED) as isize,
            "First press missed the compacted icon"
        );
        println!("First native press item={item}: selected/focused, extra state writes={writes}");
        assert_eq!(
            writes, 0,
            "Visible native mouse selection re-entered hidden state writes"
        );
    }
}
