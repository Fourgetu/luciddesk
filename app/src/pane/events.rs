//! Group commands and their persisted state transitions.
use super::*;

#[allow(clippy::too_many_lines)]
pub(super) fn handle(
    state: &Rc<RefCell<PaneApp>>,
    id: PanelId,
    event: Event,
) -> Result<bool, String> {
    if matches!(event, Event::Settings) {
        settings::show(state, id)?;
        return Ok(false);
    }
    if matches!(event, Event::PanelTheme(_) | Event::PanelMaterial(_)) {
        let mut s = state.borrow_mut();
        let old = s.workspace.clone();
        let Some(panel) = s.workspace.panel_mut(id) else {
            return Ok(false);
        };
        match event {
            Event::PanelTheme(value) => panel.set_theme(value),
            Event::PanelMaterial(value) => panel.set_backdrop(value),
            _ => unreachable!(),
        }
        let (theme, backdrop) = (panel.theme(), panel.backdrop());
        if let Err(error) = save(&mut s) {
            s.workspace = old;
            return Err(error);
        }
        if let Some(view) = s.views.iter().find(|view| view.id == id) {
            let mut model = view.model.borrow_mut();
            model.theme = theme;
            model.dark = self::theme::is_dark(theme);
            model.backdrop = backdrop;
            unsafe {
                InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0);
            }
        }
        return Ok(false);
    }
    if matches!(event, Event::Theme(_) | Event::Material(_)) {
        let mut s = state.borrow_mut();
        let old = s.workspace.clone();
        let (mut theme, mut backdrop) = s
            .workspace
            .appearance()
            .or_else(|| {
                s.workspace
                    .panels()
                    .first()
                    .map(|p| (p.theme(), p.backdrop()))
            })
            .unwrap_or((
                desktop_core::PanelTheme::System,
                desktop_core::Backdrop::Mica,
            ));
        match event {
            Event::Theme(value) => theme = value,
            Event::Material(value) => backdrop = value,
            _ => unreachable!(),
        }
        s.workspace.set_appearance(theme, backdrop);
        if let Err(error) = save(&mut s) {
            s.workspace = old;
            return Err(error);
        }
        for view in &s.views {
            {
                let mut model = view.model.borrow_mut();
                model.theme = theme;
                model.dark = self::theme::is_dark(theme);
                model.backdrop = backdrop;
            }
            unsafe {
                InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0);
            }
        }
        if let Some(window) = &s.settings {
            unsafe {
                InvalidateRect(window.hwnd().cast(), std::ptr::null(), 0);
            }
        }
        return Ok(false);
    }
    if matches!(event, Event::ClosePane) {
        let mut s = state.borrow_mut();
        if s.workspace.panel(id).is_none() {
            return Ok(false);
        }
        let old = s.workspace.clone();
        remove_panel(&mut s.workspace, id);
        if let Err(error) = save(&mut s) {
            s.workspace = old;
            hybrid::sync(&mut s)?;
            return Err(error);
        }
        let view = s
            .views
            .iter()
            .position(|view| view.id == id)
            .map(|at| s.views.remove(at));
        if let Some(view) = &view {
            let hwnd = view.window.hwnd().cast();
            window::prepare_close(hwnd);
            hybrid::unregister_drop(&mut s, hwnd);
        }
        refresh_views(&mut s);
        drop(s);
        drop(view);
        return Ok(false);
    }
    if matches!(event, Event::RenameTitle) {
        let target = state
            .borrow()
            .views
            .iter()
            .find(|v| v.id == id)
            .map(|v| (v.window.hwnd().cast(), v.model.clone()));
        if let Some((owner, model)) = target {
            let state = Rc::clone(state);
            rename::show_title(
                owner,
                model,
                Box::new(move |title| handle(&state, id, Event::SetTitle(title)).map(|_| ())),
            )?;
        }
        return Ok(false);
    }
    if let Event::PaneItemFocus = event {
        hybrid::clear_desktop_selection(&state.borrow())?;
        return Ok(false);
    }
    if let Event::MenuSelection(allow) = event {
        hybrid::menu(&state.borrow(), allow)?;
        return Ok(false);
    }
    if let Event::ItemMenuEnded(identity) = event {
        hybrid::invalidate_icon(&state.borrow(), &identity);
        let requested = hybrid::menu(&state.borrow(), false)?;
        if requested {
            let target = {
                let s = state.borrow();
                s.views.iter().find(|v| v.id == id).and_then(|v| {
                    let model = v.model.borrow();
                    model
                        .items
                        .iter()
                        .find(|item| item.identity == identity)
                        .map(|item| {
                            (
                                v.window.hwnd().cast(),
                                item.identity.clone(),
                                item.label.clone(),
                                v.model.clone(),
                            )
                        })
                })
            };
            if let Some((owner, identity, title, model)) = target {
                rename::show(owner, &identity, &title, model)?;
            }
        }
        return Ok(false);
    }
    if let Event::RenameItem(identity) = event {
        hybrid::clear_desktop_selection(&state.borrow())?;
        let target = {
            let s = state.borrow();
            s.views.iter().find(|v| v.id == id).and_then(|v| {
                let model = v.model.borrow();
                model
                    .items
                    .iter()
                    .find(|i| i.identity == identity)
                    .map(|i| (v.window.hwnd().cast(), i.label.clone(), v.model.clone()))
            })
        };
        if let Some((owner, label, model)) = target {
            rename::show(owner, &identity, &label, model)?;
        }
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
            if s.workspace.appearance().is_none() {
                s.workspace
                    .panel_mut(next)
                    .unwrap()
                    .set_backdrop(desktop_core::Backdrop::Acrylic);
            }
            save(&mut s)?;
            next
        };
        create_view(state, next)?;
        return Ok(false);
    }
    let mut s = state.borrow_mut();
    match event {
        Event::RenameTitle | Event::ClosePane | Event::Settings => {
            unreachable!("Handled before borrowing PaneApp")
        }
        Event::SetTitle(title) => {
            let old = s.workspace.clone();
            s.workspace
                .panel_mut(id)
                .ok_or("分组不存在")?
                .set_title(title.clone());
            if let Err(error) = save(&mut s) {
                s.workspace = old;
                return Err(error);
            }
            if let Some(view) = s.views.iter().find(|view| view.id == id) {
                view.model.borrow_mut().title = title;
                unsafe {
                    InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0);
                }
            }
            refresh_views(&mut s);
        }
        Event::PaneItemFocus
        | Event::MenuSelection(_)
        | Event::ItemMenuEnded(_)
        | Event::RenameItem(_) => unreachable!("Handled before borrowing PaneApp"),
        Event::Theme(_) | Event::Material(_) | Event::PanelTheme(_) | Event::PanelMaterial(_) => {
            unreachable!("Handled before borrowing PaneApp")
        }
        Event::ToggleTopmost => {
            let panel = s.workspace.panel_mut(id).ok_or("分组不存在")?;
            let enabled = !panel.always_on_top();
            panel.set_always_on_top(enabled);
            if let Err(error) = save(&mut s) {
                s.workspace
                    .panel_mut(id)
                    .unwrap()
                    .set_always_on_top(!enabled);
                return Err(error);
            }
            if let Some(view) = s.views.iter().find(|v| v.id == id) {
                window::set_layer(view.window.hwnd().cast(), enabled);
            }
        }
        Event::Refresh => hybrid::refresh_icons(&mut s),
        Event::Moving(rect) => {
            let peers: Vec<_> = s
                .views
                .iter()
                .filter(|view| view.id != id)
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
                    use windows_sys::Win32::Graphics::Gdi::{
                        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
                    };
                    let monitor = MonitorFromRect(rect, MONITOR_DEFAULTTONEAREST);
                    let mut info = MONITORINFO {
                        cbSize: size_of::<MONITORINFO>() as u32,
                        ..Default::default()
                    };
                    let work =
                        (GetMonitorInfoW(monitor, &raw mut info) != 0).then_some(info.rcWork);
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
            if s.session.is_some() {
                hybrid::tick(&mut s)?;
            }
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
            } else if hybrid::release(&mut s, id, index, point)? {
                refresh_views(&mut s);
            }
        }
        Event::Exit => windows_window::quit(),
        Event::New => unreachable!(),
    }
    Ok(false)
}
