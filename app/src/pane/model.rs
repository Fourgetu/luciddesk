//! Pane geometry shared by rendering, hit testing, dragging and resizing.
use super::*;

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone)]
pub struct GroupModel {
    pub list_view: bool,
    pub folder_sort: (u8, bool),
    pub folder_navigation: [bool; 2],
    pub folder: Option<std::path::PathBuf>,
    pub folder_status: Option<String>,
    pub options: desktop_core::PaneOptions,
    pub theme: desktop_core::PanelTheme,
    pub dark: bool,

    pub hovered_item: Option<usize>,
    pub focused: bool,
    pub auto_hide: bool,
    pub locked: bool,
    pub reveal: f32,
    pub hovered_button: Option<usize>,
    pub pressed_button: Option<usize>,
    pub backdrop: desktop_core::Backdrop,
    pub native_material: bool,
    pub title: String,
    pub items: Vec<Item>,
    pub icon_size: f32,
    // Keyboard focus may point to an unselected item after Ctrl+navigation.
    pub selected: Option<usize>,
    pub selection: std::collections::BTreeSet<usize>,
    pub selection_anchor: Option<usize>,
    pub renaming: Option<ShellIdentity>,
    pub scroll: usize,
    pub collapsed: bool,
    pub loading: bool,
}

impl GroupModel {
    pub(super) fn header_button_enabled(&self, button: usize) -> bool {
        match button {
            2 | 3 => self.folder.is_some() && self.folder_navigation[button - 2],
            _ => true,
        }
    }

    pub(super) fn header_button(&self, width: f32, x: f32, y: f32) -> Option<usize> {
        layout::header_button(width, x, y).or_else(|| {
            if self.folder.is_none() || !(5.0..33.0).contains(&y) { return None; }
            (2..4).find(|&button| {
                let left = layout::header_button_x(width, button);
                (left..left + 28.0).contains(&x)
            })
        })
    }

    pub(super) fn is_list(&self) -> bool {
        self.list_view
    }
    pub(super) fn list_columns(&self, width: f32) -> [f32; 4] {
        if self.folder.is_some() {
            layout::list_columns(width)
        } else {
            [32.0, width, width, width]
        }
    }
    pub(super) fn clear_selection(&mut self) {
        self.selected = None;
        self.selection.clear();
        self.selection_anchor = None;
    }

    pub(super) fn select_item(&mut self, index: usize, ctrl: bool, shift: bool) {
        if index >= self.items.len() {
            return;
        }
        if shift {
            let anchor = self
                .selection_anchor
                .or(self.selected)
                .unwrap_or(index)
                .min(self.items.len() - 1);
            if !ctrl {
                self.selection.clear();
            }
            self.selection.extend(anchor.min(index)..=anchor.max(index));
            self.selection_anchor = Some(anchor);
        } else {
            if ctrl {
                if !self.selection.remove(&index) {
                    self.selection.insert(index);
                }
            } else {
                self.selection.clear();
                self.selection.insert(index);
            }
            self.selection_anchor = Some(index);
        }
        self.selected = Some(index);
    }

    pub(super) fn select_all(&mut self) {
        self.selection = (0..self.items.len()).collect();
        if !self.items.is_empty() {
            self.selected = Some(self.selected.unwrap_or(0).min(self.items.len() - 1));
            self.selection_anchor = self.selected;
        } else {
            self.clear_selection();
        }
    }

    pub(super) fn selected_identities(&self) -> Vec<ShellIdentity> {
        self.selection
            .iter()
            .filter_map(|&index| self.items.get(index))
            .map(|item| item.identity.clone())
            .collect()
    }

    pub(super) fn replace_items(&mut self, items: Vec<Item>) {
        let focus = self
            .selected
            .and_then(|i| self.items.get(i))
            .map(|i| i.identity.clone());
        let anchor = self
            .selection_anchor
            .and_then(|i| self.items.get(i))
            .map(|i| i.identity.clone());
        let identities = self.selected_identities();
        self.items = items;
        self.selected =
            focus.and_then(|identity| self.items.iter().position(|i| i.identity == identity));
        self.selection_anchor =
            anchor.and_then(|identity| self.items.iter().position(|i| i.identity == identity));
        self.selection = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| identities.contains(&item.identity).then_some(index))
            .collect();
    }

    fn icon_grid_spacing(&self) -> (f32, f32) {
        let scale = (self.options.grid_scale / 100.0)
            .max((self.icon_size + 16.0) / 88.0)
            .max((self.icon_size + 34.0) / 96.0);
        (88.0 * scale, 96.0 * scale)
    }
    pub(super) fn resize_cell(&self) -> (f32, f32) {
        if self.is_list() {
            return (396.0, layout::LIST_ROW);
        }
        let grid = layout::Grid::system(0.0, 0.0, self.icon_size, self.icon_grid_spacing());
        (grid.cell_width, grid.cell_height)
    }

    pub(super) fn row_contents(&self, grid: layout::Grid) -> Vec<f32> {
        if self.is_list() {
            return vec![layout::LIST_ROW; self.items.len()];
        }
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
        if self.is_list() {
            let mut grid = layout::Grid::list(width, height);
            if self.folder.is_none() {
                grid.content_top = layout::HEADER + layout::PADDING;
                grid.visible_rows = ((height - grid.content_top - layout::PADDING)
                    / layout::LIST_ROW).floor().max(1.0) as usize;
            }
            return grid;
        }
        let mut grid = layout::Grid::system(width, height, self.icon_size, self.icon_grid_spacing());
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
        if self.is_list() {
            return RectDip {
                x,
                y,
                width: grid.cell_width,
                height: grid.cell_height,
            };
        }
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
