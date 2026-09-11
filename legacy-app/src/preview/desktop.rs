//! Managed desktop using the current pane renderer. File paths and Explorer layout flags are read-only.
use super::{
    DesktopItem, DesktopPlacement, HashMap, Item, Loaded, POINT, Panel, PanelId,
    Preview, ShellApartment, WindowFromPoint,
    Workspace, assets, items_for, mpsc, save,
};
use super::{GroupModel, View, create_view, window};
use super::{PanelSource, Path, Rc, RectDip, RefCell, WorkspaceStore, handle};
use desktop_core::{MonitorId, PointDip};
use desktop_shell::{NativeDesktopSnapshot, native_desktop_snapshot};
use desktop_window::MonitorDescriptor;
use desktop_window::enumerate_monitors;
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub(super) struct Session {
    pub monitors: Vec<(PanelId, MonitorDescriptor)>,
    pub icon_size: f32,
    pub spacing: (f32, f32),
    pub snapshot: NativeDesktopSnapshot,
    pub loaded: bool,
    pub ready: HashSet<PanelId>,
    pub expected: usize,
    pub lease: Option<crate::ManagedDesktopLease>,
    pub marker: std::path::PathBuf,
    pub next_scan: Instant,
    pub scanning: bool,
    pub reload_images: bool,
    pub shell_window: isize,
}

#[allow(clippy::too_many_lines)]
pub fn run(path: &Path, title: Option<String>) -> Result<(), String> {
    crate::recover_stale_shell_takeover(&crate::shell_takeover_marker_path(path))?;
    // Refuse takeover if the authoritative desktop inventory cannot be captured.
    let snapshot = native_desktop_snapshot()?;
    if std::env::var_os("LUCIDPANE_INSPECT").is_some() {
        eprintln!(
            "Desktop metrics: icon={}px spacing={:?} dpi={} count={}",
            snapshot.icon_size,
            snapshot.spacing,
            snapshot.dpi,
            snapshot.items.len()
        );
    }
    let monitors = enumerate_monitors();
    if std::env::var_os("LUCIDPANE_INSPECT").is_some() {
        let _ = std::fs::write(
            path.with_extension("metrics.txt"),
            format!("snapshot={snapshot:#?}\nmonitors={monitors:#?}"),
        );
    }
    if monitors.is_empty() {
        return Err("没有可用的显示器".into());
    }
    let mut store = WorkspaceStore::open(path).map_err(|e| e.to_string())?;
    let mut workspace = store.load_workspace().map_err(|e| e.to_string())?;
    if workspace.panels().is_empty() {
        let mut panel = Panel::new(
            PanelId::new(1),
            title.unwrap_or_else(|| "新建分组".into()),
            PanelSource::DesktopCollection,
            RectDip::new(360.0, 140.0, 420.0, 340.0),
        );
        panel.set_backdrop(desktop_core::Backdrop::Acrylic);
        workspace.add_panel(panel).map_err(|e| e.to_string())?;
    }
    for id in workspace.panels().iter().map(Panel::id).collect::<Vec<_>>() {
        let panel = workspace.panel_mut(id).unwrap();
        panel.set_rect(visible_panel(panel.rect(), &monitors));
    }
    let dpi = snapshot.dpi as f32 / 96.0;
    let icon_size = (snapshot.icon_size as f32).clamp(16.0, 256.0);
    let spacing = (
        (snapshot.spacing.0 as f32 / dpi).max(icon_size + 16.0),
        (snapshot.spacing.1 as f32 / dpi).max(icon_size + 34.0),
    );
    let monitors: Vec<_> = monitors
        .into_iter()
        .enumerate()
        .map(|(i, m)| (PanelId::new(u64::MAX - i as u64), m))
        .collect();
    let expected = monitors.len() + workspace.panels().len();
    let session = Session {
        monitors,
        icon_size,
        spacing,
        snapshot,
        loaded: false,
        ready: HashSet::new(),
        expected,
        lease: None,
        marker: crate::shell_takeover_marker_path(path),
        next_scan: Instant::now(),
        scanning: false,
        reload_images: false,
        shell_window: unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetShellWindow() }
            as isize,
    };
    reconcile_inventory(&mut workspace, &session);
    store
        .save_workspace(&workspace)
        .map_err(|e| e.to_string())?;
    let (_, receiver) = mpsc::channel();
    let state = Rc::new(RefCell::new(Preview {
        settings: None,
        workspace,
        store,
        views: Vec::new(),
        images: HashMap::new(),
        receiver,
        desktop: Some(session),
    }));
    let displays = state.borrow().desktop.as_ref().unwrap().monitors.clone();
    for (id, monitor) in displays {
        let (icon_size, spacing) = {
            let s = state.borrow();
            let d = s.desktop.as_ref().unwrap();
            (d.icon_size, d.spacing)
        };
        let model = Rc::new(RefCell::new(GroupModel {
            theme: desktop_core::PanelTheme::Dark, dark: true,
            desktop: true,
            managed: true,
            spacing,
            hovered_item: None,
            focused: false,
            auto_hide: false,
            reveal: 1.0,
            hovered_button: None,
            backdrop: desktop_core::Backdrop::Acrylic,
            native_material: true,
            title: format!("桌面 · {}", monitor.id.as_str()),
            items: items_for(&state.borrow(), id),
            icon_size,
            selected: None,
            renaming: None,
            scroll: 0,
            collapsed: false,
            loading: true,
        }));
        let weak = Rc::downgrade(&state);
        // Desktop window bounds are physical screen pixels, unlike pane geometry.
        let r = monitor.work_area;
        let window = window::create(
            RectDip::new(r.x as f32, r.y as f32, r.width as f32, r.height as f32),
            Rc::clone(&model),
            move |event| {
                let Some(state) = weak.upgrade() else {
                    return false;
                };
                match handle(&state, id, event) {
                    Ok(done) => done,
                    Err(error) => {
                        window::error(&error);
                        windows_window::quit();
                        false
                    }
                }
            },
        )?;
        state.borrow_mut().views.push(View { id, window, model });
    }
    let ids: Vec<_> = state
        .borrow()
        .workspace
        .panels()
        .iter()
        .map(Panel::id)
        .collect();
    for id in ids {
        create_view(&state, id)?;
    }
    scan(&mut state.borrow_mut());
    windows_window::run();
    // Restore Explorer before dropping the replacement windows.
    state.borrow_mut().desktop.as_mut().unwrap().lease.take();
    Ok(())
}

