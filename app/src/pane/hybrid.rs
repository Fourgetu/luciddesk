//! Pane UI with Explorer-owned desktop membership, drawing, layout and input.
mod audit;
mod audit_schedule;
mod icon_changes;
mod icons;
mod image_retention;
mod inventory;
mod rename_transaction;
use super::assets::RECYCLE_BIN_PARSING_NAME;
use super::search::{everything_settings, hotkey as search_hotkey};
use super::*;
use audit::DesktopAudit;
use desktop_hook::{filter::FilterSession, notifications::{DESKTOP_INPUT_MESSAGE, DESKTOP_EXIT_MESSAGE}};
pub(super) use icons::refresh_icons;
use icons::{queue_pane_icons, retain_pane_images};
use inventory::Inventory;
pub(super) use rename_transaction::commit as rename_item;
use std::{
    cell::Cell,
    time::{Duration, Instant},
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub(super) struct Session {
    wake: wake::Wake,
    // Drop the Hook before its owner HWND and before the pane windows.
    hook: Rc<FilterSession>,
    _icon_subscription: desktop_shell::DesktopChangeSubscription,
    _recycle_subscription: desktop_shell::DesktopChangeSubscription,
    _controller: windows_window::Window,
    icons_dirty: Rc<RefCell<icon_changes::Pending>>,
    icon_due: Option<Instant>,
    icon_reload: Option<icons::RefreshJob>,
    icon_refresh_failures: u32,
    view: isize,
    snapshot: Inventory,
    last_reconcile: Instant,
    audit: DesktopAudit,
    last_tick: Instant,
    tick_deferred: bool,
    last_icon_scan: Instant,
    published: RefCell<Vec<String>>,
    sender: mpsc::Sender<Loaded>,
    requested: std::collections::HashSet<String>,
    icon_failures: HashMap<String, (u32, Instant)>,
    image_retention: image_retention::ImageRetention,
    initial_batches: usize,
    menu_active: Cell<bool>,
    last_pane_input: Cell<Option<u32>>,
    pending_desktop_input: Rc<Cell<Option<u32>>>,
    dirty: Rc<Cell<bool>>,
    retry_after: Option<Instant>,
    last_failure: Option<String>,
    pending_workspace_save: bool,
    diagnostic_path: std::path::PathBuf,
}

struct OleApartment;
pub(super) fn is_alive(session: &Session) -> bool {
    session.hook.is_alive()
}
impl Drop for OleApartment {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::System::Ole::OleUninitialize();
        }
    }
}

