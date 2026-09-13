//! Column proportions keep a folder list inside its pane at any size or DPI.
pub(super) fn valid(widths: [f32; 4]) -> bool {
    widths
        .iter()
        .all(|value| value.is_finite() && *value > 0.0 && *value < 1.0)
        && (widths.iter().sum::<f32>() - 1.0).abs() < 0.001
}

pub(super) fn decode(value: &str) -> Option<[f32; 4]> {
    let values: Vec<f32> = value
        .split(',')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let widths: [f32; 4] = values.try_into().ok()?;
    valid(widths).then_some(widths)
}

pub(super) fn bounds(width: f32, proportions: Option<[f32; 4]>) -> [f32; 5] {
    let mut bounds = super::layout::list_columns(width);
    if let Some(values) = proportions.filter(|values| valid(*values)) {
        let content = width - bounds[0];
        let sum = values.iter().sum::<f32>();
        for index in 1..4 {
            bounds[index] = bounds[index - 1] + content * values[index - 1] / sum;
        }
    }
    bounds
}

pub(super) fn resize(mut bounds: [f32; 5], divider: usize, x: f32) -> [f32; 4] {
    let left = bounds[divider - 1];
    let right = bounds[divider + 1];
    let minimum = 48.0_f32.min((right - left) / 2.0);
    bounds[divider] = x.clamp(left + minimum, right - minimum);
    let content = bounds[4] - bounds[0];
    std::array::from_fn(|index| (bounds[index + 1] - bounds[index]) / content)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resizing_changes_only_neighbors_and_survives_resizing_and_persistence() {
        let original = bounds(600.0, None);
        for divider in 1..4 {
            let widths = resize(original, divider, original[divider] + 20.0);
            assert!(valid(widths));
            let changed = bounds(600.0, Some(widths));
            for index in 0..5 {
                if index != divider {
                    assert!((changed[index] - original[index]).abs() < 0.001);
                }
            }
            assert!((changed[divider] - original[divider] - 20.0).abs() < 0.001);
            let encoded = widths.map(|value| format!("{value:.6}")).join(",");
            let restored = decode(&encoded).unwrap();
            for width in [180.0, 400.0, 1200.0] {
                let resized = bounds(width, Some(restored));
                assert_eq!(resized[4], width);
                assert!(resized.windows(2).all(|pair| pair[0] < pair[1]));
            }
            for edge in [-1000.0, 10000.0] {
                let clamped = bounds(600.0, Some(resize(original, divider, edge)));
                assert!(clamped[divider] - clamped[divider - 1] >= 47.99);
                assert!(clamped[divider + 1] - clamped[divider] >= 47.99);
            }
        }
    }
    #[test]
    fn malformed_saved_widths_fall_back_to_defaults() {
        for value in ["", "0.5,0.5", "0,0,0,1", "NaN,0.2,0.3,0.5", "-1,1,0.5,0.5"] {
            assert!(decode(value).is_none());
        }
    }
}
