use desktop_core::Backdrop;
use std::ffi::c_void;
use windows::System::DispatcherQueueController;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionColorBrush, Compositor, ContainerVisual, SpriteVisual, VisualCollection,
};
use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE, DWMSBT_MAINWINDOW, DWMSBT_NONE,
    DWMSBT_TABBEDWINDOW, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DQTAT_COM_ASTA, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, WS_EX_LAYERED,
};
use windows::core::{Interface, Result};
use windows_numerics::Vector2;

pub struct MaterialController {
    hwnd: HWND,
    _queue: DispatcherQueueController,
    compositor: Compositor,
    _target: DesktopWindowTarget,
    _root: ContainerVisual,
    background: SpriteVisual,
    _tint: SpriteVisual,
    transparent: CompositionColorBrush,
    tint_brush: CompositionColorBrush,
}

impl MaterialController {
    /// # Safety
    ///
    /// `raw_hwnd` must be a live HWND owned by the calling thread for the lifetime of the
    /// controller.
    ///
    /// # Errors
    ///
    /// Returns a Windows error if the dispatcher queue, compositor, HWND target, or initial DWM
    /// material cannot be created.
    pub unsafe fn new(raw_hwnd: *mut c_void) -> Result<Self> {
        let hwnd = HWND(raw_hwnd);
        let options = DispatcherQueueOptions {
            dwSize: u32::try_from(size_of::<DispatcherQueueOptions>()).unwrap_or(u32::MAX),
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_ASTA,
        };
        let queue = unsafe { CreateDispatcherQueueController(options)? };
        let compositor = Compositor::new()?;
        let interop: ICompositorDesktopInterop = compositor.cast()?;
        let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, true)? };

        let root = compositor.CreateContainerVisual()?;
        root.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;

        let background = compositor.CreateSpriteVisual()?;
        background.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
        let tint = compositor.CreateSpriteVisual()?;
        tint.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;

        let transparent = compositor.CreateColorBrushWithColor(Color {
            A: 0,
            R: 0,
            G: 0,
            B: 0,
        })?;
        let tint_brush = compositor.CreateColorBrushWithColor(Color {
            A: 0,
            R: 28,
            G: 34,
            B: 46,
        })?;
        background.SetBrush(&transparent)?;
        tint.SetBrush(&tint_brush)?;

        let children: VisualCollection = root.Children()?;
        children.InsertAtBottom(&background)?;
        children.InsertAtTop(&tint)?;
        target.SetRoot(&root)?;

        let controller = Self {
            hwnd,
            _queue: queue,
            compositor,
            _target: target,
            _root: root,
            background,
            _tint: tint,
            transparent,
            tint_brush,
        };
        controller.prepare_window()?;
        controller.apply(Backdrop::DEFAULT)?;
        Ok(controller)
    }

    /// Applies one of the supported native or ordinary translucent materials.
    ///
    /// # Errors
    ///
    /// Returns a Windows error when changing a DWM attribute, layered-window state, or
    /// Composition brush fails.
    pub fn apply(&self, material: Backdrop) -> Result<()> {
        match material {
            Backdrop::Mica => {
                self.set_window_alpha(None)?;
                self.set_system_backdrop(DWMSBT_MAINWINDOW)?;
                self.background.SetBrush(&self.transparent)?;
                self.set_tint(0.0)?;
            }
            Backdrop::MicaAlt => {
                self.set_window_alpha(None)?;
                self.set_system_backdrop(DWMSBT_TABBEDWINDOW)?;
                self.background.SetBrush(&self.transparent)?;
                self.set_tint(0.0)?;
            }
            Backdrop::Acrylic => {
                self.set_window_alpha(None)?;
                self.set_system_backdrop(DWMSBT_TRANSIENTWINDOW)?;
                self.background.SetBrush(&self.transparent)?;
                self.set_tint(0.0)?;
            }
            Backdrop::Translucent { opacity } => {
                self.set_system_backdrop(DWMSBT_NONE)?;
                let brush = self.compositor.CreateColorBrushWithColor(Color {
                    A: 255,
                    R: 28,
                    G: 34,
                    B: 46,
                })?;
                self.background.SetBrush(&brush)?;
                self.set_tint(0.0)?;
                self.set_window_alpha(Some(unit_to_byte(opacity)))?;
            }
        }
        Ok(())
    }

    fn prepare_window(&self) -> Result<()> {
        let enabled: i32 = 1;
        let corner: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND;
        let margins = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        unsafe {
            DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const enabled).cast(),
                u32::try_from(size_of_val(&enabled)).unwrap_or(u32::MAX),
            )?;
            DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corner).cast(),
                u32::try_from(size_of_val(&corner)).unwrap_or(u32::MAX),
            )?;
            DwmExtendFrameIntoClientArea(self.hwnd, &raw const margins)?;
        }
        Ok(())
    }

    fn set_system_backdrop(&self, value: DWM_SYSTEMBACKDROP_TYPE) -> Result<()> {
        unsafe {
            DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                (&raw const value).cast(),
                u32::try_from(size_of_val(&value)).unwrap_or(u32::MAX),
            )
        }
    }

    fn set_tint(&self, opacity: f32) -> Result<()> {
        self.tint_brush.SetColor(Color {
            A: unit_to_byte(opacity),
            R: 28,
            G: 34,
            B: 46,
        })?;
        Ok(())
    }

    fn set_window_alpha(&self, alpha: Option<u8>) -> Result<()> {
        unsafe {
            let current = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let layered = isize::try_from(WS_EX_LAYERED.0).unwrap_or_default();
            let next = match alpha {
                Some(_) => current | layered,
                None => current & !layered,
            };
            if current != next {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, next);
                SetWindowPos(
                    self.hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
                )?;
            }
            if let Some(alpha) = alpha {
                SetLayeredWindowAttributes(self.hwnd, COLORREF(0), alpha, LWA_ALPHA)?;
            }
        }
        Ok(())
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn unit_to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