fn visible_panel(mut rect: RectDip, monitors: &[MonitorDescriptor]) -> RectDip {
    let center = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    let monitor = monitors
        .iter()
        .find(|m| {
            let scale = m.dpi as f32 / 96.0;
            center.0 >= m.bounds.x as f32 / scale
                && center.0 < (m.bounds.x + m.bounds.width) as f32 / scale
                && center.1 >= m.bounds.y as f32 / scale
                && center.1 < (m.bounds.y + m.bounds.height) as f32 / scale
        })
        .unwrap_or_else(|| monitors.iter().find(|m| m.primary).unwrap_or(&monitors[0]));
    let scale = monitor.dpi as f32 / 96.0;
    let area = monitor.work_area;
    rect.width = rect.width.min(area.width as f32 / scale);
    rect.height = rect.height.min(area.height as f32 / scale);
    rect.x = rect.x.clamp(
        area.x as f32 / scale,
        (area.x + area.width) as f32 / scale - rect.width,
    );
    rect.y = rect.y.clamp(
        area.y as f32 / scale,
        (area.y + area.height) as f32 / scale - rect.height,
    );
    rect
}

pub(super) fn items(state: &Preview, id: PanelId) -> Option<Vec<Item>> {
    let session = state.desktop.as_ref()?;
    let (_, monitor) = session.monitors.iter().find(|(key, _)| *key == id)?;
    Some(
        state
            .workspace
            .desktop_items()
            .iter()
            .filter_map(|item| {
                let DesktopPlacement::FreeDesktop {
                    monitor: owner,
                    position,
                } = item.placement()
                else {
                    return None;
                };
                (owner == &monitor.id).then(|| Item {
                    identity: item.identity().clone(),
                    label: item.display_name().into(),
                    image: state.images.get(&item.identity().persistent_key()).cloned(),
                    position: *position,
                })
            })
            .collect(),
    )
}

