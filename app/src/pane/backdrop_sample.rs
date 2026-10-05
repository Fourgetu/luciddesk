//! Measures the desktop pixels visible behind a pane whose own material paints
//! almost nothing, so its text color can still be judged against a background
//! the user actually sees.
//!
//! Only a flat color made (nearly) transparent needs this. Every other material
//! draws a theme-colored plate of its own, and a fully opaque color hides the
//! desktop completely, so neither depends on what is behind the window.
#![allow(clippy::cast_precision_loss)]
use luciddesk_core::Backdrop;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{CLR_INVALID, GetDC, GetPixel, ReleaseDC};
use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

/// Columns and rows of probe points inside the pane header.
///
/// The header carries only the title and at most four small buttons, so the
/// desktop still dominates it — and its median ignores them — even on a pane
/// packed with icons. The body does not, which is why the header is measured.
const COLUMNS: i32 = 5;
const ROWS: i32 = 2;

/// Whether a pane's text color depends on the desktop behind it.
pub(super) fn needed(backdrop: Backdrop, native: bool) -> bool {
    native && matches!(backdrop, Backdrop::Solid { opacity, .. } if opacity < 1.0)
}

/// Relative luminance of the desktop behind `hwnd`, or `None` when no probe
/// point could be read.
///
/// The median ignores the probe points that land on the pane's own title, tab
/// strip or header buttons, which are a minority of the header.
pub(super) fn behind(hwnd: HWND, scale: f32) -> Option<f32> {
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &raw mut rect) } == 0
        || rect.right - rect.left < 8
        || rect.bottom - rect.top < 6
    {
        return None;
    }
    let inset = (super::layout::HEADER_INSET * scale).round().max(1.0) as i32;
    let top = (rect.top + inset).min(rect.bottom - 1);
    let band = (super::layout::HEADER * scale).round() as i32;
    let bottom = (rect.top + band - inset).clamp(top + 1, rect.bottom - 1);
    let (left, right) = (rect.left + 2, rect.right - 2);
    // The screen device context returns the composited desktop, so a pane that
    // paints nothing here reports the wallpaper underneath it.
    let dc = unsafe { GetDC(std::ptr::null_mut()) };
    if dc.is_null() {
        return None;
    }
    let mut samples = Vec::with_capacity((COLUMNS * ROWS) as usize);
    for row in 0..ROWS {
        for column in 0..COLUMNS {
            let x = left + (right - left) * (2 * column + 1) / (2 * COLUMNS);
            let y = top + (bottom - top) * (2 * row + 1) / (2 * ROWS);
            let pixel = unsafe { GetPixel(dc, x, y) };
            if pixel == CLR_INVALID {
                continue;
            }
            samples.push(luminance(pixel));
        }
    }
    unsafe { ReleaseDC(std::ptr::null_mut(), dc) };
    samples.sort_by(f32::total_cmp);
    samples.get(samples.len() / 2).copied()
}

/// COLORREF is 0x00BBGGRR, in the same WCAG relative luminance `theme` uses.
fn luminance(pixel: u32) -> f32 {
    [(pixel & 0xff), (pixel >> 8 & 0xff), (pixel >> 16 & 0xff)]
        .into_iter()
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(channel, weight)| {
            let c = channel as f32 / 255.0;
            weight
                * if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_matches_the_theme_transfer_function() {
        assert!((luminance(0x000000) - 0.0).abs() < f32::EPSILON);
        assert!((luminance(0x00ffffff) - 1.0).abs() < 0.0001);
        // COLORREF packs blue in the high byte and red in the low one.
        assert!(luminance(0x000000ff) > luminance(0x00ff0000));
    }

    #[test]
    fn only_a_transparent_flat_color_needs_the_desktop() {
        let clear = Backdrop::Solid {
            color: 0xf3f3f3,
            opacity: 0.0,
        };
        let opaque = Backdrop::Solid {
            color: 0xf3f3f3,
            opacity: 1.0,
        };
        assert!(needed(clear, true));
        assert!(!needed(opaque, true));
        assert!(
            !needed(clear, false),
            "a pane without a native material draws its own opaque plate"
        );
        assert!(!needed(Backdrop::Mica, true));
        assert!(!needed(Backdrop::Acrylic, true));
    }
}
