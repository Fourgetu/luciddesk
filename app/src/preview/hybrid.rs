//! Self-rendered panes with a reversible, compacted native desktop outside them.
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
    _controller: windows_window::Window,
    view: isize,
    snapshot: NativeDesktopSnapshot,
    baseline: Vec<POINT>,
    generation: isize,
    last_scan: Instant,
    last_reconcile: Instant,
    last_tick: Instant,
    published: RefCell<Vec<(i32, i32, i32, String, u32)>>,
    sender: mpsc::Sender<Loaded>,
    requested: std::collections::HashSet<String>,
    menu_active: Cell<bool>,
    mouse_down: bool,
    drag: Option<(ShellIdentity, POINT)>,
    drops: Vec<super::drop_target::Registration>,
    dirty: Rc<Cell<bool>>,
    retry_after: Option<Instant>,
    last_failure: Option<String>,
    diagnostic_path: std::path::PathBuf,
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
    let dirty=Rc::new(Cell::new(false));
    let notify=Rc::clone(&dirty);
    let controller = windows_window::Window::new("LucidPane Hybrid Controller")
        .size(1, 1)
        .style(WS_POPUP)
        .ex_style(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)
        .on_message(move|_,message,_,_|{
            if message==SCENE_DIRTY_MESSAGE {notify.set(true);Some(0)}else{None}
        })
        .create()
        .map_err(|e| e.to_string())?;
    unsafe {
        ShowWindow(controller.hwnd().cast(), SW_HIDE);
    }
    let hook = HookSession::connect_geometry(
        view,
        controller.hwnd() as isize,
        &crate::hook_desktop::runtime_dll(path)?,
    )?;
    let store = WorkspaceStore::open(path).map_err(|e| e.to_string())?;
    {
        use std::io::Write;
        let installed = hook.request(&Request::new(QUERY_DROP_PROXY))?;
        if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(path.with_extension("log")) {
            let _ = writeln!(log, "Desktop OLE coordinate proxy installed={installed}");
        }
    }
    let mut workspace = store.load_workspace().map_err(|e| e.to_string())?;
    if workspace.panels().is_empty() {
        let mut pane = Panel::new(
            PanelId::new(1),
            title.clone().unwrap_or_else(|| "新建分组".into()),
            PanelSource::DesktopCollection,
            RectDip::new(650.0, 100.0, 440.0, 380.0),
        );
        pane.set_backdrop(desktop_core::Backdrop::Acrylic);
        workspace.add_panel(pane).map_err(|e| e.to_string())?;
    } else if let Some(title) = title {
        let id = workspace.panels()[0].id();
        workspace.panel_mut(id).unwrap().set_title(title);
    }
    let snapshot = native_desktop_snapshot()?;
    let (sender, receiver) = mpsc::channel();
    let session = Session {
        hook,
        _controller: controller,
        view,
        snapshot,
        baseline: Vec::new(),
        generation: -1,
        last_scan: Instant::now(),
        last_reconcile: Instant::now(),
        last_tick: Instant::now(),
        published: RefCell::new(Vec::new()),
        sender,
        requested: Default::default(),
        menu_active: Cell::new(false),
        mouse_down: false,
        drag: None,
        drops: Vec::new(),
        dirty,
        retry_after: None,
        last_failure: None,
        diagnostic_path: path.with_extension("log"),
    };
    let state = Rc::new(RefCell::new(Preview {
        hybrid: Some(session),
        workspace,
        store,
        views: Vec::new(),
        images: HashMap::new(),
        receiver,
        desktop: None,
    }));
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
    windows_window::run();
    state.borrow_mut().hybrid.take();
    Ok(())
}

pub(super) fn register_drop(state: &Rc<RefCell<Preview>>, id: PanelId) -> Result<(), String> {
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
            s.hybrid.as_mut().unwrap().drag = None;
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
        .hybrid
        .as_mut()
        .unwrap()
        .drops
        .push(registration);
    Ok(())
}