#[allow(clippy::too_many_lines)]
pub(super) fn reconcile_inventory(workspace: &mut Workspace, session: &Session) {
    let old: HashSet<_> = workspace
        .desktop_items()
        .iter()
        .map(|i| i.identity().persistent_key())
        .collect();
    workspace.reconcile_desktop_items(
        session
            .snapshot
            .items
            .iter()
            .map(|(item, _, _)| DesktopItem::new(item.identity.clone(), item.display_name.clone())),
    );
    let valid: HashSet<_> = workspace.panels().iter().map(Panel::id).collect();
    let mut placed = Vec::new();
    for item in workspace.desktop_items_mut() {
        if let DesktopPlacement::Pane { pane_id, .. } = item.placement()
            && valid.contains(pane_id)
        {
            continue;
        }
        let keep = old.contains(&item.identity().persistent_key())
            && matches!(item.placement(),
            DesktopPlacement::FreeDesktop { monitor,position } if session.monitors.iter().any(|(_,m)| &m.id == monitor
                && position.x>=0.0 && position.y>=0.0
                && position.x+session.spacing.0 <= m.work_area.width as f32*96.0/m.dpi as f32
                && position.y+session.spacing.1 <= m.work_area.height as f32*96.0/m.dpi as f32));
        if keep {
            continue;
        }
        let (_, x, y) = session
            .snapshot
            .items
            .iter()
            .find(|(source, _, _)| {
                source.identity.persistent_key() == item.identity().persistent_key()
            })
            .unwrap();
        let monitor = session
            .monitors
            .iter()
            .find(|(_, m)| {
                *x >= m.bounds.x
                    && *x < m.bounds.x + m.bounds.width
                    && *y >= m.bounds.y
                    && *y < m.bounds.y + m.bounds.height
            })
            .unwrap_or(&session.monitors[0])
            .1
            .clone();
        let scale = monitor.dpi as f32 / 96.0;
        placed.push(item.identity().clone());
        item.set_placement(DesktopPlacement::FreeDesktop {
            monitor: monitor.id,
            position: PointDip::new(
                ((*x - monitor.work_area.x) as f32 / scale).max(0.0),
                ((*y - monitor.work_area.y) as f32 / scale).max(0.0),
            ),
        });
    }
    for identity in placed {
        let item = workspace
            .desktop_items()
            .iter()
            .find(|i| i.identity() == &identity)
            .unwrap();
        let DesktopPlacement::FreeDesktop { monitor, position } = item.placement() else {
            continue;
        };
        let monitor = monitor.clone();
        let position = *position;
        let display = &session
            .monitors
            .iter()
            .find(|(_, m)| m.id == monitor)
            .unwrap()
            .1;
        let scale = display.dpi as f32 / 96.0;
        let size = (
            display.work_area.width as f32 / scale,
            display.work_area.height as f32 / scale,
        );
        let collides=workspace.desktop_items().iter().any(|i|i.identity()!=&identity && matches!(i.placement(),
            DesktopPlacement::FreeDesktop {monitor:m,position:p} if m==&monitor && (p.x-position.x).abs()<session.spacing.0*0.8
                && (p.y-position.y).abs()<session.spacing.1*0.8));
        if !collides
            && position.x + session.spacing.0 <= size.0
            && position.y + session.spacing.1 <= size.1
        {
            continue;
        }
        let placement = if let Some(position) = vacant_position(
            workspace,
            &monitor,
            &identity,
            position,
            session.spacing,
            size,
        ) {
            DesktopPlacement::FreeDesktop { monitor, position }
        } else {
            // Keep overflow reachable through the first scrollable pane.
            let pane_id = workspace.panels()[0].id();
            let order = workspace
                .desktop_items()
                .iter()
                .filter_map(|i| match i.placement() {
                    DesktopPlacement::Pane {
                        pane_id: p,
                        position,
                    } if *p == pane_id => Some(position.column),
                    _ => None,
                })
                .max()
                .map_or(0, |i| i.saturating_add(1));
            DesktopPlacement::Pane {
                pane_id,
                position: desktop_core::GridPosition::new(order, 0),
            }
        };
        workspace
            .desktop_item_mut(&identity)
            .unwrap()
            .set_placement(placement);
    }
}

