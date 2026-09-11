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
    rounded_backdrop: Option<HWND>,
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
        canvas_result(self.layer.opacity(opacity))?;
        if let Some(acrylic) = &self.acrylic {
            acrylic.opacity(opacity)?;
        }
        self.opacity.set(opacity);
        canvas_result(self.layer.commit())
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
        unsafe {
            let device = gpu_device()?;
            let d3d: ID3D11Device = native_interface(device.d3d_device())?;
            #[cfg(test)]
            let context = d3d.GetImmediateContext()?;
            let dxgi: IDXGIDevice = d3d.cast()?;
            let mut swap = canvas_result(device.create_swap_chain(1, 1))?;
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
                rounded_backdrop: None,
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
            // The renderer's outline has a 7 DIP radius and a half-DIP stroke outset.
            acrylic.round_corners(width, height, 7.5 * scale)?;
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

    pub fn end_frame(&self) -> Result<()> {
        if canvas_result(self.swap.present())? {
            Ok(())
        } else {
            // Canvas reports device loss as Ok(false); preserve the application's
            // error path instead of silently treating a missing frame as success.
            Err(Error::from_hresult(
                windows::Win32::Foundation::D2DERR_RECREATE_TARGET,
            ))
        }
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

#[cfg(test)]
mod animation_tests {
    use super::*;
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
