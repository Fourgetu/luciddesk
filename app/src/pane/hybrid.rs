//! Self-rendered panes with a reversible, compacted native desktop outside them.
#[path = "icon_changes.rs"]
mod icon_changes;
use super::assets::RECYCLE_BIN_PARSING_NAME;
use super::*;
use desktop_hook::{HookSession, protocol::*};
use desktop_shell::{NativeDesktopSnapshot, native_desktop_snapshot};
use std::{
    cell::Cell,
    time::{Duration, Instant},
};
use windows_sys::Win32::{Graphics::Gdi::ClientToScreen, UI::WindowsAndMessaging::*};

pub(super) struct Session {
    // Drop the Hook before its owner HWND and before the pane windows.
    hook: HookSession,
    _icon_subscription: desktop_shell::DesktopChangeSubscription,
    _recycle_subscription: desktop_shell::DesktopChangeSubscription,
    _controller: windows_window::Window,
    icons_dirty: Rc<RefCell<icon_changes::Pending>>,
    icon_due: Option<Instant>,
    icon_reload: Option<mpsc::Receiver<Vec<(String, assets::Pixels)>>>,
    view: isize,
    snapshot: NativeDesktopSnapshot,
    baseline: Vec<POINT>,
    generation: isize,
    last_scan: Instant,
    last_reconcile: Instant,
    audit: DesktopAudit,
    last_tick: Instant,
    last_sync: Instant,
    last_icon_scan: Instant,
    layout_checked: Cell<Instant>,
    layout_origin: Cell<(i32, i32)>,
    layout_hidden: RefCell<Vec<bool>>,
    published: RefCell<Vec<(i32, i32, i32, String, u32)>>,
    sender: mpsc::Sender<Loaded>,
    requested: std::collections::HashSet<String>,
    icon_failures: HashMap<String, (u32, Instant)>,
    initial_batches: usize,
    menu_active: Cell<bool>,
    last_pane_input: Cell<Option<u32>>,
    pending_desktop_input: Rc<Cell<Option<u32>>>,
    mouse_down: bool,
    drag: Option<(ShellIdentity, POINT)>,
    drops: Vec<super::drop_target::Registration>,
    dirty: Rc<Cell<bool>>,
    retry_after: Option<Instant>,
    last_failure: Option<String>,
    diagnostic_path: std::path::PathBuf,
}

struct DesktopAudit {
    requests: mpsc::Sender<()>,
    results: mpsc::Receiver<Result<desktop_shell::NativeDesktopRevision, String>>,
    pending: bool,
}

