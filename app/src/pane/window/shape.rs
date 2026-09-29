//! Downlevel HWND clipping also contains the legacy host-backdrop accent.
use windows_sys::Win32::{Foundation::HWND, Graphics::Gdi::*};

#[derive(Default)]
pub(super) struct WindowShape {
    legacy: Option<bool>,
    bounds: Option<(i32, i32, i32)>,
    pub(super) hidden: bool,
}

impl WindowShape {
    pub(super) fn update(&mut self, hwnd: HWND, width: i32, height: i32, radius: f32, scale: f32) {
        let legacy = *self.legacy.get_or_insert_with(|| {
            let mut preference = 0i32;
            unsafe {
                windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                    windows::Win32::Foundation::HWND(hwnd),
                    windows::Win32::Graphics::Dwm::DWMWA_WINDOW_CORNER_PREFERENCE,
                    (&raw mut preference).cast(),
                    size_of::<i32>() as u32,
                )
                .is_err()
            }
        });
        if !legacy {
            return;
        }
        // Match the composition outline's half-DIP outset. GDI uses a diameter.
        let diameter = if radius > 0.0 {
            ((radius + 0.5) * scale * 2.0).round() as i32
        } else {
            0
        }
        .clamp(0, width.min(height).max(0));
        let bounds = Some((width, height, diameter));
        if self.bounds == bounds {
            return;
        }
        if self.hidden || Self::apply(hwnd, bounds, false) {
            self.bounds = bounds;
        }
    }

    pub(super) fn hide(&mut self, hwnd: HWND, hidden: bool) {
        if self.hidden != hidden && Self::apply(hwnd, self.bounds, hidden) {
            self.hidden = hidden;
        }
    }

    fn apply(hwnd: HWND, bounds: Option<(i32, i32, i32)>, hidden: bool) -> bool {
        unsafe {
            let rounded = bounds.is_some_and(|(_, _, diameter)| diameter > 0);
            let region = if hidden {
                CreateRectRgn(0, 0, 0, 0)
            } else if let Some((width, height, diameter)) = bounds.filter(|_| rounded) {
                CreateRoundRectRgn(0, 0, width + 1, height + 1, diameter, diameter)
            } else {
                std::ptr::null_mut()
            };
            if (hidden || rounded) && region.is_null() {
                return false;
            }
            if SetWindowRgn(hwnd, region, 1) != 0 {
                // A successful call transfers ownership to the window manager.
                true
            } else {
                if !region.is_null() {
                    DeleteObject(region);
                }
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    #[test]
    fn downlevel_corners_follow_resize_dpi_and_drag_visibility() {
        let window = windows_window::Window::new("Downlevel corners")
            .size(240, 160)
            .style(WS_POPUP)
            .create()
            .unwrap();
        let hwnd = window.hwnd().cast();
        let mut shape = WindowShape {
            legacy: Some(true),
            ..Default::default()
        };
        unsafe {
            let region = CreateRectRgn(0, 0, 0, 0);
            shape.update(hwnd, 240, 160, 20.0, 1.0);
            assert_eq!(GetWindowRgn(hwnd, region), COMPLEXREGION);
            assert_eq!(PtInRegion(region, 0, 0), 0);
            assert_ne!(PtInRegion(region, 120, 0), 0);
            shape.hide(hwnd, true);
            assert_eq!(GetWindowRgn(hwnd, region), NULLREGION);
            shape.update(hwnd, 320, 200, 20.0, 2.0);
            assert_eq!(GetWindowRgn(hwnd, region), NULLREGION);
            shape.hide(hwnd, false);
            assert_eq!(GetWindowRgn(hwnd, region), COMPLEXREGION);
            assert_eq!(PtInRegion(region, 10, 10), 0);
            assert_ne!(PtInRegion(region, 300, 100), 0);
            shape.update(hwnd, 320, 200, 0.0, 2.0);
            assert_eq!(GetWindowRgn(hwnd, region), 0);
            DeleteObject(region);
        }
    }
}
