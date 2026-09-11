//! Private generated graphics bindings. Regenerate with tools/windows-bindings.
use canvas_core::Interface as _;
use windows::Win32::Foundation::HWND;
use windows::core::{HRESULT, Interface, Result};

use desktop_graphics::dwm;
pub use dwm::{
    DWMNCRP_DISABLED, DWMSBT_NONE, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY,
    DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_HOSTBACKDROPBRUSH, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, MARGINS,
};

// Preserve the existing native signature at the private adapter boundary.
#[allow(non_snake_case)]
pub unsafe fn DwmSetWindowAttribute(
    hwnd: HWND,
    attribute: i32,
    data: *const core::ffi::c_void,
    size: u32,
) -> Result<()> {
    HRESULT(unsafe { dwm::DwmSetWindowAttribute(hwnd.0, attribute as u32, data, size) }).ok()
}
#[allow(non_snake_case)]
pub unsafe fn DwmExtendFrameIntoClientArea(hwnd: HWND, margins: *const MARGINS) -> Result<()> {
    HRESULT(unsafe { dwm::DwmExtendFrameIntoClientArea(hwnd.0, margins) }).ok()
}

pub struct Layer(desktop_graphics::Layer);
impl Layer {
    pub fn new(
        hwnd: HWND,
        dxgi: &impl Interface,
        content: &impl Interface,
        initial_opacity: f32,
    ) -> Result<Self> {
        let dxgi_ptr = dxgi.as_raw();
        let content_ptr = content.as_raw();
        // Borrow existing COM references; the native API retains the references it needs.
        super::composition::canvas_result(unsafe {
            desktop_graphics::Layer::new(
                hwnd.0,
                canvas_core::IUnknown::from_raw_borrowed(&dxgi_ptr).unwrap(),
                canvas_core::IUnknown::from_raw_borrowed(&content_ptr).unwrap(),
                initial_opacity,
            )
        })
        .map(Self)
    }
    pub fn opacity(&self, value: f32) -> Result<()> {
        super::composition::canvas_result(self.0.opacity(value))
    }
    pub fn commit(&self) -> Result<()> {
        super::composition::canvas_result(self.0.commit())
    }
}
