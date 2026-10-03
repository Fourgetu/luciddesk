//! Disposable v6 virtual-list experiment. Never opens, hooks or sends messages to Explorer.
//! The undocumented callback ABI is confined to this executable until verified.
#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::Controls::{
    ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, LVM_ARRANGE,
    LVM_GETITEMPOSITION, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMCOUNT, LVS_AUTOARRANGE, LVS_ICON,
    LVS_OWNERDATA,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GWL_STYLE, GetWindowLongW, SendMessageW, WS_CHILD,
};

const SET_OWNER_DATA_CALLBACK: u32 = 0x10bb;
const E_NOINTERFACE: i32 = 0x8000_4002_u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;

// IOwnerDataCallback, including POINT and LVITEMINDEX passed by value on Windows x64.
#[repr(C)]
struct VTable {
    query: unsafe extern "system" fn(
        *mut Callback,
        *const windows_sys::core::GUID,
        *mut *mut c_void,
    ) -> i32,
    add_ref: unsafe extern "system" fn(*mut Callback) -> u32,
    release: unsafe extern "system" fn(*mut Callback) -> u32,
    get_position: unsafe extern "system" fn(*mut Callback, i32, *mut POINT) -> i32,
    set_position: unsafe extern "system" fn(*mut Callback, i32, POINT) -> i32,
    get_in_group: unsafe extern "system" fn(*mut Callback, i32, i32, *mut i32) -> i32,
    get_group: unsafe extern "system" fn(*mut Callback, i32, i32, *mut i32) -> i32,
    get_group_count: unsafe extern "system" fn(*mut Callback, i32, *mut i32) -> i32,
    cache_hint: unsafe extern "system" fn(*mut Callback, ItemIndex, ItemIndex) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ItemIndex {
    item: i32,
    group: i32,
}

#[repr(C)]
struct Callback {
    vtable: &'static VTable,
    refs: Cell<u32>,
    reads: Cell<u32>,
    writes: Cell<u32>,
    positions: RefCell<[POINT; 5]>,
    pinned: Cell<bool>,
}

unsafe extern "system" fn query(
    this: *mut Callback,
    iid: *const windows_sys::core::GUID,
    output: *mut *mut c_void,
) -> i32 {
    if iid.is_null() || output.is_null() {
        return E_INVALIDARG;
    }
    let iid = unsafe { &*iid };
    let unknown = windows_sys::core::GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
    let callback = windows_sys::core::GUID::from_u128(0x44c09d56_8d3b_419d_a462_7b956b105b47);
    unsafe {
        *output = null_mut();
    }
    if same_guid(iid, &unknown) || same_guid(iid, &callback) {
        unsafe {
            *output = this.cast();
            add_ref(this);
        }
        0
    } else {
        E_NOINTERFACE
    }
}

fn same_guid(a: &windows_sys::core::GUID, b: &windows_sys::core::GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}
unsafe extern "system" fn add_ref(this: *mut Callback) -> u32 {
    let state = unsafe { &*this };
    let count = state.refs.get() + 1;
    state.refs.set(count);
    count
}
unsafe extern "system" fn release(this: *mut Callback) -> u32 {
    let state = unsafe { &*this };
    let count = state.refs.get().saturating_sub(1);
    state.refs.set(count);
    count
}
unsafe extern "system" fn get_position(this: *mut Callback, item: i32, output: *mut POINT) -> i32 {
    let state = unsafe { &*this };
    let Ok(index) = usize::try_from(item) else {
        return E_INVALIDARG;
    };
    let positions = state.positions.borrow();
    let Some(position) = positions.get(index) else {
        return E_INVALIDARG;
    };
    if output.is_null() {
        return E_INVALIDARG;
    }
    unsafe {
        *output = *position;
    }
    state.reads.set(state.reads.get() + 1);
    0
}
unsafe extern "system" fn set_position(this: *mut Callback, item: i32, point: POINT) -> i32 {
    let state = unsafe { &*this };
    let Ok(index) = usize::try_from(item) else {
        return E_INVALIDARG;
    };
    let mut positions = state.positions.borrow_mut();
    let Some(position) = positions.get_mut(index) else {
        return E_INVALIDARG;
    };
    if !state.pinned.get() {
        *position = point;
    }
    state.writes.set(state.writes.get() + 1);
    0
}
unsafe extern "system" fn get_in_group(
    _: *mut Callback,
    _: i32,
    item: i32,
    output: *mut i32,
) -> i32 {
    if output.is_null() {
        return E_INVALIDARG;
    }
    unsafe {
        *output = item;
    }
    0
}
unsafe extern "system" fn get_group(_: *mut Callback, _: i32, _: i32, output: *mut i32) -> i32 {
    if output.is_null() {
        return E_INVALIDARG;
    }
    unsafe {
        *output = 0;
    }
    0
}
unsafe extern "system" fn get_group_count(_: *mut Callback, _: i32, output: *mut i32) -> i32 {
    if output.is_null() {
        return E_INVALIDARG;
    }
    unsafe {
        *output = 1;
    }
    0
}
unsafe extern "system" fn cache_hint(_: *mut Callback, _: ItemIndex, _: ItemIndex) -> i32 {
    0
}
static VTABLE: VTable = VTable {
    query,
    add_ref,
    release,
    get_position,
    set_position,
    get_in_group,
    get_group,
    get_group_count,
    cache_hint,
};

fn main() {
    let automatic = !std::env::args().any(|arg| arg == "--manual-fixture");
    unsafe {
        assert_ne!(
            InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            }),
            0
        );
        let parent = CreateWindowExW(
            0,
            windows_sys::w!("STATIC"),
            null(),
            0,
            0,
            0,
            900,
            600,
            null_mut(),
            null_mut(),
            null_mut(),
            null(),
        );
        assert!(!parent.is_null());
        let view = CreateWindowExW(
            0,
            windows_sys::w!("SysListView32"),
            null(),
            WS_CHILD | LVS_ICON | LVS_OWNERDATA | if automatic { LVS_AUTOARRANGE } else { 0 },
            0,
            0,
            900,
            600,
            parent,
            null_mut(),
            null_mut(),
            null(),
        );
        assert!(!view.is_null());
        // Mirror the styles read from this machine's Explorer, only in this disposable view.
        SendMessageW(view, LVM_SETEXTENDEDLISTVIEWSTYLE, 0, 0x14c1_4c30);
        let mut callback = Box::new(Callback {
            vtable: &VTABLE,
            refs: Cell::new(1),
            reads: Cell::new(0),
            writes: Cell::new(0),
            positions: RefCell::new(std::array::from_fn(|i| POINT {
                x: 40,
                y: 40 + i32::try_from(i).unwrap() * 100,
            })),
            pinned: Cell::new(false),
        });
        let result = SendMessageW(
            view,
            SET_OWNER_DATA_CALLBACK,
            (&raw mut *callback) as usize,
            0,
        );
        println!("register result={result}, refs={}", callback.refs.get());
        SendMessageW(view, LVM_SETITEMCOUNT, 5, 0);
        callback.positions.borrow_mut()[2] = POINT { x: 450, y: 150 };
        callback.pinned.set(true);
        let mut point = POINT::default();
        let ok = SendMessageW(view, LVM_GETITEMPOSITION, 2, (&raw mut point) as isize);
        println!(
            "before arrange: ok={ok}, point={},{} reads={} writes={}",
            point.x,
            point.y,
            callback.reads.get(),
            callback.writes.get()
        );
        SendMessageW(view, LVM_ARRANGE, 0, 0);
        SendMessageW(view, LVM_GETITEMPOSITION, 2, (&raw mut point) as isize);
        let auto = GetWindowLongW(view, GWL_STYLE) & LVS_AUTOARRANGE as i32 != 0;
        println!(
            "after arrange: point={},{} reads={} writes={} auto={auto}",
            point.x,
            point.y,
            callback.reads.get(),
            callback.writes.get()
        );
        // Destroy the sole consumer before dropping the callback allocation.
        DestroyWindow(parent);
        assert_eq!(callback.refs.get(), 1);
        assert_eq!(
            auto, automatic,
            "The fixture must not change its initial arrangement setting"
        );
        // This test records the missing preconditions instead of treating registration as
        // proof of a working backend. Do not relax the real-desktop guard based on this probe.
        assert_eq!(
            callback.reads.get(),
            0,
            "Position callback activation changed; inspect before use"
        );
        println!(
            "PASS: registration alone does not activate position callbacks in this fixture (auto={auto})"
        );
    }
}
