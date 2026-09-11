//! Neutral desktop selection; folder-view themes use a different, blue selection.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
use super::assets::Pixels;

pub fn is_dark(theme: desktop_core::PanelTheme) -> bool {
    match theme {
        desktop_core::PanelTheme::Dark => true,
        desktop_core::PanelTheme::Light => false,
        desktop_core::PanelTheme::System => {
            use windows::UI::ViewManagement::{UISettings, UIColorType};
            UISettings::new().and_then(|s| s.GetColorValue(UIColorType::Foreground))
                .map_or(true, |c| u32::from(c.R) + u32::from(c.G) + u32::from(c.B) > 384)
        }
    }
}

/// Fits the highlight to icon and measured label, excluding the inter-row gap.
pub fn selection_height(icon_size: f32, text_height: f32, cell_height: f32) -> f32 {
    (icon_size + super::layout::LABEL_OFFSET + 1.0 + text_height).min(cell_height - 2.0)
}

pub fn selection(width: u32, height: u32, dpi: u32, state: i32) -> Option<Pixels> {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return None;
    }
    let opacity = match state {
        2 => 0.10,
        5 => 0.17,
        6 => 0.24,
        _ => 0.21,
    };
    let radius = (dpi as f32 / 96.0).max(1.0);
    let mut data = vec![0; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let dx = (radius - (x as f32 + 0.5).min(width as f32 - x as f32 - 0.5)).max(0.0);
            let dy = (radius - (y as f32 + 0.5).min(height as f32 - y as f32 - 0.5)).max(0.0);
            let coverage = (radius + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0);
            let alpha = (255.0 * opacity * coverage).round() as u8;
            let at = ((y * width + x) * 4) as usize;
            data[at..at + 4].fill(alpha);
        }
    }
    Some(Pixels {
        width,
        height,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_selection_is_neutral_and_fits_text_instead_of_grid_spacing() {
        assert!((selection_height(48.0, 16.0, 98.0) - 71.0).abs() < f32::EPSILON);
        assert!((selection_height(48.0, 32.0, 98.0) - 87.0).abs() < f32::EPSILON);
        assert!(selection_height(48.0, 64.0, 98.0) < 98.0);
        let mut alphas = Vec::new();
        for state in [2, 5, 3, 6] {
            let pixels = selection(112, 103, 144, state).unwrap();
            assert!(
                pixels
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|p| p[0] == p[1] && p[1] == p[2] && p[2] == p[3])
            );
            alphas.push(pixels.data[4 * (50 * 112 + 50) + 3]);
        }
        assert!(alphas.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
