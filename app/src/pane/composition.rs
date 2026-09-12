//! Transparent content surface over the system-owned backdrop; no whole-window alpha.
#![allow(clippy::wildcard_imports)]
use super::native_graphics::*;
use desktop_core::Backdrop;
use windows::Win32::Foundation::HWND;

use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::{Error, Interface, Result};
use windows_canvas::{GpuDevice, ID2D1DeviceContext, SwapChain};

pub struct Surface {
    retry_at: std::cell::Cell<Option<std::time::Instant>>,
    hwnd: HWND,
    present: IDXGISwapChain1,
    rounded_backdrop: Option<HWND>,
    pub pane_corner_radius: u8,
    dark: bool,
    opacity: std::cell::Cell<f32>,
    acrylic: Option<super::acrylic::Acrylic>,
    _device: GpuDevice,
    #[cfg(test)]
    context: ID3D11DeviceContext,
    drawing: ID2D1DeviceContext,
    layer: desktop_graphics::Layer,
    swap: SwapChain,
    material: Option<Backdrop>,
    pub native: bool,
}

impl Surface {
    /// Suppress the DWM non-client frame; forced system rounding can still cast a shadow.
    /// Keep this separate from shared surface initialization so flyout shadows remain.
    pub fn disable_window_shadow(hwnd: HWND) -> Result<()> {
        let policy = DWMNCRP_DISABLED;
        unsafe { set_attribute(hwnd, DWMWA_NCRENDERING_POLICY, &policy) }
    }

    pub fn opacity(&self, opacity: f32) -> Result<()> {
        if self.opacity.get() == opacity {
            return Ok(());
        }
        canvas_result(self.layer.opacity(opacity))?;
        if let Some(acrylic) = &self.acrylic {
            acrylic.opacity(opacity)?;
        }
        canvas_result(self.layer.commit())?;
        self.opacity.set(opacity);
        Ok(())
    }
    pub fn new(hwnd: HWND) -> Result<Self> {
        Self::new_with_opacity(hwnd, 1.0)
    }

