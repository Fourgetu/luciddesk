//! Host backdrop composition has no window-activation policy. Windows still controls
//! backdrop transparency through accessibility settings and power policy.
use super::native_graphics::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute};
use std::{cell::RefCell, rc::Rc};
use windows::{
    System::DispatcherQueueController,
    UI::{
        Color,
        Composition::{Compositor, ContainerVisual, Desktop::DesktopWindowTarget},
    },
    Win32::{
        Foundation::HWND,
        System::WinRT::{
            Composition::ICompositorDesktopInterop, CreateDispatcherQueueController,
            DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions,
        },
    },
    core::{Interface, Result},
};
use windows_numerics::Vector2;

struct Runtime {
    compositor: Compositor,
    _queue: DispatcherQueueController,
}

thread_local! {
    static RUNTIME: RefCell<Option<Rc<Runtime>>> = const { RefCell::new(None) };
}

pub struct Acrylic {
    _runtime: Rc<Runtime>,
    _target: DesktopWindowTarget,
    root: ContainerVisual,
    backdrop: windows::UI::Composition::SpriteVisual,
    tint: windows::UI::Composition::SpriteVisual,
}

impl Acrylic {
    #[allow(dead_code)] // Used by the standalone backdrop probe.
    pub fn new(hwnd: HWND) -> Result<Self> {
        Self::new_with_opacity(hwnd, 1.0)
    }

    pub fn new_with_opacity(hwnd: HWND, initial_opacity: f32) -> Result<Self> {
        let runtime = RUNTIME.with(|slot| -> Result<Rc<Runtime>> {
            let mut slot = slot.borrow_mut();
            if let Some(runtime) = slot.as_ref() {
                return Ok(Rc::clone(runtime));
            }
            let queue = unsafe {
                CreateDispatcherQueueController(DispatcherQueueOptions {
                    dwSize: u32::try_from(size_of::<DispatcherQueueOptions>()).unwrap(),
                    threadType: DQTYPE_THREAD_CURRENT,
                    apartmentType: DQTAT_COM_NONE,
                })?
            };
            let runtime = Rc::new(Runtime {
                compositor: Compositor::new()?,
                _queue: queue,
            });
            *slot = Some(Rc::clone(&runtime));
            Ok(runtime)
        })?;
        let enabled = 1i32;
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_HOSTBACKDROPBRUSH,
                (&raw const enabled).cast(),
                4,
            )?;
        }
        let compositor = &runtime.compositor;
        let interop: ICompositorDesktopInterop = compositor.cast()?;
        // Bottom target is reserved for the material; the D3D content target stays on top.
        let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, false)? };
        let root = compositor.CreateContainerVisual()?;
        root.SetOpacity(initial_opacity)?;
        root.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
        let backdrop = compositor.CreateSpriteVisual()?;
        backdrop.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
        backdrop.SetBrush(&compositor.CreateHostBackdropBrush()?)?;
        let tint = compositor.CreateSpriteVisual()?;
        tint.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
        tint.SetBrush(&compositor.CreateColorBrushWithColor(Color {
            A: 120,
            R: 24,
            G: 27,
            B: 32,
        })?)?;
        root.Children()?.InsertAtBottom(&backdrop)?;
        root.Children()?.InsertAtTop(&tint)?;
        target.SetRoot(&root)?;
        Ok(Self {
            _runtime: runtime,
            _target: target,
            root,
            backdrop,
            tint,
        })
    }

    pub fn material(&self, material: desktop_core::Backdrop, dark: bool) -> Result<()> {
        let compositor = &self._runtime.compositor;
        let wallpaper = matches!(material, desktop_core::Backdrop::Mica | desktop_core::Backdrop::MicaAlt);
        let brush = if wallpaper {
            compositor.TryCreateBlurredWallpaperBackdropBrush()?
        } else {
            compositor.CreateHostBackdropBrush()?
        };
        self.backdrop.SetBrush(&brush)?;
        self.tint.SetBrush(&compositor.CreateColorBrushWithColor(Color {
            A: match material {
                desktop_core::Backdrop::Mica => 205,
                desktop_core::Backdrop::MicaAlt => 165,
                _ => 120,
            },
            R: if dark { 24 } else { 245 },
            G: if dark { 27 } else { 246 },
            B: if dark { 32 } else { 248 },
        })?)?;
        self.visible(true)
    }

    pub fn visible(&self, visible: bool) -> Result<()> {
        self.root.SetIsVisible(visible)
    }
    pub fn opacity(&self, opacity: f32) -> Result<()> {
        self.root.SetOpacity(opacity)
    }

    #[cfg(test)]
    pub fn opacity_value(&self) -> Result<f32> {
        self.root.Opacity()
    }
}
