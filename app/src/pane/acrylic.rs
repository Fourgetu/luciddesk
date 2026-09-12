//! Host backdrop composition has no window-activation policy. Windows still controls
//! backdrop transparency through accessibility settings and power policy.
mod effects;

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
    // Release effects before their compositor and dispatcher queue.
    material_factory: RefCell<Option<windows::UI::Composition::CompositionEffectFactory>>,
    compositor: Compositor,
    _queue: DispatcherQueueController,
}

thread_local! {
    static RUNTIME: RefCell<Option<Rc<Runtime>>> = const { RefCell::new(None) };
}

pub struct Acrylic {
    material_brush: RefCell<Option<(bool, windows::UI::Composition::CompositionBrush)>>,
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
        let material = Self::new_with_opacity(hwnd, 1.0)?;
        material.material(desktop_core::Backdrop::Acrylic, true)?;
        Ok(material)
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
                material_factory: RefCell::new(None),
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
        backdrop.SetBrush(&compositor.CreateColorBrush()?)?;
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
            material_brush: RefCell::new(None),
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
        if let desktop_core::Backdrop::Solid { color, opacity } = material {
            self.material_brush.borrow_mut().take();
            let set_color =
                |visual: &windows::UI::Composition::SpriteVisual, color: Color| -> Result<()> {
                    if let Ok(brush) = visual
                        .Brush()?
                        .cast::<windows::UI::Composition::CompositionColorBrush>()
                    {
                        brush.SetColor(color)
                    } else {
                        visual.SetBrush(&compositor.CreateColorBrushWithColor(color)?)
                    }
                };
            set_color(
                &self.backdrop,
                Color {
                    A: (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
                    R: (color >> 16) as u8,
                    G: (color >> 8) as u8,
                    B: color as u8,
                },
            )?;
            set_color(
                &self.tint,
                Color {
                    A: 0,
                    R: 0,
                    G: 0,
                    B: 0,
                },
            )?;
            return self.visible(true);
        }
        let wallpaper = matches!(
            material.base(),
            desktop_core::Backdrop::Mica | desktop_core::Backdrop::MicaAlt
        );
        let (luminosity, tint) = if wallpaper {
            effects::mica_palette(
                dark,
                matches!(material.base(), desktop_core::Backdrop::MicaAlt),
            )
        } else {
            effects::acrylic_palette(dark)
        };
        let (luminosity, tint) =
            effects::adjust_strength(luminosity, tint, material.strength().unwrap_or(50));
        let brush = (|| -> Result<windows::UI::Composition::CompositionBrush> {
            let mut cached = self.material_brush.borrow_mut();
            if let Some((old_wallpaper, brush)) = cached.as_ref() {
                if *old_wallpaper == wallpaper {
                    effects::update_colors(brush, luminosity, tint)?;
                    return Ok(brush.clone());
                }
            }
            let mut factory = self._runtime.material_factory.borrow_mut();
            if factory.is_none() {
                *factory = Some(effects::factory(compositor)?);
            }
            let backdrop: windows::UI::Composition::CompositionBrush = if wallpaper {
                compositor
                    .TryCreateBlurredWallpaperBackdropBrush()?
                    .cast()?
            } else {
                compositor.CreateHostBackdropBrush()?.cast()?
            };
            let brush = effects::brush(
                compositor,
                factory.as_ref().unwrap(),
                &backdrop,
                luminosity,
                tint,
            )?;
            *cached = Some((wallpaper, brush.clone()));
            Ok(brush)
        })()
        .or_else(|_| -> Result<windows::UI::Composition::CompositionBrush> {
            // An unavailable backdrop/effect must remain opaque and legible.
            compositor
                .CreateColorBrushWithColor(effects::mica_palette(dark, false).0)?
                .cast()
        })?;
        self.backdrop.SetBrush(&brush)?;
        let clear_tint: windows::UI::Composition::CompositionColorBrush =
            self.tint.Brush()?.cast()?;
        clear_tint.SetColor(Color {
            A: 0,
            R: 0,
            G: 0,
            B: 0,
        })?;
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

    pub fn fade_in(&self) -> Result<()> {
        let animation = self._runtime.compositor.CreateScalarKeyFrameAnimation()?;
        animation.InsertKeyFrame(0.0, 0.0)?;
        animation.InsertKeyFrame(1.0, 1.0)?;
        animation.SetDuration(windows::Foundation::TimeSpan {
            Duration: (super::animation::SETTINGS_DURATION.as_nanos() / 100) as i64,
        })?;
        self.root
            .StartAnimation(&windows::core::HSTRING::from("Opacity"), &animation)
    }

    #[cfg(test)]
    pub fn assert_solid_color(&self, color: u32, opacity: f32) {
        let brush: windows::UI::Composition::CompositionColorBrush =
            self.backdrop.Brush().unwrap().cast().unwrap();
        let actual = brush.Color().unwrap();
        assert_eq!(
            (actual.R, actual.G, actual.B),
            ((color >> 16) as u8, (color >> 8) as u8, color as u8)
        );
        assert_eq!(actual.A, (opacity * 255.0).round() as u8);
        let tint: windows::UI::Composition::CompositionColorBrush =
            self.tint.Brush().unwrap().cast().unwrap();
        assert_eq!(tint.Color().unwrap().A, 0);
        assert_eq!(self.content.as_ref().unwrap().Opacity().unwrap(), 1.0);
    }

    #[cfg(test)]
    pub fn assert_material_colors(&self, material: desktop_core::Backdrop, dark: bool) {
        let (luminosity, tint) = if material.base() == desktop_core::Backdrop::Acrylic {
            effects::acrylic_palette(dark)
        } else {
            effects::mica_palette(dark, material.base() == desktop_core::Backdrop::MicaAlt)
        };
        let (luminosity, tint) =
            effects::adjust_strength(luminosity, tint, material.strength().unwrap_or(50));
        let effect: windows::UI::Composition::CompositionEffectBrush =
            self.backdrop.Brush().unwrap().cast().unwrap();
        for (name, expected) in [("Luminosity", luminosity), ("Tint", tint)] {
            let brush: windows::UI::Composition::CompositionColorBrush = effect
                .GetSourceParameter(&windows::core::HSTRING::from(name))
                .unwrap()
                .cast()
                .unwrap();
            assert_eq!(brush.Color().unwrap(), expected);
        }
    }

    #[cfg(test)]
    pub fn assert_material_effect(&self) {
        let effect: windows::UI::Composition::CompositionEffectBrush = self
            .backdrop
            .Brush()
            .unwrap()
            .cast()
            .expect("Material must use the GPU blend graph, not fallback");
        for name in ["Backdrop", "Luminosity", "Tint"] {
            effect
                .GetSourceParameter(&windows::core::HSTRING::from(name))
                .unwrap();
        }
        let tint: windows::UI::Composition::CompositionColorBrush =
            self.tint.Brush().unwrap().cast().unwrap();
        assert_eq!(
            tint.Color().unwrap().A,
            0,
            "Do not overlay the old tint twice"
        );
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
