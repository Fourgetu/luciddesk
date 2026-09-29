//! Host backdrop composition has no window-activation policy. Windows still controls
//! backdrop transparency through accessibility settings and power policy.
#[path = "acrylic/effects.rs"]
mod effects;
#[path = "acrylic/host.rs"]
mod host;

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

struct PowerNotification(windows_sys::Win32::System::Power::HPOWERNOTIFY);
impl Drop for PowerNotification {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { windows_sys::Win32::System::Power::UnregisterPowerSettingNotification(self.0); }
        }
    }
}

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
    hwnd: HWND,
    _power_notification: PowerNotification,
    ui_settings: Option<windows::UI::ViewManagement::UISettings>,
    #[cfg(test)]
    effects_override: std::cell::Cell<Option<bool>>,
    disable_backdrop: bool,
    material_brush: RefCell<Option<(bool, windows::UI::Composition::CompositionBrush)>>,
    host_backdrop: RefCell<Option<host::HostBackdrop>>,
    #[cfg(test)]
    unavailable_backdrops: std::cell::Cell<(bool, bool)>,
    runtime: Rc<Runtime>,
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
            hwnd,
            _power_notification: PowerNotification(unsafe {
                windows_sys::Win32::System::Power::RegisterPowerSettingNotification(
                    hwnd.0,
                    &windows_sys::Win32::System::SystemServices::GUID_POWER_SAVING_STATUS,
                    windows_sys::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE,
                )
            }),
            ui_settings: windows::UI::ViewManagement::UISettings::new().ok(),
            #[cfg(test)]
            // Rendering tests control policy explicitly, independent of the
            // test machine's current transparency or battery-saver setting.
            effects_override: std::cell::Cell::new(Some(true)),
            disable_backdrop: crate::diagnostics::disable_backdrop(),
            material_brush: RefCell::new(None),
            host_backdrop: RefCell::new(None),
            #[cfg(test)]
            unavailable_backdrops: std::cell::Cell::new((false, false)),
            runtime: runtime,
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
            self.runtime.compositor.CreateRoundedRectangleGeometry()?
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
                .runtime
                .compositor
                .CreateGeometricClipWithGeometry(&geometry)?;
            self.root.SetClip(&clip)?;
        }
        self.rounded_clip = Some((geometry, bounds));
        Ok(())
    }

    pub(super) fn effects_enabled(&self) -> bool {
        #[cfg(test)]
        if let Some(enabled) = self.effects_override.get() {
            return enabled;
        }
        let advanced = self.ui_settings.as_ref()
            .and_then(|settings| settings.AdvancedEffectsEnabled().ok())
            .unwrap_or(true);
        let mut power = windows_sys::Win32::System::Power::SYSTEM_POWER_STATUS::default();
        let saver = unsafe {
            windows_sys::Win32::System::Power::GetSystemPowerStatus(&raw mut power) != 0
                && power.SystemStatusFlag == 1
        };
        advanced && !saver
    }

    #[cfg(test)]
    pub(super) fn set_effects_enabled_for_test(&self, enabled: bool) {
        self.effects_override.set(Some(enabled));
    }

    pub fn material(&self, material: desktop_core::Backdrop, dark: bool) -> Result<()> {
        let compositor = &self.runtime.compositor;
        if let desktop_core::Backdrop::Solid { color, opacity } = material {
            self.material_brush.borrow_mut().take();
            self.host_backdrop.borrow_mut().take();
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
        let brush = self
            .effect_brush(material, dark, wallpaper)
            .or_else(|error| {
                if wallpaper {
                    // Use the acrylic recipe and retain the requested strength without
                    // changing the persisted Mica/Mica Alt preference.
                    self.effect_brush(
                        desktop_core::Backdrop::Acrylic
                            .with_strength(material.strength().unwrap_or(50)),
                        dark,
                        false,
                    )
                } else {
                    Err(error)
                }
            })
            .or_else(|error| -> Result<windows::UI::Composition::CompositionBrush> {
                crate::diagnostics::render_trace(format_args!("hwnd={:?} opaque fallback: {error}", self.hwnd));
                // An unavailable backdrop/effect must remain opaque and legible.
                self.material_brush.borrow_mut().take();
                self.host_backdrop.borrow_mut().take();
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

    fn effect_brush(
        &self,
        material: desktop_core::Backdrop,
        dark: bool,
        wallpaper: bool,
    ) -> Result<windows::UI::Composition::CompositionBrush> {
        if self.disable_backdrop || !self.effects_enabled() {
            return Err(windows::Win32::Foundation::E_NOTIMPL.into());
        }
        #[cfg(test)]
        if if wallpaper {
            self.unavailable_backdrops.get().0
        } else {
            self.unavailable_backdrops.get().1
        } {
            return Err(windows::Win32::Foundation::E_NOTIMPL.into());
        }
        let compositor = &self.runtime.compositor;
        let (luminosity, tint) = material_colors(material, dark);
        if !wallpaper && self.host_backdrop.borrow().is_none() {
            *self.host_backdrop.borrow_mut() = Some(host::HostBackdrop::enable(self.hwnd)?);
        }
        let mut cached = self.material_brush.borrow_mut();
        if let Some((old_wallpaper, brush)) = cached.as_ref()
            && *old_wallpaper == wallpaper
        {
            effects::update_colors(brush, luminosity, tint)?;
            return Ok(brush.clone());
        }
        let mut factory = self.runtime.material_factory.borrow_mut();
        if factory.is_none() {
            *factory = Some(effects::factory(compositor)?);
        }
        let backdrop: windows::UI::Composition::CompositionBrush = if wallpaper {
            // Downlevel systems reject this attribute; wallpaper availability is
            // decided by the brush API, independently of composition setup.
            let _ = unsafe { set_attribute(self.hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &1i32) };
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
    }

    pub fn visible(&self, visible: bool) -> Result<()> {
        // Content can share this tree; switching to a plain background must
        // hide only the material, never the application's content visual.
        self.backdrop.SetIsVisible(visible)?;
        self.tint.SetIsVisible(visible)?;
        if !visible {
            self.host_backdrop.borrow_mut().take();
        }
        Ok(())
    }

    fn attach_content(
        &mut self,
        swap: &windows::Win32::Graphics::Dxgi::IDXGISwapChain1,
    ) -> Result<()> {
        use windows::Win32::System::WinRT::Composition::ICompositorInterop;
        let compositor = &self.runtime.compositor;
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
        let commit = self.runtime.compositor.RequestCommitAsync()?;
        Ok(Box::new(move || {
            commit.Status().map_or(true, |status| status.0 != 0)
        }))
    }

    pub fn opacity(&self, opacity: f32) -> Result<()> {
        self.root.SetOpacity(opacity)
    }

    pub fn fade_in(&self) -> Result<()> {
        let animation = self.runtime.compositor.CreateScalarKeyFrameAnimation()?;
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

    #[cfg(test)]
    pub fn assert_rounded_clip(&self, width: u32, height: u32, radius: f32) {
        let (geometry, bounds) = self.rounded_clip.as_ref().unwrap();
        assert_eq!(*bounds, (width, height, radius));
        assert!(self.root.Clip().is_ok());
        assert_eq!(geometry.Size().unwrap(), Vector2 { X: width as f32, Y: height as f32 });
        assert_eq!(geometry.CornerRadius().unwrap(), Vector2 { X: radius, Y: radius });
    }
}

/// Shared material recipe for the compositor and the illustrative settings preview.
pub(super) fn material_colors(material: desktop_core::Backdrop, dark: bool) -> (Color, Color) {
    let colors = if matches!(
        material.base(),
        desktop_core::Backdrop::Mica | desktop_core::Backdrop::MicaAlt
    ) {
        effects::mica_palette(dark, material.base() == desktop_core::Backdrop::MicaAlt)
    } else {
        effects::acrylic_palette(dark)
    };
    effects::adjust_strength(colors.0, colors.1, material.strength().unwrap_or(50))
}

#[cfg(test)]
mod tests {
    use super::*;
    use desktop_core::Backdrop;

    #[test]
    fn diagnostic_bypass_never_enables_host_or_wallpaper_backdrops() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let window = windows_window::Window::new("Backdrop isolation")
            .size(96, 64)
            .style(windows_sys::Win32::UI::WindowsAndMessaging::WS_POPUP)
            .ex_style(windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_NOREDIRECTIONBITMAP)
            .create().unwrap();
        let device = super::super::native_graphics::gpu_device().unwrap();
        let swap = device.create_swap_chain(96, 64).unwrap();
        let native_swap = super::super::native_graphics::native_interface(swap.raw_swap_chain()).unwrap();
        let mut acrylic = Acrylic::new_with_content(HWND(window.hwnd().cast()), 1.0, &native_swap).unwrap();
        for disabled_by_policy in [false, true] {
            acrylic.disable_backdrop = !disabled_by_policy;
            acrylic.effects_override.set(Some(!disabled_by_policy));
            for dark in [true, false] {
                for material in [Backdrop::Acrylic, Backdrop::Mica, Backdrop::MicaAlt] {
                    for strength in [0, 50, 100] {
                        acrylic.material(material.with_strength(strength), dark).unwrap();
                        acrylic.assert_solid_color(if dark { 0x202020 } else { 0xf3f3f3 }, 1.0);
                        assert!(acrylic.host_backdrop.borrow().is_none());
                        assert!(acrylic.material_brush.borrow().is_none());
                        acrylic.assert_content_visible();
                    }
                }
            }
        }
        acrylic.disable_backdrop = false;
        acrylic.effects_override.set(Some(true));
        acrylic.material(Backdrop::Acrylic, false).unwrap();
        acrylic.assert_material_effect();
        acrylic.assert_content_visible();
    }

    #[test]
    fn missing_wallpaper_uses_acrylic_then_opaque_color_without_hiding_content() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let window = windows_window::Window::new("Material fallback regression")
            .size(96, 64)
            .style(windows_sys::Win32::UI::WindowsAndMessaging::WS_POPUP)
            .ex_style(windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_NOREDIRECTIONBITMAP)
            .create()
            .unwrap();
        let device = super::super::native_graphics::gpu_device().unwrap();
        let swap = device.create_swap_chain(96, 64).unwrap();
        let native_swap =
            super::super::native_graphics::native_interface(swap.raw_swap_chain()).unwrap();
        let acrylic =
            Acrylic::new_with_content(HWND(window.hwnd().cast()), 1.0, &native_swap).unwrap();
        acrylic.unavailable_backdrops.set((true, false));
        for dark in [false, true] {
            for requested in [Backdrop::Mica, Backdrop::MicaAlt] {
                for strength in [0, 25, 50, 75, 100] {
                    let requested = requested.with_strength(strength);
                    acrylic.material(requested, dark).unwrap();
                    assert!(!acrylic.material_brush.borrow().as_ref().unwrap().0);
                    acrylic.assert_material_effect();
                    acrylic.assert_material_colors(
                        Backdrop::Acrylic.with_strength(requested.strength().unwrap_or(50)),
                        dark,
                    );
                    acrylic.assert_content_visible();
                }
            }
            // Fault injection must also bypass a previously cached acrylic brush.
            acrylic.unavailable_backdrops.set((true, true));
            acrylic.material(Backdrop::Mica, dark).unwrap();
            acrylic.assert_solid_color(if dark { 0x0020_2020 } else { 0x00f3_f3f3 }, 1.0);
            assert!(acrylic.material_brush.borrow().is_none());
            assert!(acrylic.host_backdrop.borrow().is_none());
            acrylic.assert_content_visible();
            acrylic.unavailable_backdrops.set((true, false));
        }
        acrylic.material(Backdrop::Mica, true).unwrap();
        let cached = acrylic.material_brush.borrow().as_ref().unwrap().1.clone();
        acrylic.visible(false).unwrap();
        assert!(acrylic.host_backdrop.borrow().is_none());
        acrylic.material(Backdrop::Mica, true).unwrap();
        assert_eq!(acrylic.material_brush.borrow().as_ref().unwrap().1, cached);
        assert!(acrylic.host_backdrop.borrow().is_some());
        acrylic
            .material(
                Backdrop::Solid {
                    color: 0x0012_3456,
                    opacity: 0.5,
                },
                true,
            )
            .unwrap();
        assert!(acrylic.host_backdrop.borrow().is_none());
        acrylic.assert_solid_color(0x0012_3456, 0.5);
        acrylic.unavailable_backdrops.set((false, false));
        acrylic.material(Backdrop::Acrylic, true).unwrap();
        acrylic.assert_material_effect();
        acrylic.assert_material_colors(Backdrop::Acrylic, true);
        acrylic.assert_content_visible();
    }
}
