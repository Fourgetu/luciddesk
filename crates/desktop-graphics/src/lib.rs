//! Private graphics ABI boundary, generated against windows-core 0.100.
use windows_core::{IUnknown, Interface, Result};
#[allow(
    dead_code,
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types
)]
mod dcomp {
    include!("bindings/dcomp.rs");
}
#[allow(
    dead_code,
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types
)]
pub mod dwm {
    include!("bindings/dwm.rs");
}
pub struct Layer {
    device: dcomp::IDCompositionDevice,
    _target: dcomp::IDCompositionTarget,
    _visual: dcomp::IDCompositionVisual,
    opacity: dcomp::IDCompositionEffectGroup,
}
impl Layer {
    /// Attaches an existing composition swap chain to the top HWND target.
    ///
    /// # Safety
    /// `hwnd` must remain valid on the calling UI thread while this layer exists.
    /// `dxgi` must borrow an actual IDXGIDevice interface pointer, and `content`
    /// must refer to a composition-compatible swap chain from that device.
    /// COM must be initialized. The caller retains ownership of both arguments.
    pub unsafe fn new(
        hwnd: *mut core::ffi::c_void,
        dxgi: &IUnknown,
        content: &IUnknown,
        initial_opacity: f32,
    ) -> Result<Self> {
        unsafe {
            let mut raw = core::ptr::null_mut();
            dcomp::DCompositionCreateDevice(
                dxgi.as_raw(),
                &dcomp::IDCompositionDevice::IID,
                &mut raw,
            )
            .ok()?;
            let device = dcomp::IDCompositionDevice::from_raw(raw);
            let target = device.CreateTargetForHwnd(hwnd, true)?;
            let visual = device.CreateVisual()?;
            let opacity = device.CreateEffectGroup()?;
            opacity.SetOpacity2(initial_opacity).ok()?;
            visual.SetEffect(&opacity).ok()?;
            visual.SetContent(content).ok()?;
            target.SetRoot(&visual).ok()?;
            device.Commit().ok()?;
            Ok(Self {
                device,
                _target: target,
                _visual: visual,
                opacity,
            })
        }
    }
    pub fn opacity(&self, value: f32) -> Result<()> {
        unsafe { self.opacity.SetOpacity2(value).ok() }
    }
    pub fn commit(&self) -> Result<()> {
        unsafe { self.device.Commit().ok() }
    }
}
