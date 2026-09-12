//! Host backdrop composition has no window-activation policy. Windows still controls
//! backdrop transparency through accessibility settings and power policy.
use super::native_graphics::{DWMWA_USE_HOSTBACKDROPBRUSH, set_attribute};
use std::{cell::RefCell, rc::Rc};
use windows::{
    System::DispatcherQueueController,
    UI::{
        Color,
        Composition::{
            CompositionRoundedRectangleGeometry, Compositor, ContainerVisual,
            Desktop::DesktopWindowTarget,
        },
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
    rounded_clip: Option<(CompositionRoundedRectangleGeometry, (u32, u32, f32))>,
    content: Option<windows::UI::Composition::SpriteVisual>,
}

impl Acrylic {
    #[allow(dead_code)] // Used by the standalone backdrop probe.
    pub fn new(hwnd: HWND) -> Result<Self> {
        Self::new_with_opacity(hwnd, 1.0)
    }

    pub fn new_with_opacity(hwnd: HWND, initial_opacity: f32) -> Result<Self> {
        Self::new_target(hwnd, initial_opacity, false)
    }

    pub fn new_with_content(
        hwnd: HWND,
        initial_opacity: f32,
        swap: &windows::Win32::Graphics::Dxgi::IDXGISwapChain1,
    ) -> Result<Self> {
        let mut material = Self::new_target(hwnd, initial_opacity, true)?;
        material.attach_content(swap)?;
        Ok(material)
    }

    fn new_target(hwnd: HWND, initial_opacity: f32, topmost: bool) -> Result<Self> {
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
            set_attribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &enabled)?;
        }
        let compositor = &runtime.compositor;
        let interop: ICompositorDesktopInterop = compositor.cast()?;
        // A standalone material uses the bottom target; a combined content tree
        // owns the top target so NOREDIRECTIONBITMAP windows remain visible.
        let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, topmost)? };
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
            rounded_clip: None,
            content: None,
        })
    }

    pub fn round_corners(&mut self, width: u32, height: u32, radius: f32) -> Result<()> {
        let bounds = (width, height, radius);
        if self
            .rounded_clip
            .as_ref()
            .is_some_and(|(_, old)| *old == bounds)
        {
            return Ok(());
        }
        let geometry = if let Some((geometry, _)) = &self.rounded_clip {
            geometry.clone()
        } else {
            self._runtime.compositor.CreateRoundedRectangleGeometry()?
        };
        geometry.SetSize(Vector2 {
            X: width as f32,
            Y: height as f32,
        })?;
        geometry.SetCornerRadius(Vector2 {
            X: radius,
            Y: radius,
        })?;
        if self.rounded_clip.is_none() {
            let clip = self
                ._runtime
                .compositor
                .CreateGeometricClipWithGeometry(&geometry)?;
            self.root.SetClip(&clip)?;
        }
        self.rounded_clip = Some((geometry, bounds));
        Ok(())
    }

    pub fn material(&self, material: desktop_core::Backdrop, dark: bool) -> Result<()> {
        let compositor = &self._runtime.compositor;
        let wallpaper = matches!(
            material,
            desktop_core::Backdrop::Mica | desktop_core::Backdrop::MicaAlt
        );
        let brush = if wallpaper {
            compositor.TryCreateBlurredWallpaperBackdropBrush()?
        } else {
            compositor.CreateHostBackdropBrush()?
        };
        self.backdrop.SetBrush(&brush)?;
        let color = Color {
            A: match material {
                desktop_core::Backdrop::Mica => 205,
                desktop_core::Backdrop::MicaAlt => 165,
                _ => 120,
            },
            R: if dark { 24 } else { 245 },
            G: if dark { 27 } else { 246 },
            B: if dark { 32 } else { 248 },
        };
        self.tint
            .SetBrush(&compositor.CreateColorBrushWithColor(color)?)?;
        self.visible(true)
    }

    pub fn visible(&self, visible: bool) -> Result<()> {
        // Content can share this tree; switching to a plain background must
        // hide only the material, never the application's content visual.
        self.backdrop.SetIsVisible(visible)?;
        self.tint.SetIsVisible(visible)
    }

    fn attach_content(
        &mut self,
        swap: &windows::Win32::Graphics::Dxgi::IDXGISwapChain1,
    ) -> Result<()> {
        use windows::Win32::System::WinRT::Composition::ICompositorInterop;
        let compositor = &self._runtime.compositor;
        let interop: ICompositorInterop = compositor.cast()?;
        let surface = unsafe { interop.CreateCompositionSurfaceForSwapChain(swap)? };
        let brush = compositor.CreateSurfaceBrushWithSurface(&surface)?;
        let visual = compositor.CreateSpriteVisual()?;
        visual.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
        visual.SetBrush(&brush)?;
        self.root.Children()?.InsertAtTop(&visual)?;
        self.content = Some(visual);
        Ok(())
    }

    pub fn commit_ready(&self) -> Result<Box<dyn Fn() -> bool>> {
        let commit = self._runtime.compositor.RequestCommitAsync()?;
        Ok(Box::new(move || {
            commit.Status().map_or(true, |status| status.0 != 0)
        }))
    }

    pub fn opacity(&self, opacity: f32) -> Result<()> {
        self.root.SetOpacity(opacity)
    }

    #[cfg(test)]
    pub fn assert_content_visible(&self) {
        assert!(self._target.IsTopmost().unwrap());
        assert!(self.root.IsVisible().unwrap());
        assert!(self.content.as_ref().unwrap().IsVisible().unwrap());
    }

    #[cfg(test)]
    pub fn opacity_value(&self) -> Result<f32> {
        self.root.Opacity()
    }
}
