//! Native Explorer geometry with persistent pane membership. The icon layer belongs to Shell.
use desktop_core::{
    DesktopItem, DesktopPlacement, GridPosition, Panel, PanelId, PanelSource, RectDip,
    ShellIdentity, Workspace,
};
use desktop_hook::{
    HookSession, conflicting_desktop_extension, desktop_view,
    protocol::{Area, OK, QUERY, QUERY_SHELL_GENERATION, Request, partition},
};
use desktop_shell::{NativeDesktopSnapshot, native_desktop_snapshot};
use desktop_storage::WorkspaceStore;
use desktop_window::{NativeFrame, NativeFrameEvent, enumerate_monitors};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SW_HIDE, SetTimer, ShowWindow, WM_TIMER, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows_window::Window;

struct Desktop {
    workspace: Workspace,
    store: WorkspaceStore,
    frames: Vec<NativeFrame>,
    hook: HookSession,
    view: isize,
    failed: bool,
    generation: isize,
    mouse_down: bool,
    drag: Option<(ShellIdentity, POINT)>,
    last_refresh: Instant,
    layout: Option<LayoutCache>,
}

struct LayoutCache {
    snapshot: NativeDesktopSnapshot,
}

/// Shell enumeration and baseline IPC belong to inventory refresh, never a move frame.
fn refresh_layout(desktop: &mut Desktop) -> Result<(), String> {
    for _ in 0..3 {
        let generation = desktop
            .hook
            .request(&Request::new(QUERY_SHELL_GENERATION))?;
        let snapshot = native_desktop_snapshot()?;
        if desktop
            .hook
            .request(&Request::new(QUERY_SHELL_GENERATION))?
            != generation
        {
            continue;
        }
        capture(&mut desktop.workspace, &snapshot);
        desktop.layout = Some(LayoutCache { snapshot });
        desktop.generation = generation;
        return Ok(());
    }
    Err("桌面项目正在变化，请稍后重试。".into())
}

