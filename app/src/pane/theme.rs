//! Neutral desktop selection; folder-view themes use a different, blue selection.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
use super::assets::Pixels;

/// A quiet neutral edge, independent of the user's text color override.
pub fn panel_border(dark: bool, backdrop: desktop_core::Backdrop) -> windows_canvas::ColorF {
    let opacity = match backdrop {
        desktop_core::Backdrop::Solid { opacity, .. }
        | desktop_core::Backdrop::Translucent { opacity } => opacity.clamp(0.0, 1.0),
        // Keep the existing border at the default strength (50), fading to
        // zero with the material and strengthening it toward the opaque end.
        other => f32::from(other.strength().unwrap_or(50)) / 50.0,
    };
    if dark {
        windows_canvas::ColorF::new(0.6, 0.6, 0.6, 0.14 * opacity)
    } else {
        windows_canvas::ColorF::new(0.0, 0.0, 0.0, 0.16 * opacity)
    }
}

/// Content-only colors. Never changes the material's theme or samples the desktop.
pub struct PanelContrast {
    pub light_text: bool,
    pub scrim: f32,
}

impl PanelContrast {
    pub fn ink(&self) -> f32 {
        if self.light_text { 1.0 } else { 0.0 }
    }
    pub fn base(&self) -> f32 {
        if self.light_text { 0.0 } else { 1.0 }
    }
}

