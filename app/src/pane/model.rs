//! Pane geometry shared by rendering, hit testing, dragging and resizing.
use super::*;

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone)]
pub struct GroupModel {
    pub merge_preview: Vec<(PanelId, String)>,
    pub merge_occluded: bool,
    pub tabs: Vec<(PanelId, String)>,
    pub active_tab: PanelId,
    pub list_view: bool,
    pub folder_sort: (u8, bool),
    pub folder_columns: Option<[f32; 4]>,
    pub folder_visible_columns: u8,
    pub folder_navigation: [bool; 2],
    pub folder: Option<std::path::PathBuf>,
    pub folder_status: Option<String>,
    pub options: luciddesk_core::PaneOptions,
    pub theme: luciddesk_core::PanelTheme,
    pub dark: bool,

    pub hovered_item: Option<usize>,
    pub scrollbar: super::scrollbar::State,
    pub focused: bool,
    pub auto_hide: bool,
    pub locked: bool,
    pub reveal: f32,
    pub hovered_tab: Option<PanelId>,
    pub hovered_button: Option<usize>,
    pub pressed_button: Option<usize>,
    pub backdrop: luciddesk_core::Backdrop,
    pub native_material: bool,
    /// Measured luminance of the desktop behind the pane, when its flat color is
    /// too transparent to establish a background of its own. See [`super::backdrop_sample`].
    pub behind: Option<f32>,
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
    pub(super) fn content_header(&self) -> f32 { layout::HEADER }
    pub(super) fn header_button_x(&self, width: f32, button: usize) -> f32 {
        if self.tabs.len() > 1 && button >= 2 {
            width - 70.0 + (button - 2) as f32 * 32.0
        } else { layout::header_button_x(width, button) }
    }
    pub(super) fn header_button_enabled(&self, button: usize) -> bool {
        match button {
            0 | 1 if self.tabs.len() > 1 => false,
            2 | 3 => self.folder.is_some() && self.folder_navigation[button - 2],
            _ => true,
        }
    }

    pub(super) fn header_button(&self, width: f32, x: f32, y: f32) -> Option<usize> {
        layout::header_button(width, x, y).filter(|_| self.tabs.len() < 2).or_else(|| {
            if self.folder.is_none() || !(layout::HEADER_INSET..layout::HEADER - layout::HEADER_INSET).contains(&y) { return None; }
            (2..4).find(|&button| {
                let left = self.header_button_x(width, button);
                (left..left + 28.0).contains(&x)
            })
        })
    }

    pub(super) fn is_list(&self) -> bool {
        self.list_view
    }
    pub(super) fn list_columns(&self, width: f32) -> [f32; 5] {
        if self.folder.is_some() {
            super::columns::visible_bounds(width, self.folder_columns, self.folder_visible_columns)
        } else {
            [32.0, width, width, width, width]
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
        // Borrow identities while remapping; avoid cloning paths and an O(items × selection) scan.
        let identities: std::collections::HashSet<_> = self.selection.iter()
            .filter_map(|&index| self.items.get(index).map(|item| &item.identity)).collect();
        let selection = items.iter().enumerate()
            .filter_map(|(index, item)| identities.contains(&item.identity).then_some(index)).collect();
        self.items = items;
        self.selected =
            focus.and_then(|identity| self.items.iter().position(|i| i.identity == identity));
        self.selection_anchor =
            anchor.and_then(|identity| self.items.iter().position(|i| i.identity == identity));
        self.selection = selection;
    }

    fn icon_grid(&self, width: f32, height: f32) -> layout::Grid {
        layout::desktop_grid(width, height, self.icon_size, self.options.grid_scale)
    }
    pub(super) fn resize_cell(&self) -> (f32, f32) {
        if self.is_list() {
            return (layout::LIST_CELL_WIDTH, layout::LIST_ROW);
        }
        let grid = self.icon_grid(0.0, 0.0);
        (grid.cell_width, grid.cell_height)
    }

    fn row_content(&self, grid: layout::Grid, row: usize) -> f32 {
        let start = row * grid.columns;
        layout::icon_row_height(grid,
            self.items[start..(start + grid.columns).min(self.items.len())].iter().map(|item| item.label.as_str()))
    }

    pub(super) fn row_contents(&self, grid: layout::Grid) -> Vec<f32> {
        if self.is_list() {
            return vec![layout::LIST_ROW; self.items.len()];
        }
        (0..self.items.len().div_ceil(grid.columns)).map(|row| self.row_content(grid, row)).collect()
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
        let mut grid = self.icon_grid(width, height);
        if !self.items.is_empty() {
            let count = self.items.len().div_ceil(grid.columns);
            let available = height - self.content_header() - layout::PADDING;
            // Positive row heights mean only a viewport-sized suffix can fit at
            // the end; measuring earlier labels cannot affect the scroll limit.
            let candidates = (available.max(0.0) / grid.cell_height).ceil() as usize + 2;
            let start = self.scroll.min(count - 1);
            let visible: Vec<_> = (start..(start + candidates).min(count))
                .map(|row| self.row_content(grid, row)).collect();
            grid.visible_rows = layout::fitting_rows(&visible, grid.cell_height, available);
            let tail_start = count.saturating_sub(candidates);
            let tail: Vec<_> = (tail_start..count).map(|row| self.row_content(grid, row)).collect();
            grid.scroll_limit = Some(tail_start + (0..tail.len())
                .find(|start| layout::fitting_rows(&tail[*start..], grid.cell_height, available) >= tail.len() - start)
                .unwrap_or(tail.len() - 1));
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
            label::scaled_content_height(
                &self.items[index].label,
                (grid.cell_width * scale).round() as u32,
                (96.0 * scale).round() as u32,
                grid.text_scale,
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
