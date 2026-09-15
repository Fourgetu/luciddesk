//! Pane UI with Explorer-owned desktop membership, drawing, layout and input.
mod icon_changes;
mod inventory;
mod rename_transaction;
use super::assets::RECYCLE_BIN_PARSING_NAME;
use super::search::{everything_settings, hotkey as search_hotkey};
use super::*;
use desktop_hook::{
    filter::FilterSession,
    notifications::{DESKTOP_INPUT_MESSAGE, SCENE_DIRTY_MESSAGE},
};
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
    icon_reload: Option<mpsc::Receiver<Vec<(String, assets::Pixels)>>>,
    view: isize,
    snapshot: Inventory,
    last_reconcile: Instant,
    audit: DesktopAudit,
    last_tick: Instant,
    tick_deferred: bool,
    last_sync: Instant,
    last_icon_scan: Instant,
    published: RefCell<Vec<String>>,
    sender: mpsc::Sender<Loaded>,
    requested: std::collections::HashSet<String>,
    icon_failures: HashMap<String, (u32, Instant)>,
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

struct DesktopAudit {
    requests: mpsc::Sender<(Vec<ShellIdentity>, bool)>,
    results: mpsc::Receiver<(Vec<String>, Result<Option<Inventory>, String>)>,
    pending: bool,
}
impl DesktopAudit {
    fn start(wake: wake::Wake) -> Result<Self, String> {
        let (requests, receiver) = mpsc::channel::<(Vec<ShellIdentity>, bool)>();
        let (sender, results) = mpsc::channel();
        std::thread::Builder::new()
            .name("desktop-audit".into())
            .spawn(move || {
                let apartment = ShellApartment::initialize_sta();
                let mut reader = desktop_shell::NativeDesktopReader::default();
                let mut previous = None;
                while let Ok((managed, force)) = receiver.recv() {
                    let keys = inventory::revision_keys(&managed);
                    let result = match &apartment {
                        Ok(_) => (|| -> Result<Option<Inventory>, String> {
                            let revision = (
                                reader.revision()?,
                                desktop_shell::desktop_source_revision()
                                    .map_err(|error| error.to_string())?,
                                keys.clone(),
                            );
                            if !force && previous.as_ref() == Some(&revision) {
                                return Ok(None);
                            }
                            let snapshot = inventory::capture(&managed)?;
                            previous = Some(revision);
                            Ok(Some(snapshot))
                        })(),
                        Err(error) => Err(error.to_string()),
                    };
                    if sender.send((keys, result)).is_err() {
                        break;
                    }
                    wake.notify();
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            requests,
            results,
            pending: false,
        })
    }
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
    if workspace.panels().is_empty() && first_run {
        let mut pane = Panel::new(
            PanelId::new(1),
            title.clone().unwrap_or_else(|| "新建分组".into()),
            RectDip::new(650.0, 100.0, 440.0, 380.0),
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
        desktop_window::enumerate_monitors(),
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
                && (p.folder().is_some() || p.is_search() || state.borrow().session.is_some())
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
            if message == WM_DESTROY {
                // Releasing a failed/stale connection must not quit independent panes.
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
            } else if message == SCENE_DIRTY_MESSAGE {
                notify.set(true);
                work_ready.notify();
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
    unsafe {
        ShowWindow(controller.hwnd().cast(), SW_HIDE);
    }
    let hook = FilterSession::connect(
        view,
        controller.hwnd() as isize,
        &crate::hook_runtime::runtime_dll(path)?,
    )?;
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
    let snapshot = inventory::capture(&managed_identities(&state.borrow()))?;
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
        view,
        snapshot,
        last_reconcile: Instant::now(),
        audit: DesktopAudit::start(state.borrow().wake.clone())?,
        last_tick: Instant::now() - Duration::from_millis(20),
        tick_deferred: false,
        last_sync: Instant::now(),
        last_icon_scan: Instant::now() - Duration::from_secs(1),
        published: RefCell::new(Vec::new()),
        sender,
        requested: Default::default(),
        icon_failures: HashMap::new(),
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
    if let Err(error) = reconcile_inventory(&mut s) {
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
    s.store
        .save_workspace(&s.workspace)
        .map_err(|error| error.to_string())?;
    refresh_views(s);
    Ok(())
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
    let names = hidden_names(s);
    let h = s.session.as_mut().unwrap();
    if *h.published.borrow() != names {
        h.hook.set_hidden(&names)?;
        *h.published.borrow_mut() = names;
    }
    h.last_sync = Instant::now();
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
            h.published.borrow_mut().clear();
            h.retry_after = Some(Instant::now() + Duration::from_millis(250));
        }
    }
    Ok(())
}

fn tick_once(s: &mut PaneApp) -> Result<(), String> {
    let h = s.session.as_mut().unwrap();
    if h.menu_active.get() {
        return Ok(());
    }
    if h.pending_workspace_save {
        s.store
            .save_workspace(&s.workspace)
            .map_err(|e| e.to_string())?;
        h.pending_workspace_save = false;
    }
    if h.last_tick.elapsed() < Duration::from_millis(20) {
        h.tick_deferred = true;
        return Ok(());
    }
    h.tick_deferred = false;
    h.last_tick = Instant::now();
    let mut urgent = h.dirty.replace(false);
    if urgent {
        h.last_reconcile = Instant::now() - Duration::from_secs(10);
    }
    let mut changed = false;
    queue_pane_icons(s, false);
    while let Ok(loaded) = s.receiver.try_recv() {
        let h = s.session.as_mut().unwrap();
        h.initial_batches = h.initial_batches.saturating_sub(1);
        changed |= !loaded.images.is_empty();
        let successful: std::collections::HashSet<_> =
            loaded.images.iter().map(|(key, _)| key.as_str()).collect();
        for key in &loaded.requested {
            h.requested.remove(key);
            if successful.contains(key.as_str()) {
                h.icon_failures.remove(key);
            } else if h
                .snapshot
                .items
                .iter()
                .any(|item| item.identity.persistent_key() == *key)
            {
                let attempts = h
                    .icon_failures
                    .get(key)
                    .map_or(1, |(attempts, _)| attempts + 1);
                h.icon_failures.insert(
                    key.clone(),
                    (
                        attempts,
                        Instant::now() + Duration::from_secs(1 << attempts.min(6)),
                    ),
                );
            }
        }
        let live: std::collections::HashSet<_> = h
            .snapshot
            .items
            .iter()
            .map(|item| item.identity.persistent_key())
            .collect();
        for (key, image) in loaded.images {
            if !live.contains(&key) {
                continue;
            }
            s.images.insert(key, Arc::new(image));
        }
    }
    if changed {
        urgent = true;
        refresh_views(s);
    }
    urgent |= refresh_changed_icons(s);
    let managed = managed_identities(s);
    let managed_keys = inventory::revision_keys(&managed);
    let h = s.session.as_mut().unwrap();
    if h.last_reconcile.elapsed() >= Duration::from_secs(2) && !h.audit.pending {
        h.audit
            .requests
            .send((managed, urgent))
            .map_err(|_| "桌面检查线程已退出")?;
        h.audit.pending = true;
    }
    if h.audit.pending {
        match h.audit.results.try_recv() {
            Ok((keys, result)) => {
                h.audit.pending = false;
                if keys != managed_keys {
                    h.dirty.set(true);
                } else {
                    h.last_reconcile = Instant::now();
                    if let Some(snapshot) = result?
                        && !inventory::same(&h.snapshot, &snapshot)
                    {
                        h.snapshot = snapshot;
                        reconcile_inventory(s)?;
                        urgent = true;
                    }
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => return Err("桌面检查线程已退出".into()),
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    // Keep a slow repair for unobservable Explorer presentation resets. Normal
    // updates are driven by input, Shell notifications and worker completions.
    if s.session.as_ref().unwrap().last_sync.elapsed() >= Duration::from_secs(30) {
        s.session.as_ref().unwrap().published.borrow_mut().clear();
    }
    if sync_due(urgent, s.session.as_ref().unwrap().last_sync.elapsed()) {
        sync(s)
    } else {
        Ok(())
    }
}

fn sync_due(urgent: bool, elapsed: Duration) -> bool {
    urgent || elapsed >= Duration::from_secs(30)
}

// A one-shot timer exists only for active input, debounce or retry deadlines.
// The supervisor's slow heartbeat handles audits and missed notifications.
pub(super) fn next_work(s: &PaneApp) -> Option<u32> {
    let h = s.session.as_ref()?;
    let now = Instant::now();
    if let Some(retry) = h.retry_after {
        return Some(
            retry
                .saturating_duration_since(now)
                .as_millis()
                .clamp(25, 1000) as u32,
        );
    }
    if h.menu_active.get() {
        return None;
    }
    if h.tick_deferred || h.dirty.get() || h.pending_desktop_input.get().is_some() {
        return Some(25);
    }
    let icon_due = h
        .icon_due
        .filter(|_| h.initial_batches == 0 && h.icon_reload.is_none());
    icon_due
        .into_iter()
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
                .clamp(25, 1000) as u32
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
    fn desktop_input_order_preserves_newer_pane_selection() {
        assert!(desktop_input_is_newer(120, Some(100)));
        assert!(!desktop_input_is_newer(120, Some(140)));
        assert!(!desktop_input_is_newer(120, Some(120)));
        assert!(desktop_input_is_newer(10, Some(u32::MAX - 10)));
    }
}

const ICON_CHANGE_MESSAGE: u32 = WM_APP + 0x352;

// Shell notifications are separate from layout revisions: Recycle Bin can change
// artwork without changing its identity, label, position or item count.
fn refresh_changed_icons(s: &mut PaneApp) -> bool {
    let h = s.session.as_mut().unwrap();
    if !h.icons_dirty.borrow().is_empty() && h.icon_due.is_none() {
        h.icon_due = Some(Instant::now() + Duration::from_millis(200));
    }
    let completed = h.icon_reload.as_ref().map(mpsc::Receiver::try_recv);
    let mut changed = false;
    match completed {
        Some(Ok(images)) => {
            h.icon_reload = None;
            let live: std::collections::HashSet<_> = h
                .snapshot
                .items
                .iter()
                .map(|item| item.identity.persistent_key())
                .collect();
            for (key, image) in images {
                if !live.contains(&key) {
                    continue;
                }
                let differs = s.images.get(&key).is_none_or(|old| {
                    old.width != image.width || old.height != image.height || old.data != image.data
                });
                if std::env::var_os("LUCIDPANE_ICON_TRACE").is_some() {
                    eprintln!(
                        "icon-result key={key} differs={differs} hash={:x}",
                        image
                            .data
                            .iter()
                            .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(*b)))
                    );
                }
                if differs {
                    s.images.insert(key, Arc::new(image));
                    changed = true;
                }
            }
        }
        Some(Err(mpsc::TryRecvError::Disconnected)) => {
            h.icon_reload = None;
        }
        _ => {}
    }
    if h.initial_batches == 0
        && h.icon_reload.is_none()
        && h.icon_due.is_some_and(|due| Instant::now() >= due)
    {
        h.icon_due = None;
        let pending = std::mem::take(&mut *h.icons_dirty.borrow_mut());
        let identities: Vec<_> = s
            .workspace
            .desktop_items()
            .iter()
            .filter(|item| matches!(item.placement(), DesktopPlacement::Pane { .. }))
            .map(|item| item.identity().clone())
            .collect();
        if !identities.is_empty() {
            let size = h.snapshot.icon_size.max(128);
            let (sender, receiver) = mpsc::channel();
            let wake = h.wake.clone();
            match std::thread::Builder::new()
                .name("pane-icon-refresh".into())
                .spawn(move || {
                    let Ok(_sta) = ShellApartment::initialize_sta() else {
                        return;
                    };
                    let affected = pending.affected(identities);
                    if std::env::var_os("LUCIDPANE_ICON_TRACE").is_some() {
                        eprintln!(
                            "icon-targets {:?}",
                            affected
                                .iter()
                                .map(ShellIdentity::persistent_key)
                                .collect::<Vec<_>>()
                        );
                    }
                    let images = affected
                        .into_iter()
                        .filter_map(|identity| {
                            assets::load(&identity, size)
                                .ok()
                                .map(|pixels| (identity.persistent_key(), pixels))
                        })
                        .collect();
                    let _ = sender.send(images);
                    wake.notify();
                }) {
                Ok(_) => {
                    h.icon_reload = Some(receiver);
                }
                Err(error) => eprintln!("Icon refresh worker failed: {error}"),
            }
        }
    }
    if changed {
        refresh_views(s);
    }
    changed
}

const RECYCLE_CHANGE_MESSAGE: u32 = WM_APP + 0x353;

fn queue_pane_icons(s: &mut PaneApp, force: bool) {
    let h = s.session.as_mut().unwrap();
    if !force && h.last_icon_scan.elapsed() < Duration::from_millis(250) {
        return;
    }
    h.last_icon_scan = Instant::now();
    let requests: Vec<_> = s
        .workspace
        .desktop_items()
        .iter()
        .filter(|item| matches!(item.placement(), DesktopPlacement::Pane { .. }))
        .filter_map(|item| {
            let key = item.identity().persistent_key();
            (!s.images.contains_key(&key)
                && h.icon_failures
                    .get(&key)
                    .is_none_or(|(attempts, next)| *attempts < 5 && Instant::now() >= *next)
                && h.requested.insert(key))
            .then(|| item.identity().clone())
        })
        .collect();
    if requests.is_empty() {
        return;
    }
    let sender = h.sender.clone();
    let wake = h.wake.clone();
    let size = h.snapshot.icon_size.max(128);
    h.initial_batches += 1;
    std::thread::spawn(move || {
        let count = requests.len();
        let started = Instant::now();
        let requested = requests.iter().map(ShellIdentity::persistent_key).collect();
        let images = load_icon_batch(requests, size);
        if std::env::var_os("LUCIDPANE_ICON_TRACE").is_some() {
            eprintln!(
                "startup-icon-batch requested={count} loaded={} elapsed_ms={}",
                images.len(),
                started.elapsed().as_millis()
            );
        }
        let _ = sender.send(Loaded { requested, images });
        wake.notify();
    });
}

fn load_icon_batch(requests: Vec<ShellIdentity>, size: i32) -> Vec<(String, assets::Pixels)> {
    let mut batches = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for (index, identity) in requests.into_iter().enumerate() {
        batches[index % 4].push(identity);
    }
    std::thread::scope(|scope| {
        let workers: Vec<_> = batches
            .into_iter()
            .filter(|batch| !batch.is_empty())
            .map(|batch| {
                scope.spawn(move || {
                    let Ok(_sta) = ShellApartment::initialize_sta() else {
                        return Vec::new();
                    };
                    batch
                        .into_iter()
                        .filter_map(|identity| match assets::load(&identity, size) {
                            Ok(image) => Some((identity.persistent_key(), image)),
                            Err(error) => {
                                eprintln!("Pane icon load failed: {error}");
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap_or_default())
            .collect()
    })
}

#[cfg(test)]
mod icon_loading_tests {
    use super::*;
    #[test]
    fn initial_batch_loads_all_requested_namespace_icons_together() {
        let identities = [
            RECYCLE_BIN_PARSING_NAME,
            "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
        ]
        .map(|name| ShellIdentity::Namespace {
            parsing_name: name.into(),
        });
        let expected: std::collections::HashSet<_> = identities
            .iter()
            .map(ShellIdentity::persistent_key)
            .collect();
        let images = load_icon_batch(identities.to_vec(), 128);
        assert_eq!(
            images
                .iter()
                .map(|(key, _)| key.clone())
                .collect::<std::collections::HashSet<_>>(),
            expected
        );
        assert!(images.iter().all(|(_, image)| !image.data.is_empty()));
    }
}

pub(super) fn refresh_icons(state: &mut PaneApp) {
    if let Some(session) = &state.session {
        session
            .icons_dirty
            .borrow_mut()
            .add([icon_changes::Change::All]);
    }
}
