//! Hybrid pane UI and persisted group interaction. Native desktop synchronization lives in hybrid.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
mod acrylic;
mod animation;
mod assets;
mod canvas;
mod composition;
mod columns;
mod display_layout;
mod drag_drop;
mod events;
mod folder;
mod hybrid;
mod keyboard;
mod label;
mod layout;
pub(crate) mod menu;
mod native_graphics;
mod peek;
mod recovery;
mod rename;
mod render;
#[cfg(test)]
mod render_bench;
mod runtime;
mod search;
mod settings;
use events::handle;
mod shell_menu;
mod snap;
mod theme;
mod wake;
mod window;
pub use hybrid::run;

use desktop_core::{
    DesktopItem, DesktopPlacement, GridPosition, Panel, PanelId, RectDip, ShellIdentity, Workspace,
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
    pub details: ItemDetails,
    pub identity: ShellIdentity,
    pub label: String,
    pub image: Option<Arc<assets::Pixels>>,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct ItemDetails {
    pub kind: String,
    pub modified: String,
    pub folder: bool,
    pub modified_time: Option<std::time::SystemTime>,
    pub size: Option<u64>,
}

fn same_items(left: &[Item], right: &[Item]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| {
            a.identity == b.identity
                && a.details == b.details
                && a.label == b.label
                && match (&a.image, &b.image) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
        })
}

mod model;
use model::GroupModel;

struct View {
    id: PanelId,
    window: windows_window::Window,
    model: Rc<RefCell<GroupModel>>,
}

struct PaneApp {
    wake: wake::Wake,
    folders: HashMap<PanelId, folder::Source>,
    settings: Option<windows_window::Window>,
    // Desktop membership is suspended while Explorer/its compatible Hook is unavailable.
    session: Option<hybrid::Session>,
    drops: Vec<drag_drop::target::Registration>,
    runtime: Option<runtime::State>,
    workspace: Workspace,
    store: WorkspaceStore,
    views: Vec<View>,
    images: HashMap<String, Arc<assets::Pixels>>,
    receiver: mpsc::Receiver<Loaded>,
}

struct Loaded {
    requested: Vec<String>,
    images: Vec<(String, assets::Pixels)>,
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
    Refresh,
    RetryDesktop,
    ExportBackup,
    RestoreBackup,
    OpenBackups,
    OpenConfigDirectory,
    ReloadConfig,
    ToggleAutoHide,
    ToggleTopmost,
    ToggleLocked,
    SetCornerRadius(f32),
    SetIconGrid(f32),
    SetPanelText(desktop_core::PanelText),
    ToggleTextProtection,
    ToggleBorder,
    ToggleSnap,
    ResetPaneOptions,
    Theme(desktop_core::PanelTheme),
    SetCollapsed(bool),
    Moving(*mut RECT),
    Sizing(*mut RECT, RECT, u32),
    Material(desktop_core::Backdrop),
    New,
    EnableSearch,
    ToggleSearch,
    NewFolder,
    MapFolder(std::path::PathBuf),
    ChangeFolder,
    SetFolder(std::path::PathBuf),
    OpenFolder,
    SortFolder(u8),
    SetFolderColumns([f32; 4]),
    NavigateFolder(std::path::PathBuf),
    FolderBack,
    FolderHome,
    Activate(usize),
    ActivateSelection,
    Peek,
    FileCommand(desktop_shell::FileCommand),
    FileDrag,
    ToggleListView,
    Drop { index: usize, point: POINT },
    Geometry(RectDip),
    Collapse,
    Sort,
    Exit,
}

fn items_for(state: &PaneApp, id: PanelId) -> Vec<Item> {
    if state.workspace.panel(id).is_some_and(Panel::is_search) {
        return Vec::new();
    }
    if state
        .workspace
        .panel(id)
        .is_some_and(|p| p.folder().is_some())
    {
        return state
            .folders
            .get(&id)
            .map(|source| source.items.clone())
            .unwrap_or_default();
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
            details: Default::default(),
            identity: item.identity().clone(),
            label: item.display_name().to_string(),
            image: state.images.get(&item.identity().persistent_key()).cloned(),
        })
        .collect()
}