pub fn run(path: &Path, title: Option<String>) -> Result<(), String> {
    if conflicting_desktop_extension() {
        return Err("检测到 Explorer 已加载 Fences 的桌面管理组件。请先退出 Fences，再启动 LucidPane 原生 Hook 模式；两个布局管理器不能同时接管桌面。".into());
    }
    if desktop_shell::desktop_icons_hidden() {
        return Err("原生桌面图标当前被隐藏。请先退出旧版 LucidPane 或在桌面菜单开启“显示桌面图标”，再启动 Hook 模式。".into());
    }
    let view = desktop_view()?;
    let store = WorkspaceStore::open(path).map_err(|e| e.to_string())?;
    let mut workspace = store.load_workspace().map_err(|e| e.to_string())?;
    if workspace.panels().is_empty() {
        workspace
            .add_panel(Panel::new(
                PanelId::new(1),
                title.unwrap_or_else(|| "原生分组".into()),
                PanelSource::DesktopCollection,
                initial_rect()?,
            ))
            .map_err(|e| e.to_string())?;
    } else if let Some(title) = title {
        let first = workspace.panels()[0].id();
        workspace.panel_mut(first).unwrap().set_title(title);
    }
    let holder: Rc<RefCell<Option<std::rc::Weak<RefCell<Desktop>>>>> = Rc::new(RefCell::new(None));
    let for_tick = Rc::clone(&holder);
    let controller = Window::new("LucidPane Native Hook Controller")
        .size(1, 1)
        .style(WS_POPUP)
        .ex_style(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)
        .on_message(move |_, message, _, _| {
            if message != WM_TIMER {
                return None;
            }
            let Some(state) = for_tick.borrow().as_ref().and_then(std::rc::Weak::upgrade) else {
                return Some(0);
            };
            let Ok(mut desktop) = state.try_borrow_mut() else {
                return Some(0);
            };
            if desktop.failed {
                return Some(0);
            }
            if let Err(error) = poll_drag(&mut desktop) {
                NativeFrame::show_error(&error);
            }
            if desktop.last_refresh.elapsed() < Duration::from_secs(2) || desktop.mouse_down {
                return Some(0);
            }
            desktop.last_refresh = Instant::now();
            let health = desktop.hook.request(&Request::new(QUERY));
            if health != Ok(OK) {
                eprintln!("Hook session health failed: {health:?}");
                desktop.failed = true;
                NativeFrame::quit();
                return Some(0);
            }
            let Ok(generation) = desktop.hook.request(&Request::new(QUERY_SHELL_GENERATION)) else {
                return Some(0);
            };
            if generation == desktop.generation {
                return Some(0);
            }
            // Persist actual Shell positions, including native drag/drop and renames.
            // Icon extraction is not performed by native_desktop_snapshot.
            if refresh_layout(&mut desktop).is_ok() {
                if let Err(error) = apply(&desktop) {
                    eprintln!("Hook inventory layout failed: {error}");
                    desktop.failed = true;
                    NativeFrame::quit();
                    return Some(0);
                }
                let Desktop {
                    workspace, store, ..
                } = &mut *desktop;
                let _ = store.save_workspace(workspace);
            }
            Some(0)
        })
        .create()
        .map_err(|e| e.to_string())?;
    unsafe {
        ShowWindow(controller.hwnd().cast(), SW_HIDE);
    }
    let dll = runtime_dll(path)?;
    let hook = HookSession::connect_geometry(view, controller.hwnd() as isize, &dll)?;
    let (texture, pixels) = crate::hook_material::capture(origin(view))?;
    hook.set_texture(texture, &pixels)?;
    let state = Rc::new(RefCell::new(Desktop {
        workspace,
        store,
        frames: Vec::new(),
        hook,
        view,
        failed: false,
        generation: -1,
        mouse_down: false,
        drag: None,
        last_refresh: Instant::now(),
        layout: None,
    }));
    *holder.borrow_mut() = Some(Rc::downgrade(&state));
    {
        let mut desktop = state.borrow_mut();
        refresh_layout(&mut desktop)?;
        apply(&desktop)?;
        let Desktop {
            workspace, store, ..
        } = &mut *desktop;
        store.save_workspace(workspace).map_err(|e| e.to_string())?;
    }
    let ids: Vec<_> = state
        .borrow()
        .workspace
        .panels()
        .iter()
        .map(Panel::id)
        .collect();
    for id in ids {
        frame(&state, id)?;
    }
    unsafe {
        SetTimer(controller.hwnd().cast(), 1, 25, None);
    }
    NativeFrame::run();
    let failed = state.borrow().failed;
    // State (and HookSession) is dropped before the controller HWND.
    drop(state);
    drop(controller);
    if failed {
        Err("Explorer 视图或显示器配置发生变化，Hook 会话已退出。请重新启动原生模式。".into())
    } else {
        Ok(())
    }
}

fn initial_rect() -> Result<RectDip, String> {
    let monitors = enumerate_monitors();
    let work = monitors.first().ok_or("没有可用的显示器")?.work_area;
    Ok(RectDip::new(
        (work.x + work.width - 460).max(work.x) as f32,
        (work.y + 80) as f32,
        420.0,
        360.0,
    ))
}

use crate::hook_runtime::runtime_dll;

fn area(rect: RectDip, origin: POINT) -> Area {
    Area {
        left: rect.x.round() as i32 - origin.x,
        top: rect.y.round() as i32 - origin.y,
        right: (rect.x + rect.width).round() as i32 - origin.x,
        bottom: (rect.y + rect.height).round() as i32 - origin.y,
    }
}

fn origin(view: isize) -> POINT {
    let mut point = POINT::default();
    unsafe {
        ClientToScreen(view as _, &raw mut point);
    }
    point
}

