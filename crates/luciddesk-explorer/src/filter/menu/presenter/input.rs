//! Preserve the Pane invocation source at the native presenter boundary.
//! This extends the existing private COM adapter; it never edits Shell objects.
use std::{
    cell::Cell,
    ffi::c_void,
    rc::Rc,
    sync::atomic::{AtomicU32, Ordering},
};
use windows::{
    Win32::{
        Foundation::{E_POINTER, HWND, POINT},
        UI::WindowsAndMessaging::HMENU,
    },
    core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface, Result},
};

const PRESENTER: GUID = GUID::from_u128(0x37a472f7_63cf_4ccf_a88b_5231a3c7d8b6);
const TIP_TEST: GUID = GUID::from_u128(0x5a8e3042_b975_46b6_b839_50baafe40541);
// CDefView sets this flag for a mouse WM_CONTEXTMENU. Without an input flag,
// the native flyout enters access-key display mode when it finishes loading.
const MOUSE: u32 = 8;
#[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
thread_local! { static TRACE_HOST: Cell<isize> = const { Cell::new(0) }; }
#[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
pub(super) fn trace_host(hwnd: isize) { TRACE_HOST.set(hwnd); }
#[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
fn trace(name: windows_sys::core::PCWSTR, value: usize) {
    let hwnd = TRACE_HOST.get();
    if hwnd != 0 { unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetPropW(hwnd as _, name, value as _); } }
}


