//! One native window per tab group; content IDs and worker channels never change.
use super::*;
use desktop_core::PaneTabs;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// A mixed group remains available when Explorer reconnects.
pub(super) fn independent(workspace: &Workspace, id: PanelId) -> bool {
    workspace
        .panel(id)
        .is_some_and(|p| p.folder().is_some() || p.is_search())
        || workspace.tab_group(id).is_some_and(|group| {
            group
                .members
                .iter()
                .any(|id| workspace.panel(*id).is_some_and(|p| p.folder().is_some()))
        })
}

pub(super) fn decorate(workspace: &Workspace, id: PanelId, model: &mut GroupModel) {
    model.active_tab = id;
    let tabs = workspace
        .tab_group(id)
        .map(|group| {
            group
                .members
                .iter()
                .filter_map(|id| workspace.panel(*id).map(|p| (*id, p.title().to_owned())))
                .collect()
        })
        .unwrap_or_default();
    model.tabs = tabs;
}

pub(super) fn select(
    state: &Rc<RefCell<PaneApp>>,
    from: PanelId,
    to: PanelId,
) -> Result<(), String> {
    if from == to {
        return Ok(());
    }
    let (hwnd, previous) = {
        let s = state.borrow();
        let group = s.workspace.tab_group(from).ok_or("标签组已关闭")?;
        if !group.members.contains(&to) {
            return Err("目标标签不属于此面板".into());
        }
        let view = s.views.iter().find(|v| v.id == from).ok_or("面板已关闭")?;
        (view.window.hwnd().cast(), s.workspace.clone())
    };
    rename::cancel(hwnd);
    {
        let mut s = state.borrow_mut();
        let mut next = match s.tab_models.get(&to) {
            Some(model) => model.clone(),
            None => create_model(&s, to)?,
        };
        next.folder_columns = folder::saved_columns(&s.store, to)?;
        next.folder_visible_columns = folder::visible_columns(&s.store, to)?;
        folder::ensure(&mut s, to)?;
        if let Some(source) = s.folders.get(&to) {
            source.set_active(false);
        }
        s.workspace.sync_tab_windows();
        let mut groups = s.workspace.tab_groups().to_vec();
        groups
            .iter_mut()
            .find(|g| g.members.contains(&from))
            .unwrap()
            .active = to;
        s.workspace
            .set_tab_groups(groups)
            .map_err(|e| e.to_string())?;
        let workspace = s.workspace.clone();
        if let Err(error) = s.store.save_workspace(&workspace) {
            s.workspace = previous;
            return Err(error.to_string());
        }
        s.tab_models.remove(&to);
        let panel = s.workspace.panel(to).unwrap();
        next.title = panel.title().to_owned();
        next.options = s.workspace.pane_options();
        next.theme = panel.theme();
        next.dark = theme::is_dark(panel.theme());
        next.backdrop = panel.backdrop();
        next.collapsed = panel.collapsed();
        next.locked = panel.locked();
        next.auto_hide = panel.auto_hide();
        next.list_view = panel.list_view();
        next.reveal = if next.collapsed { 0.0 } else { 1.0 };
        next.hovered_item = None;
        next.hovered_button = None;
        next.pressed_button = None;
        next.renaming = None;
        next.scrollbar = Default::default();
        decorate(&s.workspace, to, &mut next);
        hybrid::unregister_drop(&mut s, hwnd);
        let view = s.views.iter_mut().find(|v| v.id == from).unwrap();
        next.focused = view.model.borrow().focused;
        next.native_material = view.model.borrow().native_material;
        let old = std::mem::replace(&mut *view.model.borrow_mut(), next);
        view.id = to;
        view.target.set(to);
        s.tab_models.insert(from, old);
        if let Some(source) = s.folders.get(&from) {
            source.set_active(false);
        }
        if let Some(source) = s.folders.get(&to) {
            source.set_active(true);
        }
        if let Some(runtime) = &mut s.runtime {
            if let Some(position) = runtime.layouts.positions.get(&from).copied() {
                runtime.layouts.positions.insert(to, position);
            }
        }
        refresh_changed_views(&mut s, true);
        s.wake.notify();
    }
    let register = state.borrow().session.is_some()
        || state
            .borrow()
            .workspace
            .panel(to)
            .unwrap()
            .folder()
            .is_some();
    if register {
        hybrid::register_drop(state, to)?;
    }
    unsafe {
        PostMessageW(hwnd, window::TAB_CHANGED, 0, 0);
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
    Ok(())
}

pub(super) fn add(
    state: &Rc<RefCell<PaneApp>>,
    id: PanelId,
    folder: Option<std::path::PathBuf>,
) -> Result<(), String> {
    let next = {
        let mut s = state.borrow_mut();
        let source = s.workspace.panel(id).ok_or("面板已关闭")?.clone();
        if source.is_search() {
            return Err("搜索面板不支持标签".into());
        }
        if source.locked() {
            return Err("请先解锁面板".into());
        }
        let next = PanelId::new(
            s.workspace
                .panels()
                .iter()
                .map(|p| p.id().get())
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("标签数量超出限制")?,
        );
        let title = folder
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "新标签".into());
        let mut panel = Panel::new(next, title, source.rect());
        panel.set_folder(folder);
        if panel.folder().is_some() {
            folder::Defaults::load(&s.store)?.apply(&s.store, &mut panel)?;
        }
        let previous = s.workspace.clone();
        s.workspace.add_panel(panel).map_err(|e| e.to_string())?;
        let mut groups = s.workspace.tab_groups().to_vec();
        if let Some(group) = groups.iter_mut().find(|g| g.members.contains(&id)) {
            group.members.push(next);
        } else {
            groups.push(PaneTabs {
                members: vec![id, next],
                active: id,
            });
        }
        s.workspace
            .set_tab_groups(groups)
            .map_err(|e| e.to_string())?;
        if let Err(error) = save(&mut s) {
            s.workspace = previous;
            return Err(error);
        }
        refresh_views(&mut s);
        next
    };
    select(state, id, next)
}