fn apply(desktop: &Desktop) -> Result<(), String> {
    let started = Instant::now();
    let origin = origin(desktop.view);
    let monitors: Vec<_> = enumerate_monitors()
        .iter()
        .map(|m| Area {
            left: m.work_area.x - origin.x,
            top: m.work_area.y - origin.y,
            right: m.work_area.x + m.work_area.width - origin.x,
            bottom: m.work_area.y + m.work_area.height - origin.y,
        })
        .collect();
    let panes: Vec<_> = desktop
        .workspace
        .panels()
        .iter()
        .map(|p| area(p.rect(), origin))
        .collect();
    let mut areas = partition(&monitors, &panes)?;
    let start = areas.len() - panes.len();
    for (i, p) in desktop.workspace.panels().iter().enumerate() {
        areas[start + i] = area(NativeFrame::content_bounds(p.rect()), origin);
    }
    let cache = desktop.layout.as_ref().ok_or("桌面布局尚未初始化")?;
    let snapshot = &cache.snapshot;
    let cell_width = snapshot.spacing.0.max(snapshot.icon_size + 24);
    let cell_height = snapshot.spacing.1.max(snapshot.icon_size + 48);
    let mut next_slot = std::collections::BTreeMap::<u64, usize>::new();
    let appearances: Vec<_> = desktop
        .workspace
        .panels()
        .iter()
        .map(|panel| {
            let mut appearance = desktop_hook::protocol::PaneAppearance {
                bounds: area(panel.rect(), origin),
                material: u32::from(panel.backdrop() == desktop_core::Backdrop::Mica),
                ..Default::default()
            };
            for (to, from) in appearance.title[..95]
                .iter_mut()
                .zip(panel.title().encode_utf16())
            {
                *to = from;
            }
            appearance
        })
        .collect();
    let mut positions = Vec::new();
    for saved in desktop.workspace.desktop_items() {
        let DesktopPlacement::Pane { pane_id, .. } = saved.placement() else {
            continue;
        };
        let Some(panel) = desktop.workspace.panel(*pane_id) else {
            continue;
        };
        let Some(i) = snapshot
            .items
            .iter()
            .position(|(item, _, _)| item.identity.equivalent_to(saved.identity()))
        else {
            continue;
        };
        let target = area(NativeFrame::content_bounds(panel.rect()), origin);
        let slot = next_slot.entry(panel.id().get()).or_default();
        let (x, y) = pane_slot(target, (cell_width, cell_height), *slot)
            .ok_or_else(|| format!("“{}”空间不足，请扩大 pane 后再拖入图标。", panel.title()))?;
        *slot += 1;
        positions.push((
            snapshot.view_indices[i],
            x,
            y,
            snapshot.items[i].0.display_name.clone(),
            desktop
                .workspace
                .panels()
                .iter()
                .position(|p| p.id() == panel.id())
                .unwrap() as u32
                + 1,
        ));
    }
    let submitted = Instant::now();
    desktop
        .hook
        .apply_pane_layout(&areas, &positions, &appearances)?;
    record_layout_timing(started.elapsed(), submitted.elapsed());
    Ok(())
}

fn record_layout_timing(total: Duration, native: Duration) {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*TRACE.get_or_init(|| std::env::var_os("LUCIDPANE_TRACE_LAYOUT").is_some()) {
        return;
    }
    thread_local! { static SAMPLES: RefCell<Vec<(Duration, Duration)>> = const { RefCell::new(Vec::new()) }; }
    SAMPLES.with(|samples| {
        let mut samples = samples.borrow_mut();
        samples.push((total, native));
        if samples.len() < 60 { return; }
        let mut times: Vec<_> = samples.iter().map(|(t,_)| t.as_secs_f64()*1000.0).collect();
        times.sort_by(f64::total_cmp);
        let native_ms = samples.iter().map(|(_,n)| n.as_secs_f64()*1000.0).sum::<f64>() / 60.0;
        eprintln!("Hook layout: samples=60 median={:.2}ms p95={:.2}ms max={:.2}ms native_mean={native_ms:.2}ms", times[30], times[56], times[59]);
        samples.clear();
    });
}

fn pane_slot(bounds: Area, spacing: (i32, i32), index: usize) -> Option<(i32, i32)> {
    if spacing.0 <= 0 || spacing.1 <= 0 {
        return None;
    }
    let columns = (bounds.right - bounds.left - 16) / spacing.0;
    let rows = (bounds.bottom - bounds.top - 16) / spacing.1;
    if columns <= 0 || rows <= 0 || index >= usize::try_from(columns * rows).ok()? {
        return None;
    }
    let index = i32::try_from(index).ok()?;
    Some((
        bounds.left + 8 + (index % columns) * spacing.0,
        bounds.top + 8 + (index / columns) * spacing.1,
    ))
}