#[repr(C)]
struct Vtable {
    base: IUnknown_Vtbl,
    initialize: unsafe extern "system" fn(*mut c_void, i32, *mut c_void, HWND, i32) -> HRESULT,
    prepare: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        POINT,
        *mut c_void,
        u32,
        u32,
        i32,
        *const c_void,
        GUID,
        *mut c_void,
    ) -> HRESULT,
    show: unsafe extern "system" fn(*mut c_void, HMENU, u32, GUID),
    dismiss: unsafe extern "system" fn(*mut c_void, *const u16),
    is_open: unsafe extern "system" fn(*mut c_void) -> i32,
    invoke: unsafe extern "system" fn(*mut c_void, u32),
    is_ready: unsafe extern "system" fn(*mut c_void, GUID, POINT) -> i32,
    access_keys: unsafe extern "system" fn(*mut c_void),
}
#[repr(C)]
struct TipVtable {
    base: IUnknown_Vtbl,
    show: unsafe extern "system" fn(*mut c_void, HMENU, u32, GUID, GUID, GUID),
}
#[repr(C)]
struct Adapter {
    vtable: *const c_void,
    refs: AtomicU32,
    inner: IUnknown,
    keyboard: Rc<Cell<bool>>,
    cancelled: Rc<dyn Fn() -> bool>,
    // Optional telemetry interface shares the primary adapter's COM identity.
    owner: Option<IUnknown>,
}
impl Adapter {
    unsafe fn native(&self) -> &Vtable {
        unsafe { &**(self.inner.as_raw() as *const *const Vtable) }
    }
    fn flags(&self, flags: u32) -> u32 {
        if self.keyboard.get() {
            flags & !MOUSE
        } else {
            flags | MOUSE
        }
    }
}
#[allow(dead_code)] // Also used by the standalone native-interface diagnostic.
pub(super) fn wrap(inner: IUnknown, keyboard: Rc<Cell<bool>>) -> Result<IUnknown> {
    wrap_cancellable(inner, keyboard, Rc::new(|| false))
}
pub(super) fn wrap_cancellable(
    inner: IUnknown,
    keyboard: Rc<Cell<bool>>,
    cancelled: Rc<dyn Fn() -> bool>,
) -> Result<IUnknown> {
    // IUnknown identity may have an entirely different vtable (IInspectable on
    // the native WinRT presenter). Always acquire the exact interface whose
    // methods we forward, even when the input came from a Presenter cast.
    unsafe {
        let mut raw = std::ptr::null_mut();
        (inner.vtable().QueryInterface)(inner.as_raw(), &PRESENTER, &raw mut raw).ok()?;
        Ok(allocate(IUnknown::from_raw(raw), keyboard, None, cancelled))
    }
}
fn allocate(
    inner: IUnknown,
    keyboard: Rc<Cell<bool>>,
    owner: Option<IUnknown>,
    cancelled: Rc<dyn Fn() -> bool>,
) -> IUnknown {
    let vtable = if owner.is_some() {
        (&raw const TIP_VTABLE).cast()
    } else {
        (&raw const VTABLE).cast()
    };
    let value = Box::new(Adapter {
        vtable,
        refs: AtomicU32::new(1),
        inner,
        keyboard,
        cancelled,
        owner,
    });
    unsafe { IUnknown::from_raw(Box::into_raw(value).cast()) }
}
unsafe extern "system" fn query(
    this: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        if out.is_null() {
            return E_POINTER;
        }
        *out = std::ptr::null_mut();
        if iid.is_null() {
            return E_POINTER;
        }
        let value = &*(this as *const Adapter);
        if let Some(owner) = &value.owner {
            if *iid != TIP_TEST {
                return (owner.vtable().QueryInterface)(owner.as_raw(), iid, out);
            }
        }
        if *iid == IUnknown::IID
            || *iid
                == if value.owner.is_some() {
                    TIP_TEST
                } else {
                    PRESENTER
                }
        {
            *out = this;
            add_ref(this);
            return HRESULT(0);
        }
        // Never leak a raw native interface: querying IUnknown through it would
        // return another identity and querying Presenter would bypass this adapter.
        // NativePresenter retains/uses IClosable directly for its own lifecycle.
        if *iid != TIP_TEST {
            return windows::Win32::Foundation::E_NOINTERFACE;
        }
        let result = (value.inner.vtable().QueryInterface)(value.inner.as_raw(), iid, out);
        if result.is_ok() {
            let inner = IUnknown::from_raw(*out);
            add_ref(this);
            let owner = IUnknown::from_raw(this);
            *out = allocate(
                inner,
                value.keyboard.clone(),
                Some(owner),
                value.cancelled.clone(),
            )
            .into_raw();
        }
        result
    }
}
unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    unsafe {
        (*(this as *const Adapter))
            .refs
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }
}
unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    unsafe {
        let remaining = (*(this as *const Adapter))
            .refs
            .fetch_sub(1, Ordering::AcqRel)
            - 1;
        if remaining == 0 {
            drop(Box::from_raw(this as *mut Adapter));
        }
        remaining
    }
}
macro_rules! forward {
    ($name:ident ($($arg:ident : $ty:ty),*) -> $ret:ty) => {
        unsafe extern "system" fn $name(this: *mut c_void, $($arg: $ty),*) -> $ret {
            unsafe {
                let value = &*(this as *const Adapter);
                (value.native().$name)(value.inner.as_raw(), $($arg),*)
            }
        }
    };
}
forward!(initialize(enabled: i32, callback: *mut c_void, hwnd: HWND, host: i32) -> HRESULT);
unsafe extern "system" fn prepare(this: *mut c_void, location: u32, site: *mut c_void, point: POINT, menu: *mut c_void, first: u32, last: u32, count: i32, verbs: *const c_void, test: GUID, item: *mut c_void) -> HRESULT {
    unsafe {
        let value = &*(this as *const Adapter);
        let result = (value.native().prepare)(value.inner.as_raw(), location, site, point, menu, first, last, count, verbs, test, item);
        #[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
        trace(windows_sys::w!("LucidDesk.Menu.PrepareCalled"), 1);
        #[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
        trace(windows_sys::w!("LucidDesk.Menu.PrepareResult"), result.0 as u32 as usize);
        result
    }
}
forward!(dismiss(reason: *const u16) -> ());
forward!(is_open() -> i32);
forward!(invoke(command: u32) -> ());
unsafe extern "system" fn is_ready(this: *mut c_void, test: GUID, point: POINT) -> i32 {
    unsafe {
        let value = &*(this as *const Adapter);
        let result = (value.native().is_ready)(value.inner.as_raw(), test, point);
        #[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
        trace(windows_sys::w!("LucidDesk.Menu.ReadyCalled"), 1);
        #[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
        trace(windows_sys::w!("LucidDesk.Menu.ReadyResult"), result as u32 as usize);
        result
    }
}
forward!(access_keys() -> ());
unsafe extern "system" fn show(this: *mut c_void, menu: HMENU, flags: u32, test: GUID) {
    unsafe {
        let value = &*(this as *const Adapter);
        if (value.cancelled)() {
            return;
        }
        #[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
        trace(windows_sys::w!("LucidDesk.Menu.ShowCalled"), 1);
        (value.native().show)(value.inner.as_raw(), menu, value.flags(flags), test);
    }
}
unsafe extern "system" fn show_tip(
    this: *mut c_void,
    menu: HMENU,
    flags: u32,
    test: GUID,
    init: GUID,
    load: GUID,
) {
    unsafe {
        let value = &*(this as *const Adapter);
        if (value.cancelled)() {
            return;
        }
        let native = &**(value.inner.as_raw() as *const *const TipVtable);
        #[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
        trace(windows_sys::w!("LucidDesk.Menu.ShowCalled"), 2);
        (native.show)(
            value.inner.as_raw(),
            menu,
            value.flags(flags),
            test,
            init,
            load,
        );
    }
}
static VTABLE: Vtable = Vtable {
    base: IUnknown_Vtbl {
        QueryInterface: query,
        AddRef: add_ref,
        Release: release,
    },
    initialize,
    prepare,
    show,
    dismiss,
    is_open,
    invoke,
    is_ready,
    access_keys,
};
static TIP_VTABLE: TipVtable = TipVtable {
    base: IUnknown_Vtbl {
        QueryInterface: query,
        AddRef: add_ref,
        Release: release,
    },
    show: show_tip,
};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "native Win11 presenter lifecycle diagnostic; run alone"]
    fn native_presenter_readiness_survives_recreation() {
        use windows::Win32::System::Com::*;
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            for attempt in 0..3 {
                let native: IUnknown = CoCreateInstance(
                    &GUID::from_u128(0x86ca1aa0_34aa_4e8b_a509_50c905bae2a2),
                    None, CLSCTX_INPROC_SERVER).unwrap();
                let adapter = wrap(native.clone(), Rc::new(Cell::new(false))).unwrap();
                let ready = is_ready(adapter.as_raw(), GUID::zeroed(), POINT::default());
                eprintln!("presenter recreation attempt={attempt} ready={ready}");
                native.cast::<windows::Foundation::IClosable>().unwrap().Close().unwrap();
                assert_ne!(ready, 0, "recreated presenter must remain ready");
            }
            CoUninitialize();
        }
    }
    #[test]
    #[ignore = "requires the native Win11 presenter; run in a standalone test process"]
    fn native_presenter_uses_queried_interface_instead_of_unknown_identity() {
        use windows::Win32::System::Com::*;
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            {
                let native: IUnknown = CoCreateInstance(
                    &GUID::from_u128(0x86ca1aa0_34aa_4e8b_a509_50c905bae2a2),
                    None,
                    CLSCTX_INPROC_SERVER,
                )
                .unwrap();
                let mut raw = std::ptr::null_mut();
                (native.vtable().QueryInterface)(native.as_raw(), &PRESENTER, &raw mut raw)
                    .ok()
                    .unwrap();
                let specific = IUnknown::from_raw(raw);
                let adapter = wrap(native.clone(), Rc::new(Cell::new(false))).unwrap();
                let value = &*(adapter.as_raw() as *const Adapter);
                assert_eq!(value.inner.as_raw(), specific.as_raw());
                assert_eq!(
                    adapter
                        .cast::<windows::Foundation::IClosable>()
                        .unwrap_err()
                        .code(),
                    windows::Win32::Foundation::E_NOINTERFACE
                );
                let mut tip = std::ptr::null_mut();
                if (adapter.vtable().QueryInterface)(adapter.as_raw(), &TIP_TEST, &raw mut tip)
                    .is_ok()
                {
                    let tip = IUnknown::from_raw(tip);
                    assert_eq!(tip.cast::<IUnknown>().unwrap().as_raw(), adapter.as_raw());
                    let mut back = std::ptr::null_mut();
                    (tip.vtable().QueryInterface)(tip.as_raw(), &PRESENTER, &raw mut back)
                        .ok()
                        .unwrap();
                    assert_eq!(IUnknown::from_raw(back).as_raw(), adapter.as_raw());
                }
                eprintln!(
                    "IUnknown={:?} Presenter={:?} adapter_inner={:?}",
                    native.as_raw(),
                    specific.as_raw(),
                    value.inner.as_raw()
                );
                assert_eq!(is_open(adapter.as_raw()), 0);
                assert_ne!(
                    is_ready(adapter.as_raw(), GUID::zeroed(), POINT::default()),
                    0
                );
                if let Ok(closable) = native.cast::<windows::Foundation::IClosable>() {
                    closable.Close().unwrap();
                }
            }
            CoUninitialize();
        }
    }
    #[repr(C)]
    struct Fixture {
        vtable: *const c_void,
        refs: AtomicU32,
        calls: Rc<Cell<(u32, usize, u128)>>,
    }
    fn fixture(calls: Rc<Cell<(u32, usize, u128)>>, tip: bool) -> IUnknown {
        let value = Box::new(Fixture {
            vtable: if tip {
                (&raw const NATIVE_TIP).cast()
            } else {
                (&raw const NATIVE).cast()
            },
            refs: AtomicU32::new(1),
            calls,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(value).cast()) }
    }
    unsafe extern "system" fn fixture_query(
        this: *mut c_void,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> HRESULT {
        unsafe {
            let value = &*(this as *const Fixture);
            if *iid == TIP_TEST {
                *out = fixture(value.calls.clone(), true).into_raw();
            } else {
                *out = this;
                fixture_add(this);
            }
            HRESULT(0)
        }
    }
    unsafe extern "system" fn fixture_add(this: *mut c_void) -> u32 {
        unsafe {
            (*(this as *const Fixture))
                .refs
                .fetch_add(1, Ordering::Relaxed)
                + 1
        }
    }
    unsafe extern "system" fn fixture_release(this: *mut c_void) -> u32 {
        unsafe {
            let n = (*(this as *const Fixture))
                .refs
                .fetch_sub(1, Ordering::AcqRel)
                - 1;
            if n == 0 {
                drop(Box::from_raw(this as *mut Fixture));
            }
            n
        }
    }
    unsafe extern "system" fn record(this: *mut c_void, menu: HMENU, flags: u32, test: GUID) {
        unsafe {
            (*(this as *const Fixture))
                .calls
                .set((flags, menu.0 as usize, test.to_u128()));
        }
    }
    unsafe extern "system" fn record_tip(
        this: *mut c_void,
        menu: HMENU,
        flags: u32,
        test: GUID,
        init: GUID,
        load: GUID,
    ) {
        assert_eq!(init.to_u128(), 123);
        assert_eq!(load.to_u128(), 456);
        unsafe {
            record(this, menu, flags, test);
        }
    }
    static NATIVE: Vtable = Vtable {
        base: IUnknown_Vtbl {
            QueryInterface: fixture_query,
            AddRef: fixture_add,
            Release: fixture_release,
        },
        initialize,
        prepare,
        show: record,
        dismiss,
        is_open,
        invoke,
        is_ready,
        access_keys,
    };
    static NATIVE_TIP: TipVtable = TipVtable {
        base: IUnknown_Vtbl {
            QueryInterface: fixture_query,
            AddRef: fixture_add,
            Release: fixture_release,
        },
        show: record_tip,
    };
    #[test]
    fn reused_presenter_preserves_source_and_arguments_on_both_native_paths() {
        let calls = Rc::new(Cell::new((0, 0, 0)));
        let keyboard = Rc::new(Cell::new(false));
        let presenter = wrap(fixture(calls.clone(), false), keyboard.clone()).unwrap();
        unsafe {
            let mut raw = std::ptr::null_mut();
            (presenter.vtable().QueryInterface)(presenter.as_raw(), &TIP_TEST, &raw mut raw)
                .ok()
                .unwrap();
            let tip = IUnknown::from_raw(raw);
            let identity: IUnknown = tip.cast().unwrap();
            assert_eq!(identity.as_raw(), presenter.as_raw());
            assert_eq!(
                presenter
                    .cast::<windows::Foundation::IClosable>()
                    .unwrap_err()
                    .code(),
                windows::Win32::Foundation::E_NOINTERFACE
            );
            assert_eq!(
                tip.cast::<windows::Foundation::IClosable>()
                    .unwrap_err()
                    .code(),
                windows::Win32::Foundation::E_NOINTERFACE
            );
            // Reuse one instance across alternating invocations, including a
            // stale mouse flag on a keyboard request. Preserve unrelated bits.
            for (key, input, expected) in
                [(false, 0x11, 0x19), (true, 0x19, 0x11), (false, 0x11, 0x19)]
            {
                keyboard.set(key);
                show(
                    presenter.as_raw(),
                    HMENU(321usize as _),
                    input,
                    GUID::from_u128(987),
                );
                assert_eq!(calls.get(), (expected, 321, 987));
                show_tip(
                    tip.as_raw(),
                    HMENU(654usize as _),
                    input,
                    GUID::from_u128(789),
                    GUID::from_u128(123),
                    GUID::from_u128(456),
                );
                assert_eq!(calls.get(), (expected, 654, 789));
            }
        }
        drop(presenter);
        assert_eq!(Rc::strong_count(&calls), 1);
    }

    #[test]
    fn cancelled_extension_completion_cannot_show_either_popup_path() {
        let calls = Rc::new(Cell::new((0, 0, 0)));
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let token = cancelled.clone();
        let presenter = wrap_cancellable(
            fixture(calls.clone(), false),
            Rc::new(Cell::new(false)),
            Rc::new(move || token.load(std::sync::atomic::Ordering::Acquire)),
        )
        .unwrap();
        unsafe {
            let mut raw = std::ptr::null_mut();
            (presenter.vtable().QueryInterface)(presenter.as_raw(), &TIP_TEST, &raw mut raw)
                .ok()
                .unwrap();
            let tip = IUnknown::from_raw(raw);
            // Mimic cancellation on the desktop thread while the Shell STA is busy.
            std::thread::spawn(move || cancelled.store(true, std::sync::atomic::Ordering::Release))
                .join()
                .unwrap();
            show(presenter.as_raw(), HMENU::default(), 0, GUID::zeroed());
            show_tip(
                tip.as_raw(),
                HMENU::default(),
                0,
                GUID::zeroed(),
                GUID::from_u128(123),
                GUID::from_u128(456),
            );
            assert_eq!(calls.get(), (0, 0, 0));
        }
    }
}