pub fn run(path: &Path, title: Option<String>) -> Result<(), String> {
    unsafe {
        windows::Win32::System::Ole::OleInitialize(None).map_err(|e| e.to_string())?;
    }
    let _ole = OleApartment;
    let first_run = !path.exists();
    let mut store = WorkspaceStore::open(path).map_err(|e| e.to_string())?;
    super::peek::load(&store)?;
    fonts::load(&store)?;
    header_divider::load(&store)?;
    search_hotkey::load(&store)?;
    everything_settings::load(&store)?;
    let mut workspace = store.load_workspace().map_err(|e| e.to_string())?;
    // Migrate older workspaces that allowed multiple search panes.
    let duplicates: Vec<_> = workspace
        .panels()
        .iter()
        .filter(|p| p.is_search())
        .skip(1)
        .map(Panel::id)
        .collect();
    if !duplicates.is_empty() {
        for id in duplicates {
            remove_panel(&mut workspace, id);
        }
        store
            .save_workspace(&workspace)
            .map_err(|e| e.to_string())?;
    }
    let monitors = desktop_window::enumerate_monitors();
    if workspace.panels().is_empty() && first_run {
        let mut pane = Panel::new(
            PanelId::new(1),
            title.clone().unwrap_or_else(|| "新建分组".into()),
            display_layout::first_pane(&monitors, workspace.pane_options().grid_scale),
        );
        pane.set_backdrop(desktop_core::Backdrop::Acrylic);
        workspace.add_panel(pane).map_err(|e| e.to_string())?;
    } else if let Some(title) = title.filter(|_| !workspace.panels().is_empty()) {
        let id = workspace.panels()[0].id();
        workspace.panel_mut(id).unwrap().set_title(title);
    }
    let (_, receiver) = mpsc::channel();
    let state = Rc::new(RefCell::new(PaneApp {
        wake: Default::default(),
        folders: HashMap::new(),
        tab_models: HashMap::new(),
        settings: None,
        session: None,
        drops: Vec::new(),
        runtime: Some(runtime::State::new(path.to_path_buf())),
        workspace,
        store,
        views: Vec::new(),
        images: HashMap::new(),
        receiver,
    }));
    display_layout::initialize(
        &mut state.borrow_mut(),
        monitors,
    )?;
    {
        let mut s = state.borrow_mut();
        let workspace = s.workspace.clone();
        s.store
            .save_workspace(&workspace)
            .map_err(|e| e.to_string())?;
    }
    runtime::reconnect(&state);
    let search_enabled = everything_settings::enabled(&state.borrow().store)?;
    let ids: Vec<_> = state
        .borrow()
        .workspace
        .panels()
        .iter()
        .filter(|p| {
            (!p.is_search() || search_enabled)
                && (tabs::independent(&state.borrow().workspace, p.id()) || state.borrow().session.is_some())
        })
        .map(Panel::id)
        .collect();
    for id in ids {
        create_view(&state, id)?;
    }
    let tray_state = Rc::downgrade(&state);
    let appearance_state = Rc::downgrade(&state);
    let tray = crate::tray::Tray::new(
        move || {
            appearance_state
                .upgrade()
                .and_then(|state| {
                    let s = state.borrow();
                    s.workspace.appearance().or_else(|| {
                        s.workspace
                            .panels()
                            .first()
                            .map(|p| (p.theme(), p.backdrop()))
                    })
                })
                .unwrap_or((
                    desktop_core::PanelTheme::System,
                    desktop_core::Backdrop::Mica,
                ))
        },
        move |action| {
            let Some(state) = tray_state.upgrade() else {
                return;
            };
            match action {
                crate::tray::Action::NewFolder => {
                    if let Err(error) = handle(&state, PanelId::new(0), Event::NewFolder) {
                        window::error(&error);
                    }
                }
                crate::tray::Action::Settings => {
                    if let Err(error) = handle(&state, PanelId::new(0), Event::Settings) {
                        window::error(&error);
                    }
                }
                crate::tray::Action::Exit => windows_window::quit(),
                crate::tray::Action::Show => {
                    let windows: Vec<_> = state
                        .borrow()
                        .views
                        .iter()
                        .map(|v| v.window.hwnd())
                        .collect();
                    for &hwnd in &windows {
                        unsafe {
                            ShowWindow(hwnd.cast(), SW_SHOWNOACTIVATE);
                        }
                    }
                    if let Some(&hwnd) = windows.first() {
                        unsafe {
                            SetForegroundWindow(hwnd.cast());
                        }
                    }
                }
                crate::tray::Action::New => {
                    if let Err(error) = handle(&state, PanelId::new(0), Event::New) {
                        window::error(&error);
                    }
                }
            }
        },
    )?;
    display_layout::record(&mut state.borrow_mut())?;
    let supervisor = runtime::supervisor(&state)?;
    if state.borrow().session.is_none() {
        settings::show(&state, PanelId::new(0))?;
    }
    windows_window::run();
    drop(supervisor);
    drop(tray);
    state.borrow_mut().session.take();
    Ok(())
}