fn capture(workspace: &mut Workspace, snapshot: &NativeDesktopSnapshot) {
    workspace.reconcile_desktop_items(
        snapshot
            .items
            .iter()
            .map(|(item, _, _)| DesktopItem::new(item.identity.clone(), item.display_name.clone())),
    );
    let panels: Vec<_> = workspace.panels().iter().map(Panel::id).collect();
    for item in workspace.desktop_items_mut() {
        if matches!(item.placement(),DesktopPlacement::Pane {pane_id,..} if !panels.contains(pane_id))
        {
            item.set_placement(DesktopPlacement::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pane_grid_reflows_without_overlap_and_rejects_overflow() {
        let bounds = Area {
            left: 0,
            top: 0,
            right: 352,
            bottom: 216,
        };
        assert_eq!(pane_slot(bounds, (100, 100), 0), Some((8, 8)));
        assert_eq!(pane_slot(bounds, (100, 100), 2), Some((208, 8)));
        assert_eq!(pane_slot(bounds, (100, 100), 3), Some((8, 108)));
        assert_eq!(pane_slot(bounds, (100, 100), 5), Some((208, 108)));
        assert_eq!(pane_slot(bounds, (100, 100), 6), None);
        let narrow = Area {
            right: 216,
            ..bounds
        };
        assert_eq!(pane_slot(narrow, (100, 100), 2), Some((8, 108)));
        assert_eq!(
            pane_slot(
                Area {
                    right: 80,
                    ..bounds
                },
                (100, 100),
                0
            ),
            None
        );
    }
}

fn poll_drag(desktop: &mut Desktop) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, WindowFromPoint};
    let down = unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON)) } < 0;
    if unsafe { GetAsyncKeyState(i32::from(VK_ESCAPE)) } < 0 {
        desktop.drag = None;
    }
    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&raw mut cursor) } == 0 {
        return Ok(());
    }
    let surface = unsafe { WindowFromPoint(cursor) };
    if down && !desktop.mouse_down && surface == desktop.view as _ {
        if desktop
            .hook
            .request(&Request::new(QUERY_SHELL_GENERATION))?
            != desktop.generation
        {
            refresh_layout(desktop)?;
        }
        let origin = origin(desktop.view);
        let mut query = Request::new(desktop_hook::protocol::QUERY_HIT);
        query.x = cursor.x - origin.x;
        query.y = cursor.y - origin.y;
        let index = desktop.hook.request(&query)? - 1;
        if index >= 0 {
            let snapshot = &desktop
                .layout
                .as_ref()
                .ok_or("桌面布局尚未初始化")?
                .snapshot;
            if let Some(i) = snapshot
                .view_indices
                .iter()
                .position(|&i| i as isize == index)
            {
                desktop.drag = Some((snapshot.items[i].0.identity.clone(), cursor));
            }
        }
    }
    let released = !down && desktop.mouse_down;
    desktop.mouse_down = down;
    if !released {
        return Ok(());
    }
    let Some((identity, start)) = desktop.drag.take() else {
        return Ok(());
    };
    if cursor.x.abs_diff(start.x) < 8 && cursor.y.abs_diff(start.y) < 8 {
        return Ok(());
    }
    if surface != desktop.view as _ && !desktop.frames.iter().any(|f| f.hwnd() == surface) {
        return Ok(());
    }
    let pane = desktop
        .workspace
        .panels()
        .iter()
        .find(|p| {
            area(NativeFrame::content_bounds(p.rect()), POINT::default())
                .contains(cursor.x, cursor.y)
        })
        .map(Panel::id);
    let previous = desktop.workspace.clone();
    let Some(item) = desktop
        .workspace
        .desktop_items_mut()
        .iter_mut()
        .find(|i| i.identity().equivalent_to(&identity))
    else {
        return Ok(());
    };
    item.set_placement(pane.map_or_else(DesktopPlacement::default, |pane_id| {
        DesktopPlacement::Pane {
            pane_id,
            position: GridPosition::default(),
        }
    }));
    if let Err(error) = apply(desktop) {
        desktop.workspace = previous;
        if let Err(restore) = apply(desktop) {
            eprintln!("Hook drop rollback failed: {restore}; original error: {error}");
            desktop.failed = true;
            NativeFrame::quit();
        }
        return Err(error);
    }
    desktop
        .store
        .save_workspace(&desktop.workspace)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn frame(state: &Rc<RefCell<Desktop>>, id: PanelId) -> Result<(), String> {
    let panel = state
        .borrow()
        .workspace
        .panel(id)
        .cloned()
        .ok_or("分组不存在")?;
    let weak = Rc::downgrade(state);
    let window = NativeFrame::new_hook(panel.title().into(), panel.rect(), move |event| {
        let preview = matches!(event, NativeFrameEvent::GeometryPreview { .. });
        let Some(state) = weak.upgrade() else {
            return false;
        };
        match event_handler(&state, id, event) {
            Ok(()) => true,
            Err(error) => {
                // Reject impossible live proposals without a modal dialog in the move loop.
                if !preview {
                    NativeFrame::show_error(&error);
                }
                false
            }
        }
    })?;
    state.borrow_mut().frames.push(window);
    Ok(())
}

