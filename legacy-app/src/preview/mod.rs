//! Shared pane renderer for isolated preview and reversible desktop takeover.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
mod acrylic;
mod animation;
mod assets;
mod composition;
mod native_graphics;
mod canvas;
mod desktop;
mod drag_image;
mod label;
mod layout;
pub(crate) mod menu;
mod render;
mod rename;
mod settings;
mod shell_menu;
mod snap;
mod theme;
mod window;
mod legacy;
pub use legacy::run;
pub use desktop::run as run_desktop;

use desktop_core::{
    DesktopItem, DesktopPlacement, GridPosition, Panel, PanelId, PanelSource, RectDip,
    ShellIdentity, Workspace,
};
use desktop_shell::{ShellApartment, open_shell_identity};
use desktop_storage::WorkspaceStore;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, mpsc};
use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{InvalidateRect, ScreenToClient};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsWindow, PostMessageW, WindowFromPoint,
};

#[derive(Clone)]
pub struct Item {
    pub identity: ShellIdentity,
    pub label: String,
    pub image: Option<Arc<assets::Pixels>>,
    pub position: desktop_core::PointDip,
}

fn same_items(left: &[Item], right: &[Item]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| {
            a.identity == b.identity
                && a.label == b.label
                && a.position == b.position
                && match (&a.image, &b.image) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
        })
}

#[allow(clippy::struct_excessive_bools)]
pub struct GroupModel {
    pub theme: desktop_core::PanelTheme,
    pub dark: bool,
    pub desktop: bool,
    pub managed: bool,
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
    fn resize_cell(&self) -> (f32, f32) {
        if self.managed {
            let grid = layout::Grid::system(0.0, 0.0, self.icon_size, self.spacing, self.desktop);
            (grid.cell_width, grid.cell_height)
        } else {
            ((self.icon_size + 28.0).max(88.0), self.icon_size + 48.0)
        }
    }

    fn row_contents(&self, grid: layout::Grid) -> Vec<f32> {
        self.items.chunks(grid.columns).map(|items| {
            items.iter().map(|item| {
                theme::selection_height(self.icon_size,
                    label::content_height(&item.label, grid.cell_width.round() as u32),
                    grid.cell_height)
            }).fold(self.icon_size + layout::LABEL_OFFSET + 1.0, f32::max)
        }).collect()
    }

    fn grid(&self, width: f32, height: f32) -> layout::Grid {
        let mut grid = if self.managed {
            layout::Grid::system(width, height, self.icon_size, self.spacing, self.desktop)
        } else {
            layout::Grid::new(width, height, self.icon_size)
        };
        if self.managed && !self.desktop && !self.items.is_empty() {
            let rows = self.row_contents(grid);
            let available = height - layout::HEADER - layout::PADDING;
            let start = self.scroll.min(rows.len() - 1);
            grid.visible_rows = layout::fitting_rows(&rows[start..], grid.cell_height, available);
            grid.scroll_limit = Some((0..rows.len()).find(|start| {
                layout::fitting_rows(&rows[*start..], grid.cell_height, available) >= rows.len() - start
            }).unwrap_or(rows.len() - 1));
        }
        grid
    }
    fn cell(&self, grid: layout::Grid, index: usize) -> (f32, f32) {
        if self.desktop {
            let p = self.items[index].position;
            (p.x, p.y)
        } else {
            grid.cell(index, self.scroll)
        }
    }
    fn selection_bounds(&self, grid: layout::Grid, index: usize, scale: f32) -> RectDip {
        let (x, y) = self.cell(grid, index);
        let height = if self.managed {
            theme::selection_height(grid.icon_size,
                label::content_height_at_dpi(&self.items[index].label,
                    (grid.cell_width * scale).round() as u32,
                    (96.0 * scale).round() as u32) / scale,
                grid.cell_height)
        } else { grid.cell_height - 3.0 };
        RectDip {
            x: x + if self.managed { 0.0 } else { 2.0 }, y,
            width: grid.cell_width - if self.managed { 0.0 } else { 4.0 },
            height,
        }
    }

    fn hit(&self, grid: layout::Grid, x: f32, y: f32, scale: f32) -> Option<usize> {
        if self.collapsed { return None; }
        let contains = |index| {
            let r = self.selection_bounds(grid, index, scale);
            x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
        };
        if self.desktop {
            (0..self.items.len()).find(|&index| contains(index))
        } else {
            grid.hit(x, y, self.scroll, self.items.len()).filter(|&index| contains(index))
        }
    }
}

struct View {
    id: PanelId,
    window: windows_window::Window,
    model: Rc<RefCell<GroupModel>>,
}

struct Preview {
    settings: Option<windows_window::Window>,
    workspace: Workspace,
    store: WorkspaceStore,
    views: Vec<View>,
    images: HashMap<String, Arc<assets::Pixels>>,
    receiver: mpsc::Receiver<Loaded>,
    desktop: Option<desktop::Session>,
}

enum Loaded {
    Desktop(Result<desktop_shell::NativeDesktopSnapshot, String>),
        Inventory(Result<Vec<DesktopItem>, String>),
    Image(String, assets::Pixels),
}

#[derive(Clone)]
enum Event {
    PaneItemFocus,
    MenuSelection(bool),
    ItemMenuEnded(ShellIdentity),
    RenameItem(ShellIdentity),
    RenameTitle,
    SetTitle(String),
    ClosePane,
    Settings,
    Ready,
    Refresh,
    ToggleAutoHide,
    ToggleTopmost,
    Theme(desktop_core::PanelTheme),
    PanelTheme(desktop_core::PanelTheme),
    PanelMaterial(desktop_core::Backdrop),
    SetCollapsed(bool),
    Moving(*mut RECT),
    Material(desktop_core::Backdrop),
    Tick,
    New,
    Activate(usize),
    Drop { index: usize, point: POINT },
    Geometry(RectDip),
    Collapse,
    Sort,
    Exit,
}


fn items_for(state: &Preview, id: PanelId) -> Vec<Item> {
    if let Some(items) = desktop::items(state, id) {
        return items;
    }
    let mut items: Vec<_> = state
        .workspace
        .desktop_items()
        .iter()
        .filter_map(|item| {
            if let DesktopPlacement::Pane { pane_id, position } = item.placement() {
                (*pane_id == id).then_some(((position.row, position.column), item))
            } else {
                None
            }
        })
        .collect();
    items.sort_by_key(|(position, _)| *position);
    items
        .into_iter()
        .map(|(_, item)| Item {
            identity: item.identity().clone(),
            label: item.display_name().to_string(),
            image: state.images.get(&item.identity().persistent_key()).cloned(),
            position: desktop_core::PointDip::default(),
        })
        .collect()
}