    pub fn new_pane(hwnd: HWND) -> Result<Self> {
        let mut surface = Self::new(hwnd)?;
        Self::disable_window_shadow(hwnd)?;
        // On Windows 11, forced DWM rounding casts an activation shadow even with
        // non-client rendering disabled. Round our backdrop instead of the HWND.
        unsafe { set_attribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_DONOTROUND)? };
        surface.rounded_backdrop = Some(hwnd);
        Ok(surface)
    }

    pub fn new_with_opacity(hwnd: HWND, initial_opacity: f32) -> Result<Self> {
        Self::new_with_device(hwnd, initial_opacity, gpu_device()?)
    }

    fn new_with_device(hwnd: HWND, initial_opacity: f32, device: GpuDevice) -> Result<Self> {
        unsafe {
            let d3d: ID3D11Device = native_interface(device.d3d_device())?;
            #[cfg(test)]
            let context = d3d.GetImmediateContext()?;
            let dxgi: IDXGIDevice = d3d.cast()?;
            let mut bounds = windows_sys::Win32::Foundation::RECT::default();
            windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd.0, &raw mut bounds);
            let mut swap = canvas_result(
                device.create_swap_chain(bounds.right.max(1) as u32, bounds.bottom.max(1) as u32),
            )?;
            // Obtain Canvas's persistent context once. Subsequent frames keep the
            // renderer's explicit BeginDraw/EndDraw so all drawing errors propagate.
            let drawing: ID2D1DeviceContext = {
                let session = canvas_result(swap.begin_draw())?;
                session.raw().clone()
            };
            let native_swap: IDXGISwapChain1 = native_interface(swap.raw_swap_chain())?;
            let layer = create_layer(hwnd, &dxgi, &native_swap, initial_opacity)?;
            let margins = MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            };
            extend_frame(hwnd, &margins)?;
            let dark = 1i32;
            // The content renderer owns the single border. Suppress the second DWM outline.
            let border = 0xffff_fffeu32;
            let _ = set_attribute(hwnd, DWMWA_BORDER_COLOR, &border);
            let _ = set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark);
            let corner = DWMWCP_ROUND;
            let _ = set_attribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner);
            Ok(Self {
                retry_at: std::cell::Cell::new(None),
                hwnd,
                present: native_swap,
                rounded_backdrop: None,
                pane_corner_radius: 7,
                dark: true,
                opacity: std::cell::Cell::new(initial_opacity),
                acrylic: None,
                _device: device,
                #[cfg(test)]
                context,
                drawing,
                layer,
                swap,
                material: None,
                native: false,
            })
        }
    }

    pub fn theme(&mut self, hwnd: HWND, dark: bool) {
        if self.dark != dark {
            self.dark = dark;
            self.material = None;
            let value = i32::from(dark);
            unsafe {
                let _ = set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &value);
            }
        }
    }

    pub fn material(&mut self, hwnd: HWND, material: Backdrop) {
        if self.material == Some(material) {
            return;
        }
        let kind = match material {
            Backdrop::Mica | Backdrop::MicaAlt => DWMSBT_NONE,
            Backdrop::Acrylic | Backdrop::Translucent { .. } => DWMSBT_NONE,
        };
        self.native = unsafe { set_attribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &kind).is_ok() }
            && kind != DWMSBT_NONE;
        if matches!(
            material,
            Backdrop::Acrylic | Backdrop::Mica | Backdrop::MicaAlt
        ) {
            if self.acrylic.is_none() {
                self.acrylic =
                    super::acrylic::Acrylic::new_with_opacity(hwnd, self.opacity.get()).ok();
            }
            self.native = self
                .acrylic
                .as_ref()
                .is_some_and(|acrylic| acrylic.material(material, self.dark).is_ok());
            if !self.native
                && let Some(acrylic) = &self.acrylic
            {
                let _ = acrylic.visible(false);
            }
        } else if let Some(acrylic) = &self.acrylic {
            let _ = acrylic.visible(false);
        }
        self.material = Some(material);
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Err(Error::from_hresult(
                windows::Win32::Foundation::E_INVALIDARG,
            ));
        }
        if (self.swap.width(), self.swap.height()) != (width, height) {
            canvas_result(self.swap.resize(width, height))?;
        }
        if let (Some(hwnd), Some(acrylic)) = (self.rounded_backdrop, &mut self.acrylic) {
            let scale =
                unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd.0) } as f32 / 96.0;
            // Include the outline's half-DIP outset when clipping the backdrop.
            acrylic.round_corners(
                width,
                height,
                if self.pane_corner_radius == 0 {
                    0.0
                } else {
                    (f32::from(self.pane_corner_radius) + 0.5) * scale
                },
            )?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn present(&mut self, width: u32, height: u32, pixels: &[u8]) -> Result<()> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|count| count.checked_mul(4));
        if expected != Some(pixels.len()) {
            return Err(Error::from_hresult(
                windows::Win32::Foundation::E_INVALIDARG,
            ));
        }
        self.resize(width, height)?;
        unsafe {
            let swap: IDXGISwapChain1 = native_interface(self.swap.raw_swap_chain())?;
            let buffer: ID3D11Texture2D = swap.GetBuffer(0)?;
            self.context
                .UpdateSubresource(&buffer, 0, None, pixels.as_ptr().cast(), width * 4, 0);
        }
        self.end_frame()?;
        canvas_result(self.layer.commit())
    }

    /// Canvas owns buffer binding and resizing; the renderer owns the draw bracket.
    pub fn begin_frame(&mut self, width: u32, height: u32) -> Result<ID2D1DeviceContext> {
        self.resize(width, height)?;
        Ok(self.drawing.clone())
    }

    /// Backpressure before rasterization, rather than drawing frames a full
    /// presentation queue cannot accept. The pending timer retains the redraw.
    pub fn try_begin_frame(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<Option<ID2D1DeviceContext>> {
        if let Some(at) = self.retry_at.get() {
            let remaining = at.saturating_duration_since(std::time::Instant::now());
            if !remaining.is_zero() {
                // A timer can run just before the deadline. Preserve a wakeup
                // even after that callback validated the previous paint request.
                self.schedule_retry(remaining.as_millis() as u32 + 1)?;
                return Ok(None);
            }
        }
        self.begin_frame(width, height).map(Some)
    }

    pub fn end_frame(&self) -> Result<()> {
        // DWM owns display synchronization. Never make the common UI thread wait
        // for every pane's vertical blank; a full queue retries the latest state.
        let result = unsafe { self.present.Present(0, DXGI_PRESENT_DO_NOT_WAIT) };
        if result == DXGI_ERROR_WAS_STILL_DRAWING {
            self.schedule_retry(16)?;
            self.retry_at.set(Some(
                std::time::Instant::now() + std::time::Duration::from_millis(16),
            ));
            return Ok(());
        }
        self.retry_at.set(None);
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::KillTimer(self.hwnd.0, PRESENT_RETRY);
        }
        result.ok()
    }

    fn schedule_retry(&self, millis: u32) -> Result<()> {
        if unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetTimer(
                self.hwnd.0,
                PRESENT_RETRY,
                millis,
                Some(retry_present),
            )
        } == 0
        {
            return Err(Error::from_thread());
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn readback(&self) -> Result<Vec<u8>> {
        unsafe {
            let swap: IDXGISwapChain1 = native_interface(self.swap.raw_swap_chain())?;
            let source: ID3D11Texture2D = swap.GetBuffer(0)?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            source.GetDesc(&raw mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            self.context.GetDevice()?.CreateTexture2D(
                &raw const desc,
                None,
                Some(&raw mut staging),
            )?;
            let staging = staging.unwrap();
            self.context.CopyResource(&staging, &source);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped))?;
            let mut pixels = vec![0; (desc.Width * desc.Height * 4) as usize];
            for y in 0..desc.Height as usize {
                let row = std::slice::from_raw_parts(
                    mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize),
                    desc.Width as usize * 4,
                );
                pixels[y * row.len()..(y + 1) * row.len()].copy_from_slice(row);
            }
            self.context.Unmap(&staging, 0);
            Ok(pixels)
        }
    }
}

