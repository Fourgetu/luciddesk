//! Pane geometry shared by rendering, hit testing, dragging and resizing.
use super::*;

#[allow(clippy::struct_excessive_bools)]
pub struct GroupModel {
    pub theme: desktop_core::PanelTheme,
    pub dark: bool,

    pub spacing: (f32, f32),
    pub hovered_item: Option<usize>,
    pub focused: bool,
    pub auto_hide: bool,
    pub reveal: f32,
    pub hovered_button: Option<usize>,
    pub backdrop: desktop_core::Backdrop,
    pub native_material: bool,
    pub title: String,
    pub items: Vec<Item>,
    pub icon_size: f32,
    pub selected: Option<usize>,
    pub renaming: Option<ShellIdentity>,
    pub scroll: usize,
    pub collapsed: bool,
    pub loading: bool,
}

impl GroupModel {
    pub(super) fn resize_cell(&self) -> (f32, f32) {
        let grid = layout::Grid::system(0.0, 0.0, self.icon_size, self.spacing);
        (grid.cell_width, grid.cell_height)
    }

    pub(super) fn row_contents(&self, grid: layout::Grid) -> Vec<f32> {
        self.items
            .chunks(grid.columns)
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        theme::selection_height(
                            self.icon_size,
                            label::content_height(&item.label, grid.cell_width.round() as u32),
                            grid.cell_height,
                        )
                    })
                    .fold(self.icon_size + layout::LABEL_OFFSET + 1.0, f32::max)
            })
            .collect()
    }

    pub(super) fn grid(&self, width: f32, height: f32) -> layout::Grid {
        let mut grid = layout::Grid::system(width, height, self.icon_size, self.spacing);
        if !self.items.is_empty() {
            let rows = self.row_contents(grid);
            let available = height - layout::HEADER - layout::PADDING;
            let start = self.scroll.min(rows.len() - 1);
            grid.visible_rows = layout::fitting_rows(&rows[start..], grid.cell_height, available);
            grid.scroll_limit = Some(
                (0..rows.len())
                    .find(|start| {
                        layout::fitting_rows(&rows[*start..], grid.cell_height, available)
                            >= rows.len() - start
                    })
                    .unwrap_or(rows.len() - 1),
            );
        }
        grid
    }
    pub(super) fn cell(&self, grid: layout::Grid, index: usize) -> (f32, f32) {
        grid.cell(index, self.scroll)
    }
    pub(super) fn selection_bounds(&self, grid: layout::Grid, index: usize, scale: f32) -> RectDip {
        let (x, y) = self.cell(grid, index);
        let height = theme::selection_height(
            grid.icon_size,
            label::content_height_at_dpi(
                &self.items[index].label,
                (grid.cell_width * scale).round() as u32,
                (96.0 * scale).round() as u32,
            ) / scale,
            grid.cell_height,
        );
        RectDip {
            x,
            y,
            width: grid.cell_width,
            height,
        }
    }

    pub(super) fn hit(&self, grid: layout::Grid, x: f32, y: f32, scale: f32) -> Option<usize> {
        if self.collapsed {
            return None;
        }
        let contains = |index| {
            let r = self.selection_bounds(grid, index, scale);
            x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
        };
        grid.hit(x, y, self.scroll, self.items.len())
            .filter(|&index| contains(index))
    }
}