pub(super) fn scan(state: &mut Preview) {
    let Some(session) = state.desktop.as_mut() else {
        return;
    };
    if session.scanning {
        return;
    }
    if session.lease.is_some()
        && session.shell_window
            != unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetShellWindow() } as isize
    {
        session.lease.take();
        windows_window::quit();
        return;
    }
    session.scanning = true;
    let reload_images = std::mem::take(&mut session.reload_images);
    session.next_scan = Instant::now() + Duration::from_secs(3);
    let (sender, receiver) = mpsc::channel();
    state.receiver = receiver;
    let known: HashSet<_> = state.images.keys().cloned().collect();
    let previous: HashMap<_, _> = session
        .snapshot
        .items
        .iter()
        .map(|(i, _, _)| {
            (
                i.identity.persistent_key(),
                (i.modified, i.display_name.clone()),
            )
        })
        .collect();
    let size = (session.icon_size
        * session
            .monitors
            .iter()
            .map(|(_, m)| m.dpi)
            .max()
            .unwrap_or(96) as f32
        / 96.0)
        .ceil() as i32;
    std::thread::spawn(move || {
        let _apartment = match ShellApartment::initialize_sta() {
            Ok(a) => a,
            Err(e) => {
                let _ = sender.send(Loaded::Desktop(Err(e.to_string())));
                return;
            }
        };
        let snapshot = match native_desktop_snapshot() {
            Ok(s) => s,
            Err(e) => {
                let _ = sender.send(Loaded::Desktop(Err(e)));
                return;
            }
        };
        let identities: Vec<_> = snapshot
            .items
            .iter()
            .filter(|(item, _, _)| {
                reload_images
                    || !known.contains(&item.identity.persistent_key())
                    || previous.get(&item.identity.persistent_key())
                        != Some(&(item.modified, item.display_name.clone()))
            })
            .map(|(item, _, _)| item.identity.clone())
            .collect();
        if sender.send(Loaded::Desktop(Ok(snapshot))).is_err() {
            return;
        }
        for identity in identities {
            match assets::load(&identity, size) {
                Ok(image) => {
                    if sender
                        .send(Loaded::Image(identity.persistent_key(), image))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Loaded::Desktop(Err(format!("读取桌面图标失败：{error}"))));
                }
            }
        }
    });
}

