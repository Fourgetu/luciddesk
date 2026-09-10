#![windows_subsystem = "windows"]

mod native_desktop;
mod hook_desktop;
mod hook_material;
mod preview;
mod tray;

use desktop_compositor::MaterialController;
use desktop_core::{
    DesktopItem, DesktopPlacement, MonitorId, Panel, PanelIcon, PanelId, PanelSource, PointDip,
    RectDip, ShellIdentity, Workspace,
};
use desktop_shell::{
    DesktopChangeSubscription, DesktopShellItem, KnownFolder, PortalItem, PortalItemKind,
    ProcessExitWaiter, ShellApartment, choose_icon_file, desktop_icon_view_available,
    desktop_icons_hidden, enumerate_desktop_namespace, item_from_path, known_folder_path,
    open_path, open_shell_identity, scan_folder, set_desktop_icons_hidden, system_icon_for_path,
};
use desktop_storage::WorkspaceStore;
use desktop_window::{
    DesktopHost, DesktopItemSurface, DesktopSurfaceEvent, DesktopSurfaceItem,
    DesktopSurfaceRenderModel, GroupEvent, GroupRenderModel, GroupWindow, MonitorDescriptor,
    RenderIcon, RenderItem, RenderItemKind, SharedDesktopSurfaceModel, ShellOwnedDesktopHost,
    close_window, enumerate_monitors, portal_content_changed_message, post_desktop_surface_changed,
    window_contains_screen_point,
};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[allow(clippy::too_many_lines)]
fn main() -> Result<(), String> {
    // Capture monitor metrics in the same coordinate space used by the window renderer.
    // windows-window otherwise initializes DPI awareness only when the first HWND is created.
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if run_restore_guard_if_requested(&arguments)? {
        return Ok(());
    }
    let _shell_apartment = ShellApartment::initialize_sta()
        .map_err(|error| format!("failed to initialize the Shell STA: {error}"))?;
    if run_restore_shell_if_requested(&arguments)? {
        return Ok(());
    }
    let options = parse_options(arguments)?;
    let managed_desktop_requested =
        options.folder.is_none() && options.mode == LaunchMode::ManagedDesktop;
    let database_path = database_path()?;
    if let Some(parent) = database_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create application data directory: {error}"))?;
    }
    if options.mode == LaunchMode::Preview {
        return preview::run(
            &database_path.with_file_name("preview.db"),
            options.folder.as_deref(),
            options.title,
        );
    }
    if options.mode == LaunchMode::HybridDesktop {
        return preview::run_hybrid(&database_path.with_file_name("hook-desktop.db"), options.title)
            .inspect_err(|error| desktop_window::NativeFrame::show_error(error));
    }
    if options.mode == LaunchMode::RedrawnDesktop {
        if options.folder.is_some() {
            return Err("桌面模式不接受文件夹路径，请使用 --preview".into());
        }
        return preview::run_desktop(
            &database_path.with_file_name("redrawn-desktop.db"),
            options.title,
        );
    }
    let takeover_marker_path = shell_takeover_marker_path(&database_path);
    recover_stale_shell_takeover(&takeover_marker_path)?;
    if options.mode == LaunchMode::HookDesktop {
        if options.folder.is_some() { return Err("原生 Hook 模式不接受文件夹参数".into()); }
        return hook_desktop::run(&database_path.with_file_name("hook-desktop.db"), options.title)
            .inspect_err(|error| desktop_window::NativeFrame::show_error(error));
    }
    if options.folder.is_none() && options.mode == LaunchMode::NativeDesktop {
        return native_desktop::run(
            &database_path.with_file_name("native-frames.db"),
            options.title,
        );
    }
    let mut store = WorkspaceStore::open(&database_path)
        .map_err(|error| format!("failed to open workspace: {error}"))?;
    let mut workspace = store
        .load_workspace()
        .map_err(|error| format!("failed to load workspace: {error}"))?;

    if let Some(folder) = options.folder.as_deref()
        && !folder.is_dir()
    {
        return Err(format!(
            "portal folder does not exist: {}",
            folder.display()
        ));
    }

    let panel_id = PanelId::new(1);
    if workspace.panel(panel_id).is_none() {
        let source = if let Some(folder) = options.folder.as_ref() {
            PanelSource::Folder {
                path: folder.clone(),
            }
        } else if managed_desktop_requested {
            PanelSource::DesktopCollection
        } else {
            PanelSource::ManualCollection { collection_id: 1 }
        };
        let mut panel = Panel::new(
            panel_id,
            options.title.clone().unwrap_or_else(|| {
                options
                    .folder
                    .as_deref()
                    .map_or_else(|| "新建分组".to_string(), folder_title)
            }),
            source,
            RectDip::default(),
        );
        if let Some(icon) = options.icon.clone() {
            panel.set_icon(PanelIcon::Custom(icon));
        }
        workspace
            .add_panel(panel)
            .map_err(|error| format!("failed to create default panel: {error}"))?;
    } else if let Some(panel) = workspace.panel_mut(panel_id) {
        if let Some(folder) = options.folder.as_ref() {
            panel.set_source(PanelSource::Folder {
                path: folder.clone(),
            });
            if options.title.is_none() {
                panel.set_title(folder_title(folder));
            }
        } else if managed_desktop_requested {
            panel.set_source(PanelSource::DesktopCollection);
            panel.replace_items([]);
            if options.title.is_none() {
                panel.set_title("新建分组");
            }
        } else {
            panel.set_source(PanelSource::ManualCollection { collection_id: 1 });
            panel.replace_items([]);
            if options.title.is_none() {
                panel.set_title("新建分组");
            }
        }
        if let Some(title) = options.title.clone() {
            panel.set_title(title);
        }
        if let Some(icon) = options.icon.clone() {
            panel.set_icon(PanelIcon::Custom(icon));
        }
    }

    let monitors = if managed_desktop_requested {
        let monitors = enumerate_monitors();
        if monitors.is_empty() {
            return Err("managed desktop requires at least one active monitor".to_string());
        }
        monitors
    } else {
        Vec::new()
    };
    let desktop_inventory = if managed_desktop_requested {
        let inventory = enumerate_desktop_namespace(0)
            .map_err(|error| format!("failed to enumerate the Desktop Shell Namespace: {error}"))?;
        reconcile_managed_desktop(&mut workspace, &inventory, &monitors);
        inventory
    } else {
        Vec::new()
    };

    let initial_panel = workspace
        .panel(panel_id)
        .cloned()
        .ok_or_else(|| "default panel is missing".to_string())?;
    let initial_items = load_panel_items(&initial_panel, &workspace, &desktop_inventory)?;

    store
        .save_workspace(&workspace)
        .map_err(|error| format!("failed to persist initial pane: {error}"))?;
    let store = Rc::new(RefCell::new(store));
    let items = Rc::new(RefCell::new(initial_items));
    let model = Rc::new(RefCell::new(render_model(&initial_panel, &items.borrow())));
    let workspace = Rc::new(RefCell::new(workspace));
    let desktop_inventory = Rc::new(RefCell::new(desktop_inventory));
    let surface_bindings = Rc::new(RefCell::new(Vec::<SurfaceBinding>::new()));
    let monitors = Rc::new(RefCell::new(monitors));
    let pane_hwnd = Rc::new(Cell::new(0_isize));
    let shell_change_subscription = Rc::new(RefCell::new(None::<DesktopChangeSubscription>));
    let managed_desktop_lease = Rc::new(RefCell::new(None::<ManagedDesktopLease>));
    let material = Rc::new(RefCell::new(None::<MaterialController>));
    let material_for_messages = Rc::clone(&material);
    let items_for_messages = Rc::clone(&items);
    let model_for_messages = Rc::clone(&model);
    let workspace_for_messages = Rc::clone(&workspace);
    let store_for_messages = Rc::clone(&store);
    let refresh_pending = Arc::new(AtomicBool::new(false));
    let refresh_pending_for_messages = Arc::clone(&refresh_pending);
    let desktop_inventory_for_messages = Rc::clone(&desktop_inventory);
    let surface_bindings_for_messages = Rc::clone(&surface_bindings);
    let monitors_for_messages = Rc::clone(&monitors);
    let shell_change_subscription_for_messages = Rc::clone(&shell_change_subscription);
    let managed_desktop_lease_for_messages = Rc::clone(&managed_desktop_lease);
    let surface_context = ManagedSurfaceContext {
        workspace: Rc::clone(&workspace),
        inventory: Rc::clone(&desktop_inventory),
        bindings: Rc::clone(&surface_bindings),
        pane_hwnd: Rc::clone(&pane_hwnd),
        pane_items: Rc::clone(&items),
        pane_model: Rc::clone(&model),
        store: Rc::clone(&store),
        panel_id,
    };
    let surface_context_for_messages = surface_context.clone();
    let desktop_surfaces = Rc::new(RefCell::new(Vec::<DesktopItemSurface>::new()));
    let desktop_surfaces_for_messages = Rc::clone(&desktop_surfaces);
    if managed_desktop_requested {
        for monitor in monitors.borrow().iter() {
            let (surface, binding) = build_desktop_surface(monitor, &surface_context)?;
            surface_bindings.borrow_mut().push(binding);
            desktop_surfaces.borrow_mut().push(surface);
        }
    }

    let window = GroupWindow::new(
        initial_panel.backdrop(),
        initial_panel.rect(),
        initial_panel.collapsed(),
        &model,
        move |raw_hwnd, event| match event {
            GroupEvent::MaterialSelected(selected) => {
                if let Some(controller) = material_for_messages.borrow().as_ref()
                    && let Err(error) = controller.apply(selected)
                {
                    eprintln!("failed to apply {}: {error}", selected.label());
                }
                if let Some(panel) = workspace_for_messages.borrow_mut().panel_mut(panel_id) {
                    panel.set_backdrop(selected);
                }
            }
            GroupEvent::ActivateItem(index) => {
                if let Some(item) = items_for_messages.borrow().get(index)
                    && let Err(error) = item.open(raw_hwnd as isize)
                {
                    eprintln!("failed to open pane item: {error}");
                }
            }
            GroupEvent::RefreshRequested => {
                if managed_desktop_requested {
                    let monitors = monitors_for_messages.borrow();
                    let refresh_result = refresh_managed_desktop(
                        &mut workspace_for_messages.borrow_mut(),
                        &mut desktop_inventory_for_messages.borrow_mut(),
                        &mut surface_bindings_for_messages.borrow_mut(),
                        &monitors,
                    );
                    if let Err(error) = refresh_result {
                        eprintln!("failed to refresh managed desktop: {error}");
                    }
                }
                let refreshed = {
                    let mut workspace = workspace_for_messages.borrow_mut();
                    workspace.panel_mut(panel_id).map(|panel| panel.clone())
                };
                if let Some(panel) = refreshed {
                    match load_panel_items(
                        &panel,
                        &workspace_for_messages.borrow(),
                        &desktop_inventory_for_messages.borrow(),
                    ) {
                        Ok(refreshed) => {
                            *model_for_messages.borrow_mut() = render_model(&panel, &refreshed);
                            *items_for_messages.borrow_mut() = refreshed;
                        }
                        Err(error) => eprintln!("failed to refresh pane: {error}"),
                    }
                }
                refresh_pending_for_messages.store(false, Ordering::Release);
                persist_workspace(
                    &store_for_messages,
                    &workspace_for_messages,
                    "content refresh",
                );
            }
            GroupEvent::GeometryChanged(rect) => {
                if let Some(panel) = workspace_for_messages.borrow_mut().panel_mut(panel_id) {
                    panel.set_rect(rect);
                }
            }
            GroupEvent::CollapsedChanged(collapsed) => {
                if let Some(panel) = workspace_for_messages.borrow_mut().panel_mut(panel_id) {
                    panel.set_collapsed(collapsed);
                }
            }
            GroupEvent::TitleChanged(title) => {
                if let Some(panel) = workspace_for_messages.borrow_mut().panel_mut(panel_id) {
                    panel.set_title(title);
                }
            }
            GroupEvent::ChooseIconRequested => {
                match choose_icon_file(raw_hwnd as isize) {
                    Ok(Some(path)) => {
                        let mut workspace = workspace_for_messages.borrow_mut();
                        if let Some(panel) = workspace.panel_mut(panel_id) {
                            panel.set_icon(PanelIcon::Custom(path));
                            let updated = render_model(panel, &items_for_messages.borrow());
                            drop(workspace);
                            *model_for_messages.borrow_mut() = updated;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => eprintln!("failed to choose pane icon: {error}"),
                }
                persist_workspace(
                    &store_for_messages,
                    &workspace_for_messages,
                    "pane icon change",
                );
            }
            GroupEvent::ResetIconRequested => {
                {
                    let mut workspace = workspace_for_messages.borrow_mut();
                    if let Some(panel) = workspace.panel_mut(panel_id) {
                        panel.set_icon(PanelIcon::Automatic);
                        let updated = render_model(panel, &items_for_messages.borrow());
                        drop(workspace);
                        *model_for_messages.borrow_mut() = updated;
                    }
                }
                persist_workspace(
                    &store_for_messages,
                    &workspace_for_messages,
                    "pane icon reset",
                );
            }
            GroupEvent::ItemsDropped(paths) => {
                let panel = {
                    let mut workspace = workspace_for_messages.borrow_mut();
                    let Some(panel) = workspace.panel_mut(panel_id) else {
                        return;
                    };
                    if !matches!(panel.source(), PanelSource::ManualCollection { .. }) {
                        eprintln!("drop-to-add is only supported by manual panes");
                        return;
                    }
                    add_dropped_items(panel, paths);
                    panel.clone()
                };
                match load_panel_items(
                    &panel,
                    &workspace_for_messages.borrow(),
                    &desktop_inventory_for_messages.borrow(),
                ) {
                    Ok(refreshed) => {
                        *model_for_messages.borrow_mut() = render_model(&panel, &refreshed);
                        *items_for_messages.borrow_mut() = refreshed;
                    }
                    Err(error) => eprintln!("failed to add dropped desktop items: {error}"),
                }
                persist_workspace(
                    &store_for_messages,
                    &workspace_for_messages,
                    "manual item drop",
                );
            }
            GroupEvent::MoveItem { from, to } => {
                let managed = workspace_for_messages
                    .borrow()
                    .panel(panel_id)
                    .is_some_and(|panel| matches!(panel.source(), PanelSource::DesktopCollection));
                if managed {
                    let identities = {
                        let mut pane_items = items_for_messages.borrow_mut();
                        if from >= pane_items.len() || to >= pane_items.len() || from == to {
                            return;
                        }
                        let item = pane_items.remove(from);
                        pane_items.insert(to, item);
                        pane_items
                            .iter()
                            .filter_map(PaneItem::shell_identity)
                            .cloned()
                            .collect::<Vec<_>>()
                    };
                    let mut workspace = workspace_for_messages.borrow_mut();
                    for (order, identity) in identities.iter().enumerate() {
                        if let Some(item) = workspace.desktop_item_mut(identity) {
                            item.set_placement(DesktopPlacement::Pane {
                                pane_id: panel_id,
                                position: desktop_core::GridPosition::new(
                                    0,
                                    u32::try_from(order).unwrap_or(u32::MAX),
                                ),
                            });
                        }
                    }
                    if let Some(panel) = workspace.panel(panel_id) {
                        *model_for_messages.borrow_mut() =
                            render_model(panel, &items_for_messages.borrow());
                    }
                    return;
                }
                let panel = {
                    let mut workspace = workspace_for_messages.borrow_mut();
                    let Some(panel) = workspace.panel_mut(panel_id) else {
                        return;
                    };
                    if !panel.move_item(from, to) {
                        return;
                    }
                    panel.clone()
                };
                match load_panel_items(
                    &panel,
                    &workspace_for_messages.borrow(),
                    &desktop_inventory_for_messages.borrow(),
                ) {
                    Ok(reordered) => {
                        *model_for_messages.borrow_mut() = render_model(&panel, &reordered);
                        *items_for_messages.borrow_mut() = reordered;
                    }
                    Err(error) => eprintln!("failed to reorder pane items: {error}"),
                }
            }
            GroupEvent::DropItem {
                index,
                screen_x,
                screen_y,
            } => {
                if window_contains_screen_point(raw_hwnd as isize, screen_x, screen_y) {
                    return;
                }
                let identity = items_for_messages
                    .borrow()
                    .get(index)
                    .and_then(PaneItem::shell_identity)
                    .cloned();
                let Some(identity) = identity else {
                    return;
                };
                let Some((monitor, position)) = free_placement_from_screen(
                    &monitors_for_messages.borrow(),
                    screen_x,
                    screen_y,
                )
                else {
                    return;
                };
                if let Some(item) = workspace_for_messages
                    .borrow_mut()
                    .desktop_item_mut(&identity)
                {
                    item.set_placement(DesktopPlacement::FreeDesktop { monitor, position });
                }
                rebuild_surface_bindings(
                    &workspace_for_messages.borrow(),
                    &desktop_inventory_for_messages.borrow(),
                    &mut surface_bindings_for_messages.borrow_mut(),
                );
                let panel = workspace_for_messages.borrow().panel(panel_id).cloned();
                if let Some(panel) = panel
                    && let Ok(updated) = load_panel_items(
                        &panel,
                        &workspace_for_messages.borrow(),
                        &desktop_inventory_for_messages.borrow(),
                    )
                {
                    *model_for_messages.borrow_mut() = render_model(&panel, &updated);
                    *items_for_messages.borrow_mut() = updated;
                }
            }
            GroupEvent::SelectionChanged(selection) => {
                let selected_name = selection.and_then(|index| {
                    items_for_messages
                        .borrow()
                        .get(index)
                        .map(|item| item.display_name.clone())
                });
                if let Some(name) = selected_name {
                    if let Ok(mut model) = model_for_messages.try_borrow_mut() {
                        model.subtitle = format!("Selected: {name}");
                    } else {
                        eprintln!("selection render update was deferred because the model is busy");
                    }
                } else if let Some(panel) = workspace_for_messages.borrow().panel(panel_id) {
                    let updated = render_model(panel, &items_for_messages.borrow());
                    if let Ok(mut model) = model_for_messages.try_borrow_mut() {
                        *model = updated;
                    } else {
                        eprintln!("selection clear was deferred because the model is busy");
                    }
                }
            }
            GroupEvent::ShellRestarted => {
                if !managed_desktop_requested {
                    return;
                }
                match ShellOwnedDesktopHost::new() {
                    Ok(mut host) => {
                        if let Err(error) = host.attach(raw_hwnd) {
                            eprintln!("failed to reattach the Pane after Explorer restart: {error}");
                        }
                        for binding in surface_bindings_for_messages.borrow().iter() {
                            if let Err(error) =
                                host.attach(binding.hwnd as *mut std::ffi::c_void)
                            {
                                eprintln!(
                                    "failed to reattach desktop surface after Explorer restart: {error}"
                                );
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("Explorer restarted but its Shell window is unavailable: {error}");
                    }
                }
                if let Err(error) = set_desktop_icons_hidden(true) {
                    eprintln!("failed to re-hide Explorer icons after restart: {error}");
                }
                match DesktopChangeSubscription::register(
                    raw_hwnd as isize,
                    portal_content_changed_message(),
                ) {
                    Ok(subscription) => {
                        shell_change_subscription_for_messages
                            .replace(Some(subscription));
                    }
                    Err(error) => {
                        eprintln!("failed to restore Desktop Shell notifications: {error}");
                    }
                }
                if let Err(error) = refresh_managed_desktop(
                    &mut workspace_for_messages.borrow_mut(),
                    &mut desktop_inventory_for_messages.borrow_mut(),
                    &mut surface_bindings_for_messages.borrow_mut(),
                    &monitors_for_messages.borrow(),
                ) {
                    eprintln!("failed to reconcile the desktop after Explorer restart: {error}");
                }
                persist_workspace(
                    &store_for_messages,
                    &workspace_for_messages,
                    "Explorer restart",
                );
            }
            GroupEvent::DisplayConfigurationChanged => {
                if !managed_desktop_requested {
                    return;
                }
                let refreshed_monitors = enumerate_monitors();
                if refreshed_monitors.is_empty() {
                    eprintln!("display configuration changed but no active monitor was found");
                    return;
                }
                if let Err(error) = recreate_desktop_surfaces(
                    refreshed_monitors,
                    &surface_context_for_messages,
                    &desktop_surfaces_for_messages,
                    &monitors_for_messages,
                ) {
                    eprintln!("failed to rebuild desktop surfaces after display change: {error}");
                    return;
                }
                if let Ok(mut host) = ShellOwnedDesktopHost::new()
                    && let Err(error) = host.attach(raw_hwnd)
                {
                    eprintln!("failed to restore Pane Z-order after display change: {error}");
                }
                persist_workspace(
                    &store_for_messages,
                    &workspace_for_messages,
                    "display reconfiguration",
                );
            }
            GroupEvent::ShellSettingsChanged => {
                let takeover_active = managed_desktop_lease_for_messages.borrow().is_some();
                let explorer_icons_hidden = desktop_icons_hidden();
                if managed_desktop_requested
                    && takeover_active
                    && desktop_icon_view_available()
                    && !explorer_icons_hidden
                {
                    managed_desktop_lease_for_messages.borrow_mut().take();
                    close_window(raw_hwnd as isize);
                }
            }
            GroupEvent::SessionEnding => {
                persist_workspace(
                    &store_for_messages,
                    &workspace_for_messages,
                    "session shutdown",
                );
                managed_desktop_lease_for_messages.borrow_mut().take();
                close_window(raw_hwnd as isize);
            }
            GroupEvent::CommitRequested => {
                persist_workspace(
                    &store_for_messages,
                    &workspace_for_messages,
                    "interaction commit",
                );
            }
        },
    )
    .map_err(|error| format!("failed to create group window: {error}"))?;
    pane_hwnd.set(window.hwnd_token());
    if managed_desktop_requested {
        shell_change_subscription.replace(Some(
            DesktopChangeSubscription::register(
                window.hwnd_token(),
                portal_content_changed_message(),
            )
            .map_err(|error| format!("failed to watch the Desktop Shell Namespace: {error}"))?,
        ));
    }

    // SAFETY: GroupWindow owns this live HWND on the current UI thread and outlives the
    // controller stored below.
    let controller = unsafe { MaterialController::new(window.hwnd()) }
        .map_err(|error| format!("failed to initialize materials: {error}"))?;
    material.replace(Some(controller));
    let _attachment = window.attach_to_desktop();
    if managed_desktop_requested {
        managed_desktop_lease.replace(Some(ManagedDesktopLease::acquire(takeover_marker_path)?));
    }

    let hwnd = window.hwnd_token();
    let refresh_pending_for_watcher = Arc::clone(&refresh_pending);
    let watched_folders = panel_folders(&initial_panel);
    let watcher = if watched_folders.is_empty() {
        None
    } else {
        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<notify::Event>| match result {
                Ok(_) => {
                    if !refresh_pending_for_watcher.swap(true, Ordering::AcqRel)
                        && !desktop_window::post_portal_content_changed(hwnd)
                    {
                        refresh_pending_for_watcher.store(false, Ordering::Release);
                    }
                }
                Err(error) => eprintln!("folder watcher error: {error}"),
            },
            Config::default(),
        )
        .map_err(|error| format!("failed to create folder watcher: {error}"))?;
        for folder in watched_folders {
            watcher
                .watch(&folder, RecursiveMode::NonRecursive)
                .map_err(|error| format!("failed to watch {}: {error}", folder.display()))?;
        }
        Some(watcher)
    };

    GroupWindow::run();
    drop(watcher);
    shell_change_subscription.borrow_mut().take();
    drop(window);
    drop(desktop_surfaces);
    managed_desktop_lease.borrow_mut().take();
    store
        .borrow_mut()
        .save_workspace(&workspace.borrow())
        .map_err(|error| format!("failed to save workspace: {error}"))?;
    Ok(())
}

fn database_path() -> Result<PathBuf, String> {
    if let Some(root) = std::env::var_os("LUCIDPANE_DATA_DIR") {
        return Ok(PathBuf::from(root).join("lucidpane.db"));
    }
    let root = known_folder_path(KnownFolder::LocalAppData)
        .map_err(|error| format!("failed to resolve LocalAppData: {error}"))?;
    Ok(root.join("LucidPane").join("lucidpane.db"))
}

struct SurfaceBinding {
    monitor: MonitorDescriptor,
    model: SharedDesktopSurfaceModel,
    identities: Rc<RefCell<Vec<ShellIdentity>>>,
    hwnd: isize,
}

#[derive(Clone)]
struct ManagedSurfaceContext {
    workspace: Rc<RefCell<Workspace>>,
    inventory: Rc<RefCell<Vec<DesktopShellItem>>>,
    bindings: Rc<RefCell<Vec<SurfaceBinding>>>,
    pane_hwnd: Rc<Cell<isize>>,
    pane_items: Rc<RefCell<Vec<PaneItem>>>,
    pane_model: Rc<RefCell<GroupRenderModel>>,
    store: Rc<RefCell<WorkspaceStore>>,
    panel_id: PanelId,
}

fn persist_workspace(
    store: &Rc<RefCell<WorkspaceStore>>,
    workspace: &Rc<RefCell<Workspace>>,
    reason: &str,
) {
    if let Err(error) = store.borrow_mut().save_workspace(&workspace.borrow()) {
        eprintln!("failed to persist workspace after {reason}: {error}");
    }
}

fn build_desktop_surface(
    monitor: &MonitorDescriptor,
    context: &ManagedSurfaceContext,
) -> Result<(DesktopItemSurface, SurfaceBinding), String> {
    let (surface_model, identities) = surface_contents(
        &context.workspace.borrow(),
        &context.inventory.borrow(),
        &monitor.id,
    );
    let surface_model: SharedDesktopSurfaceModel = Rc::new(RefCell::new(surface_model));
    let identities = Rc::new(RefCell::new(identities));
    let identities_for_events = Rc::clone(&identities);
    let context_for_events = context.clone();
    let monitor_id = monitor.id.clone();
    let surface =
        DesktopItemSurface::new(
            monitor,
            &surface_model,
            move |raw_hwnd, event| match event {
                DesktopSurfaceEvent::ActivateItem(index) => {
                    if let Some(identity) = identities_for_events.borrow().get(index)
                        && let Err(error) = open_shell_identity(raw_hwnd as isize, identity)
                    {
                        eprintln!("failed to open desktop Shell item: {error}");
                    }
                }
                DesktopSurfaceEvent::MoveItem { index, position } => {
                    if let Some(identity) = identities_for_events.borrow().get(index)
                        && let Some(item) = context_for_events
                            .workspace
                            .borrow_mut()
                            .desktop_item_mut(identity)
                    {
                        item.set_placement(DesktopPlacement::FreeDesktop {
                            monitor: monitor_id.clone(),
                            position,
                        });
                    }
                }
                DesktopSurfaceEvent::DropItem {
                    index,
                    screen_x,
                    screen_y,
                } => {
                    let target = context_for_events.pane_hwnd.get();
                    if target != 0
                        && window_contains_screen_point(target, screen_x, screen_y)
                        && let Some(identity) = identities_for_events.borrow().get(index).cloned()
                    {
                        assign_desktop_item_to_pane(
                            &mut context_for_events.workspace.borrow_mut(),
                            &identity,
                            context_for_events.panel_id,
                        );
                        rebuild_surface_bindings(
                            &context_for_events.workspace.borrow(),
                            &context_for_events.inventory.borrow(),
                            &mut context_for_events.bindings.borrow_mut(),
                        );
                        let panel = context_for_events
                            .workspace
                            .borrow()
                            .panel(context_for_events.panel_id)
                            .cloned();
                        if let Some(panel) = panel
                            && let Ok(updated) = load_panel_items(
                                &panel,
                                &context_for_events.workspace.borrow(),
                                &context_for_events.inventory.borrow(),
                            )
                        {
                            *context_for_events.pane_model.borrow_mut() =
                                render_model(&panel, &updated);
                            *context_for_events.pane_items.borrow_mut() = updated;
                        }
                    }
                    persist_workspace(
                        &context_for_events.store,
                        &context_for_events.workspace,
                        "desktop drag",
                    );
                }
                DesktopSurfaceEvent::SelectionChanged(_) => {}
            },
        )
        .map_err(|error| format!("failed to create desktop item surface: {error}"))?;
    let binding = SurfaceBinding {
        monitor: monitor.clone(),
        model: surface_model,
        identities,
        hwnd: surface.hwnd_token(),
    };
    Ok((surface, binding))
}

fn recreate_desktop_surfaces(
    refreshed_monitors: Vec<MonitorDescriptor>,
    context: &ManagedSurfaceContext,
    surfaces: &Rc<RefCell<Vec<DesktopItemSurface>>>,
    monitors: &Rc<RefCell<Vec<MonitorDescriptor>>>,
) -> Result<(), String> {
    let previous: HashSet<_> = context
        .workspace
        .borrow()
        .desktop_items()
        .iter()
        .map(|item| item.identity().persistent_key())
        .collect();
    normalize_desktop_placements(
        &mut context.workspace.borrow_mut(),
        &refreshed_monitors,
        &previous,
    );
    let mut new_surfaces = Vec::with_capacity(refreshed_monitors.len());
    let mut new_bindings = Vec::with_capacity(refreshed_monitors.len());
    for monitor in &refreshed_monitors {
        let (surface, binding) = build_desktop_surface(monitor, context)?;
        new_surfaces.push(surface);
        new_bindings.push(binding);
    }
    *surfaces.borrow_mut() = new_surfaces;
    *context.bindings.borrow_mut() = new_bindings;
    *monitors.borrow_mut() = refreshed_monitors;
    Ok(())
}

fn refresh_managed_desktop(
    workspace: &mut Workspace,
    inventory: &mut Vec<DesktopShellItem>,
    surfaces: &mut [SurfaceBinding],
    monitors: &[MonitorDescriptor],
) -> Result<(), String> {
    let refreshed = enumerate_desktop_namespace(0)
        .map_err(|error| format!("Desktop Shell Namespace enumeration failed: {error}"))?;
    reconcile_managed_desktop(workspace, &refreshed, monitors);
    inventory.clone_from(&refreshed);
    rebuild_surface_bindings(workspace, inventory, surfaces);
    Ok(())
}

fn rebuild_surface_bindings(
    workspace: &Workspace,
    inventory: &[DesktopShellItem],
    surfaces: &mut [SurfaceBinding],
) {
    for surface in surfaces {
        let (model, identities) = surface_contents(workspace, inventory, &surface.monitor.id);
        *surface.model.borrow_mut() = model;
        *surface.identities.borrow_mut() = identities;
        let _ = post_desktop_surface_changed(surface.hwnd);
    }
}

fn assign_desktop_item_to_pane(
    workspace: &mut Workspace,
    identity: &ShellIdentity,
    pane_id: PanelId,
) -> bool {
    let next = u32::try_from(
        workspace
            .desktop_items()
            .iter()
            .filter(|item| {
                matches!(
                    item.placement(),
                    DesktopPlacement::Pane { pane_id: id, .. } if *id == pane_id
                )
            })
            .count(),
    )
    .unwrap_or(u32::MAX);
    let Some(item) = workspace.desktop_item_mut(identity) else {
        return false;
    };
    item.set_placement(DesktopPlacement::Pane {
        pane_id,
        position: desktop_core::GridPosition::new(0, next),
    });
    true
}

fn reconcile_managed_desktop(
    workspace: &mut Workspace,
    inventory: &[DesktopShellItem],
    monitors: &[MonitorDescriptor],
) {
    let previous: HashSet<_> = workspace
        .desktop_items()
        .iter()
        .map(|item| item.identity().persistent_key())
        .collect();
    workspace.reconcile_desktop_items(
        inventory
            .iter()
            .map(|item| DesktopItem::new(item.identity.clone(), item.display_name.clone())),
    );
    normalize_desktop_placements(workspace, monitors, &previous);
}

fn normalize_desktop_placements(
    workspace: &mut Workspace,
    monitors: &[MonitorDescriptor],
    previous: &HashSet<String>,
) {
    let Some(primary) = monitors
        .iter()
        .find(|monitor| monitor.primary)
        .or_else(|| monitors.first())
    else {
        return;
    };
    let panel_ids: HashSet<_> = workspace.panels().iter().map(Panel::id).collect();
    let monitor_ids: HashSet<_> = monitors.iter().map(|monitor| monitor.id.clone()).collect();
    let mut occupied: HashSet<(MonitorId, i32, i32)> = workspace
        .desktop_items()
        .iter()
        .filter_map(|item| match item.placement() {
            DesktopPlacement::FreeDesktop { monitor, position }
                if previous.contains(&item.identity().persistent_key())
                    && monitor_ids.contains(monitor) =>
            {
                Some((
                    monitor.clone(),
                    position_component_key(position.x),
                    position_component_key(position.y),
                ))
            }
            _ => None,
        })
        .collect();

    for item in workspace.desktop_items_mut() {
        let key = item.identity().persistent_key();
        match item.placement() {
            DesktopPlacement::Pane { pane_id, .. } if panel_ids.contains(pane_id) => continue,
            DesktopPlacement::FreeDesktop { monitor, position }
                if previous.contains(&key) && monitor_ids.contains(monitor) =>
            {
                let monitor = monitors
                    .iter()
                    .find(|descriptor| descriptor.id == *monitor)
                    .unwrap_or(primary);
                item.set_placement(DesktopPlacement::FreeDesktop {
                    monitor: monitor.id.clone(),
                    position: clamp_free_position(*position, monitor),
                });
                continue;
            }
            _ => {}
        }
        let position = next_free_position(primary, &occupied);
        occupied.insert((
            primary.id.clone(),
            position_component_key(position.x),
            position_component_key(position.y),
        ));
        item.set_placement(DesktopPlacement::FreeDesktop {
            monitor: primary.id.clone(),
            position,
        });
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn next_free_position(
    monitor: &MonitorDescriptor,
    occupied: &HashSet<(MonitorId, i32, i32)>,
) -> PointDip {
    let scale = monitor.dpi as f32 / 96.0;
    let start_x = ((monitor.work_area.x - monitor.bounds.x) as f32 / scale).max(8.0);
    let start_y = ((monitor.work_area.y - monitor.bounds.y) as f32 / scale).max(8.0);
    let usable_height = monitor.work_area.height as f32 / scale;
    let rows = ((usable_height - start_y) / 88.0).floor().max(1.0) as usize;
    for slot in 0..10_000_usize {
        let position = PointDip::new(
            start_x + (slot / rows) as f32 * 96.0,
            start_y + (slot % rows) as f32 * 88.0,
        );
        if !occupied.contains(&(
            monitor.id.clone(),
            position_component_key(position.x),
            position_component_key(position.y),
        )) {
            return position;
        }
    }
    PointDip::new(start_x, start_y)
}

#[allow(clippy::cast_precision_loss)]
fn clamp_free_position(position: PointDip, monitor: &MonitorDescriptor) -> PointDip {
    let scale = monitor.dpi as f32 / 96.0;
    let min_x = ((monitor.work_area.x - monitor.bounds.x) as f32 / scale).max(0.0);
    let min_y = ((monitor.work_area.y - monitor.bounds.y) as f32 / scale).max(0.0);
    let max_x = (min_x + monitor.work_area.width as f32 / scale - 96.0).max(min_x);
    let max_y = (min_y + monitor.work_area.height as f32 / scale - 88.0).max(min_y);
    PointDip::new(
        position.x.clamp(min_x, max_x),
        position.y.clamp(min_y, max_y),
    )
}

#[allow(clippy::cast_precision_loss)]
fn free_placement_from_screen(
    monitors: &[MonitorDescriptor],
    screen_x: i32,
    screen_y: i32,
) -> Option<(MonitorId, PointDip)> {
    let monitor = monitors.iter().find(|monitor| {
        screen_x >= monitor.bounds.x
            && screen_x < monitor.bounds.x + monitor.bounds.width
            && screen_y >= monitor.bounds.y
            && screen_y < monitor.bounds.y + monitor.bounds.height
    })?;
    let scale = monitor.dpi as f32 / 96.0;
    let position = PointDip::new(
        (screen_x - monitor.bounds.x) as f32 / scale - 48.0,
        (screen_y - monitor.bounds.y) as f32 / scale - 44.0,
    );
    Some((monitor.id.clone(), clamp_free_position(position, monitor)))
}

#[allow(clippy::cast_possible_truncation)]
fn position_component_key(value: f32) -> i32 {
    value.round() as i32
}

fn surface_contents(
    workspace: &Workspace,
    inventory: &[DesktopShellItem],
    monitor: &MonitorId,
) -> (DesktopSurfaceRenderModel, Vec<ShellIdentity>) {
    let mut model = DesktopSurfaceRenderModel::default();
    let mut identities = Vec::new();
    for item in workspace.desktop_items() {
        let DesktopPlacement::FreeDesktop {
            monitor: item_monitor,
            position,
        } = item.placement()
        else {
            continue;
        };
        if item_monitor != monitor {
            continue;
        }
        let key = item.identity().persistent_key();
        let Some(shell_item) = inventory
            .iter()
            .find(|shell_item| shell_item.identity.persistent_key() == key)
        else {
            continue;
        };
        model.items.push(DesktopSurfaceItem {
            label: shell_item.display_name.clone(),
            icon: shell_item.system_icon.map(|icon| RenderIcon {
                image_list: icon.image_list,
                index: icon.index,
            }),
            position: *position,
        });
        identities.push(shell_item.identity.clone());
    }
    (model, identities)
}

fn panel_folders(panel: &Panel) -> Vec<PathBuf> {
    match panel.source() {
        PanelSource::Folder { path } => vec![path.clone()],
        PanelSource::DesktopCollection | PanelSource::ManualCollection { .. } => Vec::new(),
    }
}

fn folder_title(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[derive(Clone, Debug)]
struct PaneItem {
    display_name: String,
    kind: PortalItemKind,
    icon: Option<RenderIcon>,
    activation: PaneActivation,
}

#[derive(Clone, Debug)]
enum PaneActivation {
    Path(PathBuf),
    Shell(ShellIdentity),
}

impl PaneItem {
    fn from_portal(item: PortalItem) -> Self {
        Self {
            display_name: item.display_name,
            kind: item.kind,
            icon: item.system_icon.map(|icon| RenderIcon {
                image_list: icon.image_list,
                index: icon.index,
            }),
            activation: PaneActivation::Path(item.path),
        }
    }

    fn open(&self, owner: isize) -> Result<(), desktop_shell::ShellError> {
        match &self.activation {
            PaneActivation::Path(path) => open_path(owner, path),
            PaneActivation::Shell(identity) => open_shell_identity(owner, identity),
        }
    }

    fn shell_identity(&self) -> Option<&ShellIdentity> {
        match &self.activation {
            PaneActivation::Path(_) => None,
            PaneActivation::Shell(identity) => Some(identity),
        }
    }
}

fn load_panel_items(
    panel: &Panel,
    workspace: &Workspace,
    desktop_inventory: &[DesktopShellItem],
) -> Result<Vec<PaneItem>, String> {
    match panel.source() {
        PanelSource::Folder { path } => scan_folder(path)
            .map(|items| items.into_iter().map(PaneItem::from_portal).collect())
            .map_err(|error| format!("failed to scan {}: {error}", path.display())),
        PanelSource::ManualCollection { .. } => Ok(panel
            .item_paths()
            .iter()
            .filter_map(|path| match item_from_path(path) {
                Ok(item) => Some(PaneItem::from_portal(item)),
                Err(error) => {
                    eprintln!("skipping unavailable pane item {}: {error}", path.display());
                    None
                }
            })
            .collect()),
        PanelSource::DesktopCollection => {
            let mut placed: Vec<_> = workspace
                .desktop_items()
                .iter()
                .filter_map(|item| match item.placement() {
                    DesktopPlacement::Pane { pane_id, position } if *pane_id == panel.id() => {
                        Some((position, item))
                    }
                    _ => None,
                })
                .collect();
            placed.sort_by_key(|(position, _)| (position.row, position.column));
            Ok(placed
                .into_iter()
                .filter_map(|(_, item)| {
                    let key = item.identity().persistent_key();
                    desktop_inventory
                        .iter()
                        .find(|shell_item| shell_item.identity.persistent_key() == key)
                        .map(|shell_item| PaneItem {
                            display_name: shell_item.display_name.clone(),
                            kind: shell_item.kind(),
                            icon: shell_item.system_icon.map(|icon| RenderIcon {
                                image_list: icon.image_list,
                                index: icon.index,
                            }),
                            activation: PaneActivation::Shell(shell_item.identity.clone()),
                        })
                })
                .collect())
        }
    }
}

fn add_dropped_items(panel: &mut Panel, paths: impl IntoIterator<Item = PathBuf>) -> usize {
    if !matches!(panel.source(), PanelSource::ManualCollection { .. }) {
        return 0;
    }
    paths
        .into_iter()
        .filter(|path| panel.add_item(path.clone()))
        .count()
}

fn render_model(panel: &Panel, items: &[PaneItem]) -> GroupRenderModel {
    let icon_path = match (panel.icon(), panel.source()) {
        (PanelIcon::Custom(path), _) | (PanelIcon::Automatic, PanelSource::Folder { path }) => {
            Some(path)
        }
        (PanelIcon::Automatic, _) => None,
    };
    let header_icon = icon_path
        .and_then(|path| system_icon_for_path(path))
        .map(|icon| RenderIcon {
            image_list: icon.image_list,
            index: icon.index,
        });
    GroupRenderModel {
        title: panel.title().to_string(),
        subtitle: match panel.source() {
            PanelSource::Folder { .. } => format!("{} folder items | auto grid", items.len()),
            PanelSource::DesktopCollection => {
                format!("{} grouped icons | managed desktop", items.len())
            }
            PanelSource::ManualCollection { .. } => {
                format!("{} desktop icons | drop to add", items.len())
            }
        },
        header_icon,
        custom_header_icon: match panel.icon() {
            PanelIcon::Automatic => None,
            PanelIcon::Custom(path) => Some(path.clone()),
        },
        items: items
            .iter()
            .map(|item| RenderItem {
                label: item.display_name.clone(),
                icon: item.icon,
                kind: match item.kind {
                    PortalItemKind::Directory => RenderItemKind::Directory,
                    PortalItemKind::File => RenderItemKind::File,
                    PortalItemKind::Shortcut => RenderItemKind::Shortcut,
                },
            })
            .collect(),
    }
}

const RESTORE_GUARD_OPTION: &str = "--desktop-restore-guard";
const RESTORE_SHELL_OPTION: &str = "--restore-shell";
const TAKEOVER_MARKER_FILE: &str = "shell-takeover.state";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct ManagedDesktopLease {
    original_hidden: bool,
    changed: bool,
    marker_path: Option<PathBuf>,
}

impl ManagedDesktopLease {
    fn acquire(marker_path: PathBuf) -> Result<Self, String> {
        let original_hidden = desktop_icons_hidden();
        if original_hidden {
            return Ok(Self {
                original_hidden,
                changed: false,
                marker_path: None,
            });
        }

        let marker = ShellTakeoverMarker {
            process_id: std::process::id(),
            original_hidden,
        };
        write_shell_takeover_marker(&marker_path, marker)?;
        if let Err(error) = spawn_restore_guard(original_hidden, &marker_path) {
            let _ = remove_shell_takeover_marker(&marker_path);
            return Err(error);
        }
        if let Err(error) = set_desktop_icons_hidden(true) {
            let _ = remove_shell_takeover_marker(&marker_path);
            return Err(format!("failed to hide Explorer desktop icons: {error}"));
        }
        Ok(Self {
            original_hidden,
            changed: true,
            marker_path: Some(marker_path),
        })
    }
}

impl Drop for ManagedDesktopLease {
    fn drop(&mut self) {
        if self.changed {
            match set_desktop_icons_hidden(self.original_hidden) {
                Ok(()) => {
                    if let Some(marker_path) = self.marker_path.as_deref()
                        && let Err(error) = remove_shell_takeover_marker(marker_path)
                    {
                        eprintln!("failed to remove desktop takeover marker: {error}");
                    }
                }
                Err(error) => eprintln!("failed to restore Explorer desktop icons: {error}"),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShellTakeoverMarker {
    process_id: u32,
    original_hidden: bool,
}

impl ShellTakeoverMarker {
    fn encode(self) -> String {
        format!(
            "version=1\npid={}\noriginal_hidden={}\n",
            self.process_id,
            u8::from(self.original_hidden)
        )
    }

    fn decode(value: &str) -> Result<Self, String> {
        let mut version = None;
        let mut process_id = None;
        let mut original_hidden = None;
        for line in value.lines() {
            let Some((key, value)) = line.split_once('=') else {
                return Err("invalid desktop takeover marker line".to_string());
            };
            match key {
                "version" => version = Some(value),
                "pid" => {
                    process_id = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| "invalid desktop takeover process id".to_string())?,
                    );
                }
                "original_hidden" => {
                    original_hidden = Some(match value {
                        "0" => false,
                        "1" => true,
                        _ => return Err("invalid desktop takeover visibility state".to_string()),
                    });
                }
                _ => return Err(format!("unknown desktop takeover marker key: {key}")),
            }
        }
        if version != Some("1") {
            return Err("unsupported desktop takeover marker version".to_string());
        }
        Ok(Self {
            process_id: process_id
                .ok_or_else(|| "desktop takeover marker is missing its process id".to_string())?,
            original_hidden: original_hidden.ok_or_else(|| {
                "desktop takeover marker is missing its visibility state".to_string()
            })?,
        })
    }
}

fn shell_takeover_marker_path(database_path: &Path) -> PathBuf {
    database_path.with_file_name(TAKEOVER_MARKER_FILE)
}

fn write_shell_takeover_marker(
    marker_path: &Path,
    marker: ShellTakeoverMarker,
) -> Result<(), String> {
    let temporary_path = marker_path.with_extension("state.tmp");
    if temporary_path.exists() {
        fs::remove_file(&temporary_path).map_err(|error| {
            format!(
                "failed to replace stale desktop takeover marker {}: {error}",
                temporary_path.display()
            )
        })?;
    }
    let mut file = fs::File::create(&temporary_path).map_err(|error| {
        format!(
            "failed to create desktop takeover marker {}: {error}",
            temporary_path.display()
        )
    })?;
    file.write_all(marker.encode().as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("failed to persist desktop takeover marker: {error}"))?;
    fs::rename(&temporary_path, marker_path).map_err(|error| {
        format!(
            "failed to publish desktop takeover marker {}: {error}",
            marker_path.display()
        )
    })
}

fn read_shell_takeover_marker(marker_path: &Path) -> Result<ShellTakeoverMarker, String> {
    let value = fs::read_to_string(marker_path).map_err(|error| {
        format!(
            "failed to read desktop takeover marker {}: {error}",
            marker_path.display()
        )
    })?;
    ShellTakeoverMarker::decode(&value)
}

fn remove_shell_takeover_marker(marker_path: &Path) -> Result<(), String> {
    match fs::remove_file(marker_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to remove desktop takeover marker {}: {error}",
            marker_path.display()
        )),
    }
}

fn restore_shell_takeover(marker_path: &Path) -> Result<bool, String> {
    if !marker_path.exists() {
        return Ok(false);
    }
    let marker = read_shell_takeover_marker(marker_path)?;
    set_desktop_icons_hidden(marker.original_hidden)
        .map_err(|error| format!("failed to restore Explorer desktop icons: {error}"))?;
    remove_shell_takeover_marker(marker_path)?;
    Ok(true)
}

fn recover_stale_shell_takeover(marker_path: &Path) -> Result<(), String> {
    if !marker_path.exists() {
        return Ok(());
    }
    let marker = read_shell_takeover_marker(marker_path).map_err(|error| {
        format!("{error}; run LucidPane with {RESTORE_SHELL_OPTION} to force recovery")
    })?;
    if let Ok(waiter) = ProcessExitWaiter::open(marker.process_id)
        && !waiter
            .has_exited()
            .map_err(|error| format!("failed to inspect the active LucidPane process: {error}"))?
    {
        return Err(format!(
            "Managed Desktop Mode is already active in process {}; close it before starting another instance",
            marker.process_id
        ));
    }
    restore_shell_takeover(marker_path)?;
    Ok(())
}

fn run_restore_shell_if_requested(arguments: &[OsString]) -> Result<bool, String> {
    if arguments.first().and_then(|argument| argument.to_str()) != Some(RESTORE_SHELL_OPTION) {
        return Ok(false);
    }
    if arguments.len() != 1 {
        return Err(format!("{RESTORE_SHELL_OPTION} does not accept arguments"));
    }
    let database_path = database_path()?;
    let marker_path = shell_takeover_marker_path(&database_path);
    if marker_path.exists() {
        match restore_shell_takeover(&marker_path) {
            Ok(true) => {}
            Ok(false) => unreachable!("the takeover marker was checked above"),
            Err(error) => {
                set_desktop_icons_hidden(false).map_err(|restore_error| {
                    format!("{error}; forced Explorer recovery also failed: {restore_error}")
                })?;
                remove_shell_takeover_marker(&marker_path)?;
            }
        }
    } else {
        set_desktop_icons_hidden(false)
            .map_err(|error| format!("failed to show Explorer desktop icons: {error}"))?;
    }
    Ok(true)
}

fn spawn_restore_guard(original_hidden: bool, marker_path: &Path) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to locate the desktop restore helper: {error}"))?;
    let mut child = Command::new(executable)
        .arg(RESTORE_GUARD_OPTION)
        .arg(std::process::id().to_string())
        .arg(if original_hidden { "hidden" } else { "visible" })
        .arg(marker_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("failed to start the desktop restore helper: {error}"))?;
    let mut ready = [0_u8; 6];
    child
        .stdout
        .take()
        .ok_or_else(|| "desktop restore helper did not expose its handshake".to_string())?
        .read_exact(&mut ready)
        .map_err(|error| format!("desktop restore helper did not become ready: {error}"))?;
    if ready != *b"ready\n" {
        return Err("desktop restore helper returned an invalid handshake".to_string());
    }
    Ok(())
}

fn run_restore_guard_if_requested(arguments: &[OsString]) -> Result<bool, String> {
    if arguments.first().and_then(|argument| argument.to_str()) != Some(RESTORE_GUARD_OPTION) {
        return Ok(false);
    }
    if arguments.len() != 4 {
        return Err("invalid desktop restore helper arguments".to_string());
    }
    let process_id = arguments[1]
        .to_string_lossy()
        .parse::<u32>()
        .map_err(|_| "invalid parent process id for desktop restore helper".to_string())?;
    let original_hidden = match arguments[2].to_str() {
        Some("hidden") => true,
        Some("visible") => false,
        _ => return Err("invalid restore state for desktop restore helper".to_string()),
    };
    let marker_path = PathBuf::from(&arguments[3]);
    let waiter = ProcessExitWaiter::open(process_id)
        .map_err(|error| format!("failed to watch the LucidPane process: {error}"))?;
    std::io::stdout()
        .write_all(b"ready\n")
        .and_then(|()| std::io::stdout().flush())
        .map_err(|error| format!("failed to signal desktop restore readiness: {error}"))?;
    waiter
        .wait()
        .map_err(|error| format!("failed while waiting to restore the desktop: {error}"))?;
    set_desktop_icons_hidden(original_hidden)
        .map_err(|error| format!("failed to restore Explorer desktop icons: {error}"))?;
    remove_shell_takeover_marker(&marker_path)?;
    Ok(true)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LaunchMode {
    #[default]
    RedrawnDesktop,
    Preview,
    NativeDesktop,
    HookDesktop,
    HybridDesktop,
    ManagedDesktop,
    Manual,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AppOptions {
    folder: Option<PathBuf>,
    title: Option<String>,
    icon: Option<PathBuf>,
    mode: LaunchMode,
}

fn parse_options(arguments: impl IntoIterator<Item = OsString>) -> Result<AppOptions, String> {
    let mut options = AppOptions::default();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--title") => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--title requires a value".to_string())?;
                options.title = Some(value.to_string_lossy().into_owned());
            }
            Some("--icon") => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--icon requires a file path".to_string())?;
                let path = PathBuf::from(value);
                if !path.is_file() {
                    return Err(format!("custom icon does not exist: {}", path.display()));
                }
                options.icon = Some(path);
            }
            Some("--managed-desktop") => options.mode = LaunchMode::ManagedDesktop,
            Some("--desktop") => options.mode = LaunchMode::RedrawnDesktop,
            Some("--native-desktop") => options.mode = LaunchMode::NativeDesktop,
            Some("--hook-desktop") => options.mode = LaunchMode::HookDesktop,
            Some("--hybrid-desktop") => options.mode = LaunchMode::HybridDesktop,
            Some("--preview") => options.mode = LaunchMode::Preview,
            Some("--manual") => options.mode = LaunchMode::Manual,
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown option: {value}"));
            }
            _ if options.folder.is_none() => {
                options.folder = Some(PathBuf::from(argument));
                options.mode = LaunchMode::Preview;
            }
            _ => return Err("only one portal folder can be supplied".to_string()),
        }
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::{
        AppOptions, LaunchMode, ShellTakeoverMarker, add_dropped_items,
        assign_desktop_item_to_pane, free_placement_from_screen, normalize_desktop_placements,
        parse_options,
    };
    use desktop_core::{
        DesktopItem, DesktopPlacement, GridPosition, MonitorId, Panel, PanelId, PanelSource,
        RectDip, ShellIdentity, Workspace,
    };
    use desktop_window::{MonitorDescriptor, PixelRect};
    use std::collections::HashSet;
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn command_line_accepts_custom_title_and_folder() {
        let options = parse_options([
            OsString::from("--title"),
            OsString::from("Work"),
            OsString::from(r"D:\Projects"),
        ])
        .unwrap();
        assert_eq!(
            options,
            AppOptions {
                folder: Some(PathBuf::from(r"D:\Projects")),
                title: Some("Work".into()),
                icon: None,
                mode: LaunchMode::Preview,
            }
        );
    }

    #[test]
    fn command_line_defaults_to_redrawn_desktop_with_explicit_preview_fallback() {
        assert_eq!(parse_options([]).unwrap().mode, LaunchMode::RedrawnDesktop);
        assert_eq!(
            parse_options([OsString::from("--preview")]).unwrap().mode,
            LaunchMode::Preview
        );
        assert_eq!(
            parse_options([OsString::from("--managed-desktop")])
                .unwrap()
                .mode,
            LaunchMode::ManagedDesktop
        );
        assert_eq!(
            parse_options([OsString::from("--manual")]).unwrap().mode,
            LaunchMode::Manual
        );
    }

    #[test]
    fn command_line_rejects_unknown_options() {
        let error = parse_options([OsString::from("--unknown")]).unwrap_err();
        assert!(error.contains("unknown option"));
    }

    #[test]
    fn shell_takeover_marker_roundtrips_and_rejects_unknown_versions() {
        let marker = ShellTakeoverMarker {
            process_id: 42,
            original_hidden: false,
        };
        assert_eq!(
            ShellTakeoverMarker::decode(&marker.encode()).unwrap(),
            marker
        );
        assert!(
            ShellTakeoverMarker::decode("version=2\npid=42\noriginal_hidden=0\n")
                .unwrap_err()
                .contains("version")
        );
    }

    #[test]
    fn dropped_desktop_items_are_added_only_to_manual_panes() {
        let mut manual = Panel::new(
            PanelId::new(1),
            "New Pane",
            PanelSource::ManualCollection { collection_id: 1 },
            RectDip::default(),
        );
        let paths = [
            PathBuf::from(r"C:\Users\Test\Desktop\Editor.lnk"),
            PathBuf::from(r"C:\Users\Test\Desktop\Notes.txt"),
        ];
        assert_eq!(add_dropped_items(&mut manual, paths.clone()), 2);
        assert_eq!(add_dropped_items(&mut manual, paths), 0);

        let mut portal = Panel::new(
            PanelId::new(2),
            "Folder",
            PanelSource::Folder {
                path: PathBuf::from(r"C:\Users\Test\Desktop"),
            },
            RectDip::default(),
        );
        assert_eq!(
            add_dropped_items(&mut portal, [PathBuf::from("ignored.lnk")]),
            0
        );
        assert!(portal.item_paths().is_empty());

        let mut desktop = Panel::new(
            PanelId::new(3),
            "Desktop",
            PanelSource::DesktopCollection,
            RectDip::default(),
        );
        assert_eq!(
            add_dropped_items(&mut desktop, [PathBuf::from("ignored.lnk")]),
            0
        );
        assert!(desktop.item_paths().is_empty());
    }

    #[test]
    fn managed_layout_places_new_items_on_free_desktop_and_preserves_membership() {
        let pane_id = PanelId::new(1);
        let panel = Panel::new(
            pane_id,
            "New Pane",
            PanelSource::DesktopCollection,
            RectDip::default(),
        );
        let mut workspace = Workspace::from_panels(vec![panel]).unwrap();
        let first = ShellIdentity::FileSystem {
            path: PathBuf::from(r"C:\Users\Test\Desktop\One.lnk"),
            volume_id: None,
            file_id: None,
        };
        let second = ShellIdentity::Namespace {
            parsing_name: "::{645FF040-5081-101B-9F08-00AA002F954E}".into(),
        };
        workspace.reconcile_desktop_items([
            DesktopItem::new(first.clone(), "One"),
            DesktopItem::new(second.clone(), "Recycle Bin"),
        ]);
        let monitor = MonitorDescriptor {
            id: MonitorId::new("DISPLAY1"),
            bounds: PixelRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            work_area: PixelRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            dpi: 96,
            primary: true,
        };
        normalize_desktop_placements(
            &mut workspace,
            std::slice::from_ref(&monitor),
            &HashSet::new(),
        );
        assert_ne!(
            workspace.desktop_items()[0].placement(),
            workspace.desktop_items()[1].placement()
        );

        workspace
            .desktop_item_mut(&first)
            .unwrap()
            .set_placement(DesktopPlacement::Pane {
                pane_id,
                position: GridPosition::new(0, 0),
            });
        let previous = [first.persistent_key(), second.persistent_key()]
            .into_iter()
            .collect();
        normalize_desktop_placements(&mut workspace, &[monitor], &previous);
        assert!(matches!(
            workspace.desktop_item(&first).unwrap().placement(),
            DesktopPlacement::Pane { pane_id: id, .. } if *id == pane_id
        ));
    }

    #[test]
    fn disappearing_monitor_moves_free_items_to_the_primary_monitor() {
        let identity = ShellIdentity::Namespace {
            parsing_name: "shell:test-item".into(),
        };
        let mut item = DesktopItem::new(identity.clone(), "Test");
        item.set_placement(DesktopPlacement::FreeDesktop {
            monitor: MonitorId::new("REMOVED"),
            position: desktop_core::PointDip::new(900.0, 900.0),
        });
        let mut workspace = Workspace::new();
        workspace.reconcile_desktop_items([item]);
        let previous = [identity.persistent_key()].into_iter().collect();
        let primary = MonitorDescriptor {
            id: MonitorId::new("PRIMARY"),
            bounds: PixelRect {
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
            },
            work_area: PixelRect {
                x: 0,
                y: 0,
                width: 1280,
                height: 680,
            },
            dpi: 96,
            primary: true,
        };

        normalize_desktop_placements(&mut workspace, &[primary], &previous);

        assert!(matches!(
            workspace.desktop_item(&identity).unwrap().placement(),
            DesktopPlacement::FreeDesktop { monitor, .. } if monitor.as_str() == "PRIMARY"
        ));
    }

    #[test]
    fn dropping_a_desktop_item_into_a_pane_only_changes_membership() {
        let pane_id = PanelId::new(1);
        let panel = Panel::new(
            pane_id,
            "New Pane",
            PanelSource::DesktopCollection,
            RectDip::default(),
        );
        let identity = ShellIdentity::FileSystem {
            path: PathBuf::from(r"C:\Users\Test\Desktop\Editor.lnk"),
            volume_id: Some(7),
            file_id: Some(9),
        };
        let mut workspace = Workspace::from_panels(vec![panel]).unwrap();
        workspace.reconcile_desktop_items([DesktopItem::new(identity.clone(), "Editor")]);

        assert!(assign_desktop_item_to_pane(
            &mut workspace,
            &identity,
            pane_id
        ));
        let item = workspace.desktop_item(&identity).unwrap();
        assert_eq!(item.identity(), &identity);
        assert!(matches!(
            item.placement(),
            DesktopPlacement::Pane { pane_id: id, .. } if *id == pane_id
        ));
    }

    #[test]
    fn dropping_outside_a_pane_maps_screen_pixels_to_monitor_local_dips() {
        let monitor = MonitorDescriptor {
            id: MonitorId::new("DISPLAY2"),
            bounds: PixelRect {
                x: -2560,
                y: 0,
                width: 2560,
                height: 1440,
            },
            work_area: PixelRect {
                x: -2560,
                y: 0,
                width: 2560,
                height: 1400,
            },
            dpi: 144,
            primary: false,
        };
        let (monitor_id, position) = free_placement_from_screen(&[monitor], -2410, 150).unwrap();
        assert_eq!(monitor_id.as_str(), "DISPLAY2");
        assert!((position.x - 52.0).abs() < f32::EPSILON);
        assert!((position.y - 56.0).abs() < f32::EPSILON);
    }
}