fn refresh(s: &mut Preview) -> Result<(), String> {
    normalize_pane_orders(s);
    let h = s.hybrid.as_mut().unwrap();
    let mut accepted = None;
    for _ in 0..3 {
        let generation = h.hook.request(&Request::new(QUERY_SHELL_GENERATION))?;
        let snapshot = native_desktop_snapshot()?;
        if !valid_inventory(&snapshot) { continue; }
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
            && same_inventory(&snapshot, &native_desktop_snapshot()?) {
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
    h.snapshot = snapshot;
    h.baseline = baseline;
    h.generation = generation;
    h.published.borrow_mut().clear();
    let requests: Vec<_> = h
        .snapshot
        .items
        .iter()
        .filter_map(|(i, _, _)| {
            h.requested
                .insert(i.identity.persistent_key())
                .then_some(i.identity.clone())
        })
        .collect();
    let sender = h.sender.clone();
    let size = h.snapshot.icon_size;
    if !requests.is_empty() {
        std::thread::spawn(move || {
            let Ok(_sta) = ShellApartment::initialize_sta() else {
                return;
            };
            for identity in requests {
                match assets::load(&identity, size) {
                    Ok(image) => {
                        if sender
                            .send(Loaded::Image(identity.persistent_key(), image))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) => eprintln!("Hybrid icon load failed: {error}"),
                }
            }
        });
    }
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

pub(super) fn sync(s: &mut Preview) -> Result<(), String> {
    if s.hybrid.is_none() {
        return Ok(());
    }
    let mut last_error = String::new();
    for _ in 0..2 {
        let h = s.hybrid.as_ref().unwrap();
        if h.menu_active.get() {
            return Ok(());
        }
        if h.hook.request(&Request::new(QUERY_SHELL_GENERATION))? != h.generation {
            refresh(s)?;
        }
        match publish(s) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = error;
                refresh(s)?;
            }
        }
    }
    Err(last_error)
}

fn publish(s: &Preview) -> Result<(), String> {
    let Some(h) = &s.hybrid else {
        return Ok(());
    };
    if h.menu_active.get() {
        return Ok(());
    }
    let mut origin = POINT::default();
    unsafe {
        ClientToScreen(h.view as _, &raw mut origin);
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
    let hidden: Vec<_> = h
        .snapshot
        .items
        .iter()
        .map(|(item, _, _)| {
            s.workspace
                .desktop_item(&item.identity)
                .is_some_and(|i| matches!(i.placement(), DesktopPlacement::Pane { .. }))
                && s.images.contains_key(&item.identity.persistent_key())
        })
        .collect();
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
    if std::env::var_os("LUCIDPANE_HIT_AUDIT").is_some() { audit_hits(h, &batch); }
    *h.published.borrow_mut() = batch;
    Ok(())
}

fn audit_hits(h: &Session, batch: &[(i32, i32, i32, String, u32)]) {
    use std::io::Write;
    let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(h.diagnostic_path.with_file_name("hit-audit.log")) else { return; };
    let _ = writeln!(log, "AUDIT {:?} items={} hidden={}", std::time::SystemTime::now(), batch.len(), batch.iter().filter(|i| i.4 == HIDDEN_ITEM).count());
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
        let _ = writeln!(log, "item={index} hidden={} target={x},{y} rect={rect:?} target_hit={expected_hit} rect_hit={rect_hit} insertion={insertion} gap_before={gap_before} gap_after={gap_after} match={} name={name}", *hidden == HIDDEN_ITEM, *hidden == HIDDEN_ITEM || (expected_hit == *index as isize && rect_hit == *index as isize));
    }
}

pub(super) fn menu(s: &Preview, allow: bool) -> Result<(), String> {
    if let Some(h) = &s.hybrid {
        h.hook.request(&Request::new(if allow {
            MENU_SELECTION_BEGIN
        } else {
            MENU_SELECTION_END
        }))?;
        h.menu_active.set(allow);
    }
    Ok(())
}

pub(super) fn tick(s: &mut Preview) -> Result<(), String> {
    let h = s.hybrid.as_ref().unwrap();
    if h.retry_after.is_some_and(|deadline| Instant::now() < deadline) {
        return Ok(());
    }
    match tick_once(s) {
        Ok(()) => {
            let h = s.hybrid.as_mut().unwrap();
            h.retry_after = None;
            h.last_failure = None;
        }
        Err(error) => {
            let h = s.hybrid.as_mut().unwrap();
            if h.last_failure.as_ref() != Some(&error) {
                use std::io::Write;
                eprintln!("Hybrid synchronization deferred: {error}");
                if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(&h.diagnostic_path) {
                    let _ = writeln!(log, "{:?} synchronization deferred: {error}", std::time::SystemTime::now());
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

fn tick_once(s: &mut Preview) -> Result<(), String> {
    let h = s.hybrid.as_mut().unwrap();
    if h.last_tick.elapsed() < Duration::from_millis(20) || h.menu_active.get() {
        return Ok(());
    }
    h.last_tick = Instant::now();
    if h.dirty.replace(false) { refresh(s)?; }
    let mut changed = false;
    while let Ok(Loaded::Image(key, image)) = s.receiver.try_recv() {
        s.images.insert(key, Arc::new(image));
        changed = true;
    }
    if changed {
        for v in &s.views {
            v.model.borrow_mut().loading = false;
        }
        refresh_views(s);
    }
    poll_drag(s)?;
    let h = s.hybrid.as_mut().unwrap();
    if h.last_scan.elapsed() > Duration::from_millis(500) && !h.mouse_down {
        h.last_scan = Instant::now();
        if h.hook.request(&Request::new(QUERY))? != OK {
            return Err("桌面 Hook 已退出".into());
        }
        // Explorer can reorder its owner-data model without changing item count or
        // sending the public sort messages. Reconcile identities even in that case.
        if h.hook.request(&Request::new(QUERY_SHELL_GENERATION))? != h.generation
            || h.last_reconcile.elapsed() > Duration::from_secs(1)
        {
            refresh(s)?;
        }
    }
    sync(s)
}

fn valid_inventory(snapshot: &NativeDesktopSnapshot) -> bool {
    let mut identities = std::collections::HashSet::new();
    let mut indices = std::collections::HashSet::new();
    snapshot.items.len() == snapshot.view_indices.len()
        && snapshot.items.iter().all(|(item, _, _)| identities.insert(item.identity.persistent_key()))
        && snapshot.view_indices.iter().all(|index| *index >= 0 && indices.insert(*index))
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

fn poll_drag(s: &mut Preview) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON,
    };
    let h = s.hybrid.as_mut().unwrap();
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
        let count = h.hook.request(&Request::new(QUERY_MOVE_REQUESTS)).unwrap_or(REJECTED);
        let record = format!("Desktop drag {:?}: {},{} -> {},{}; native move requests={count}", identity.persistent_key(), start.x, start.y, point.x, point.y);
        eprintln!("{record}");
        if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(&h.diagnostic_path) {
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
    s: &mut Preview,
    id: PanelId,
    index: usize,
    point: POINT,
) -> Result<bool, String> {
    let Some(h) = &s.hybrid else {
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
                    system_icon: None,
                },
                0,
                0,
            )
        };
        let before = NativeDesktopSnapshot {
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
        assert!(!valid_inventory(&transient), "A torn Shell snapshot must never reach storage");
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