pub(super) fn release(
    state: &mut Preview,
    source: PanelId,
    index: usize,
    point: POINT,
) -> Result<bool, String> {
    let Some(session) = state.desktop.as_ref() else {
        return Ok(false);
    };
    let hwnd = unsafe { WindowFromPoint(point) };
    let target = state
        .views
        .iter()
        .find(|v| v.window.hwnd().cast::<std::ffi::c_void>() == hwnd && v.model.borrow().desktop)
        .map(|v| v.id);
    // Empty areas pass through the sparse region to Explorer. Never treat another app as desktop.
    let mut class = [0u16; 128];
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW(hwnd, class.as_mut_ptr(), 128);
    }
    let class =
        String::from_utf16_lossy(&class[..class.iter().position(|c| *c == 0).unwrap_or(128)]);
    if target.is_none()
        && !matches!(
            class.as_str(),
            "Progman" | "WorkerW" | "SHELLDLL_DefView" | "SysListView32"
        )
    {
        return Ok(false);
    }
    if target.is_none() {
        let root = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetAncestor(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::GA_ROOT,
            )
        };
        let mut root_class = [0u16; 64];
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW(
                root,
                root_class.as_mut_ptr(),
                64,
            );
        }
        let root_name = String::from_utf16_lossy(
            &root_class[..root_class.iter().position(|c| *c == 0).unwrap_or(64)],
        );
        if !matches!(root_name.as_str(), "Progman" | "WorkerW") {
            return Ok(false);
        }
    }
    let Some((_, monitor)) = session.monitors.iter().find(|(_, m)| {
        point.x >= m.work_area.x
            && point.x < m.work_area.x + m.work_area.width
            && point.y >= m.work_area.y
            && point.y < m.work_area.y + m.work_area.height
    }) else {
        return Ok(false);
    };
    let monitor = monitor.clone();
    let spacing = session.spacing;
    let icon_size = session.icon_size;
    let old = state.workspace.clone();
    let Some(item) = items_for(state, source).get(index).cloned() else {
        return Ok(false);
    };
    let scale = monitor.dpi as f32 / 96.0;
    let position = PointDip::new(
        ((point.x - monitor.work_area.x) as f32 / scale - spacing.0 / 2.0).max(0.0),
        ((point.y - monitor.work_area.y) as f32 / scale - icon_size / 2.0).max(0.0),
    );
    let Some(position) = vacant_position(
        &state.workspace,
        &monitor.id,
        &item.identity,
        position,
        spacing,
        (
            monitor.work_area.width as f32 / scale,
            monitor.work_area.height as f32 / scale,
        ),
    ) else {
        return Ok(false);
    };
    state
        .workspace
        .desktop_item_mut(&item.identity)
        .unwrap()
        .set_placement(DesktopPlacement::FreeDesktop {
            monitor: monitor.id,
            position,
        });
    if let Err(e) = save(state) {
        state.workspace = old;
        return Err(e);
    }
    Ok(true)
}

pub(super) fn release_panel(state: &mut Preview, id: PanelId) -> Result<(), String> {
    let Some(session) = &state.desktop else { return Ok(()); };
    let monitors = session.monitors.clone();
    let spacing = session.spacing;
    let identities: Vec<_> = state.workspace.desktop_items().iter()
        .filter(|item| matches!(item.placement(), DesktopPlacement::Pane { pane_id, .. } if *pane_id == id))
        .map(|item| item.identity().clone()).collect();
    for identity in identities {
        let target = monitors.iter().find_map(|(_, monitor)| {
            let scale = monitor.dpi as f32 / 96.0;
            vacant_position(&state.workspace, &monitor.id, &identity, PointDip::default(), spacing,
                (monitor.work_area.width as f32 / scale, monitor.work_area.height as f32 / scale))
                .map(|position| DesktopPlacement::FreeDesktop { monitor: monitor.id.clone(), position })
        }).ok_or("桌面空间不足，无法关闭分组")?;
        state.workspace.desktop_item_mut(&identity).unwrap().set_placement(target);
    }
    Ok(())
}

fn vacant_position(
    workspace: &Workspace,
    monitor: &MonitorId,
    moving: &desktop_core::ShellIdentity,
    desired: PointDip,
    spacing: (f32, f32),
    size: (f32, f32),
) -> Option<PointDip> {
    let columns = (size.0 / spacing.0).floor() as usize;
    let rows = (size.1 / spacing.1).floor() as usize;
    if columns == 0 || rows == 0 {
        return None;
    }
    let column = ((desired.x / spacing.0).round() as usize).min(columns - 1);
    let row = ((desired.y / spacing.1).round() as usize).min(rows - 1);
    for offset in 0..columns * rows {
        let at = (column * rows + row + offset) % (columns * rows);
        let p = PointDip::new(
            (at / rows) as f32 * spacing.0,
            (at % rows) as f32 * spacing.1,
        );
        if !workspace.desktop_items().iter().any(|item| item.identity()!=moving && matches!(item.placement(),DesktopPlacement::FreeDesktop { monitor:m,position }
            if m==monitor && (position.x-p.x).abs()<spacing.0*0.8 && (position.y-p.y).abs()<spacing.1*0.8)) { return Some(p); }
    }
    None
}