pub(super) fn choose_folder(state: &Rc<RefCell<PaneApp>>, id: PanelId) -> Result<(), String> {
    let owner = state
        .borrow()
        .views
        .iter()
        .find(|v| v.id == id)
        .ok_or("面板已关闭")?
        .window
        .hwnd() as isize;
    let weak = Rc::downgrade(state);
    if !window::post_action(owner as _, move || {
        let result = (|| {
            let Some(path) = folder::choose(owner)? else {
                return Ok(());
            };
            let Some(state) = weak.upgrade() else {
                return Ok(());
            };
            // The source may have changed during the modal picker.
            let active = state
                .borrow()
                .workspace
                .tab_group(id)
                .map_or(id, |g| g.active);
            add(&state, active, Some(path))
        })();
        if let Err(error) = result {
            window::error(&error);
        }
    }) {
        return Err("无法打开文件夹选择器".into());
    }
    Ok(())
}

pub(super) fn close(state: &Rc<RefCell<PaneApp>>, id: PanelId) -> Result<(), String> {
    let group = state.borrow().workspace.tab_group(id).cloned();
    let Some(group) = group else {
        handle(state, id, Event::ClosePane)?;
        return Ok(());
    };
    if state
        .borrow()
        .workspace
        .panel(id)
        .is_some_and(Panel::locked)
    {
        return Err("请先解锁面板".into());
    }
    if group.active == id {
        let at = group
            .members
            .iter()
            .position(|member| *member == id)
            .unwrap();
        let next = if at + 1 < group.members.len() {
            group.members[at + 1]
        } else {
            group.members[at - 1]
        };
        select(state, id, next)?;
    }
    let mut s = state.borrow_mut();
    let previous = s.workspace.clone();
    remove_panel(&mut s.workspace, id);
    if let Err(error) = save(&mut s) {
        s.workspace = previous;
        let _ = hybrid::sync(&mut s);
        return Err(error);
    }
    s.folders.remove(&id);
    s.tab_models.remove(&id);
    refresh_changed_views(&mut s, true);
    Ok(())
}