pub(super) fn connect(state: &Rc<RefCell<PaneApp>>, path: &Path) -> Result<(), String> {
    if desktop_hook::conflicting_desktop_extension() {
        return Err("请先退出其他桌面整理软件".into());
    }
    if desktop_shell::desktop_icons_hidden() {
        return Err("请先退出旧的全桌面接管版本，恢复桌面图标显示".into());
    }
    let view = desktop_hook::desktop_view()?;
    let dirty = Rc::new(Cell::new(false));
    let notify = Rc::clone(&dirty);
    let icons_dirty = Rc::new(RefCell::new(icon_changes::Pending::default()));
    let icon_notify = Rc::clone(&icons_dirty);
    let input_receiver = Rc::downgrade(state);
    let pending_desktop_input = Rc::new(Cell::new(None));
    let pending_input = Rc::clone(&pending_desktop_input);
    let work_ready = state.borrow().wake.clone();
    let controller = windows_window::Window::new("LucidPane Hybrid Controller")
        .size(1, 1)
        .style(WS_POPUP)
        .ex_style(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)
        .on_message(move |_, message, wparam, lparam| {
            if unsafe { crate::window_visibility::defer_show(message, lparam, false) } {
                return Some(0);
            }
            if message == WM_DESTROY {
                // Releasing a failed/stale connection must not quit independent panes.
                Some(0)
            } else if message == DESKTOP_EXIT_MESSAGE {
                work_ready.notify();
                Some(0)
            } else if message == ICON_CHANGE_MESSAGE {
                notify.set(true);
                work_ready.notify();
                if std::env::var_os("LUCIDPANE_ICON_TRACE").is_some() {
                    eprintln!("icon-notify event={:x}", lparam);
                }
                icon_notify
                    .borrow_mut()
                    .add(unsafe { icon_changes::capture(wparam, lparam as u32) });
                Some(0)
            } else if message == RECYCLE_CHANGE_MESSAGE {
                work_ready.notify();
                if std::env::var_os("LUCIDPANE_ICON_TRACE").is_some() {
                    eprintln!("recycle-notify event={:x}", lparam);
                }
                icon_notify
                    .borrow_mut()
                    .add([icon_changes::Change::Name(RECYCLE_BIN_PARSING_NAME.into())]);
                Some(0)
            } else if message == DESKTOP_INPUT_MESSAGE {
                work_ready.notify();
                if wparam as isize == view {
                    pending_input.set(Some(lparam as u32));
                    if let Some(state) = input_receiver.upgrade() {
                        if let Ok(s) = state.try_borrow() {
                            clear_pane_selection_on_desktop_input(&s);
                        }
                    }
                }
                Some(0)
            } else {
                None
            }
        })
        .create()
        .map_err(|e| e.to_string())?;
    // Subscribe before reading so changes during initialization are not lost.
    let icon_subscription = desktop_shell::DesktopChangeSubscription::register(
        controller.hwnd() as isize,
        ICON_CHANGE_MESSAGE,
    )
    .map_err(|error| error.to_string())?;
    let recycle_subscription = desktop_shell::DesktopChangeSubscription::register_recycle_bin(
        controller.hwnd() as isize,
        RECYCLE_CHANGE_MESSAGE,
    )
    .map_err(|error| error.to_string())?;
    // Warm and validate the Shell view from the client before injecting work
    // into Explorer's STA. During startup, the HWND can exist before ShellWindows
    // is ready; asking Explorer to initialize it from its own callback can stall.
    // Reuse this snapshot below instead of performing a second enumeration.
    let snapshot = inventory::capture(&managed_identities(&state.borrow()))?;
    if desktop_hook::desktop_view()? != view {
        return Err("读取清单期间桌面视图已替换，将重新定位".into());
    }
    let hook = FilterSession::connect(
        view,
        controller.hwnd() as isize,
        &crate::hook_runtime::runtime_dll(path)?,
    )?;
    let (sender, receiver) = mpsc::channel();
    let session = Session {
        wake: state.borrow().wake.clone(),
        hook: Rc::new(hook),
        _icon_subscription: icon_subscription,
        _recycle_subscription: recycle_subscription,
        _controller: controller,
        icons_dirty,
        icon_due: None,
        icon_reload: None,
        icon_refresh_failures: 0,
        view,
        snapshot,
        last_reconcile: Instant::now(),
        audit: DesktopAudit::start(state.borrow().wake.clone())?,
        last_tick: Instant::now() - Duration::from_millis(20),
        tick_deferred: false,
        last_icon_scan: Instant::now() - Duration::from_secs(1),
        published: RefCell::new(Vec::new()),
        sender,
        requested: Default::default(),
        icon_failures: HashMap::new(),
        image_retention: Default::default(),
        initial_batches: 0,
        menu_active: Cell::new(false),
        last_pane_input: Cell::new(None),
        pending_desktop_input,
        dirty,
        retry_after: None,
        last_failure: None,
        pending_workspace_save: false,
        diagnostic_path: path.with_extension("log"),
    };
    let mut s = state.borrow_mut();
    s.receiver = receiver;
    s.session = Some(session);
    // A fresh hook has no desired membership. Cached pane images may produce
    // no loading-completion event, so publish now rather than waiting for one.
    if let Err(error) = reconcile_inventory(&mut s).and_then(|()| sync(&mut s)) {
        s.session.take();
        return Err(error);
    }
    Ok(())
}