pub(super) fn ready(state: &mut Preview, id: PanelId) -> Result<(), String> {
    if std::env::var("LUCIDPANE_INSPECT").as_deref() == Ok("native") {
        return Ok(());
    }
    let Some(d) = state.desktop.as_mut() else {
        return Ok(());
    };
    if d.lease.is_some() {
        return Ok(());
    }
    d.ready.insert(id);
    if d.loaded && d.ready.len() >= d.expected {
        d.lease = Some(crate::ManagedDesktopLease::acquire(d.marker.clone())?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use desktop_core::{GridPosition, ShellIdentity};
    use desktop_shell::{DesktopShellItem, ShellAttributes};
    use desktop_window::PixelRect;

    fn source(name: &str, x: i32, y: i32) -> (DesktopShellItem, i32, i32) {
        (
            DesktopShellItem {
                identity: ShellIdentity::Namespace {
                    parsing_name: format!("test:{name}"),
                },
                display_name: name.into(),
                attributes: ShellAttributes::default(),
                modified: None,
                system_icon: None,
            },
            x,
            y,
        )
    }
    fn fixture() -> Preview {
        let mut workspace = Workspace::new();
        workspace
            .add_panel(Panel::new(
                PanelId::new(1),
                "Group",
                PanelSource::DesktopCollection,
                RectDip::default(),
            ))
            .unwrap();
        let bounds = PixelRect {
            x: 0,
            y: 0,
            width: 240,
            height: 300,
        };
        let session = Session {
            monitors: vec![(
                PanelId::new(u64::MAX),
                MonitorDescriptor {
                    id: MonitorId::new("screen"),
                    bounds,
                    work_area: bounds,
                    dpi: 96,
                    primary: true,
                },
            )],
            icon_size: 48.0,
            spacing: (80.0, 100.0),
            snapshot: NativeDesktopSnapshot {
                icon_size: 48,
                spacing: (80, 100),
                dpi: 96,
                items: vec![source("A", 0, 0), source("B", 0, 100), source("C", 80, 0)],
                view_indices: vec![0, 1, 2],
            },
            loaded: false,
            ready: HashSet::new(),
            expected: 2,
            lease: None,
            marker: Path::new("unused-test-marker").into(),
            next_scan: Instant::now(),
            scanning: false,
            reload_images: false,
            shell_window: 0,
        };
        reconcile_inventory(&mut workspace, &session);
        let (_, receiver) = mpsc::channel();
        Preview {
            settings: None,
                workspace,
            store: WorkspaceStore::open_in_memory().unwrap(),
            views: Vec::new(),
            images: HashMap::new(),
            receiver,
            desktop: Some(session),
        }
    }
    #[test]
    fn refresh_keeps_visible_images_and_queues_reload_during_an_active_scan() {
        let mut state = fixture();
        let image = std::sync::Arc::new(assets::Pixels {
            width: 1,
            height: 1,
            data: vec![255; 4],
        });
        state.images.insert("cached".into(), image.clone());
        state.desktop.as_mut().unwrap().scanning = true;
        let state = Rc::new(RefCell::new(state));
        handle(&state, PanelId::new(1), super::super::Event::Refresh).unwrap();
        let state = state.borrow();
        assert!(std::sync::Arc::ptr_eq(&state.images["cached"], &image));
        assert!(state.desktop.as_ref().unwrap().reload_images);
    }

    #[test]
    fn desktop_transfer_preserves_siblings_and_reloads_membership() {
        let mut state = fixture();
        let before = state.workspace.desktop_items().to_vec();
        super::super::transfer(&mut state, PanelId::new(u64::MAX), 0, PanelId::new(1), 0).unwrap();
        assert_eq!(&state.workspace.desktop_items()[1..], &before[1..]);
        state.workspace = state.store.load_workspace().unwrap();
        assert_eq!(items_for(&state, PanelId::new(u64::MAX)).len(), 2);
        assert_eq!(items_for(&state, PanelId::new(1))[0].label, "A");
        assert_eq!(
            state.workspace.desktop_items()[0].placement(),
            &DesktopPlacement::Pane {
                pane_id: PanelId::new(1),
                position: GridPosition::new(0, 0)
            }
        );
    }
    #[test]
    fn discovery_keeps_existing_positions_and_places_collisions_in_free_cells() {
        let mut state = fixture();
        let before = state.workspace.desktop_items().to_vec();
        let session = state.desktop.as_mut().unwrap();
        session.snapshot.items.push(source("D", 0, 0));
        reconcile_inventory(&mut state.workspace, session);
        assert_eq!(&state.workspace.desktop_items()[..3], &before);
        assert!(
            matches!(state.workspace.desktop_items()[3].placement(),DesktopPlacement::FreeDesktop {position,..} if *position==PointDip::new(0.0,200.0))
        );
    }
    #[test]
    fn full_desktop_keeps_overflow_in_a_scrollable_pane() {
        let mut state = fixture();
        state.workspace = Workspace::new();
        state
            .workspace
            .add_panel(Panel::new(
                PanelId::new(1),
                "Group",
                PanelSource::DesktopCollection,
                RectDip::default(),
            ))
            .unwrap();
        let session = state.desktop.as_mut().unwrap();
        session.monitors[0].1.work_area.width = 80;
        session.monitors[0].1.work_area.height = 100;
        reconcile_inventory(&mut state.workspace, session);
        assert_eq!(items_for(&state, PanelId::new(u64::MAX)).len(), 1);
        assert_eq!(items_for(&state, PanelId::new(1)).len(), 2);
    }
    #[test]
    fn moving_to_same_cell_ignores_self_and_full_target_rejects_move() {
        let state = fixture();
        let identity = state.workspace.desktop_items()[0].identity();
        assert_eq!(
            vacant_position(
                &state.workspace,
                &MonitorId::new("screen"),
                identity,
                PointDip::default(),
                (80.0, 100.0),
                (80.0, 100.0)
            ),
            Some(PointDip::default())
        );
        let outsider = ShellIdentity::Namespace {
            parsing_name: "test:other".into(),
        };
        assert_eq!(
            vacant_position(
                &state.workspace,
                &MonitorId::new("screen"),
                &outsider,
                PointDip::new(999.0, 999.0),
                (80.0, 100.0),
                (80.0, 100.0)
            ),
            None
        );
    }
    #[test]
    fn incomplete_startup_cannot_hide_explorer() {
        let mut state = fixture();
        ready(&mut state, PanelId::new(1)).unwrap();
        ready(&mut state, PanelId::new(u64::MAX)).unwrap();
        assert!(state.desktop.as_ref().unwrap().lease.is_none());
        assert_eq!(state.desktop.as_ref().unwrap().ready.len(), 2);
    }

    #[test]
    fn smaller_work_area_recovers_saved_icons_and_pane_bounds() {
        let mut state = fixture();
        let id = state.workspace.desktop_items()[0].identity().clone();
        state
            .workspace
            .desktop_item_mut(&id)
            .unwrap()
            .set_placement(DesktopPlacement::FreeDesktop {
                monitor: MonitorId::new("screen"),
                position: PointDip::new(900.0, 900.0),
            });
        reconcile_inventory(&mut state.workspace, state.desktop.as_ref().unwrap());
        assert_eq!(
            state.workspace.desktop_items()[0].placement(),
            &DesktopPlacement::FreeDesktop {
                monitor: MonitorId::new("screen"),
                position: PointDip::default()
            }
        );
        let monitor = state.desktop.as_ref().unwrap().monitors[0].1.clone();
        let rect = visible_panel(RectDip::new(1500.0, 1000.0, 400.0, 400.0), &[monitor]);
        assert_eq!(
            rect,
            RectDip {
                x: 0.0,
                y: 0.0,
                width: 240.0,
                height: 300.0
            }
        );
    }
}