fn create_view(state: &Rc<RefCell<PaneApp>>, id: PanelId) -> Result<(), String> {
    folder::ensure(&mut state.borrow_mut(), id)?;
    let (panel, items) = {
        let s = state.borrow();
        (s.workspace.panel(id).unwrap().clone(), items_for(&s, id))
    };
    let model = Rc::new(RefCell::new(GroupModel {
        folder_sort: (0, false),
        folder_columns: folder::saved_columns(&state.borrow().store, id)?,
        folder_navigation: [false; 2],
        list_view: panel.list_view(),
        folder: panel.folder().map(Path::to_path_buf),
        folder_status: None,
        options: state.borrow().workspace.pane_options(),
        theme: panel.theme(),
        dark: theme::is_dark(panel.theme()),

        hovered_item: None,
        focused: false,
        auto_hide: panel.auto_hide(),
        locked: panel.locked(),
        reveal: if panel.collapsed() { 0.0 } else { 1.0 },
        hovered_button: None,
        pressed_button: None,
        backdrop: panel.backdrop(),
        native_material: false,
        title: panel.title().to_string(),
        items,
        icon_size: 48.0,
        selected: None,
        selection: Default::default(),
        selection_anchor: None,
        renaming: None,
        scroll: 0,
        collapsed: panel.collapsed(),
        // Desktop membership is available before creating the view. Only folder
        // sources have an asynchronous inventory to wait for; icons load separately.
        loading: panel.folder().is_some(),
    }));
    let weak = Rc::downgrade(state);
    let callback = move |event| {
        let Some(state) = weak.upgrade() else {
            return false;
        };
        let wake_needed = !matches!(&event, Event::Moving(_) | Event::Sizing(..));
        let result = handle(&state, id, event);
        if wake_needed {
            state.borrow().wake.notify();
        }
        match result {
            Ok(done) => done,
            Err(error) => {
                eprintln!("{error}");
                window::error(&error);
                false
            }
        }
    };
    let window = if panel.is_search() {
        search::create(panel.rect(), Rc::clone(&model), callback)?
    } else {
        window::create(panel.rect(), Rc::clone(&model), callback)?
    };
    window::set_layer(window.hwnd().cast(), panel.always_on_top());
    state.borrow_mut().views.push(View { id, window, model });
    display_layout::place(&state.borrow(), id);
    if state.borrow().session.is_some() || panel.folder().is_some() {
        hybrid::register_drop(state, id)?;
    }
    Ok(())
}

fn refresh_views(state: &mut PaneApp) {
    refresh_changed_views(state, false);
}

fn refresh_changed_views(state: &mut PaneApp, force: bool) {
    for view in &state.views {
        if state.workspace.panel(view.id).is_some_and(Panel::is_search) {
            continue;
        }
        let items = items_for(state, view.id);
        let mut model = view.model.borrow_mut();
        if model.folder.is_some() {
            model.folder_sort = state
                .folders
                .get(&view.id)
                .map_or((0, false), |source| source.sort);
            let navigation = state.folders.get(&view.id)
                .map_or([false; 2], folder::Source::navigation);
            let status = state
                .folders
                .get(&view.id)
                .and_then(|source| source.status.clone());
            let loading = state
                .folders
                .get(&view.id)
                .is_none_or(|source| source.loading);
            if model.folder_status != status || model.loading != loading || model.folder_navigation != navigation {
                model.folder_navigation = navigation;
                model.folder_status = status;
                model.loading = loading;
                unsafe {
                    InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0);
                }
            }
        }
        if !force && same_items(&model.items, &items) && !model.loading {
            continue;
        }
        model.replace_items(items);
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

fn save(state: &mut PaneApp) -> Result<(), String> {
    hybrid::sync(state)?;
    state
        .store
        .save_workspace(&state.workspace)
        .map_err(|e| format!("保存分组失败：{e}"))
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
    if workspace
        .panel(id)
        .is_some_and(|panel| panel.folder().is_some() || panel.is_search())
    {
        return;
    }
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
fn normalize_pane_orders(state: &mut PaneApp) {
    let ids: Vec<_> = state.workspace.panels().iter().map(Panel::id).collect();
    for id in ids {
        let items = items_for(state, id);
        set_order(&mut state.workspace, id, &items);
    }
}

#[cfg(test)]
fn transfer(
    state: &mut PaneApp,
    source: PanelId,
    index: usize,
    target: PanelId,
    at: usize,
) -> Result<(), String> {
    transfer_many(state, source, &[index], target, at)
}

fn transfer_many(
    state: &mut PaneApp,
    source: PanelId,
    indices: &[usize],
    target: PanelId,
    at: usize,
) -> Result<(), String> {
    if state.workspace.panel(target).is_none() {
        return Err("目标面板不可用".into());
    }
    let items = items_for(state, source);
    let selected: std::collections::BTreeSet<_> = indices.iter().copied().collect();
    if selected.is_empty() || selected.iter().any(|i| *i >= items.len()) {
        return Err("选中项目已变化，请重新拖动".into());
    }
    let old = state.workspace.clone();
    let moving: Vec<_> = items
        .iter()
        .enumerate()
        .filter(|(i, _)| selected.contains(i))
        .map(|(_, item)| item.clone())
        .collect();
    let remaining: Vec<_> = items
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !selected.contains(i))
        .map(|(_, item)| item)
        .collect();
    let insertion = if source == target {
        at.saturating_sub(selected.iter().filter(|i| **i < at).count())
    } else {
        at
    };
    set_order(&mut state.workspace, source, &remaining);
    let mut destination = if source == target {
        remaining
    } else {
        items_for(state, target)
    };
    destination.splice(
        insertion.min(destination.len())..insertion.min(destination.len()),
        moving,
    );
    set_order(&mut state.workspace, target, &destination);
    if let Err(e) = save(state) {
        state.workspace = old;
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