fn luminance(rgb: [f32; 3]) -> f32 {
    rgb.into_iter()
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(c, weight)| {
            weight
                * if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}

pub fn panel_contrast(
    backdrop: desktop_core::Backdrop,
    dark: bool,
    mode: desktop_core::PanelText,
    native: bool,
) -> PanelContrast {
    use desktop_core::{Backdrop, PanelText};
    let forced = match mode {
        PanelText::Auto => None,
        PanelText::Light => Some(true),
        PanelText::Dark => Some(false),
    };
    // Standard material recipes already provide a stable theme-colored base.
    // Add protection for custom transparency/strength and manual overrides.
    if !native
        || (!matches!(
            backdrop,
            Backdrop::Solid { .. } | Backdrop::Translucent { .. }
        ) && backdrop.strength().unwrap_or(50) >= 50
            && forced.is_none_or(|light| light == dark))
    {
        return PanelContrast {
            light_text: forced.unwrap_or(dark),
            scrim: 0.0,
        };
    }
    let (low, high, nominal) = if let Backdrop::Solid { color, opacity } = backdrop {
        let rgb = [
            (color >> 16 & 255) as f32 / 255.0,
            (color >> 8 & 255) as f32 / 255.0,
            (color & 255) as f32 / 255.0,
        ];
        let alpha = opacity.clamp(0.0, 1.0);
        (
            rgb.map(|c| c * alpha),
            rgb.map(|c| c * alpha + 1.0 - alpha),
            rgb,
        )
    } else {
        // Estimate how far a weakened native recipe can depart from its theme
        // base. This changes continuously with strength, rather than jumping
        // from a full scrim to none at the default value. Solid colors above
        // use exact compositing bounds; native effects remain an estimate.
        let base = if dark { 32.0 / 255.0 } else { 243.0 / 255.0 };
        let coverage = (f32::from(backdrop.strength().unwrap_or(0)) / 50.0).min(1.0);
        (
            [base * coverage; 3],
            [base * coverage + 1.0 - coverage; 3],
            [base; 3],
        )
    };
    let light = forced.unwrap_or_else(|| luminance(nominal) < 0.179);
    let worst = if light { high } else { low };
    let contrast = |alpha: f32| {
        let background =
            luminance(worst.map(|c| c * (1.0 - alpha) + if light { 0.0 } else { alpha }));
        if light {
            1.05 / (background + 0.05)
        } else {
            (background + 0.05) / 0.05
        }
    };
    // Leave contrast headroom for secondary text and selection fills, including
    // black/white content behind a transparent solid material.
    let mut lower = 0.0;
    let mut upper = 1.0;
    if contrast(0.0) >= 7.0 {
        upper = 0.0;
    } else {
        for _ in 0..16 {
            let mid = (lower + upper) * 0.5;
            if contrast(mid) >= 7.0 {
                upper = mid;
            } else {
                lower = mid;
            }
        }
    }
    PanelContrast {
        light_text: light,
        scrim: upper,
    }
}

pub fn is_dark(theme: desktop_core::PanelTheme) -> bool {
    match theme {
        desktop_core::PanelTheme::Dark => true,
        desktop_core::PanelTheme::Light => false,
        desktop_core::PanelTheme::System => {
            use windows::UI::ViewManagement::{UIColorType, UISettings};
            UISettings::new()
                .and_then(|s| s.GetColorValue(UIColorType::Foreground))
                .map_or(true, |c| {
                    u32::from(c.R) + u32::from(c.G) + u32::from(c.B) > 384
                })
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
    fn border_follows_background_opacity_and_material_strength() {
        use desktop_core::Backdrop;
        for dark in [false, true] {
            let full = panel_border(
                dark,
                Backdrop::Solid {
                    color: 0x123456,
                    opacity: 1.0,
                },
            );
            for opacity in [0.0, 0.25, 0.5, 1.0] {
                let border = panel_border(
                    dark,
                    Backdrop::Solid {
                        color: 0xabcdef,
                        opacity,
                    },
                );
                assert!((border.a - full.a * opacity).abs() < 0.00001);
                assert_eq!((border.r, border.g, border.b), (full.r, full.g, full.b));
            }
            for material in [Backdrop::Acrylic, Backdrop::Mica] {
                let alphas: Vec<_> = [0, 25, 50, 75, 100]
                    .into_iter()
                    .map(|strength| panel_border(dark, material.with_strength(strength)).a)
                    .collect();
                assert_eq!(alphas[0], 0.0);
                assert!(alphas.windows(2).all(|pair| pair[0] < pair[1]));
                assert_eq!(alphas[2], full.a);
            }
        }
    }

    #[test]
    fn solid_text_remains_readable_over_extreme_desktop_colors() {
        use desktop_core::{Backdrop, PanelText};
        for color in [0, 0xffffff, 0x808080, 0xff0000, 0x00ff00, 0x0000ff] {
            for opacity in [0.0, 0.1, 0.5, 0.9, 1.0] {
                for mode in [PanelText::Auto, PanelText::Light, PanelText::Dark] {
                    let style =
                        panel_contrast(Backdrop::Solid { color, opacity }, true, mode, true);
                    for behind in [0.0, 1.0] {
                        let rgb = [16, 8, 0].map(|shift| {
                            let c = ((color >> shift) & 255) as f32 / 255.0;
                            (c * opacity + behind * (1.0 - opacity)) * (1.0 - style.scrim)
                                + style.base() * style.scrim
                        });
                        let l = luminance(rgb);
                        let ratio = if style.light_text {
                            1.05 / (l + 0.05)
                        } else {
                            (l + 0.05) / 0.05
                        };
                        assert!(ratio >= 4.499, "{color:x} {opacity} {mode:?}: {ratio}");
                    }
                }
            }
        }
        for (color, light) in [(0xffffff, false), (0, true)] {
            let style = panel_contrast(
                Backdrop::Solid {
                    color,
                    opacity: 1.0,
                },
                !light,
                PanelText::Auto,
                true,
            );
            assert_eq!(style.light_text, light);
            assert_eq!(
                style.scrim, 0.0,
                "Opaque readable colors need no protection"
            );
        }
    }

    #[test]
    fn transparent_materials_and_manual_overrides_are_protected() {
        use desktop_core::{Backdrop, PanelText};
        for dark in [false, true] {
            let standard = panel_contrast(Backdrop::Acrylic, dark, PanelText::Auto, true);
            assert_eq!(standard.light_text, dark);
            assert_eq!(standard.scrim, 0.0);
            let clear = panel_contrast(
                Backdrop::Acrylic.with_strength(0),
                dark,
                PanelText::Auto,
                true,
            );
            assert!(clear.scrim > 0.0);
            let forced = panel_contrast(
                Backdrop::Mica,
                dark,
                if dark {
                    PanelText::Dark
                } else {
                    PanelText::Light
                },
                true,
            );
            assert_eq!(forced.light_text, !dark);
            assert!(forced.scrim > 0.0);
        }
    }

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
