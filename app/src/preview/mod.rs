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
mod desktop;
mod hybrid;
mod drop_target;
mod drag_image;
mod label;
mod layout;
pub(crate) mod menu;
mod render;
mod shell_menu;
mod snap;
mod theme;
mod window;
pub use desktop::run as run_desktop;
pub use hybrid::run as run_hybrid;

use desktop_core::{
    DesktopItem, DesktopPlacement, GridPosition, Panel, PanelId, PanelSource, RectDip,
    ShellIdentity, Workspace,
};
use desktop_shell::{ShellApartment, enumerate_desktop_references, open_shell_identity};
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
    pub scroll: usize,
    pub collapsed: bool,
    pub loading: bool,
}

impl GroupModel {
    fn grid(&self, width: f32, height: f32) -> layout::Grid {
        if self.managed {
            layout::Grid::system(width, height, self.icon_size, self.spacing, self.desktop)
        } else {
            layout::Grid::new(width, height, self.icon_size)
        }
    }
    fn cell(&self, grid: layout::Grid, index: usize) -> (f32, f32) {
        if self.desktop {
            let p = self.items[index].position;
            (p.x, p.y)
        } else {
            grid.cell(index, self.scroll)
        }
    }
    fn hit(&self, grid: layout::Grid, x: f32, y: f32) -> Option<usize> {
        if self.desktop {
            self.items.iter().position(|item| {
                x >= item.position.x
                    && x < item.position.x + grid.cell_width
                    && y >= item.position.y
                    && y < item.position.y + grid.cell_height
            })
        } else {
            grid.hit(x, y, self.scroll, self.items.len())
        }
    }
}

struct View {
    id: PanelId,
    window: windows_window::Window,
    model: Rc<RefCell<GroupModel>>,
}

