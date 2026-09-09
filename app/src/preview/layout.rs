//! A single layout is shared by painting, hit testing, scrolling and keyboard navigation.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    pub columns: usize,
    pub cell_width: f32,
    pub cell_height: f32,
    pub icon_size: f32,
    pub visible_rows: usize,
}

pub const HEADER: f32 = 38.0;
pub const PADDING: f32 = 12.0;

pub fn header_button(width: f32, x: f32, y: f32) -> Option<usize> {
    if !(5.0..33.0).contains(&y) {
        return None;
    }
    if (width - 38.0..width - 10.0).contains(&x) {
        Some(1)
    } else if (width - 70.0..width - 42.0).contains(&x) {
        Some(0)
    } else {
        None
    }
}

impl Grid {
    pub fn system(
        width: f32,
        height: f32,
        icon_size: f32,
        spacing: (f32, f32),
        desktop: bool,
    ) -> Self {
        let cell_width = spacing.0.max(icon_size + 16.0);
        let cell_height = spacing.1.max(icon_size + 34.0);
        Self {
            columns: ((width - PADDING * 2.0) / cell_width).floor().max(1.0) as usize,
            cell_width,
            cell_height,
            icon_size,
            visible_rows: ((height - if desktop { 0.0 } else { HEADER + PADDING * 2.0 })
                / cell_height)
                .floor()
                .max(1.0) as usize,
        }
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn new(width: f32, height: f32, icon_size: f32) -> Self {
        let minimum_width = (icon_size + 28.0).max(88.0);
        let columns = ((width - PADDING * 2.0) / minimum_width).floor().max(1.0) as usize;
        let cell_height = icon_size + 48.0;
        Self {
            columns,
            cell_width: ((width - PADDING * 2.0) / columns as f32).max(1.0),
            cell_height,
            icon_size,
            visible_rows: ((height - HEADER - PADDING * 2.0) / cell_height)
                .floor()
                .max(1.0) as usize,
        }
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn cell(self, index: usize, scroll: usize) -> (f32, f32) {
        (
            PADDING + (index % self.columns) as f32 * self.cell_width,
            HEADER + PADDING + ((index / self.columns) as f32 - scroll as f32) * self.cell_height,
        )
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn hit(self, x: f32, y: f32, scroll: usize, count: usize) -> Option<usize> {
        if x < PADDING || y < HEADER + PADDING {
            return None;
        }
        let column = ((x - PADDING) / self.cell_width).floor() as usize;
        if column >= self.columns {
            return None;
        }
        let row = ((y - HEADER - PADDING) / self.cell_height).floor() as usize + scroll;
        let index = row * self.columns + column;
        (index < count).then_some(index)
    }

    pub fn max_scroll(self, count: usize) -> usize {
        count
            .div_ceil(self.columns)
            .saturating_sub(self.visible_rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resize_reflows_and_hit_testing_tracks_scrolled_rows() {
        let wide = Grid::new(500.0, 260.0, 48.0);
        let narrow = Grid::new(290.0, 260.0, 48.0);
        assert_eq!(wide.columns, 5);
        assert_eq!(narrow.columns, 3);
        let (x, y) = narrow.cell(7, 1);
        assert_eq!(narrow.hit(x + 20.0, y + 20.0, 1, 12), Some(7));
        assert_eq!(narrow.hit(20.0, 20.0, 0, 12), None);
        assert_eq!(narrow.hit(280.0, 60.0, 0, 12), None);
        assert_eq!(narrow.max_scroll(12), 2);
    }
}