const PRESENT_RETRY: usize = 0x4c50_4750;
unsafe extern "system" fn retry_present(
    hwnd: windows_sys::Win32::Foundation::HWND,
    _: u32,
    id: usize,
    _: u32,
) {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::KillTimer(hwnd, id);
        windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::KillTimer(self.hwnd.0, PRESENT_RETRY);
        }
    }
}

#[cfg(test)]
mod animation_tests {
    use super::*;
    #[test]
    fn warp_surface_draws_resizes_and_defers_without_losing_the_wakeup() {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let window = windows_window::Window::new("WARP rendering regression")
            .size(96, 64)
            .style(WS_POPUP)
            .ex_style(WS_EX_NOREDIRECTIONBITMAP)
            .create()
            .unwrap();
        let hwnd = HWND(window.hwnd().cast());
        let mut surface =
            Surface::new_with_device(hwnd, 1.0, GpuDevice::new_warp().unwrap()).unwrap();
        // Simulate an early retry callback: try_begin_frame must rearm a wakeup.
        surface.retry_at.set(Some(
            std::time::Instant::now() + std::time::Duration::from_millis(16),
        ));
        assert!(surface.try_begin_frame(96, 64).unwrap().is_none());
        unsafe {
            windows_sys::Win32::Graphics::Gdi::ValidateRect(hwnd.0, std::ptr::null());
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let mut message = MSG::default();
            unsafe {
                while PeekMessageW(&raw mut message, hwnd.0, WM_TIMER, WM_TIMER, PM_REMOVE) != 0 {
                    DispatchMessageW(&message);
                }
                if windows_sys::Win32::Graphics::Gdi::GetUpdateRect(hwnd.0, std::ptr::null_mut(), 0)
                    != 0
                {
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "retry lost the final redraw"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        surface.retry_at.set(None);
        for (width, height) in [(96, 64), (144, 96), (96, 64)] {
            let target = surface.try_begin_frame(width, height).unwrap().unwrap();
            super::super::canvas::draw(&target, 1.0, |frame| {
                frame.clear(windows_canvas::ColorF::new(1.0, 0.0, 0.0, 1.0));
                frame.finish()
            })
            .unwrap();
            let pixels = surface.readback().unwrap();
            assert!(
                pixels
                    .chunks_exact(4)
                    .all(|pixel| pixel == [0, 0, 255, 255])
            );
            surface.end_frame().unwrap();
            surface.retry_at.set(None);
        }
        // Destruction must cancel even a still-pending queue retry.
        surface.schedule_retry(16).unwrap();
        drop(surface);
        assert_eq!(unsafe { KillTimer(hwnd.0, PRESENT_RETRY) }, 0);
    }

    #[test]
    fn fade_applies_to_native_material_and_finishes_after_a_delayed_tick() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let window = windows_window::Window::new("Fade integration")
            .size(240, 160)
            .style(windows_sys::Win32::UI::WindowsAndMessaging::WS_POPUP)
            .ex_style(windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_NOREDIRECTIONBITMAP)
            .create()
            .unwrap();
        let hwnd = HWND(window.hwnd().cast());
        {
            let mut surface = Surface::new_with_opacity(hwnd, 0.0).unwrap();
            surface.material(hwnd, Backdrop::Acrylic);
            assert!(surface.native);
            // Material attachment and the first content upload must not expose
            // an opaque frame before the fade gets its first sample.
            assert_eq!(
                surface.acrylic.as_ref().unwrap().opacity_value().unwrap(),
                0.0
            );
            surface.present(240, 160, &[255; 240 * 160 * 4]).unwrap();
            assert_eq!(
                surface.acrylic.as_ref().unwrap().opacity_value().unwrap(),
                0.0
            );
            let fade =
                super::super::animation::Fade::new(std::time::Duration::from_millis(120)).unwrap();
            for (millis, expected) in [(0, 0.0), (60, 0.5), (800, 1.0)] {
                let opacity = fade
                    .sample(std::time::Duration::from_millis(millis))
                    .unwrap();
                surface.opacity(opacity).unwrap();
                let material = surface.acrylic.as_ref().unwrap().opacity_value().unwrap();
                assert!((material - expected).abs() < 0.001);
            }
        }
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd.0);
        }
    }
}