pub(super) fn reorder(
    state: &Rc<RefCell<PaneApp>>,
    id: PanelId,
    moved: PanelId,
    before: PanelId,
) -> Result<(), String> {
    let mut s = state.borrow_mut();
    if s.workspace.panel(id).is_some_and(Panel::locked) {
        return Err("请先解锁面板".into());
    }
    let previous = s.workspace.clone();
    let mut groups = s.workspace.tab_groups().to_vec();
    let group = groups
        .iter_mut()
        .find(|g| g.members.contains(&id))
        .ok_or("标签组已关闭")?;
    let from = group
        .members
        .iter()
        .position(|member| *member == moved)
        .ok_or("标签已关闭")?;
    let to = group
        .members
        .iter()
        .position(|member| *member == before)
        .ok_or("标签已关闭")?;
    group.members.remove(from);
    group.members.insert(to, moved);
    s.workspace
        .set_tab_groups(groups)
        .map_err(|e| e.to_string())?;
    if let Err(error) = save(&mut s) {
        s.workspace = previous;
        return Err(error);
    }
    refresh_changed_views(&mut s, true);
    Ok(())
}

/// Shared geometry for painting and pointer input; keep the active tab visible.
pub(super) fn strip(model: &GroupModel, width: f32) -> Vec<(PanelId, RectDip)> {
    if model.tabs.len() < 2 {
        return Vec::new();
    }
    let right_buttons = if model.folder.is_some() { 70.0 } else { 0.0 };
    // Use one inset for the top, left and right edges.
    let inset = layout::HEADER_INSET;
    let available = (width - right_buttons - inset * 2.0).max(72.0);
    let count = ((available / 72.0).floor() as usize)
        .max(1)
        .min(model.tabs.len());
    let active = model
        .tabs
        .iter()
        .position(|(id, _)| *id == model.active_tab)
        .unwrap_or(0);
    let start = active
        .saturating_sub(count / 2)
        .min(model.tabs.len() - count);
    let gap = 6.0;
    let size = (available - gap * (count - 1) as f32) / count as f32;
    model
        .tabs
        .iter()
        .skip(start)
        .take(count)
        .enumerate()
        .map(|(at, (id, _))| {
            (
                *id,
                RectDip {
                    x: inset + at as f32 * (size + gap),
                    y: inset,
                    width: size,
                    height: layout::HEADER - inset * 2.0,
                },
            )
        })
        .collect()
}