fn create_view(state: &Rc<RefCell<Preview>>, id: PanelId) -> Result<(), String> {
    let (panel, items) = {
        let s = state.borrow();
        (s.workspace.panel(id).unwrap().clone(), items_for(&s, id))
    };
    let model = Rc::new(RefCell::new(GroupModel {
        theme: panel.theme(),
        dark: theme::is_dark(panel.theme()),
        desktop: false,
        managed: state.borrow().desktop.is_some(),
        spacing: state
            .borrow()
            .desktop
            .as_ref()
            .map_or((88.0, 96.0), |s| s.spacing),
        hovered_item: None,
        focused: false,
        auto_hide: panel.auto_hide(),
        reveal: if panel.collapsed() { 0.0 } else { 1.0 },
        hovered_button: None,
        backdrop: panel.backdrop(),
        native_material: false,
        title: panel.title().to_string(),
        items,
        icon_size: state
            .borrow()
            .desktop
            .as_ref()
            .map_or(48.0, |s| s.icon_size),
        selected: None,
        renaming: None,
        scroll: 0,
        collapsed: panel.collapsed(),
        loading: id == PanelId::new(1),
    }));
    let weak = Rc::downgrade(state);
    let window = window::create(panel.rect(), Rc::clone(&model), move |event| {
        let Some(state) = weak.upgrade() else {
            return false;
        };
        match handle(&state, id, event) {
            Ok(done) => done,
            Err(error) => {
                eprintln!("{error}");
                window::error(&error);
                false
            }
        }
    })?;
    window::set_layer(window.hwnd().cast(), panel.always_on_top());
    state.borrow_mut().views.push(View { id, window, model });
    Ok(())
}

fn refresh_views(state: &mut Preview) {
    refresh_changed_views(state, false);
}

fn refresh_changed_views(state: &mut Preview, force: bool) {
    for view in &state.views {
        let items = items_for(state, view.id);
        let mut model = view.model.borrow_mut();
        if !force && same_items(&model.items, &items) && !model.loading {
            continue;
        }
        let selected = model
            .selected
            .and_then(|index| model.items.get(index))
            .map(|item| item.identity.clone());
        model.items = items;
        model.selected = selected.and_then(|identity| {
            model
                .items
                .iter()
                .position(|item| item.identity == identity)
        });
        model.hovered_item = None;
        let hwnd = view.window.hwnd().cast();
        let mut bounds = RECT::default();
        unsafe {
            GetWindowRect(hwnd, &raw mut bounds);
        }
        let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
        model.scroll = model.scroll.min(
            model
                .grid(
                    (bounds.right - bounds.left) as f32 / scale,
                    (bounds.bottom - bounds.top) as f32 / scale,
                )
                .max_scroll(model.items.len()),
        );
        if model.selected.is_some_and(|i| i >= model.items.len()) {
            model.selected = None;
        }
        unsafe {
            InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0);
        }
    }
}

fn save(state: &mut Preview) -> Result<(), String> {
    state
        .store
        .save_workspace(&state.workspace)
        .map_err(|e| format!("保存分组失败：{e}"))
}

