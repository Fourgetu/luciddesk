//! Diagnostic adapter for the private presenter COM interface already researched
//! in menu_host_probe. No system function patches or object field writes.
use std::ffi::c_void;
use windows::{core::{implement, Interface, IUnknown, GUID, HRESULT, Result}, Win32::{
    Foundation::{HWND, E_NOINTERFACE}, System::Com::{IServiceProvider, IServiceProvider_Impl, CoCreateInstance, CLSCTX_INPROC_SERVER}, UI::Shell::IShellView,
}};

#[implement(IServiceProvider)]
pub struct Site { presenter: IUnknown, _callback: IUnknown }
impl IServiceProvider_Impl for Site_Impl {
    fn QueryService(&self, service: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        unsafe {
            *out = std::ptr::null_mut();
            if *service != GUID::from_u128(0xb306c5b1_b4f2_473c_b6ff_701b246ce2d2) { return Err(E_NOINTERFACE.into()); }
            let _ = std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/isolated-presenter-query.log"), format!("presenter_service_requested iid={:?}", *iid));
            (self.presenter.vtable().QueryInterface)(self.presenter.as_raw(), iid, out).ok()
        }
    }
}
impl Drop for Site {
    fn drop(&mut self) {
        if let Ok(closable) = self.presenter.cast::<windows::Foundation::IClosable>() { let _ = closable.Close(); }
    }
}
pub fn close(site: &IServiceProvider) {
    if let Ok(closable) = unsafe { site.QueryService::<windows::Foundation::IClosable>(&GUID::from_u128(0xb306c5b1_b4f2_473c_b6ff_701b246ce2d2)) } { let _ = closable.Close(); }
}
pub fn create(view: &IShellView, hwnd: HWND) -> Result<IServiceProvider> {
    unsafe {
        let object: IUnknown = CoCreateInstance(&GUID::from_u128(0x86ca1aa0_34aa_4e8b_a509_50c905bae2a2), None, CLSCTX_INPROC_SERVER)?;
        let mut raw = std::ptr::null_mut();
        (object.vtable().QueryInterface)(object.as_raw(), &GUID::from_u128(0x37a472f7_63cf_4ccf_a88b_5231a3c7d8b6), &raw mut raw).ok()?;
        let presenter = IUnknown::from_raw(raw);
        let unknown: IUnknown = view.cast()?;
        (unknown.vtable().QueryInterface)(unknown.as_raw(), &GUID::from_u128(0x9a19ddcf_9ed4_4a18_89de_c3b4dd1d7ae3), &raw mut raw).ok()?;
        let callback = IUnknown::from_raw(raw);
        type Initialize = unsafe extern "system" fn(*mut c_void, i32, *mut c_void, HWND, i32) -> HRESULT;
        let table = *presenter.as_raw().cast::<*const usize>();
        let initialize: Initialize = std::mem::transmute(*table.add(3));
        initialize(presenter.as_raw(), 1, callback.as_raw(), hwnd, 1).ok()?;
        let _ = std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/isolated-presenter-init.log"), format!("initialized=true hwnd={hwnd:?}"));
        Ok(Site { presenter, _callback: callback }.into())
    }
}
