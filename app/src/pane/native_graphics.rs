//! Interoperation between application bindings and the Canvas/generated graphics ABI.
use canvas_core::Interface as _;
use desktop_graphics::dwm;
pub use dwm::{
    DWMNCRP_DISABLED, DWMSBT_NONE, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY,
    DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_HOSTBACKDROPBRUSH, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND, MARGINS,
};
use windows::Win32::Foundation::HWND;
use windows::core::{Error, HRESULT, IUnknown, Interface, Result};

/// # Safety
/// The attribute and value must match the DWM API's expected layout.
pub unsafe fn set_attribute<T>(hwnd: HWND, attribute: i32, value: &T) -> Result<()> {
    HRESULT(unsafe {
        dwm::DwmSetWindowAttribute(
            hwnd.0,
            attribute as u32,
            std::ptr::from_ref(value).cast(),
            size_of::<T>() as u32,
        )
    })
    .ok()
}

/// # Safety
/// The HWND must belong to a live window.
pub unsafe fn extend_frame(hwnd: HWND, margins: &MARGINS) -> Result<()> {
    HRESULT(unsafe { dwm::DwmExtendFrameIntoClientArea(hwnd.0, margins) }).ok()
}

pub fn create_layer(
    hwnd: HWND,
    dxgi: &windows::Win32::Graphics::Dxgi::IDXGIDevice,
    content: &windows::Win32::Graphics::Dxgi::IDXGISwapChain1,
    opacity: f32,
) -> Result<desktop_graphics::Layer> {
    let dxgi_ptr = dxgi.as_raw();
    let content_ptr = content.as_raw();
    canvas_result(unsafe {
        desktop_graphics::Layer::new(
            hwnd.0,
            canvas_core::IUnknown::from_raw_borrowed(&dxgi_ptr).unwrap(),
            canvas_core::IUnknown::from_raw_borrowed(&content_ptr).unwrap(),
            opacity,
        )
    })
}

/// QueryInterface returns an owned reference without transferring the source's ownership.
pub fn native_interface<T: Interface>(source: &impl canvas_core::Interface) -> Result<T> {
    let raw = source.as_raw();
    unsafe { IUnknown::from_raw_borrowed(&raw).unwrap().cast() }
}

pub fn canvas_result<T>(result: canvas_core::Result<T>) -> Result<T> {
    result.map_err(|error| Error::from_hresult(HRESULT(error.code().0)))
}
