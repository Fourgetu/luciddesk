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

pub(super) fn visible_bounds(width: f32, proportions: Option<[f32; 4]>, visible: u8) -> [f32; 5] {
    let original = bounds(width, proportions);
    let visible = visible | 1;
    let sum: f32 = (0..4).filter(|i| visible & (1 << i) != 0)
        .map(|i| original[i + 1] - original[i]).sum();
    let mut result = [original[0]; 5];
    for i in 0..4 {
        result[i + 1] = result[i] + if visible & (1 << i) != 0 {
            (original[i + 1] - original[i]) / sum * (width - original[0])
        } else { 0.0 };
    }
    let last = (0..4).rfind(|i| visible & (1 << i) != 0).unwrap();
    result[last + 1..].fill(width);
    result
}

pub(super) fn resize_visible(bounds: [f32; 5], divider: usize, x: f32,
    mut proportions: [f32; 4], visible: u8) -> [f32; 4] {
    let left = (0..divider).rfind(|i| (visible | 1) & (1 << i) != 0).unwrap();
    let start = bounds[left];
    let end = bounds[divider + 1];
    let minimum = 48.0_f32.min((end - start) / 2.0);
    let fraction = (x.clamp(start + minimum, end - minimum) - start) / (end - start);
    let pair = proportions[left] + proportions[divider];
    proportions[left] = pair * fraction;
    proportions[divider] = pair * (1.0 - fraction);
    proportions
}

#[cfg(test)]
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
    fn optional_columns_fill_the_pane_and_resize_across_hidden_neighbors() {
        let proportions = [0.4, 0.2, 0.25, 0.15];
        for visible in (1..=15).step_by(2) {
            let bounds = visible_bounds(600.0, Some(proportions), visible);
            assert_eq!(bounds[4], 600.0);
            for column in 0..4 {
                assert_eq!(bounds[column + 1] > bounds[column], visible & (1 << column) != 0);
            }
            for divider in (1..4).filter(|i| visible & (1 << i) != 0) {
                let changed = resize_visible(bounds, divider, bounds[divider] + 10.0, proportions, visible);
                assert!(valid(changed));
                for hidden in (0..4).filter(|i| visible & (1 << i) == 0) {
                    assert_eq!(changed[hidden], proportions[hidden]);
                }
                let resized = visible_bounds(600.0, Some(changed), visible);
                assert!((resized[divider] - bounds[divider] - 10.0).abs() < 0.001);
            }
        }
        assert_eq!(visible_bounds(600.0, Some(proportions), 0), [30.0, 600.0, 600.0, 600.0, 600.0]);
    }
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
