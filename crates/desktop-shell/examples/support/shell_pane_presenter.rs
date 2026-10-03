//! Explicit opt-in probe of the unpublished Win11 presenter contract.
use std::{cell::Cell, ffi::c_void};
use windows::{
    Win32::{
        Foundation::{E_NOINTERFACE, E_POINTER, HWND},
        System::Com::{
            CLSCTX_INPROC_SERVER, CoCreateInstance, IServiceProvider, IServiceProvider_Impl,
        },
        UI::Shell::IShellView,
    },
    core::{GUID, HRESULT, IUnknown, Interface, Result, implement},
};
use windows_core::ComObjectInner;
#[path = "presenter_command_route.rs"]
mod command_route;
#[path = "presenter_trace.rs"]
mod trace;
#[path = "../../../desktop-explorer/src/filter/menu/presenter/input.rs"]
mod input_adapter;
pub fn log(args: std::fmt::Arguments<'_>) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/shell-pane-presenter-trace.log"
        ))
    {
        let _ = writeln!(file, "pid={} {args}", std::process::id());
    }
}
macro_rules! println { ($($arg:tt)*) => { log(format_args!($($arg)*)) }; }

const SERVICE: GUID = GUID::from_u128(0xb306c5b1_b4f2_473c_b6ff_701b246ce2d2);
#[implement(IServiceProvider)]
struct Site {
    presenter: IUnknown,
    input_adapter: Option<IUnknown>,
    _callback: IUnknown,
    closed: Cell<bool>,
    route: std::cell::RefCell<Option<command_route::CommandRoute>>,
}
impl IServiceProvider_Impl for Site_Impl {
    fn QueryService(
        &self,
        service: *const GUID,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> Result<()> {
        unsafe {
            if out.is_null() || service.is_null() || iid.is_null() {
                return Err(E_POINTER.into());
            }
            *out = std::ptr::null_mut();
            if *service != SERVICE {
                return Err(E_NOINTERFACE.into());
            }
            let source = self.input_adapter.as_ref().unwrap_or(&self.presenter);
            let result = (source.vtable().QueryInterface)(source.as_raw(), iid, out);
            println!(
                "compact_service_requested=true iid={:?} result={result:?}",
                *iid
            );
            result.ok()
        }
    }
}
impl Site {
    fn close(&self) {
        if !self.closed.replace(true) {
            self.route.borrow_mut().take();
            if let Ok(closable) = self.presenter.cast::<windows::Foundation::IClosable>() {
                let _ = closable.Close();
            }
        }
    }
}
impl Drop for Site {
    fn drop(&mut self) {
        self.close();
    }
}
pub struct PresenterSite(windows_core::ComObject<Site>);
impl PresenterSite {
    pub fn inspect_input(&self, hwnd: HWND) {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetTimer(
                hwnd.0,
                0x4c505454,
                250,
                Some(trace::inspect),
            );
        }
    }
    pub fn close(&self) {
        self.0.close();
    }
    pub fn service(&self) -> IServiceProvider {
        self.0.to_interface()
    }
    pub fn create(view: &IShellView, hwnd: HWND, host_kind: i32) -> Result<Self> {
        unsafe {
            // Read the same process gate used by this build's presenter. These
            // diagnostics never write the Shell's process mode or identity cache.
            use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
            let storage = GetModuleHandleW(windows_sys::w!("Windows.Storage.dll"));
            if !storage.is_null() {
                if let Some(proc) = GetProcAddress(storage, windows_sys::s!("IsProcessAnExplorer"))
                {
                    let check: unsafe extern "system" fn() -> i32 = std::mem::transmute(proc);
                    println!("process_gate.is_explorer={} host_kind={host_kind}", check());
                }
                if let Some(proc) = GetProcAddress(
                    storage,
                    windows_sys::s!("Global_WindowsStorage_esServerMode"),
                ) {
                    let mode: unsafe extern "system" fn() -> *const i32 = std::mem::transmute(proc);
                    if let Some(value) = mode().as_ref() {
                        println!("process_gate.server_mode={value}");
                    }
                }
            }
            let object: IUnknown = CoCreateInstance(
                &GUID::from_u128(0x86ca1aa0_34aa_4e8b_a509_50c905bae2a2),
                None,
                CLSCTX_INPROC_SERVER,
            )?;
            let mut raw = std::ptr::null_mut();
            (object.vtable().QueryInterface)(
                object.as_raw(),
                &GUID::from_u128(0x37a472f7_63cf_4ccf_a88b_5231a3c7d8b6),
                &raw mut raw,
            )
            .ok()?;
            let presenter = IUnknown::from_raw(raw);
            let unknown: IUnknown = view.cast()?;
            (unknown.vtable().QueryInterface)(
                unknown.as_raw(),
                &GUID::from_u128(0x9a19ddcf_9ed4_4a18_89de_c3b4dd1d7ae3),
                &raw mut raw,
            )
            .ok()?;
            let callback = IUnknown::from_raw(raw);
            let site = Site {
                input_adapter: if std::env::args().any(|arg| arg == "--input-adapter") {
                    Some(input_adapter::wrap(presenter.clone(), std::rc::Rc::new(Cell::new(false)))?)
                } else { None },
                presenter,
                _callback: callback,
                closed: Cell::new(false),
                route: std::cell::RefCell::new(None),
            }
            .into_object();
            type Initialize =
                unsafe extern "system" fn(*mut c_void, i32, *mut c_void, HWND, i32) -> HRESULT;
            let table = *site.presenter.as_raw().cast::<*const usize>();
            let initialize: Initialize = std::mem::transmute(*table.add(3));
            initialize(
                site.presenter.as_raw(),
                1,
                site._callback.as_raw(),
                hwnd,
                host_kind,
            )
            .ok()?;
            *site.route.borrow_mut() = Some(command_route::CommandRoute::attach(
                view.GetWindow()?.0,
                &site.presenter,
            )?);
            Ok(Self(site))
        }
    }
}