#[allow(clippy::too_many_lines)]
fn handle(state: &Rc<RefCell<Preview>>, id: PanelId, event: Event) -> Result<bool, String> {
    if matches!(event, Event::Settings) { settings::show(state, id)?; return Ok(false); }
    if matches!(event, Event::PanelTheme(_) | Event::PanelMaterial(_)) {
        let mut s = state.borrow_mut();
        let old = s.workspace.clone();
        let Some(panel) = s.workspace.panel_mut(id) else { return Ok(false); };
        match event {
            Event::PanelTheme(value) => panel.set_theme(value),
            Event::PanelMaterial(value) => panel.set_backdrop(value),
            _ => unreachable!(),
        }
        let (theme, backdrop) = (panel.theme(), panel.backdrop());
        if let Err(error) = save(&mut s) { s.workspace = old; return Err(error); }
        if let Some(view) = s.views.iter().find(|view| view.id == id) {
            let mut model = view.model.borrow_mut();
            model.theme = theme;
            model.dark = self::theme::is_dark(theme);
            model.backdrop = backdrop;
            unsafe { InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0); }
        }
        return Ok(false);
    }
    if matches!(event, Event::Theme(_) | Event::Material(_)) {
        let mut s = state.borrow_mut();
        let old = s.workspace.clone();
        let (mut theme, mut backdrop) = s.workspace.appearance()
            .or_else(|| s.workspace.panels().first().map(|p| (p.theme(), p.backdrop())))
            .unwrap_or((desktop_core::PanelTheme::System, desktop_core::Backdrop::Mica));
        match event { Event::Theme(value) => theme = value, Event::Material(value) => backdrop = value, _ => unreachable!() }
        s.workspace.set_appearance(theme, backdrop);
        if let Err(error) = save(&mut s) { s.workspace = old; return Err(error); }
        for view in &s.views {
            if view.model.borrow().desktop { continue; }
            { let mut model = view.model.borrow_mut(); model.theme = theme; model.dark = self::theme::is_dark(theme); model.backdrop = backdrop; }
            unsafe { InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0); }
        }
        if let Some(window) = &s.settings { unsafe { InvalidateRect(window.hwnd().cast(), std::ptr::null(), 0); } }
        return Ok(false);
    }
    if matches!(event, Event::ClosePane) {
        let mut s = state.borrow_mut();
        if s.workspace.panel(id).is_none() { return Ok(false); }
        let old = s.workspace.clone();
        if let Err(error) = desktop::release_panel(&mut s, id) {
            s.workspace = old;
            return Err(error);
        }
        remove_panel(&mut s.workspace, id);
        if let Err(error) = save(&mut s) {
            s.workspace = old;
            return Err(error);
        }
        let view = s.views.iter().position(|view| view.id == id).map(|at| s.views.remove(at));
        if let Some(view) = &view {
            let hwnd = view.window.hwnd().cast();
            window::prepare_close(hwnd);
        }
        refresh_views(&mut s);
        drop(s);
        drop(view);
        return Ok(false);
    }
    if matches!(event, Event::RenameTitle) {
        let target = state.borrow().views.iter().find(|v| v.id == id)
            .map(|v| (v.window.hwnd().cast(), v.model.clone()));
        if let Some((owner, model)) = target {
            let state = Rc::clone(state);
            rename::show_title(owner, model, Box::new(move |title| {
                handle(&state, id, Event::SetTitle(title)).map(|_| ())
            }))?;
        }
        return Ok(false);
    }
    if let Event::PaneItemFocus = event {
        return Ok(false);
    }
    if let Event::MenuSelection(_) = event {
        return Ok(false);
    }
    if let Event::ItemMenuEnded(identity) = event {
        let requested = false;
        if requested {
            let target = {
                let s = state.borrow();
                s.views.iter().find(|v| v.id == id).and_then(|v| {
                    let model = v.model.borrow();
                    model.items.iter().find(|item| item.identity == identity)
                        .map(|item| (v.window.hwnd().cast(), item.identity.clone(), item.label.clone(), v.model.clone()))
                })
            };
            if let Some((owner, identity, title, model)) = target {
                rename::show(owner, &identity, &title, model)?;
            }
        }
        return Ok(false);
    }
    if let Event::RenameItem(identity) = event {
        let target = {
            let s = state.borrow();
            s.views.iter().find(|v| v.id == id).and_then(|v| {
                let model = v.model.borrow();
                model.items.iter().find(|i| i.identity == identity)
                    .map(|i| (v.window.hwnd().cast(), i.label.clone(), v.model.clone()))
            })
        };
        if let Some((owner, label, model)) = target { rename::show(owner, &identity, &label, model)?; }
        return Ok(false);
    }
    if matches!(event, Event::New) {
        let next = {
            let mut s = state.borrow_mut();
            let next = PanelId::new(
                s.workspace
                    .panels()
                    .iter()
                    .map(|p| p.id().get())
                    .max()
                    .unwrap_or(0)
                    + 1,
            );
            s.workspace
                .add_panel(Panel::new(
                    next,
                    format!("分组 {}", next.get()),
                    PanelSource::DesktopCollection,
                    RectDip::new(240.0, 240.0, 480.0, 360.0),
                ))
                .map_err(|e| e.to_string())?;
            if s.workspace.appearance().is_none() { s.workspace
                .panel_mut(next)
                .unwrap()
                .set_backdrop(desktop_core::Backdrop::Acrylic); }
            save(&mut s)?;
            next
        };
        create_view(state, next)?;
        return Ok(false);
    }
    let mut s = state.borrow_mut();
    match event {
        Event::RenameTitle | Event::ClosePane | Event::Settings => unreachable!("Handled before borrowing Preview"),
        Event::SetTitle(title) => {
            let old = s.workspace.clone();
            s.workspace.panel_mut(id).ok_or("分组不存在")?.set_title(title.clone());
            if let Err(error) = save(&mut s) { s.workspace = old; return Err(error); }
            if let Some(view) = s.views.iter().find(|view| view.id == id) {
                view.model.borrow_mut().title = title;
                unsafe { InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0); }
            }
            refresh_views(&mut s);
        }
        Event::PaneItemFocus | Event::MenuSelection(_) | Event::ItemMenuEnded(_) | Event::RenameItem(_) => unreachable!("Handled before borrowing Preview"),
        Event::Theme(_) | Event::Material(_) | Event::PanelTheme(_) | Event::PanelMaterial(_) => unreachable!("Handled before borrowing Preview"),
        Event::ToggleTopmost => {
            let panel = s.workspace.panel_mut(id).ok_or("分组不存在")?;
            let enabled = !panel.always_on_top();
            panel.set_always_on_top(enabled);
            if let Err(error) = save(&mut s) {
                s.workspace.panel_mut(id).unwrap().set_always_on_top(!enabled);
                return Err(error);
            }
            if let Some(view) = s.views.iter().find(|v| v.id == id) {
                window::set_layer(view.window.hwnd().cast(), enabled);
            }
        }
        Event::Ready => desktop::ready(&mut s, id)?,
        Event::Refresh => {
            if let Some(session) = s.desktop.as_mut() {
                session.reload_images = true;
                session.next_scan = std::time::Instant::now();
            }
            desktop::scan(&mut s);
        }
        Event::Moving(rect) => {
            let peers: Vec<_> = s
                .views
                .iter()
                .filter(|view| view.id != id && !view.model.borrow().desktop)
                .filter_map(|view| {
                    let mut bounds = RECT::default();
                    (unsafe { GetWindowRect(view.window.hwnd().cast(), &raw mut bounds) } != 0)
                        .then_some(bounds)
                })
                .collect();
            if let Some(view) = s.views.iter().find(|view| view.id == id) {
                let scale =
                    unsafe { GetDpiForWindow(view.window.hwnd().cast()) }.max(96) as f32 / 96.0;
                unsafe {
                    use windows_sys::Win32::Graphics::Gdi::{MonitorFromRect, GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTONEAREST};
                    let monitor = MonitorFromRect(rect, MONITOR_DEFAULTTONEAREST);
                    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                    let work = (GetMonitorInfoW(monitor, &raw mut info) != 0).then_some(info.rcWork);
                    snap::snap(
                        &mut *rect,
                        &peers,
                        work.as_ref(),
                        5, // Screen rectangles use physical pixels: keep a 5px gap at every DPI.
                        (14.0 * scale).round() as i32,
                    );
                }
            }
        }
        Event::ToggleAutoHide => {
            if s.workspace.panel(id).is_none() {
                return Ok(false);
            }
            let old = s.workspace.clone();
            let panel = s.workspace.panel_mut(id).unwrap();
            let enabled = !panel.auto_hide();
            panel.set_auto_hide(enabled);
            if let Err(error) = save(&mut s) {
                s.workspace = old;
                return Err(error);
            }
            if let Some(view) = s.views.iter().find(|view| view.id == id) {
                view.model.borrow_mut().auto_hide = enabled;
            }
        }
        Event::Tick => {
            let mut changed = false;
            let done = loop {
                match s.receiver.try_recv() {
                    Ok(Loaded::Desktop(result)) => match result {
                        Ok(snapshot) => {
                            let mut session = s.desktop.take().unwrap();
                            session.snapshot = snapshot;
                            let old = s.workspace.clone();
                            desktop::reconcile_inventory(&mut s.workspace, &session);
                            changed |= old != s.workspace;
                            s.desktop = Some(session);
                            if changed {
                                save(&mut s)?;
                            }
                        }
                        Err(error) => {
                            if s.desktop.as_ref().is_some_and(|d| !d.loaded) {
                                return Err(error);
                            }
                            eprintln!("{error}");
                        }
                    },
                    Ok(Loaded::Image(key, pixels)) => {
                        s.images.insert(key, Arc::new(pixels));
                        changed = true;
                    }
                                        Ok(Loaded::Inventory(result)) => {
                        for view in &s.views {
                            view.model.borrow_mut().loading = false;
                        }
                        reconcile(&mut s.workspace, result?);
                        save(&mut s)?;
                        changed = true;
                    }
                    Err(mpsc::TryRecvError::Empty) => break false,
                    Err(mpsc::TryRecvError::Disconnected) => break true,
                }
            };
            let became_ready = done && s.desktop.as_ref().is_some_and(|d| !d.loaded);
            if became_ready {
                let empty: Vec<_> = s
                    .views
                    .iter()
                    .filter(|v| v.model.borrow().desktop && v.model.borrow().items.is_empty())
                    .map(|v| v.id)
                    .collect();
                // Empty desktop regions have no subsequent WM_PAINT. Their initial transparent
                // frame already succeeded; all windows with content must present loaded images.
                s.desktop
                    .as_mut()
                    .unwrap()
                    .ready
                    .retain(|id| empty.contains(id));
            }
            if done {
                for view in &s.views {
                    view.model.borrow_mut().loading = false;
                }
                if let Some(d) = s.desktop.as_mut() {
                    d.loaded = true;
                    d.scanning = false;
                }
            }
            if changed || became_ready || (done && s.desktop.is_none()) {
                let force = became_ready || (done && s.desktop.is_none());
                refresh_changed_views(&mut s, force);
            }
            if s.desktop.is_some() {
                if s.desktop
                    .as_ref()
                    .is_some_and(|d| !d.scanning && std::time::Instant::now() >= d.next_scan)
                {
                    desktop::scan(&mut s);
                }
                return Ok(false);
            }
            return Ok(done);
        }
        Event::Activate(index) => {
            if let Some(view) = s.views.iter().find(|v| v.id == id)
                && let Some(item) = view.model.borrow().items.get(index)
            {
                open_shell_identity(view.window.hwnd() as isize, &item.identity)
                    .map_err(|e| e.to_string())?;
            }
        }
        Event::Geometry(rect) => {
            if s.workspace.panel(id).is_none() {
                return Ok(false);
            }
            if let Some(panel) = s.workspace.panel_mut(id) {
                panel.set_rect(if panel.collapsed() {
                    RectDip {
                        height: panel.rect().height,
                        ..rect
                    }
                } else {
                    rect
                });
            }
            save(&mut s)?;
        }
        Event::Collapse | Event::SetCollapsed(_) => {
            if s.workspace.panel(id).is_none() {
                return Ok(false);
            }
            let panel = s.workspace.panel_mut(id).unwrap();
            let collapsed = if let Event::SetCollapsed(value) = event {
                value
            } else {
                !panel.collapsed()
            };
            if panel.collapsed() == collapsed {
                return Ok(false);
            }
            panel.set_collapsed(collapsed);
            let bounds = panel.rect();
            if let Some(view) = s.views.iter().find(|v| v.id == id) {
                view.model.borrow_mut().collapsed = collapsed;
                let hwnd = view.window.hwnd().cast();
                unsafe {
                    PostMessageW(
                        hwnd,
                        window::ANIMATE_FOLD,
                        0,
                        (if collapsed {
                            layout::HEADER
                        } else {
                            bounds.height
                        })
                        .round() as isize,
                    );
                }
            }
            save(&mut s)?;
        }
        Event::Sort => {
            if s.workspace.panel(id).is_none() {
                return Ok(false);
            }
            let mut items = items_for(&s, id);
            items.sort_by_key(|i| i.label.to_lowercase());
            let old = s.workspace.clone();
            set_order(&mut s.workspace, id, &items);
            if let Err(error) = save(&mut s) {
                s.workspace = old;
                return Err(error);
            }
            refresh_views(&mut s);
        }
        Event::Drop { index, point } => {
            let source = items_for(&s, id);
            let Some(_) = source.get(index) else {
                return Ok(false);
            };
            let target = s.views.iter().rev().find_map(|view| {
                let hwnd = view.window.hwnd().cast();
                if unsafe { WindowFromPoint(point) } != hwnd {
                    return None;
                }
                if unsafe { IsWindow(hwnd) } == 0 {
                    return None;
                }
                let mut bounds = RECT::default();
                unsafe {
                    GetWindowRect(hwnd, &raw mut bounds);
                }
                if point.x < bounds.left
                    || point.x >= bounds.right
                    || point.y < bounds.top
                    || point.y >= bounds.bottom
                    || view.model.borrow().collapsed
                    || view.model.borrow().desktop
                {
                    return None;
                }
                let mut local = point;
                unsafe {
                    ScreenToClient(hwnd, &raw mut local);
                }
                let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
                let model = view.model.borrow();
                let grid = model.grid(
                    (bounds.right - bounds.left) as f32 / scale,
                    (bounds.bottom - bounds.top) as f32 / scale,
                );
                let at = grid
                    .hit(
                        local.x as f32 / scale,
                        local.y as f32 / scale,
                        model.scroll,
                        model.items.len(),
                    )
                    .unwrap_or(model.items.len());
                Some((view.id, at))
            });
            if let Some((target, at)) = target {
                transfer(&mut s, id, index, target, at)?;
                for view in &s.views {
                    view.model.borrow_mut().selected = None;
                }
                refresh_views(&mut s);
            } else if desktop::release(&mut s, id, index, point)? {
                refresh_views(&mut s);
            }
        }
        Event::Exit => windows_window::quit(),
        Event::New => unreachable!(),
    }
    Ok(false)
}