fn event_handler(
    state: &Rc<RefCell<Desktop>>,
    id: PanelId,
    event: NativeFrameEvent,
) -> Result<(), String> {
    if let NativeFrameEvent::MenuRequested { owner, x, y } = event {
        let material = state
            .borrow()
            .workspace
            .panel(id)
            .ok_or("分组不存在")?
            .backdrop();
        let command = crate::preview::menu::show_hook(owner as _, POINT { x, y }, material);
        let event = match command {
            1 => NativeFrameEvent::NewFrame,
            4 => NativeFrameEvent::Exit,
            5 => NativeFrameEvent::MaterialChanged(desktop_core::Backdrop::Acrylic),
            6 => NativeFrameEvent::MaterialChanged(desktop_core::Backdrop::Mica),
            11 => NativeFrameEvent::RemoveFrame,
            10 => {
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                        owner as _,
                        windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F2 as usize,
                        0,
                    );
                }
                return Ok(());
            }
            _ => return Ok(()),
        };
        event_handler(state, id, event)?;
        if command == 11 {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(owner as _);
            }
        }
        return Ok(());
    }
    if matches!(event, NativeFrameEvent::Exit) {
        NativeFrame::quit();
        return Ok(());
    }
    let mut desktop = state.try_borrow_mut().map_err(|_| "布局更新正在进行")?;
    let preview = matches!(event, NativeFrameEvent::GeometryPreview { .. });
    desktop.last_refresh = Instant::now();
    let previous = desktop.workspace.clone();
    let mut new_id = None;
    match event {
        NativeFrameEvent::MaterialChanged(material) => desktop
            .workspace
            .panel_mut(id)
            .ok_or("分组不存在")?
            .set_backdrop(material),
        NativeFrameEvent::MenuRequested { .. } => unreachable!(),
        NativeFrameEvent::GeometryPreview { current }
        | NativeFrameEvent::GeometryChanged { current, .. } => {
            desktop
                .workspace
                .panel_mut(id)
                .ok_or("分组不存在")?
                .set_rect(current);
        }
        NativeFrameEvent::TitleChanged(title) => {
            desktop
                .workspace
                .panel_mut(id)
                .ok_or("分组不存在")?
                .set_title(title);
        }
        NativeFrameEvent::NewFrame => {
            let next = PanelId::new(
                desktop
                    .workspace
                    .panels()
                    .iter()
                    .map(|p| p.id().get())
                    .max()
                    .unwrap_or(0)
                    + 1,
            );
            let mut rect = desktop.workspace.panel(id).ok_or("分组不存在")?.rect();
            rect.x -= rect.width + 16.0;
            desktop
                .workspace
                .add_panel(Panel::new(
                    next,
                    "原生分组",
                    PanelSource::DesktopCollection,
                    rect,
                ))
                .map_err(|e| e.to_string())?;
            new_id = Some(next);
        }
        NativeFrameEvent::RemoveFrame => {
            desktop.workspace.remove_panel(id);
            for item in desktop.workspace.desktop_items_mut() {
                if matches!(item.placement(),DesktopPlacement::Pane {pane_id,..} if *pane_id==id) {
                    item.set_placement(DesktopPlacement::default());
                }
            }
        }
        NativeFrameEvent::Exit => unreachable!(),
    }
    if let Err(error) = apply(&desktop) {
        desktop.workspace = previous;
        if let Err(restore) = apply(&desktop) {
            eprintln!("Hook frame rollback failed: {restore}; original error: {error}");
            desktop.failed = true;
            NativeFrame::quit();
            return Err(format!("{error}；恢复失败，已退出 Hook：{restore}"));
        }
        return Err(error);
    }
    if preview {
        return Ok(());
    }
    let Desktop {
        workspace, store, ..
    } = &mut *desktop;
    store.save_workspace(workspace).map_err(|e| e.to_string())?;
    if desktop.workspace.panels().is_empty() {
        NativeFrame::quit();
    }
    drop(desktop);
    if let Some(id) = new_id {
        frame(state, id)?;
    }
    Ok(())
}
