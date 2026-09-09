//! Transparent content surface over the system-owned backdrop; no whole-window alpha.
#![allow(clippy::wildcard_imports)]
use desktop_core::Backdrop;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Dxgi::{Common::*, *};
use windows::Win32::UI::Controls::MARGINS;
use windows::core::{Interface, Result};

pub struct Surface {
    dark: bool,
    acrylic: Option<super::acrylic::Acrylic>,
    _device: ID3D11Device,
    context: ID3D11DeviceContext,
    drawing: ID2D1DeviceContext,
    composition: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    opacity_effect: IDCompositionEffectGroup,
    swap: IDXGISwapChain1,
    size: (u32, u32),
    material: Option<Backdrop>,
    pub native: bool,
}

impl Surface {
    /// Panes draw their own outline; suppress the activation-dependent DWM frame.
    /// Keep this separate from shared surface initialization so flyout shadows remain.
    pub fn disable_window_shadow(hwnd: HWND) -> Result<()> {
        let policy = DWMNCRP_DISABLED;
        unsafe { DwmSetWindowAttribute(hwnd, DWMWA_NCRENDERING_POLICY, (&raw const policy).cast(), 4) }
    }

    pub fn opacity(&self, opacity: f32) -> Result<()> {
        unsafe {
            self.opacity_effect.SetOpacity2(opacity)?;
        }
        if let Some(acrylic) = &self.acrylic {
            acrylic.opacity(opacity)?;
        }
        unsafe { self.composition.Commit() }
    }
    pub fn new(hwnd: HWND) -> Result<Self> {
        unsafe {
            let mut device = None;
            let mut context = None;
            let mut result = Ok(());
            for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
                result = D3D11CreateDevice(
                    None,
                    driver,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&raw mut device),
                    None,
                    Some(&raw mut context),
                );
                if result.is_ok() {
                    break;
                }
            }
            result?;
            let device = device.unwrap();
            let dxgi: IDXGIDevice = device.cast()?;
            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let drawing = d2d
                .CreateDevice(&dxgi)?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
            let swap = factory.CreateSwapChainForComposition(
                &device,
                &DXGI_SWAP_CHAIN_DESC1 {
                    Width: 1,
                    Height: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 2,
                    Scaling: DXGI_SCALING_STRETCH,
                    SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                    AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                    ..Default::default()
                },
                None,
            )?;
            let composition: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = composition.CreateTargetForHwnd(hwnd, true)?;
            let visual = composition.CreateVisual()?;
            let opacity_effect = composition.CreateEffectGroup()?;
            opacity_effect.SetOpacity2(1.0)?;
            visual.SetEffect(&opacity_effect)?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            composition.Commit()?;
            let margins = MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            };
            DwmExtendFrameIntoClientArea(hwnd, &raw const margins)?;
            let dark = 1i32;
            // The content renderer owns the single border. Suppress the second DWM outline.
            let border = 0xffff_fffeu32;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, (&raw const border).cast(), 4);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const dark).cast(),
                4,
            );
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corner).cast(),
                4,
            );
            Ok(Self {
                dark: true,
                acrylic: None,
                _device: device,
                context: context.unwrap(),
                drawing,
                composition,
                _target: target,
                _visual: visual,
                opacity_effect,
                swap,
                size: (1, 1),
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
            unsafe { let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&raw const value).cast(), 4); }
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
        self.native = unsafe {
            DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, (&raw const kind).cast(), 4)
                .is_ok()
        } && kind != DWMSBT_NONE;
        if matches!(material, Backdrop::Acrylic | Backdrop::Mica | Backdrop::MicaAlt) {
            if self.acrylic.is_none() {
                self.acrylic = super::acrylic::Acrylic::new(hwnd).ok();
            }
            self.native = self
                .acrylic
                .as_ref()
                .is_some_and(|acrylic| acrylic.material(material, self.dark).is_ok());
            if !self.native && let Some(acrylic) = &self.acrylic {
                let _ = acrylic.visible(false);
            }
        } else if let Some(acrylic) = &self.acrylic {
            let _ = acrylic.visible(false);
        }
        self.material = Some(material);
    }

    pub fn present(&mut self, width: u32, height: u32, pixels: &[u8]) -> Result<()> {
        unsafe {
            if self.size != (width, height) {
                self.swap.ResizeBuffers(
                    2,
                    width,
                    height,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )?;
                self.size = (width, height);
            }
            let buffer: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            self.context
                .UpdateSubresource(&buffer, 0, None, pixels.as_ptr().cast(), width * 4, 0);
            self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
            self.composition.Commit()?;
            Ok(())
        }
    }

    /// Bind the current back buffer without copying a full-window CPU bitmap.
    pub fn begin_frame(&mut self, width: u32, height: u32) -> Result<ID2D1RenderTarget> {
        unsafe {
            self.drawing.SetTarget(None::<&ID2D1Image>);
            if self.size != (width, height) {
                self.swap.ResizeBuffers(
                    2,
                    width,
                    height,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )?;
                self.size = (width, height);
            }
            let buffer: IDXGISurface = self.swap.GetBuffer(0)?;
            let bitmap = self.drawing.CreateBitmapFromDxgiSurface(
                &buffer,
                Some(&D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: Common::D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: Common::D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    ..Default::default()
                }),
            )?;
            self.drawing.SetTarget(&bitmap);
            self.drawing
                .SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            self.drawing.cast()
        }
    }

    pub fn end_frame(&self) -> Result<()> {
        unsafe {
            self.drawing.SetTarget(None::<&ID2D1Image>);
            self.swap.Present(1, DXGI_PRESENT(0)).ok()
        }
    }

    #[cfg(test)]
    pub fn readback(&self) -> Result<Vec<u8>> {
        unsafe {
            let source: ID3D11Texture2D = self.swap.GetBuffer(0)?;
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