pub(super) fn hit(model: &GroupModel, width: f32, x: f32, y: f32) -> Option<PanelId> {
    strip(model, width)
        .into_iter()
        .find(|(_, r)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
        .map(|(hit, _)| hit)
}

pub(super) fn adjacent(model: &GroupModel, step: i8) -> Option<PanelId> {
    let at = model
        .tabs
        .iter()
        .position(|(id, _)| *id == model.active_tab)?;
    let next = (at as isize + step as isize).rem_euclid(model.tabs.len() as isize) as usize;
    Some(model.tabs[next].0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_share_one_window_keep_selection_and_release_only_closed_membership() {
        let _apartment = ShellApartment::initialize_sta().unwrap();
        let state = Rc::new(RefCell::new(super::super::tests::test_state()));
        state
            .borrow_mut()
            .workspace
            .set_appearance(desktop_core::PanelTheme::Dark, desktop_core::Backdrop::Mica);
        let first = PanelId::new(1);
        create_view(&state, first).unwrap();
        let hwnd = state.borrow().views[0].window.hwnd();
        state.borrow().views[0]
            .model
            .borrow_mut()
            .select_item(1, false, false);
        let original = state.borrow().workspace.desktop_items().to_vec();
        add(&state, first, None).unwrap();
        let second = state.borrow().views[0].id;
        assert_ne!(first, second);
        assert_eq!(state.borrow().views.len(), 1);
        assert_eq!(state.borrow().views[0].window.hwnd(), hwnd);
        assert_eq!(state.borrow().workspace.desktop_items(), original);
        assert!(state.borrow().views[0].model.borrow().items.is_empty());
        handle(&state, second, Event::SetTitle("资料".into())).unwrap();
        select(&state, second, first).unwrap();
        assert_eq!(state.borrow().views[0].model.borrow().selected, Some(1));
        assert_eq!(state.borrow().views[0].model.borrow().tabs[1].1, "资料");
        add(&state, first, None).unwrap();
        let third = state.borrow().views[0].id;
        select(&state, third, first).unwrap();
        handle(&state, first, Event::CloseTabId(third)).unwrap();
        assert_eq!(state.borrow().views[0].id, first);
        assert!(state.borrow().workspace.panel(third).is_none());
        handle(&state, first, Event::MoveTabId(second, -1)).unwrap();
        assert_eq!(state.borrow().views[0].id, first);
        assert_eq!(
            state.borrow().workspace.tab_group(first).unwrap().members[0],
            second
        );
        close(&state, first).unwrap();
        assert_eq!(state.borrow().views.len(), 1);
        assert_eq!(state.borrow().views[0].id, second);
        assert!(state.borrow().workspace.tab_groups().is_empty());
        assert!(state.borrow().workspace.desktop_items().iter().all(|item| !matches!(item.placement(), DesktopPlacement::Pane { pane_id, .. } if *pane_id == first)));
        assert_eq!(
            state.borrow().store.load_workspace().unwrap(),
            state.borrow().workspace
        );
        window::prepare_close(hwnd.cast());
        drop(state);
        // Exercise both sources in one UI apartment, as the application does.
        folder_tabs_keep_navigation_and_do_not_cross_deliver_results();
    }

    fn folder_tabs_keep_navigation_and_do_not_cross_deliver_results() {
        let _apartment = ShellApartment::initialize_sta().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("child")).unwrap();
        std::fs::write(root.path().join("child").join("inside.txt"), b"inside").unwrap();
        let state = Rc::new(RefCell::new(super::super::tests::test_state()));
        state
            .borrow_mut()
            .workspace
            .set_appearance(desktop_core::PanelTheme::Dark, desktop_core::Backdrop::Mica);
        let first = PanelId::new(1);
        create_view(&state, first).unwrap();
        add(&state, first, Some(root.path().to_path_buf())).unwrap();
        let folder = state.borrow().views[0].id;
        folder::navigate(
            &mut state.borrow_mut(),
            folder,
            Some(root.path().join("child")),
        )
        .unwrap();
        select(&state, folder, first).unwrap();
        let before = state.borrow().views[0]
            .model
            .borrow()
            .items
            .iter()
            .map(|i| i.label.clone())
            .collect::<Vec<_>>();
        folder::poll(&mut state.borrow_mut());
        assert_eq!(
            state.borrow().views[0]
                .model
                .borrow()
                .items
                .iter()
                .map(|i| i.label.clone())
                .collect::<Vec<_>>(),
            before
        );
        select(&state, first, folder).unwrap();
        assert_eq!(
            state.borrow().folders[&folder].path,
            root.path().join("child")
        );
        assert!(state.borrow().folders[&folder].navigation()[0]);
        handle(&state, folder, Event::ClosePane).unwrap();
        assert!(state.borrow().views.is_empty());
        assert!(state.borrow().folders.is_empty());
        assert!(root.path().join("child").join("inside.txt").exists());
    }

    #[test]
    fn tab_hit_geometry_never_overlaps_content_and_active_tab_survives_overflow() {
        let mut state = super::super::tests::test_state();
        state
            .workspace
            .set_appearance(desktop_core::PanelTheme::Dark, desktop_core::Backdrop::Mica);
        let mut model = create_model(&state, PanelId::new(1)).unwrap();
        model.tabs = (1..=12)
            .map(|id| (PanelId::new(id), format!("标签 {id}")))
            .collect();
        model.active_tab = PanelId::new(10);
        for folder in [None, Some(std::path::PathBuf::from("C:/"))] {
            model.folder = folder;
            for collapsed in [false, true] {
                model.collapsed = collapsed;
                for width in [260.0, 420.0, 800.0] {
                    let strip = strip(&model, width);
                    assert!(strip.iter().any(|(hit, _)| *hit == model.active_tab));
                    for (expected, rect) in strip {
                        let (x, y) = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
                        assert_eq!(hit(&model, width, x, y), Some(expected));
                        assert!(rect.y >= 0.0 && rect.y + rect.height <= layout::HEADER);
                        assert!(model.header_button(width, x, y).is_none());
                        assert!(model.grid(width, 360.0).hit(x, y, 0, 3).is_none());
                        assert!(rect.x >= 0.0 && rect.x + rect.width <= width);
                    }
                }
            }
        }
    }
}