struct Preview {
    hybrid: Option<hybrid::Session>,
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

#[derive(Clone, Copy)]
enum Event {
    MenuSelection(bool),
    Ready,
    Refresh,
    ToggleAutoHide,
    ToggleTopmost,
    Theme(desktop_core::PanelTheme),
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

#[allow(clippy::too_many_lines)]
fn inventory(folder: Option<&Path>) -> Result<Vec<DesktopItem>, String> {
    // All reads and image extraction are independent of the native desktop's layout settings.
    let inventory = if let Some(folder) = folder {
        if !folder.is_dir() {
            return Err(format!("目录不存在：{}", folder.display()));
        }
        std::fs::read_dir(folder)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .map(|entry| {
                let path = entry.path();
                let label = entry.file_name().to_string_lossy().into_owned();
                DesktopItem::new(
                    ShellIdentity::FileSystem {
                        path,
                        volume_id: None,
                        file_id: None,
                    },
                    label,
                )
            })
            .collect()
    } else {
        enumerate_desktop_references(0)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|item| DesktopItem::new(item.identity, item.display_name))
            .collect()
    };
    Ok(inventory)
}

pub fn run(path: &Path, folder: Option<&Path>, title: Option<String>) -> Result<(), String> {
    // Explicit folder previews are independent from the saved Desktop preview inventory.
    let owned_path;
    let path = if folder.is_some() {
        owned_path = path.with_file_name("folder-preview.db");
        &owned_path
    } else {
        path
    };
    let mut store = WorkspaceStore::open(path).map_err(|e| e.to_string())?;
    let mut workspace = store.load_workspace().map_err(|e| e.to_string())?;
    if workspace.panels().is_empty() {
        for (id, title, x) in [(1, "桌面项目 · 独立预览", 160.0), (2, "新建分组", 700.0)]
        {
            workspace
                .add_panel(Panel::new(
                    PanelId::new(id),
                    title,
                    PanelSource::DesktopCollection,
                    RectDip::new(x, 160.0, 480.0, 400.0),
                ))
                .map_err(|e| e.to_string())?;
            workspace
                .panel_mut(PanelId::new(id))
                .unwrap()
                .set_backdrop(desktop_core::Backdrop::Acrylic);
        }
    }
    if let Some(title) = title {
        workspace
            .panel_mut(PanelId::new(1))
            .unwrap()
            .set_title(title);
    }
    let valid: Vec<_> = workspace.panels().iter().map(Panel::id).collect();
    store
        .save_workspace(&workspace)
        .map_err(|e| e.to_string())?;
    let (sender, receiver) = mpsc::channel();
    let folder = folder.map(Path::to_path_buf);
    std::thread::spawn(move || {
        let _apartment = match ShellApartment::initialize_sta() {
            Ok(apartment) => apartment,
            Err(error) => {
                let _ = sender.send(Loaded::Inventory(Err(error.to_string())));
                return;
            }
        };
        let result = inventory(folder.as_deref());
        let requests: Vec<_> = result
            .as_ref()
            .map(|items| items.iter().map(|item| item.identity().clone()).collect())
            .unwrap_or_default();
        if sender.send(Loaded::Inventory(result)).is_err() {
            return;
        }
        for identity in requests {
            if let Ok(image) = assets::load(&identity, 96)
                && sender
                    .send(Loaded::Image(identity.persistent_key(), image))
                    .is_err()
            {
                break;
            }
        }
    });
    let state = Rc::new(RefCell::new(Preview {
        hybrid: None,
        workspace,
        store,
        views: Vec::new(),
        images: HashMap::new(),
        receiver,
        desktop: None,
    }));
    for id in valid {
        create_view(&state, id)?;
    }
    windows_window::run();
    Ok(())
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
        managed: state.borrow().desktop.is_some() || state.borrow().hybrid.is_some(),
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
    if state.borrow().hybrid.is_some() { hybrid::register_drop(state,id)?; }
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
    hybrid::sync(state)?;
    state
        .store
        .save_workspace(&state.workspace)
        .map_err(|e| format!("保存分组失败：{e}"))
}

#[allow(clippy::too_many_lines)]
fn handle(state: &Rc<RefCell<Preview>>, id: PanelId, event: Event) -> Result<bool, String> {
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
            s.workspace
                .panel_mut(next)
                .unwrap()
                .set_backdrop(desktop_core::Backdrop::Acrylic);
            save(&mut s)?;
            next
        };
        create_view(state, next)?;
        return Ok(false);
    }
    let mut s = state.borrow_mut();
    match event {
        Event::MenuSelection(allow) => hybrid::menu(&s, allow)?,
        Event::Theme(theme) => {
            let old = s.workspace.clone();
            s.workspace.panel_mut(id).ok_or("分组不存在")?.set_theme(theme);
            if let Err(error) = save(&mut s) { s.workspace = old; return Err(error); }
            if let Some(view) = s.views.iter().find(|v| v.id == id) {
                let mut model = view.model.borrow_mut();
                model.theme = theme;
                model.dark = self::theme::is_dark(theme);
                unsafe { InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0); }
            }
        }
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
        Event::Material(material) => {
            if s.workspace.panel(id).is_none() {
                return Ok(false);
            }
            let old = s.workspace.clone();
            s.workspace.panel_mut(id).unwrap().set_backdrop(material);
            if let Err(error) = save(&mut s) {
                s.workspace = old;
                return Err(error);
            }
            if let Some(view) = s.views.iter().find(|view| view.id == id) {
                view.model.borrow_mut().backdrop = material;
                unsafe {
                    InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0);
                }
            }
        }
        Event::Tick => {
            if s.hybrid.is_some() { hybrid::tick(&mut s)?; return Ok(false); }
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
            } else if hybrid::release(&mut s, id, index, point)? || desktop::release(&mut s, id, index, point)? {
                refresh_views(&mut s);
            }
        }
        Event::Exit => windows_window::quit(),
        Event::New => unreachable!(),
    }
    Ok(false)
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
            hybrid: None,
            workspace,
            store: WorkspaceStore::open_in_memory().unwrap(),
            views: Vec::new(),
            images: HashMap::new(),
            receiver,
            desktop: None,
        }
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
            spacing: (88.0, 96.0), selected: None, scroll: 0, collapsed: false, loading: false,
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