impl DesktopAudit {
    fn start() -> Result<Self, String> {
        let (requests, receiver) = mpsc::channel();
        let (sender, results) = mpsc::channel();
        std::thread::Builder::new()
            .name("desktop-audit".into())
            .spawn(move || {
                let apartment = desktop_shell::ShellApartment::initialize_sta();
                // The reader is created, used and released on this STA, before
                // apartment teardown. Dropping requests ends the worker.
                let mut reader = desktop_shell::NativeDesktopReader::default();
                while receiver.recv().is_ok() {
                    let result = match &apartment {
                        Ok(_) => reader.revision(),
                        Err(error) => Err(error.to_string()),
                    };
                    if sender.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("无法启动桌面检查：{error}"))?;
        Ok(Self {
            requests,
            results,
            pending: false,
        })
    }
}

struct OleApartment;
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
    let input_state = Rc::new(RefCell::new(std::rc::Weak::<RefCell<PaneApp>>::new()));
    let input_receiver = Rc::clone(&input_state);
    let pending_desktop_input = Rc::new(Cell::new(None));
    let pending_input = Rc::clone(&pending_desktop_input);
    let controller = windows_window::Window::new("LucidPane Hybrid Controller")
        .size(1, 1)
        .style(WS_POPUP)
        .ex_style(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)
        .on_message(move |_, message, wparam, lparam| {
            if message == ICON_CHANGE_MESSAGE {
                if std::env::var_os("LUCIDPANE_ICON_TRACE").is_some() {
                    eprintln!("icon-notify event={:x}", lparam);
                }
                icon_notify
                    .borrow_mut()
                    .add(unsafe { icon_changes::capture(wparam, lparam as u32) });
                Some(0)
            } else if message == RECYCLE_CHANGE_MESSAGE {
                if std::env::var_os("LUCIDPANE_ICON_TRACE").is_some() {
                    eprintln!("recycle-notify event={:x}", lparam);
                }
                icon_notify
                    .borrow_mut()
                    .add([icon_changes::Change::Name(RECYCLE_BIN_PARSING_NAME.into())]);
                Some(0)
            } else if message == SCENE_DIRTY_MESSAGE {
                notify.set(true);
                Some(0)
            } else if message == DESKTOP_INPUT_MESSAGE {
                if wparam as isize == view {
                    pending_input.set(Some(lparam as u32));
                    if let Some(state) = input_receiver.borrow().upgrade() {
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
    let hook = HookSession::connect_geometry(
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
    let first_run = !path.exists();
    let store = WorkspaceStore::open(path).map_err(|e| e.to_string())?;
    super::peek::load(&store)?;
    {
        use std::io::Write;
        let installed = hook.request(&Request::new(QUERY_DROP_PROXY))?;
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.with_extension("log"))
        {
            let _ = writeln!(log, "Desktop OLE coordinate proxy installed={installed}");
        }
    }
    let mut workspace = store.load_workspace().map_err(|e| e.to_string())?;
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
    let snapshot = native_desktop_snapshot()?;
    let (sender, receiver) = mpsc::channel();
    let session = Session {
        hook,
        _icon_subscription: icon_subscription,
        _recycle_subscription: recycle_subscription,
        _controller: controller,
        icons_dirty,
        icon_due: None,
        icon_reload: None,
        view,
        snapshot,
        baseline: Vec::new(),
        generation: -1,
        last_scan: Instant::now(),
        last_reconcile: Instant::now(),
        audit: DesktopAudit::start()?,
        last_tick: Instant::now(),
        last_sync: Instant::now(),
        last_icon_scan: Instant::now() - Duration::from_secs(1),
        layout_checked: Cell::new(Instant::now()),
        layout_origin: Cell::new((0, 0)),
        layout_hidden: RefCell::new(Vec::new()),
        published: RefCell::new(Vec::new()),
        sender,
        requested: Default::default(),
        icon_failures: HashMap::new(),
        initial_batches: 0,
        menu_active: Cell::new(false),
        last_pane_input: Cell::new(None),
        pending_desktop_input,
        mouse_down: false,
        drag: None,
        drops: Vec::new(),
        dirty,
        retry_after: None,
        last_failure: None,
        diagnostic_path: path.with_extension("log"),
    };
    let state = Rc::new(RefCell::new(PaneApp {
        settings: None,
        session: Some(session),
        workspace,
        store,
        views: Vec::new(),
        images: HashMap::new(),
        receiver,
    }));
    *input_state.borrow_mut() = Rc::downgrade(&state);
    refresh(&mut state.borrow_mut())?;
    let ids: Vec<_> = state
        .borrow()
        .workspace
        .panels()
        .iter()
        .map(Panel::id)
        .collect();
    for id in ids {
        create_view(&state, id)?;
        let s = state.borrow();
        let v = s.views.last().unwrap();
        unsafe {
            SetTimer(v.window.hwnd().cast(), 1, 25, None);
        }
    }
    let tray_state = Rc::downgrade(&state);
    let tray = crate::tray::Tray::new(move |action| {
        let Some(state) = tray_state.upgrade() else {
            return;
        };
        match action {
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
    })?;
    windows_window::run();
    drop(tray);
    state.borrow_mut().session.take();
    Ok(())
}

pub(super) fn register_drop(state: &Rc<RefCell<PaneApp>>, id: PanelId) -> Result<(), String> {
    let hwnd = state
        .borrow()
        .views
        .iter()
        .find(|v| v.id == id)
        .unwrap()
        .window
        .hwnd();
    let weak = Rc::downgrade(state);
    let registration = super::drop_target::Registration::new(
        windows::Win32::Foundation::HWND(hwnd.cast()),
        move |identities, commit| {
            let Some(state) = weak.upgrade() else {
                return false;
            };
            let Ok(mut s) = state.try_borrow_mut() else {
                return false;
            };
            if s.views
                .iter()
                .find(|v| v.id == id)
                .is_none_or(|v| v.model.borrow().collapsed)
            {
                return false;
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
            let old = s.workspace.clone();
            normalize_pane_orders(&mut s);
            let mut at = items_for(&s, id).len();
            for identity in keys {
                let item = s.workspace.desktop_item_mut(&identity).unwrap();
                item.set_placement(DesktopPlacement::Pane {
                    pane_id: id,
                    position: GridPosition::new(at as u32, 0),
                });
                at += 1;
            }
            // The OLE transaction owns this release; discard the legacy mouse observation.
            s.session.as_mut().unwrap().drag = None;
            if let Err(error) = save(&mut s) {
                s.workspace = old;
                let _ = sync(&mut s);
                eprintln!("Hybrid collection rejected: {error}");
                return false;
            }
            refresh_views(&mut s);
            true
        },
    )
    .map_err(|e| e.to_string())?;
    state
        .borrow_mut()
        .session
        .as_mut()
        .unwrap()
        .drops
        .push(registration);
    Ok(())
}

pub(super) fn unregister_drop(state: &mut PaneApp, hwnd: windows_sys::Win32::Foundation::HWND) {
    if let Some(session) = state.session.as_mut() {
        session
            .drops
            .retain(|registration| registration.window().0 != hwnd);
    }
}

fn refresh(s: &mut PaneApp) -> Result<(), String> {
    normalize_pane_orders(s);
    let h = s.session.as_mut().unwrap();
    let mut accepted = None;
    for _ in 0..3 {
        let generation = h.hook.request(&Request::new(QUERY_SHELL_GENERATION))?;
        let snapshot = native_desktop_snapshot()?;
        if !valid_inventory(&snapshot) {
            continue;
        }
        let mut baseline = Vec::new();
        for index in &snapshot.view_indices {
            let mut q = Request::new(QUERY_ORIGINAL_POSITION);
            q.item = *index;
            let x = h.hook.request(&q)?;
            q.x = 1;
            let y = h.hook.request(&q)?;
            if x == REJECTED || y == REJECTED {
                return Err("读取原生布局失败".into());
            }
            baseline.push(POINT {
                x: x as i32,
                y: y as i32,
            });
        }
        if h.hook.request(&Request::new(QUERY_SHELL_GENERATION))? == generation
            && same_inventory(&snapshot, &native_desktop_snapshot()?)
        {
            accepted = Some((snapshot, baseline, generation));
            break;
        }
    }
    let (snapshot, baseline, generation) = accepted.ok_or("桌面正在变化，未发布不完整布局")?;
    h.last_reconcile = Instant::now();
    if generation == h.generation
        && same_inventory(&h.snapshot, &snapshot)
        && h.baseline
            .iter()
            .zip(&baseline)
            .all(|(a, b)| a.x == b.x && a.y == b.y)
    {
        h.snapshot.revision = snapshot.revision;
        // Re-publish occasionally even if Shell refreshed its drawing state through
        // an internal path that did not produce an observable inventory revision.
        h.published.borrow_mut().clear();
        return Ok(());
    }
    s.workspace.reconcile_desktop_items(
        snapshot
            .items
            .iter()
            .map(|(i, _, _)| DesktopItem::new(i.identity.clone(), i.display_name.clone())),
    );
    let valid: Vec<_> = s.workspace.panels().iter().map(Panel::id).collect();
    for item in s.workspace.desktop_items_mut() {
        if matches!(item.placement(),DesktopPlacement::Pane{pane_id,..} if !valid.contains(pane_id))
        {
            item.set_placement(DesktopPlacement::default());
        }
    }
    let live: std::collections::HashSet<_> = snapshot
        .items
        .iter()
        .map(|(item, _, _)| item.identity.persistent_key())
        .collect();
    s.images.retain(|key, _| live.contains(key));
    h.requested.retain(|key| live.contains(key));
    h.icon_failures.retain(|key, _| live.contains(key));
    h.snapshot = snapshot;
    h.baseline = baseline;
    h.generation = generation;
    h.published.borrow_mut().clear();
    queue_pane_icons(s, true);
    s.store
        .save_workspace(&s.workspace)
        .map_err(|e| e.to_string())?;
    refresh_views(s);
    Ok(())
}

/// Pack visible items into the original native slots, keeping each monitor independent.
fn compact(slots: &[POINT], hidden: &[bool], monitors: &[Area]) -> Vec<POINT> {
    let mut result = slots.to_vec();
    for monitor in 0..=monitors.len() {
        let owner = |p: &POINT| {
            monitors
                .iter()
                .position(|m| m.contains(p.x, p.y))
                .unwrap_or(monitors.len())
        };
        let mut indices: Vec<_> = (0..slots.len())
            .filter(|i| owner(&slots[*i]) == monitor)
            .collect();
        indices.sort_by_key(|i| (slots[*i].x, slots[*i].y));
        let destinations: Vec<_> = indices.iter().map(|i| slots[*i]).collect();
        for (index, point) in indices
            .into_iter()
            .filter(|i| !hidden[*i])
            .zip(destinations)
        {
            result[index] = point;
        }
    }
    result
}

pub(super) fn sync(s: &mut PaneApp) -> Result<(), String> {
    if s.session.is_none() {
        return Ok(());
    }
    let mut last_error = String::new();
    for _ in 0..2 {
        let h = s.session.as_ref().unwrap();
        if h.menu_active.get() {
            return Ok(());
        }
        if h.hook.request(&Request::new(QUERY_SHELL_GENERATION))? != h.generation {
            refresh(s)?;
        }
        match publish(s) {
            Ok(()) => {
                s.session.as_mut().unwrap().last_sync = Instant::now();
                return Ok(());
            }
            Err(error) => {
                last_error = error;
                refresh(s)?;
            }
        }
    }
    Err(last_error)
}

fn publish(s: &PaneApp) -> Result<(), String> {
    let Some(h) = &s.session else {
        return Ok(());
    };
    if h.menu_active.get() {
        return Ok(());
    }
    let mut origin = POINT::default();
    unsafe {
        ClientToScreen(h.view as _, &raw mut origin);
    }
    let pane_keys: std::collections::HashSet<_> = s
        .workspace
        .desktop_items()
        .iter()
        .filter(|item| matches!(item.placement(), DesktopPlacement::Pane { .. }))
        .map(|item| item.identity().persistent_key())
        .collect();
    let hidden: Vec<_> = h
        .snapshot
        .items
        .iter()
        .map(|(item, _, _)| {
            let key = item.identity.persistent_key();
            pane_keys.contains(&key) && s.images.contains_key(&key)
        })
        .collect();
    if !h.published.borrow().is_empty()
        && *h.layout_hidden.borrow() == hidden
        && h.layout_origin.get() == (origin.x, origin.y)
        && h.layout_checked.get().elapsed() < Duration::from_secs(1)
    {
        return Ok(());
    }
    let monitors: Vec<_> = desktop_window::enumerate_monitors()
        .iter()
        .map(|m| Area {
            left: m.bounds.x - origin.x,
            top: m.bounds.y - origin.y,
            right: m.bounds.x + m.bounds.width - origin.x,
            bottom: m.bounds.y + m.bounds.height - origin.y,
        })
        .collect();
    h.layout_checked.set(Instant::now());
    h.layout_origin.set((origin.x, origin.y));
    *h.layout_hidden.borrow_mut() = hidden.clone();
    let positions = compact(&h.baseline, &hidden, &monitors);
    let batch: Vec<_> = h
        .snapshot
        .items
        .iter()
        .enumerate()
        .map(|(i, (item, _, _))| {
            (
                h.snapshot.view_indices[i],
                positions[i].x,
                positions[i].y,
                item.display_name.clone(),
                if hidden[i] { HIDDEN_ITEM } else { 0 },
            )
        })
        .collect();
    if *h.published.borrow() == batch {
        return Ok(());
    }
    // The validated transaction also accepts baseline cells just outside the work area.
    h.hook.apply_pane_layout(
        &[Area {
            left: -100_000,
            top: -100_000,
            right: 100_001,
            bottom: 100_001,
        }],
        &batch,
        &[],
    )?;
    if std::env::var_os("LUCIDPANE_HIT_AUDIT").is_some() {
        audit_hits(h, &batch);
    }
    *h.published.borrow_mut() = batch;
    Ok(())
}

fn audit_hits(h: &Session, batch: &[(i32, i32, i32, String, u32)]) {
    use std::io::Write;
    let Ok(mut log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(h.diagnostic_path.with_file_name("hit-audit.log"))
    else {
        return;
    };
    let _ = writeln!(
        log,
        "AUDIT {:?} items={} hidden={}",
        std::time::SystemTime::now(),
        batch.len(),
        batch.iter().filter(|i| i.4 == HIDDEN_ITEM).count()
    );
    for (index, x, y, name, hidden) in batch {
        let mut rect = [0isize; 4];
        for (axis, coord) in rect.iter_mut().enumerate() {
            let mut q = Request::new(QUERY_ICON_RECT);
            q.item = *index;
            q.x = axis as i32;
            *coord = h.hook.request(&q).unwrap_or(REJECTED);
        }
        let mut q = Request::new(QUERY_HIT);
        q.x = *x + h.snapshot.spacing.0 / 2;
        q.y = *y + h.snapshot.icon_size / 2;
        let expected_hit = h.hook.request(&q).unwrap_or(REJECTED) - 1;
        q.command = QUERY_INSERTION_TARGET;
        let insertion = h.hook.request(&q).unwrap_or(REJECTED);
        q.y = *y + 10;
        let gap_before = h.hook.request(&q).unwrap_or(REJECTED);
        q.y = *y + h.snapshot.spacing.1 - 10;
        let gap_after = h.hook.request(&q).unwrap_or(REJECTED);
        q.command = QUERY_HIT;
        q.x = ((rect[0] + rect[2]) / 2) as i32;
        q.y = ((rect[1] + rect[3]) / 2) as i32;
        let rect_hit = h.hook.request(&q).unwrap_or(REJECTED) - 1;
        let _ = writeln!(
            log,
            "item={index} hidden={} target={x},{y} rect={rect:?} target_hit={expected_hit} rect_hit={rect_hit} insertion={insertion} gap_before={gap_before} gap_after={gap_after} match={} name={name}",
            *hidden == HIDDEN_ITEM,
            *hidden == HIDDEN_ITEM
                || (expected_hit == *index as isize && rect_hit == *index as isize)
        );
    }
}

pub(super) fn clear_desktop_selection(s: &PaneApp) -> Result<(), String> {
    if let Some(h) = &s.session {
        h.last_pane_input
            .set(Some(unsafe { GetMessageTime() } as u32));
        h.hook.post_clear_desktop_selection()?;
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

pub(super) fn menu(s: &PaneApp, allow: bool) -> Result<bool, String> {
    if let Some(h) = &s.session {
        if allow {
            h.last_pane_input
                .set(Some(unsafe { GetMessageTime() } as u32));
        }
        let result = h.hook.request(&Request::new(if allow {
            MENU_SELECTION_BEGIN
        } else {
            MENU_SELECTION_END
        }))?;
        h.menu_active.set(allow);
        return Ok(result == RENAME_REQUESTED);
    }
    Ok(false)
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
    if h.last_tick.elapsed() < Duration::from_millis(20) || h.menu_active.get() {
        return Ok(());
    }
    h.last_tick = Instant::now();
    let mut urgent = h.dirty.replace(false);
    if urgent {
        refresh(s)?;
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
                .any(|(item, _, _)| item.identity.persistent_key() == *key)
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
            .map(|(item, _, _)| item.identity.persistent_key())
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
        for v in &s.views {
            v.model.borrow_mut().loading = false;
        }
        refresh_views(s);
    }
    refresh_changed_icons(s);
    let was_down = s.session.as_ref().unwrap().mouse_down;
    poll_drag(s)?;
    let h = s.session.as_mut().unwrap();
    urgent |= was_down && !h.mouse_down;
    if h.last_scan.elapsed() > Duration::from_millis(500) && !h.mouse_down {
        h.last_scan = Instant::now();
        if h.hook.request(&Request::new(QUERY))? != OK {
            return Err("桌面 Hook 已退出".into());
        }
        // Explorer can reorder its owner-data model without changing item count or
        // sending the public sort messages. Reconcile identities even in that case.
        if h.hook.request(&Request::new(QUERY_SHELL_GENERATION))? != h.generation {
            refresh(s)?;
            urgent = true;
        } else if h.last_reconcile.elapsed() > Duration::from_secs(1) && !h.audit.pending {
            h.audit
                .requests
                .send(())
                .map_err(|_| "桌面检查线程已退出")?;
            h.audit.pending = true;
        }
    }
    let h = s.session.as_mut().unwrap();
    if !h.mouse_down {
        let audit = h.audit.pending.then(|| h.audit.results.try_recv());
        match audit {
            Some(Ok(result)) => {
                h.audit.pending = false;
                let revision = result?;
                let unchanged = h.snapshot.revision == revision && baseline_matches(h)?;
                h.last_reconcile = Instant::now();
                if !unchanged {
                    // Revision checks never resolve file metadata or publish layouts.
                    // Read a validated full snapshot only when something changed.
                    refresh(s)?;
                    urgent = true;
                } else {
                    // Preserve the periodic re-publish that repairs internal
                    // Explorer presentation resets, without re-reading metadata.
                    h.published.borrow_mut().clear();
                    urgent = true;
                }
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                h.audit.pending = false;
                return Err("桌面检查线程已退出".into());
            }
            None | Some(Err(mpsc::TryRecvError::Empty)) => {}
        }
    }
    // Explicit user operations still call sync immediately. Only repeated timer
    // checks are coalesced; dirty scenes and release updates are never delayed.
    if sync_due(urgent, s.session.as_ref().unwrap().last_sync.elapsed()) {
        sync(s)
    } else {
        Ok(())
    }
}

fn sync_due(urgent: bool, elapsed: Duration) -> bool {
    urgent || elapsed >= Duration::from_millis(100)
}

fn baseline_matches(h: &Session) -> Result<bool, String> {
    if h.baseline.len() != h.snapshot.view_indices.len() {
        return Ok(false);
    }
    for (&index, point) in h.snapshot.view_indices.iter().zip(&h.baseline) {
        let mut query = Request::new(QUERY_ORIGINAL_POSITION);
        query.item = index;
        if h.hook.request(&query)? != point.x as isize {
            return Ok(false);
        }
        query.x = 1;
        if h.hook.request(&query)? != point.y as isize {
            return Ok(false);
        }
    }
    Ok(true)
}

fn valid_inventory(snapshot: &NativeDesktopSnapshot) -> bool {
    let mut identities = std::collections::HashSet::new();
    let mut indices = std::collections::HashSet::new();
    snapshot.items.len() == snapshot.view_indices.len()
        && snapshot
            .items
            .iter()
            .all(|(item, _, _)| identities.insert(item.identity.persistent_key()))
        && snapshot
            .view_indices
            .iter()
            .all(|index| *index >= 0 && indices.insert(*index))
}

fn same_inventory(a: &NativeDesktopSnapshot, b: &NativeDesktopSnapshot) -> bool {
    a.view_indices == b.view_indices
        && a.icon_size == b.icon_size
        && a.spacing == b.spacing
        && a.dpi == b.dpi
        && a.items.len() == b.items.len()
        && a.items.iter().zip(&b.items).all(|((a, _, _), (b, _, _))| {
            a.identity == b.identity && a.display_name == b.display_name && a.modified == b.modified
        })
}

fn poll_drag(s: &mut PaneApp) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON,
    };
    let h = s.session.as_mut().unwrap();
    let down = unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } < 0;
    if unsafe { GetAsyncKeyState(VK_ESCAPE as i32) } < 0 {
        h.drag = None;
    }
    let mut point = POINT::default();
    unsafe {
        GetCursorPos(&raw mut point);
    }
    let surface = unsafe { WindowFromPoint(point) };
    if down && !h.mouse_down && surface == h.view as _ {
        let mut origin = POINT::default();
        unsafe {
            ClientToScreen(h.view as _, &raw mut origin);
        }
        let mut q = Request::new(QUERY_HIT);
        q.x = point.x - origin.x;
        q.y = point.y - origin.y;
        let index = h.hook.request(&q)? - 1;
        if let Some(i) = h
            .snapshot
            .view_indices
            .iter()
            .position(|i| *i as isize == index)
        {
            h.drag = Some((h.snapshot.items[i].0.identity.clone(), point));
        }
    }
    let released = !down && h.mouse_down;
    h.mouse_down = down;
    if !released {
        return Ok(());
    }
    let Some((identity, start)) = h.drag.take() else {
        return Ok(());
    };
    {
        use std::io::Write;
        let count = h
            .hook
            .request(&Request::new(QUERY_MOVE_REQUESTS))
            .unwrap_or(REJECTED);
        let record = format!(
            "Desktop drag {:?}: {},{} -> {},{}; native move requests={count}",
            identity.persistent_key(),
            start.x,
            start.y,
            point.x,
            point.y
        );
        eprintln!("{record}");
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&h.diagnostic_path)
        {
            let _ = writeln!(log, "{record}");
        }
    }
    if start.x.abs_diff(point.x) < 8 && start.y.abs_diff(point.y) < 8 {
        return Ok(());
    }
    let target = s
        .views
        .iter()
        .find(|v| {
            v.window.hwnd().cast::<std::ffi::c_void>() == surface && !v.model.borrow().collapsed
        })
        .map(|v| v.id);
    if let Some(id) = target {
        normalize_pane_orders(s);
        let at = items_for(s, id).len();
        if let Some(item) = s.workspace.desktop_item_mut(&identity) {
            item.set_placement(DesktopPlacement::Pane {
                pane_id: id,
                position: GridPosition::new(at as u32, 0),
            });
        }
        save(s)?;
        refresh_views(s);
    }
    Ok(())
}

pub(super) fn release(
    s: &mut PaneApp,
    id: PanelId,
    index: usize,
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
    let Some(item) = items.get(index) else {
        return Ok(false);
    };
    if let Some(entry) = s.workspace.desktop_item_mut(&item.identity) {
        entry.set_placement(DesktopPlacement::default());
    }
    save(s)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_press_order_survives_activation_races_and_rejects_stale_input() {
        // A first desktop press follows pane input even while the pane is still
        // foreground. No activation state participates in this decision.
        assert!(desktop_input_is_newer(120, Some(100)));
        assert!(desktop_input_is_newer(120, None));
        // The same notification delivered after a new pane choice is stale.
        assert!(!desktop_input_is_newer(120, Some(140)));
        assert!(!desktop_input_is_newer(120, Some(120)));
        // Windows message timestamps wrap without changing event order.
        assert!(desktop_input_is_newer(10, Some(u32::MAX - 10)));
        assert!(!desktop_input_is_newer(u32::MAX - 10, Some(10)));
    }

    #[test]
    fn duplicate_ticks_coalesce_but_dirty_and_release_updates_are_immediate() {
        for elapsed in [0, 20, 40, 60, 80] {
            assert!(!sync_due(false, Duration::from_millis(elapsed)));
            assert!(sync_due(true, Duration::from_millis(elapsed)));
        }
        assert!(sync_due(false, Duration::from_millis(100)));
    }
    #[test]
    fn same_count_reorder_is_detected_and_membership_follows_identity() {
        let make = |name: &str| {
            (
                desktop_shell::DesktopShellItem {
                    identity: ShellIdentity::Namespace {
                        parsing_name: name.into(),
                    },
                    display_name: name.into(),
                    attributes: desktop_shell::ShellAttributes::default(),
                    modified: None,
                },
                0,
                0,
            )
        };
        let before = NativeDesktopSnapshot {
            revision: Default::default(),
            icon_size: 48,
            spacing: (100, 100),
            dpi: 96,
            items: vec![make("a"), make("b")],
            view_indices: vec![0, 1],
        };
        let mut after = before.clone();
        assert!(valid_inventory(&before));
        let mut transient = before.clone();
        transient.items[1] = transient.items[0].clone();
        assert!(
            !valid_inventory(&transient),
            "A torn Shell snapshot must never reach storage"
        );
        transient = before.clone();
        transient.view_indices[1] = 0;
        assert!(!valid_inventory(&transient));
        after.items.swap(0, 1);
        assert!(!same_inventory(&before, &after));
        let mut workspace = Workspace::new();
        workspace.reconcile_desktop_items(
            before
                .items
                .iter()
                .map(|(i, _, _)| DesktopItem::new(i.identity.clone(), i.display_name.clone())),
        );
        workspace
            .desktop_item_mut(&before.items[0].0.identity)
            .unwrap()
            .set_placement(DesktopPlacement::Pane {
                pane_id: PanelId::new(1),
                position: GridPosition::default(),
            });
        workspace.reconcile_desktop_items(
            after
                .items
                .iter()
                .map(|(i, _, _)| DesktopItem::new(i.identity.clone(), i.display_name.clone())),
        );
        let hidden: Vec<_> = after
            .items
            .iter()
            .map(|(i, _, _)| {
                matches!(
                    workspace.desktop_item(&i.identity).unwrap().placement(),
                    DesktopPlacement::Pane { .. }
                )
            })
            .collect();
        assert_eq!(hidden, [false, true]);
        assert!(same_inventory(&after, &after));
    }
    #[test]
    fn packing_removes_middle_and_leading_gaps_without_crossing_monitors() {
        let slots = vec![
            POINT { x: 0, y: 0 },
            POINT { x: 0, y: 100 },
            POINT { x: 0, y: 200 },
            POINT { x: 1000, y: 0 },
            POINT { x: 1000, y: 100 },
        ];
        let monitors = [
            Area {
                left: 0,
                top: 0,
                right: 500,
                bottom: 500,
            },
            Area {
                left: 1000,
                top: 0,
                right: 1500,
                bottom: 500,
            },
        ];
        let result = compact(&slots, &[true, false, false, true, false], &monitors);
        assert_eq!((result[1].x, result[1].y), (0, 0));
        assert_eq!((result[2].x, result[2].y), (0, 100));
        assert_eq!((result[4].x, result[4].y), (1000, 0));
        let restored = compact(&slots, &[false; 5], &monitors);
        assert!(
            restored
                .iter()
                .zip(slots)
                .all(|(a, b)| a.x == b.x && a.y == b.y)
        );
    }
}

const ICON_CHANGE_MESSAGE: u32 = WM_APP + 0x352;

// Shell notifications are separate from layout revisions: Recycle Bin can change
// artwork without changing its identity, label, position or item count.
fn refresh_changed_icons(s: &mut PaneApp) {
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
                .map(|(item, _, _)| item.identity.persistent_key())
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
}

const RECYCLE_CHANGE_MESSAGE: u32 = WM_APP + 0x353;

pub(super) fn invalidate_icon(s: &PaneApp, identity: &ShellIdentity) {
    if let Some(h) = &s.session {
        let name = match identity {
            ShellIdentity::FileSystem { path, .. } => path.to_string_lossy().into_owned(),
            ShellIdentity::Namespace { parsing_name } => parsing_name.clone(),
        };
        h.icons_dirty
            .borrow_mut()
            .add([icon_changes::Change::Name(name)]);
    }
}

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