fn remove_panel(workspace: &mut Workspace, id: PanelId) {
    workspace.remove_panel(id);
    for item in workspace.desktop_items_mut() {
        if matches!(item.placement(), DesktopPlacement::Pane { pane_id, .. } if *pane_id == id) {
            item.set_placement(DesktopPlacement::default());
        }
    }
}

fn set_order(workspace: &mut Workspace, id: PanelId, items: &[Item]) {
    // Monitor surfaces are not panes. Their remaining icons retain absolute positions.
    if workspace.panel(id).is_none() {
        return;
    }
    for (position, item) in items.iter().enumerate() {
        if let Some(entry) = workspace.desktop_item_mut(&item.identity) {
            entry.set_placement(DesktopPlacement::Pane {
                pane_id: id,
                position: GridPosition::new(position as u32, 0),
            });
        }
    }
}

// Persist a unique pane-local order before Shell replaces its inventory order.
// Also repair legacy grid coordinates and gaps left by items dragged out.
#[cfg(test)]
fn normalize_pane_orders(state: &mut Preview) {
    let ids: Vec<_> = state.workspace.panels().iter().map(Panel::id).collect();
    for id in ids {
        let items = items_for(state, id);
        set_order(&mut state.workspace, id, &items);
    }
}

fn transfer(
    state: &mut Preview,
    source: PanelId,
    index: usize,
    target: PanelId,
    at: usize,
) -> Result<(), String> {
    if state.workspace.panel(target).is_none() {
        return Err("目标分组不存在".into());
    }
    let mut remaining = items_for(state, source);
    if index >= remaining.len() {
        return Err("图标列表已经变化，请重试".into());
    }
    let old = state.workspace.clone();
    let item = remaining.remove(index);
    set_order(&mut state.workspace, source, &remaining);
    let mut destination = if source == target {
        remaining
    } else {
        items_for(state, target)
    };
    destination.insert(at.min(destination.len()), item);
    set_order(&mut state.workspace, target, &destination);
    if let Err(error) = save(state) {
        state.workspace = old;
        return Err(error);
    }
    Ok(())
}