pub(super) fn register_drop(state: &Rc<RefCell<PaneApp>>, id: PanelId) -> Result<(), String> {
    if state
        .borrow()
        .workspace
        .panel(id)
        .is_some_and(Panel::is_search)
    {
        return Ok(());
    }
    let hwnd = state
        .borrow()
        .views
        .iter()
        .find(|v| v.id == id)
        .unwrap()
        .window
        .hwnd();
    let weak = Rc::downgrade(state);
    let registration = super::drag_drop::target::Registration::new(
        windows::Win32::Foundation::HWND(hwnd.cast()),
        if state
            .borrow()
            .workspace
            .panel(id)
            .is_some_and(|p| p.folder().is_some())
        {
            windows::Win32::System::Ole::DROPEFFECT_COPY
        } else {
            windows::Win32::System::Ole::DROPEFFECT_LINK
        },
        move |identities, commit| {
            let Some(state) = weak.upgrade() else {
                return false;
            };
            let Ok(s) = state.try_borrow_mut() else {
                return false;
            };
            if s.views
                .iter()
                .find(|v| v.id == id)
                .is_none_or(|v| v.model.borrow().collapsed)
            {
                return false;
            }
            if let Some(path) = s.folders.get(&id).map(|source| source.path.clone()) {
                if !folder::accepts_copy(identities, &path) {
                    return false;
                }
                if !commit {
                    return true;
                }
                let items = identities.to_vec();
                drop(s);
                return window::post_action(hwnd.cast(), move || {
                    if let Err(error) = desktop_shell::copy_to_folder(
                        windows::Win32::Foundation::HWND(hwnd.cast()),
                        &items,
                        &path,
                    ) {
                        window::error(&error.to_string());
                    }
                });
            }
            let keys: Option<Vec<_>> = identities
                .iter()
                .map(|identity| {
                    s.workspace
                        .desktop_items()
                        .iter()
                        .find(|i| i.identity().equivalent_to(identity))
                        .map(|i| i.identity().clone())
                })
                .collect();
            let Some(keys) = keys else {
                return false;
            };
            if !commit {
                return true;
            }
            // Explorer is the OLE caller. Apply membership only after returning
            // from Drop, so its desktop STA can process our asynchronous request.
            drop(s);
            window::post_action(hwnd.cast(), move || {
                let mut s = state.borrow_mut();
                if s.workspace.panel(id).is_none()
                    || keys
                        .iter()
                        .any(|identity| s.workspace.desktop_item(identity).is_none())
                {
                    return;
                }
                let old = s.workspace.clone();
                normalize_pane_orders(&mut s);
                let mut at = items_for(&s, id).len();
                for identity in keys {
                    s.workspace
                        .desktop_item_mut(&identity)
                        .unwrap()
                        .set_placement(DesktopPlacement::Pane {
                            pane_id: id,
                            position: GridPosition::new(at as u32, 0),
                        });
                    at += 1;
                }
                if let Err(error) = save(&mut s) {
                    s.workspace = old;
                    let _ = sync(&mut s);
                    eprintln!("Desktop collection rejected: {error}");
                }
                refresh_views(&mut s);
            })
        },
    )
    .map_err(|e| e.to_string())?;
    state.borrow_mut().drops.push(registration);
    Ok(())
}

pub(super) fn unregister_drop(state: &mut PaneApp, hwnd: windows_sys::Win32::Foundation::HWND) {
    state
        .drops
        .retain(|registration| registration.window().0 != hwnd);
}

fn managed_identities(s: &PaneApp) -> Vec<ShellIdentity> {
    s.workspace
        .desktop_items()
        .iter()
        .filter(|item| matches!(item.placement(), DesktopPlacement::Pane { .. }))
        .map(|item| item.identity().clone())
        .collect()
}
// Apply the captured inventory without another synchronous Shell enumeration.
fn reconcile_inventory(s: &mut PaneApp) -> Result<(), String> {
    s.session
        .as_mut()
        .unwrap()
        .image_retention
        .invalidate(&mut s.images);
    normalize_pane_orders(s);
    let snapshot = &s.session.as_ref().unwrap().snapshot;
    s.workspace.reconcile_desktop_items(
        snapshot
            .items
            .iter()
            .map(|item| DesktopItem::new(item.identity.clone(), item.display_name.clone())),
    );
    let valid: Vec<_> = s.workspace.panels().iter().map(Panel::id).collect();
    for item in s.workspace.desktop_items_mut() {
        if matches!(item.placement(), DesktopPlacement::Pane { pane_id, .. } if !valid.contains(pane_id))
        {
            item.set_placement(DesktopPlacement::default());
        }
    }
    let live: std::collections::HashSet<_> = snapshot
        .items
        .iter()
        .map(|item| item.identity.persistent_key())
        .collect();
    s.images.retain(|key, _| live.contains(key));
    let h = s.session.as_mut().unwrap();
    h.requested.retain(|key| live.contains(key));
    h.icon_failures.retain(|key, _| live.contains(key));
    h.last_reconcile = Instant::now();
    queue_pane_icons(s, true);
    s.session.as_mut().unwrap().pending_workspace_save = true;
    persist_workspace(s)?;
    refresh_views(s);
    Ok(())
}