fn reconcile(workspace: &mut Workspace, inventory: Vec<DesktopItem>) {
    workspace.reconcile_desktop_items(inventory);
    let valid: Vec<_> = workspace.panels().iter().map(Panel::id).collect();
    let mut next = workspace
        .desktop_items()
        .iter()
        .filter_map(|item| match item.placement() {
            DesktopPlacement::Pane { pane_id, position } if *pane_id == PanelId::new(1) => {
                Some(position.column)
            }
            _ => None,
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    for item in workspace.desktop_items_mut() {
        if !matches!(item.placement(),DesktopPlacement::Pane{pane_id,..} if valid.contains(pane_id))
        {
            item.set_placement(DesktopPlacement::Pane {
                pane_id: PanelId::new(1),
                position: GridPosition::new(next, 0),
            });
            next = next.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> Preview {
        let mut workspace = Workspace::new();
        for id in [1, 2] {
            workspace
                .add_panel(Panel::new(
                    PanelId::new(id),
                    format!("Group {id}"),
                    PanelSource::DesktopCollection,
                    RectDip::default(),
                ))
                .unwrap();
        }
        let inventory = ["A", "B", "C"]
            .into_iter()
            .map(|name| {
                DesktopItem::new(
                    ShellIdentity::Namespace {
                        parsing_name: format!("test:{name}"),
                    },
                    name,
                )
            })
            .collect();
        reconcile(&mut workspace, inventory);
        let (_, receiver) = mpsc::channel();
        Preview {
            settings: None,
            workspace,
            store: WorkspaceStore::open_in_memory().unwrap(),
            views: Vec::new(),
            images: HashMap::new(),
            receiver,
            desktop: None,
        }
    }

    #[test]
    fn snapped_content_bottom_and_scrollbar_use_the_same_row_metrics() {
        let mut model = GroupModel {
            theme: desktop_core::PanelTheme::Dark, dark: true,
            desktop: false, managed: true, hovered_item: None, hovered_button: None,
            focused: false, auto_hide: false, reveal: 1.0, backdrop: desktop_core::Backdrop::Mica,
            native_material: false, title: "Sizing test".into(), items: vec![], icon_size: 48.0,
            spacing: (88.0, 96.0), selected: None, renaming: None, scroll: 0, collapsed: false, loading: false,
        };
        for label in ["Short", "Warhammer 40,000 ????"] {
            model.items = (0..10).map(|i| Item {
                identity: ShellIdentity::Namespace { parsing_name: format!("test-{i}") },
                label: if i == 9 { label.into() } else { "Icon".into() },
                image: None, position: desktop_core::PointDip { x: 0.0, y: 0.0 },
            }).collect();
            let grid = model.grid(376.0, 500.0);
            let rows = model.row_contents(grid);
            let height = layout::pane_content_height(3, grid.cell_height, &rows);
            assert_eq!(height - (layout::HEADER + layout::PADDING + 2.0 * grid.cell_height + rows[2]), layout::PADDING);
            for reduction in [0.0, 5.0, 10.0] {
                let grid = model.grid(376.0, height - reduction);
                assert_eq!(grid.max_scroll(model.items.len()), 0);
                assert_eq!(grid.visible_rows, 3);
            }
            assert_eq!(model.grid(376.0, height - 14.0).max_scroll(model.items.len()), 1);
        }
    }

    #[test]
    fn unrelated_keys_do_not_select_first_icon_or_emit_pane_focus() {
        // Real focus/default-key dispatch interacts with process-wide windowing
        // state left by other live UI fixtures. Exercise it in a fresh process,
        // while still requiring all of the native message assertions to pass.
        const ISOLATED: &str = "LUCIDPANE_KEYBOARD_TEST_CHILD";
        if std::env::var_os(ISOLATED).is_none() {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "preview::tests::unrelated_keys_do_not_select_first_icon_or_emit_pane_focus", "--test-threads=1"])
                .env(ISOLATED, "1")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            let timed_out = loop {
                if child.try_wait().unwrap().is_some() { break false; }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    break true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            };
            let output = child.wait_with_output().unwrap();
            assert!(!timed_out && output.status.success(), "keyboard child: status={}, timed_out={timed_out}\n{}\n{}", output.status,
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return;
        }
        // Only this disposable test process: native failures must produce a
        // failing exit status instead of leaving a modal crash dialog behind.
        unsafe {
            use windows_sys::Win32::System::Diagnostics::Debug::{GetErrorMode, SetErrorMode, SEM_NOGPFAULTERRORBOX};
            SetErrorMode(GetErrorMode() | SEM_NOGPFAULTERRORBOX);
        }
        use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_KEYDOWN, WM_SETFOCUS, WM_KILLFOCUS};
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let model = Rc::new(RefCell::new(GroupModel {
            theme: desktop_core::PanelTheme::Dark, dark: true,
            desktop: false, managed: true, hovered_item: None, hovered_button: None,
            focused: false, auto_hide: false, reveal: 1.0, backdrop: desktop_core::Backdrop::Mica,
            native_material: false, title: "Keyboard regression".into(), items: vec![], icon_size: 48.0,
            spacing: (88.0, 96.0), selected: None, renaming: None, scroll: 0, collapsed: false, loading: false,
        }));
        model.borrow_mut().items = (0..6).map(|index| Item {
            identity: ShellIdentity::Namespace { parsing_name: format!("test:{index}") },
            label: format!("Item {index}"), image: None,
            position: desktop_core::PointDip::default(),
        }).collect();
        let focus_events = Rc::new(std::cell::Cell::new(0));
        let observed = Rc::clone(&focus_events);
        let pane = window::create(RectDip::new(40.0, 40.0, 200.0, 160.0), Rc::clone(&model), move |event| {
            if matches!(event, Event::PaneItemFocus) { observed.set(observed.get() + 1); }
            false
        }).unwrap();
        let hwnd = pane.hwnd().cast();
        for selected in [None, Some(3)] {
            model.borrow_mut().selected = selected;
            model.borrow_mut().scroll = 1;
            unsafe {
                SendMessageW(hwnd, WM_KILLFOCUS, 0, 0);
                SendMessageW(hwnd, WM_SETFOCUS, 0, 0);
            }
            let before = focus_events.get();
            // Letters, digits, modifiers, space, Tab, Backspace and unhandled function keys.
            for key in [0x41, 0x5a, 0x30, 0x10, 0x11, 0x12, 0x20, 0x09, 0x08, 0x70] {
                unsafe { SendMessageW(hwnd, WM_KEYDOWN, key, 0); }
                assert_eq!(model.borrow().selected, selected, "key={key:x}");
                assert_eq!(model.borrow().scroll, 1, "key={key:x}");
                assert_eq!(focus_events.get(), before, "key={key:x}");
            }
        }
        model.borrow_mut().selected = Some(0);
        unsafe { SendMessageW(hwnd, WM_KEYDOWN, 0x27, 0); } // Right still navigates.
        assert_eq!(model.borrow().selected, Some(1));
        unsafe { SendMessageW(hwnd, WM_KEYDOWN, 0x1b, 0); } // Escape still clears.
        assert_eq!(model.borrow().selected, None);
        unsafe { SendMessageW(hwnd, WM_KEYDOWN, 0x41, 0); }
        assert_eq!(model.borrow().selected, None);
        // Keep a real popup open past the fold duration and inspect the pane
        // before dismissing it. The old in-callback modal loop loses its ticks.
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            const RESULT: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.FoldMenuTest");
            unsafe extern "system" fn check_fold(hwnd: windows_sys::Win32::Foundation::HWND, _: u32, id: usize, _: u32) {
                unsafe {
                    KillTimer(hwnd, id);
                    let mut r = RECT::default();
                    GetClientRect(hwnd, &raw mut r);
                    let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
                    let mut popup = std::ptr::null_mut();
                    loop {
                        popup = FindWindowExW(std::ptr::null_mut(), popup, std::ptr::null(), windows_sys::w!("\u{5206}\u{7ec4}\u{83dc}\u{5355}"));
                        if popup.is_null() || GetWindow(popup, GW_OWNER) == hwnd { break; }
                    }
                    let passed = GetWindow(popup, GW_OWNER) == hwnd && r.bottom == (300.0 * dpi).round() as i32;
                    SetPropW(hwnd, RESULT, (if passed { 1usize } else { 2usize }) as _);
                    if GetWindow(popup, GW_OWNER) == hwnd { PostMessageW(popup, WM_CLOSE, 0, 0); }
                }
            }
            model.borrow_mut().collapsed = false;
            SendMessageW(hwnd, window::ANIMATE_FOLD, 0, 300);
            SetTimer(hwnd, 98, 500, Some(check_fold));
            SendMessageW(hwnd, WM_CONTEXTMENU, 0, -1);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while GetPropW(hwnd, RESULT).is_null() && std::time::Instant::now() < deadline {
                let mut msg = MSG::default();
                while PeekMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&raw const msg);
                    DispatchMessageW(&raw const msg);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert_eq!(RemovePropW(hwnd, RESULT) as usize, 1, "fold must finish while the popup is still open");
            assert_eq!(model.borrow().reveal, 1.0);
            KillTimer(hwnd, 98);
        }
        // Client hover must appear immediately, then clear on either a
        // non-client border move or a leave notification without re-arming.
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, WM_MOUSEMOVE, WM_NCMOUSEMOVE};
            use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
            let mut rect = windows_sys::Win32::Foundation::RECT::default();
            GetClientRect(hwnd, &raw mut rect);
            let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
            let x = rect.right - (24.0 * dpi) as i32;
            let y = (19.0 * dpi) as i32;
            let position = ((y as isize) << 16) | x as isize;
            for leave in [WM_NCMOUSEMOVE, WM_MOUSELEAVE] {
                SendMessageW(hwnd, WM_MOUSEMOVE, 0, position);
                assert_eq!(model.borrow().hovered_button, Some(1));
                SendMessageW(hwnd, leave, 0, 0);
                assert_eq!(model.borrow().hovered_button, None);
                assert_eq!(model.borrow().hovered_item, None);
            }
        }
        // Empty panes keep the proposed size even inside the grid magnet.
        model.borrow_mut().items.clear();
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            for dpi in [1.0, 1.25, 1.5, 2.0] {
                for edge in [WMSZ_LEFT, WMSZ_RIGHT, WMSZ_TOP, WMSZ_BOTTOM,
                    WMSZ_TOPLEFT, WMSZ_TOPRIGHT, WMSZ_BOTTOMLEFT, WMSZ_BOTTOMRIGHT] {
                    let mut rect = RECT { left: 0, top: 0, right: (293.0 * dpi) as i32, bottom: (259.0 * dpi) as i32 };
                    let before = rect;
                    SendMessageW(hwnd, WM_SIZING, edge as usize, (&raw mut rect) as isize);
                    assert_eq!((rect.left, rect.top, rect.right, rect.bottom),
                        (before.left, before.top, before.right, before.bottom));
                }
            }
        }
        // Simulate a leave notification consumed by the nested menu loop.
        // A hidden pane cannot be under the pointer: resync must clear both
        // stale header and item highlights without another mouse movement.
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, WM_APP};
            ShowWindow(hwnd, SW_HIDE);
            model.borrow_mut().hovered_button = Some(1);
            model.borrow_mut().hovered_item = Some(0);
            SendMessageW(hwnd, WM_APP + 11, 0, 0);
        }
        assert_eq!(model.borrow().hovered_button, None);
        assert_eq!(model.borrow().hovered_item, None);
        window::prepare_close(hwnd);
        drop(pane);
    }

    #[test]
    fn pane_layer_switch_and_wallpaper_material_initialize() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowLongW, GWL_EXSTYLE, WS_EX_TOPMOST};
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let model = Rc::new(RefCell::new(GroupModel {
            theme: desktop_core::PanelTheme::Dark, dark: true,
            desktop: false, managed: true, hovered_item: None, hovered_button: None,
            focused: false, auto_hide: false, reveal: 1.0, backdrop: desktop_core::Backdrop::Mica,
            native_material: false, title: "Layer test".into(), items: vec![], icon_size: 48.0,
            spacing: (88.0, 96.0), selected: None, renaming: None, scroll: 0, collapsed: false, loading: false,
        }));
        let pane = window::create(RectDip::new(40.0, 40.0, 200.0, 160.0), Rc::clone(&model), |_| false).unwrap();
        assert!(model.borrow().native_material, "System wallpaper brush was unavailable");
        let hwnd = pane.hwnd().cast();
        let mut frame_enabled = 1i32;
        unsafe {
            windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                windows::Win32::Foundation::HWND(hwnd),
                windows::Win32::Graphics::Dwm::DWMWA_NCRENDERING_ENABLED,
                (&raw mut frame_enabled).cast(), 4,
            ).unwrap();
        }
        assert_eq!(frame_enabled, 0, "Pane must not use the DWM activation frame");
        for material in [desktop_core::Backdrop::Mica, desktop_core::Backdrop::MicaAlt, desktop_core::Backdrop::Acrylic] {
            for dark in [false, true] {
                { let mut m = model.borrow_mut(); m.backdrop = material; m.dark = dark; }
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(hwnd, 0x0008, 0, 0); // WM_KILLFOCUS
                    windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(hwnd, 0x000f, 0, 0); // WM_PAINT
                }
                assert!(model.borrow().native_material, "Composition material failed after focus loss");
            }
        }
        window::set_layer(hwnd, false);
        window::set_layer(hwnd, true);
        assert_ne!(unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST, 0);
        window::set_layer(hwnd, false);
        assert_eq!(unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST, 0);
        window::prepare_close(hwnd);
        drop(pane);
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, MSG, PeekMessageW, WM_QUIT, PM_REMOVE};
            assert_eq!(IsWindow(hwnd), 0);
            let mut message = MSG::default();
            assert_eq!(PeekMessageW(&raw mut message, std::ptr::null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE), 0,
                "closing one pane must not quit the app");
        }
    }

    #[test]
    fn desktop_sort_preserves_pane_order_after_a_gap_and_legacy_grid() {
        let mut state = test_state();
        let id = PanelId::new(1);
        // Two rows from the former native-pane layout share a column.
        for (item, position) in state.workspace.desktop_items_mut().iter_mut().zip([
            GridPosition::new(0, 0), GridPosition::new(1, 0), GridPosition::new(0, 1),
        ]) {
            item.set_placement(DesktopPlacement::Pane { pane_id: id, position });
        }
        let names = |s: &Preview| items_for(s, id).into_iter().map(|i| i.label).collect::<Vec<_>>();
        assert_eq!(names(&state), ["A", "B", "C"]);
        normalize_pane_orders(&mut state);
        let mut inventory = state.workspace.desktop_items().to_vec();
        inventory.reverse();
        state.workspace.reconcile_desktop_items(inventory);
        assert_eq!(names(&state), ["A", "B", "C"]);
        let a = items_for(&state, id)[0].identity.clone();
        state.workspace.desktop_item_mut(&a).unwrap().set_placement(DesktopPlacement::default());
        normalize_pane_orders(&mut state);
        let at = items_for(&state, id).len();
        state.workspace.desktop_item_mut(&a).unwrap().set_placement(DesktopPlacement::Pane {
            pane_id: id, position: GridPosition::new(at as u32, 0),
        });
        let mut inventory = state.workspace.desktop_items().to_vec();
        inventory.reverse();
        state.workspace.reconcile_desktop_items(inventory);
        assert_eq!(names(&state), ["B", "C", "A"]);
        state.store.save_workspace(&state.workspace).unwrap();
        state.workspace = state.store.load_workspace().unwrap();
        assert_eq!(names(&state), ["B", "C", "A"]);
    }

    #[test]
    fn moving_between_groups_keeps_identity_unique_and_persists_order() {
        let mut state = test_state();
        let before: Vec<_> = state
            .workspace
            .desktop_items()
            .iter()
            .map(|i| i.identity().clone())
            .collect();
        transfer(&mut state, PanelId::new(1), 1, PanelId::new(2), 0).unwrap();
        assert_eq!(
            items_for(&state, PanelId::new(1))
                .iter()
                .map(|i| i.label.as_str())
                .collect::<Vec<_>>(),
            ["A", "C"]
        );
        assert_eq!(items_for(&state, PanelId::new(2))[0].label, "B");
        assert_eq!(
            state
                .workspace
                .desktop_items()
                .iter()
                .map(|i| i.identity().clone())
                .collect::<Vec<_>>(),
            before
        );
        state.workspace = state.store.load_workspace().unwrap();
        assert_eq!(items_for(&state, PanelId::new(2))[0].label, "B");
        transfer(&mut state, PanelId::new(2), 0, PanelId::new(1), 1).unwrap();
        assert_eq!(
            items_for(&state, PanelId::new(1))
                .iter()
                .map(|i| i.label.as_str())
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
        assert!(items_for(&state, PanelId::new(2)).is_empty());
    }

    #[test]
    fn closing_groups_releases_items_and_persists_an_empty_workspace() {
        let mut state = test_state();
        transfer(&mut state, PanelId::new(1), 0, PanelId::new(2), 0).unwrap();
        let identities: Vec<_> = state.workspace.desktop_items().iter()
            .map(|item| item.identity().clone()).collect();
        let state = Rc::new(RefCell::new(state));
        handle(&state, PanelId::new(1), Event::ClosePane).unwrap();
        {
            let s = state.borrow();
            assert!(s.workspace.panel(PanelId::new(1)).is_none());
            assert_eq!(items_for(&s, PanelId::new(2))[0].label, "A");
            assert_eq!(s.workspace.desktop_items().iter()
                .filter(|item| matches!(item.placement(), DesktopPlacement::FreeDesktop { .. })).count(), 2);
        }
        handle(&state, PanelId::new(2), Event::ClosePane).unwrap();
        let s = state.borrow();
        let loaded = s.store.load_workspace().unwrap();
        assert!(loaded.panels().is_empty());
        assert_eq!(loaded.desktop_items().iter().map(|item| item.identity().clone()).collect::<Vec<_>>(), identities);
        assert!(loaded.desktop_items().iter().all(|item| matches!(item.placement(), DesktopPlacement::FreeDesktop { .. })));
    }

    #[test]
    fn appearance_is_global_while_behavior_remains_per_group() {
        let state=Rc::new(RefCell::new(test_state()));
        let id=PanelId::new(1);
        let other=state.borrow().workspace.panel(PanelId::new(2)).unwrap().clone();
        let before=state.borrow().workspace.panel(id).unwrap().clone();
        for event in [Event::Theme(desktop_core::PanelTheme::Dark),
            Event::Material(desktop_core::Backdrop::Acrylic),Event::ToggleAutoHide,Event::ToggleTopmost] {
            handle(&state,id,event).unwrap();
        }
        let s=state.borrow();
        let stored=s.store.load_workspace().unwrap();
        let panel=stored.panel(id).unwrap();
        assert_eq!(panel.theme(),desktop_core::PanelTheme::Dark);
        assert_eq!(panel.backdrop(),desktop_core::Backdrop::Acrylic);
        assert_eq!(panel.auto_hide(),!before.auto_hide());
        assert_eq!(panel.always_on_top(),!before.always_on_top());
        let other_stored=stored.panel(PanelId::new(2)).unwrap();
        assert_eq!(other_stored.theme(),desktop_core::PanelTheme::Dark);
        assert_eq!(other_stored.backdrop(),desktop_core::Backdrop::Acrylic);
        assert_eq!(other_stored.auto_hide(),other.auto_hide());
        assert_eq!(other_stored.always_on_top(),other.always_on_top());
        assert_eq!(stored.appearance(),Some((desktop_core::PanelTheme::Dark,desktop_core::Backdrop::Acrylic)));
    }

    #[test]
    fn panel_menu_appearance_targets_only_its_panel_and_survives_reload() {
        let state = Rc::new(RefCell::new(test_state()));
        let id = PanelId::new(1);
        handle(&state, id, Event::Theme(desktop_core::PanelTheme::Dark)).unwrap();
        handle(&state, id, Event::Material(desktop_core::Backdrop::Mica)).unwrap();
        handle(&state, id, Event::PanelMaterial(desktop_core::Backdrop::Acrylic)).unwrap();
        handle(&state, id, Event::PanelTheme(desktop_core::PanelTheme::Light)).unwrap();
        let s = state.borrow();
        for workspace in [s.workspace.clone(), s.store.load_workspace().unwrap()] {
            let current = workspace.panel(id).unwrap();
            let other = workspace.panel(PanelId::new(2)).unwrap();
            assert_eq!(current.backdrop(), desktop_core::Backdrop::Acrylic);
            assert_eq!(current.theme(), desktop_core::PanelTheme::Light);
            assert_eq!(other.backdrop(), desktop_core::Backdrop::Mica);
            assert_eq!(other.theme(), desktop_core::PanelTheme::Dark);
            assert_eq!(workspace.appearance(), Some((desktop_core::PanelTheme::Dark, desktop_core::Backdrop::Mica)));
        }
    }

    #[test]
    fn settings_window_applies_clicks_and_closes_without_exiting() {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let _apartment=desktop_shell::ShellApartment::initialize_sta().unwrap();
        let state=Rc::new(RefCell::new(test_state()));
        settings::show(&state,PanelId::new(1)).unwrap();
        let hwnd=state.borrow().settings.as_ref().unwrap().hwnd().cast();
        unsafe {
            let mut outer=RECT::default();let mut client=RECT::default();
            GetWindowRect(hwnd,&raw mut outer);GetClientRect(hwnd,&raw mut client);
            let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
            let mut monitor = windows_sys::Win32::Graphics::Gdi::MONITORINFO {
                cbSize: size_of::<windows_sys::Win32::Graphics::Gdi::MONITORINFO>() as u32,
                ..Default::default()
            };
            windows_sys::Win32::Graphics::Gdi::GetMonitorInfoW(
                windows_sys::Win32::Graphics::Gdi::MonitorFromWindow(hwnd, windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST), &raw mut monitor);
            assert_eq!(client.right, ((900.0 * dpi).round() as i32).min(monitor.rcWork.right - monitor.rcWork.left), "initial width must already use DPI before resizing");
            assert_eq!(client.bottom, ((520.0 * dpi).round() as i32).min(monitor.rcWork.bottom - monitor.rcWork.top), "initial height must fit the monitor");
            assert_eq!(outer.right-outer.left,client.right,"no system side frame");
            assert_eq!(outer.bottom-outer.top,client.bottom,"no system caption band");
            SendMessageW(hwnd,WM_SYSCOMMAND,SC_MAXIMIZE as usize,0);
            assert_ne!(IsZoomed(hwnd),0);
            SendMessageW(hwnd,WM_SYSCOMMAND,SC_RESTORE as usize,0);
            assert_eq!(IsZoomed(hwnd),0);
            {
                // Explorer/Hook can synchronously send these messages while a
                // desktop update holds the mutable workspace borrow.
                let _updating=state.borrow_mut();
                SendMessageW(hwnd,WM_NCHITTEST,0,0);
                SendMessageW(hwnd,WM_ACTIVATE,WA_INACTIVE as usize,0);
                SendMessageW(hwnd,WM_PAINT,0,0);
            }
            let scale=GetDpiForWindow(hwnd).max(96) as f32/96.0;
            let mut bounds=RECT::default();GetClientRect(hwnd,&raw mut bounds);
            let x=bounds.right-(80.0*scale) as i32;
            let y=(172.0*scale) as i32;
            let point=((y as isize)<<16)|(x as isize&0xffff);
            SendMessageW(hwnd,WM_LBUTTONDOWN,1,point);
            SendMessageW(hwnd,WM_LBUTTONUP,0,point);
            assert_eq!(state.borrow().store.load_workspace().unwrap().panel(PanelId::new(1)).unwrap().theme(),desktop_core::PanelTheme::Dark);
            SendMessageW(hwnd,WM_CLOSE,0,0);
            assert_eq!(IsWindow(hwnd),0);
            assert!(state.borrow().settings.is_none());
            for _ in 0..3 {
                settings::show(&state,PanelId::new(2)).unwrap();
                let reopened = state.borrow().settings.as_ref().unwrap().hwnd().cast();
                assert_ne!(IsWindowVisible(reopened),0);
                SendMessageW(reopened, WM_KEYDOWN, 0x1b, 0);
                let mut message = MSG::default();
                while PeekMessageW(&raw mut message, reopened, WM_CLOSE, WM_CLOSE, PM_REMOVE) != 0 {
                    DispatchMessageW(&raw const message);
                }
                assert_eq!(IsWindow(reopened),0);
                assert!(state.borrow().settings.is_none());
                assert_eq!(PeekMessageW(&raw mut message, std::ptr::null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE),0,
                    "settings destruction must not quit the app");
            }

        }
    }

    #[test]
    fn reconciliation_preserves_groups_and_appends_new_items_after_existing_order() {
        let mut state = test_state();
        transfer(&mut state, PanelId::new(1), 0, PanelId::new(2), 0).unwrap();
        let mut fresh = state.workspace.desktop_items().to_vec();
        fresh.push(DesktopItem::new(
            ShellIdentity::Namespace {
                parsing_name: "test:D".into(),
            },
            "D",
        ));
        reconcile(&mut state.workspace, fresh);
        assert_eq!(items_for(&state, PanelId::new(2))[0].label, "A");
        assert_eq!(
            items_for(&state, PanelId::new(1))
                .iter()
                .map(|i| i.label.as_str())
                .collect::<Vec<_>>(),
            ["B", "C", "D"]
        );
        let before = state.workspace.clone();
        assert!(transfer(&mut state, PanelId::new(1), 0, PanelId::new(999), 0).is_err());
        assert_eq!(state.workspace, before);
    }
}