fn persist_pending(pending: &mut bool, save: impl FnOnce() -> Result<(), String>) -> Result<bool, String> {
    if !*pending { return Ok(false); }
    save()?;
    *pending = false;
    Ok(true)
}

fn persist_workspace(s: &mut PaneApp) -> Result<bool, String> {
    persist_pending(&mut s.session.as_mut().unwrap().pending_workspace_save, || {
        s.store.save_workspace(&s.workspace).map_err(|error| error.to_string())
    })
}
/// Hide a managed item only after its Pane image is available. Both normal
/// synchronization and committed renames must publish the same complete set.
fn hidden_names(s: &PaneApp) -> Vec<String> {
    let mut names: Vec<_> = s
        .workspace
        .desktop_items()
        .iter()
        .filter(|item| {
            matches!(item.placement(), DesktopPlacement::Pane { .. })
                && s.images.contains_key(&item.identity().persistent_key())
        })
        .map(|item| {
            item.identity()
                .activation_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

pub(super) fn sync(s: &mut PaneApp) -> Result<(), String> {
    let Some(h) = s.session.as_ref() else {
        return Ok(());
    };
    if h.menu_active.get() {
        return Ok(());
    }
    if !h.hook.is_alive() {
        return Err("Explorer 视图过滤连接已断开".into());
    }
    let h = s.session.as_mut().unwrap();
    if h.dirty.get() || !h.icons_dirty.borrow().is_empty() {
        h.image_retention.invalidate(&mut s.images);
    }
    let live = retain_pane_images(
        &s.workspace,
        &mut s.images,
        &mut h.image_retention,
        Instant::now(),
    );
    let names = hidden_names(s);
    let h = s.session.as_mut().unwrap();
    if *h.published.borrow() != names {
        h.hook.set_hidden(&names)?;
        *h.published.borrow_mut() = names;
    }
    h.icon_failures.retain(|key, _| live.contains(key));
    queue_pane_icons(s, true);
    Ok(())
}

pub(super) fn clear_desktop_selection(s: &PaneApp) -> Result<(), String> {
    if let Some(h) = &s.session {
        h.last_pane_input
            .set(Some(unsafe { GetMessageTime() } as u32));
        if !h.menu_active.get() {
            h.hook.clear_selection()?;
        }
    }
    Ok(())
}

fn desktop_input_is_newer(input: u32, pane_input: Option<u32>) -> bool {
    // GetMessageTime wraps every 49.7 days. Equal ticks conservatively preserve
    // the pane's choice; compare event times, never foreground activation order.
    pane_input.is_none_or(|pane| input.wrapping_sub(pane) as i32 > 0)
}

fn clear_pane_selection_on_desktop_input(s: &PaneApp) {
    let Some(h) = &s.session else {
        return;
    };
    let Some(input) = h.pending_desktop_input.take() else {
        return;
    };
    // The first native press can arrive before Explorer activates its desktop.
    // Ordering protects a newer pane selection without discarding that press.
    if h.menu_active.get() || !desktop_input_is_newer(input, h.last_pane_input.get()) {
        return;
    }
    for view in &s.views {
        let Ok(mut model) = view.model.try_borrow_mut() else {
            // A nested paint/COM callback may still borrow the model. Keep the
            // event for the existing UI tick instead of silently losing it.
            h.pending_desktop_input.set(Some(input));
            continue;
        };
        let changed = model.selected.is_some() || !model.selection.is_empty() || model.focused;
        model.clear_selection();
        model.focused = false;
        if changed {
            unsafe {
                InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0);
            }
        }
    }
}

pub(super) fn pause_for_preview(s: &PaneApp, allow: bool) -> Result<(), String> {
    if let Some(h) = &s.session {
        if allow {
            h.last_pane_input
                .set(Some(unsafe { GetMessageTime() } as u32));
        }
        h.hook.pause(allow)?;
        h.menu_active.set(allow);
    }
    Ok(())
}

pub(super) fn begin_item_menu(s: &PaneApp) -> Result<Rc<FilterSession>, String> {
    let h = s.session.as_ref().ok_or("桌面过滤连接尚未就绪")?;
    if h.menu_active.replace(true) {
        return Err("已有活动菜单或预览".into());
    }
    h.last_pane_input
        .set(Some(unsafe { GetMessageTime() } as u32));
    Ok(Rc::clone(&h.hook))
}
pub(super) fn end_item_menu(s: &PaneApp) {
    if let Some(h) = &s.session {
        h.menu_active.set(false);
    }
}

pub(super) fn tick(s: &mut PaneApp) -> Result<(), String> {
    clear_pane_selection_on_desktop_input(s);
    let h = s.session.as_ref().unwrap();
    if h.retry_after
        .is_some_and(|deadline| Instant::now() < deadline)
    {
        return Ok(());
    }
    match tick_once(s) {
        Ok(()) => {
            let h = s.session.as_mut().unwrap();
            h.retry_after = None;
            h.last_failure = None;
        }
        Err(error) => {
            let h = s.session.as_mut().unwrap();
            if h.last_failure.as_ref() != Some(&error) {
                use std::io::Write;
                eprintln!("Hybrid synchronization deferred: {error}");
                if let Ok(mut log) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&h.diagnostic_path)
                {
                    let _ = writeln!(
                        log,
                        "{:?} synchronization deferred: {error}",
                        std::time::SystemTime::now()
                    );
                }
                h.last_failure = Some(error);
            }
            // Sorting can invalidate the inventory while it is being read. Leave
            // the pane and its membership alive; re-read before the next publish.
            h.dirty.set(true);
            h.retry_after = Some(Instant::now() + Duration::from_millis(250));
        }
    }
    Ok(())
}

fn tick_once(s: &mut PaneApp) -> Result<(), String> {
    if s.session.as_ref().unwrap().menu_active.get() {
        return Ok(());
    }
    if persist_workspace(s)? {
        // The accepted inventory may already compare equal after a failed save.
        // Complete its presentation independently of the next audit result.
        refresh_views(s);
    }
    let h = s.session.as_mut().unwrap();
    if h.last_tick.elapsed() < Duration::from_millis(20) {
        h.tick_deferred = true;
        return Ok(());
    }
    h.tick_deferred = false;
    h.last_tick = Instant::now();
    let mut urgent = h.dirty.replace(false);
    if urgent {
        h.audit.invalidate();
    }
    urgent |= icons::tick(s, urgent);
    urgent |= poll_inventory(s, urgent)?;
    // The Hook repairs its retained desired set. Publish only when application
    // state changed, rather than duplicating its watchdog with periodic SETs.
    if urgent {
        sync(s)
    } else {
        Ok(())
    }
}

fn poll_inventory(s: &mut PaneApp, urgent: bool) -> Result<bool, String> {
    let h = s.session.as_ref().unwrap();
    if h.audit.due(h.last_reconcile.elapsed()) {
        let managed = managed_identities(s);
        s.session.as_mut().unwrap().audit.submit(managed, urgent)?;
    }
    let Some(result) = s.session.as_mut().unwrap().audit.poll()? else {
        return Ok(false);
    };
    let managed_keys = inventory::revision_keys(&managed_identities(s));
    let h = s.session.as_mut().unwrap();
    if result.member_keys != managed_keys {
        h.dirty.set(true);
        return Ok(false);
    }
    h.last_reconcile = Instant::now();
    let snapshot = result.inventory?;
    let changed = snapshot
        .as_ref()
        .is_some_and(|next| !inventory::same(&h.snapshot, next));
    h.audit.complete(changed);
    // One scheduler owns fallback audits and hook repair. Changed inventories
    // publish membership through sync(); unchanged ones still repair missed events.
    if !changed {
        h.hook.repair()?;
    }
    if let Some(snapshot) = snapshot.filter(|_| changed) {
        h.snapshot = snapshot;
        reconcile_inventory(s)?;
    }
    Ok(changed)
}

// Schedule the next input, retry, audit, cache expiry or repair deadline.
// Worker completions and Shell changes wake the supervisor independently.
pub(super) fn next_work(s: &PaneApp) -> Option<u32> {
    let h = s.session.as_ref()?;
    let now = Instant::now();
    if let Some(retry) = h.retry_after {
        return Some(
            retry
                .saturating_duration_since(now)
                .as_millis()
                .clamp(25, u32::MAX as u128) as u32,
        );
    }
    if h.menu_active.get() {
        return None;
    }
    if h.tick_deferred
        || h.dirty.get()
        || h.pending_desktop_input.get().is_some()
        || h.audit.due(Duration::ZERO)
    {
        return Some(25);
    }
    let icon_due = h
        .icon_due
        .filter(|_| h.initial_batches == 0 && h.icon_reload.is_none());
    icon_due
        .into_iter()
        .chain(h.audit.remaining(h.last_reconcile.elapsed()).map(|delay| now + delay))
        .chain(h.image_retention.deadline())
        .chain(
            h.icon_failures
                .iter()
                .filter(|(key, (attempts, _))| *attempts < 5 && !h.requested.contains(*key))
                .map(|(_, (_, due))| *due),
        )
        .min()
        .map(|due| {
            due.saturating_duration_since(now)
                .as_millis()
                .clamp(25, u32::MAX as u128) as u32
        })
}

pub(super) fn release(
    s: &mut PaneApp,
    id: PanelId,
    indices: &[usize],
    point: POINT,
) -> Result<bool, String> {
    let Some(h) = &s.session else {
        return Ok(false);
    };
    let surface = unsafe { WindowFromPoint(point) };
    if surface != h.view as _ && surface != unsafe { GetParent(h.view as _) } {
        return Ok(false);
    }
    let items = items_for(s, id);
    if indices.is_empty() || indices.iter().any(|index| *index >= items.len()) {
        return Ok(false);
    }
    let old = s.workspace.clone();
    for index in indices {
        let Some(item) = items.get(*index) else {
            return Ok(false);
        };
        if let Some(entry) = s.workspace.desktop_item_mut(&item.identity) {
            entry.set_placement(DesktopPlacement::default());
        }
    }
    if let Err(error) = save(s) {
        s.workspace = old;
        return Err(error);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_membership_uses_cached_images_and_waits_only_for_missing_ones() {
        let mut s = crate::pane::tests::test_state();
        s.images.clear();
        assert!(hidden_names(&s).is_empty());
        let identities: Vec<_> = s.workspace.desktop_items().iter()
            .filter(|item| matches!(item.placement(), DesktopPlacement::Pane { .. }))
            .map(|item| item.identity().clone()).collect();
        assert!(identities.len() > 1);
        let image = Arc::new(assets::Pixels { width: 1, height: 1, data: vec![0; 4] });
        s.images.insert(identities[0].persistent_key(), image.clone());
        assert_eq!(hidden_names(&s), vec![identities[0].activation_name().to_string_lossy().into_owned()]);
        for identity in &identities {
            s.images.insert(identity.persistent_key(), image.clone());
        }
        assert_eq!(hidden_names(&s).len(), identities.len());
        s.workspace.desktop_item_mut(&identities[0]).unwrap()
            .set_placement(DesktopPlacement::default());
        assert_eq!(hidden_names(&s).len(), identities.len() - 1,
            "cached pixels must not cause an item released to Explorer to be hidden");
    }

    #[test]
    fn failed_inventory_save_remains_pending_until_persisted_and_presented() {
        let mut pending = true;
        let mut presented = 0;
        for fails in [true, true, false, false] {
            let result = persist_pending(&mut pending, || {
                if fails { Err("injected database busy".into()) } else { Ok(()) }
            });
            if fails {
                assert!(result.is_err());
                assert!(pending);
            } else if result.unwrap() {
                presented += 1;
            }
        }
        assert_eq!(presented, 1);
        assert!(!pending);
        assert!(!persist_pending(&mut pending, || panic!("already saved")).unwrap());
    }
    #[test]
    fn desktop_input_order_preserves_newer_pane_selection() {
        assert!(desktop_input_is_newer(120, Some(100)));
        assert!(!desktop_input_is_newer(120, Some(140)));
        assert!(!desktop_input_is_newer(120, Some(120)));
        assert!(desktop_input_is_newer(10, Some(u32::MAX - 10)));
    }
}

const ICON_CHANGE_MESSAGE: u32 = WM_APP + 0x352;

const RECYCLE_CHANGE_MESSAGE: u32 = WM_APP + 0x353;
