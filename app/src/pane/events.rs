//! Group commands and their persisted state transitions.
use super::search::everything_settings;
use super::*;

fn enable_search_view(
    state: &Rc<RefCell<PaneApp>>,
    id: PanelId,
    create: impl FnOnce(&Rc<RefCell<PaneApp>>, PanelId) -> Result<(), String>,
) -> Result<bool, String> {
    if !state.borrow().views.iter().any(|v| v.id == id) {
        if let Err(error) = create(state, id) {
            everything_settings::set_enabled(&state.borrow().store, false)?;
            return Err(error);
        }
    }
    if let Err(error) = everything_settings::set_enabled(&state.borrow().store, true) {
        let view = {
            let mut s = state.borrow_mut();
            s.views
                .iter()
                .position(|v| v.id == id)
                .map(|at| s.views.remove(at))
        };
        drop(view);
        return Err(error);
    }
    Ok(false)
}

#[cfg(test)]
mod search_lifecycle_tests {
    use super::*;
    #[test]
    fn failed_search_window_can_be_enabled_again() {
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let state = Rc::new(RefCell::new(super::super::tests::test_state()));
        state.borrow_mut().workspace.set_appearance(
            desktop_core::PanelTheme::Dark,
            desktop_core::Backdrop::Translucent { opacity: 1.0 },
        );
        handle(&state, PanelId::new(0), Event::EnableSearch).unwrap();
        let id = state.borrow().views[0].id;
        handle(&state, id, Event::ClosePane).unwrap();
        let failed = enable_search_view(&state, id, |_, _| {
            Err("injected window creation failure".into())
        });
        assert!(failed.is_err());
        assert!(state.borrow().views.is_empty());
        assert!(!everything_settings::enabled(&state.borrow().store).unwrap());
        handle(&state, id, Event::EnableSearch).unwrap();
        assert_eq!(state.borrow().views[0].id, id);
        assert!(everything_settings::enabled(&state.borrow().store).unwrap());
    }
}

pub(super) fn activate_with(
    state: &Rc<RefCell<PaneApp>>,
    id: PanelId,
    index: usize,
    open: impl FnOnce(isize, &ShellIdentity) -> Result<(), String> + 'static,
) -> Result<(), String> {
    let target = {
        let s = state.borrow();
        s.views.iter().find(|view| view.id == id).and_then(|view| {
            view.model
                .borrow()
                .items
                .get(index)
                .map(|item| (view.window.hwnd() as isize, item.identity.clone()))
        })
    };
    if let Some((owner, identity)) = target {
        // Shell execution can pump messages for every pane. Run after both
        // the app borrows and the current window/event callbacks have returned.
        if !window::post_action(owner as _, move || {
            if unsafe { IsWindow(owner as _) } == 0 {
                return;
            }
            if let Err(error) = open(owner, &identity) {
                window::error(&error);
            }
        }) {
            return Err(crate::i18n::text("ui-could-not-schedule-opening-the-item").into());
        }
    }
    Ok(())
}

pub(super) fn activate_item_with(
    state: &Rc<RefCell<PaneApp>>,
    id: PanelId,
    index: usize,
    open: impl FnOnce(isize, &ShellIdentity) -> Result<(), String> + 'static,
) -> Result<(), String> {
    let target = folder::entry_mode::navigation_target(&state.borrow(), id, index)?;
    if let Some(path) = target {
        folder::navigate(&mut state.borrow_mut(), id, Some(path))
    } else {
        activate_with(state, id, index, open)
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn handle(
    state: &Rc<RefCell<PaneApp>>,
    id: PanelId,
    event: Event,
) -> Result<bool, String> {
    if matches!(event, Event::SetTitleEmojiColor(_) | Event::ResetPaneOptions) {
        let color = if let Event::SetTitleEmojiColor(color) = event { color } else { true };
        let s = state.borrow();
        title_emoji::save(&s.store, color)?;
        for view in &s.views { unsafe { InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0); } }
        if matches!(event, Event::SetTitleEmojiColor(_)) { return Ok(false); }
    }
    if matches!(event, Event::ToggleCompactMenu | Event::ResetPaneOptions) {
        compact_menu::save(&state.borrow().store, matches!(event, Event::ResetPaneOptions) || !compact_menu::enabled())?;
        if matches!(event, Event::ToggleCompactMenu) { return Ok(false); }
    }
    if matches!(event, Event::ToggleHeaderDivider | Event::ResetPaneOptions) {
        let s = state.borrow();
        header_divider::save(&s.store, matches!(event, Event::ResetPaneOptions) || !header_divider::enabled())?;
        for view in &s.views { unsafe { InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0); } }
        if matches!(event, Event::ToggleHeaderDivider) { return Ok(false); }
    }
    match event {
        Event::DetachTab(target) => { tabs::detach(state, target)?; return Ok(false); }
        Event::PreviewPaneMove => { tabs::preview_merge(&state.borrow(), id); return Ok(false); }
        Event::FinishPaneMove(commit) => { tabs::finish_move(state, id, commit)?; return Ok(false); }
        Event::RenameTab(target) => {
            let (owner, model) = {
                let s = state.borrow();
                if s.workspace.panel(target).is_none_or(Panel::locked) { return Ok(false); }
                if !s.workspace.tab_group(id).is_some_and(|g| g.members.contains(&target)) { return Ok(false); }
                let view = s.views.iter().find(|v| v.id == id).ok_or(crate::i18n::text("ui-panel-closed"))?;
                (view.window.hwnd().cast(), view.model.clone())
            };
            let state = Rc::clone(state);
            rename::show_tab_title(owner, model, Some(target), Box::new(move |title| handle(&state, target, Event::SetTitle(title)).map(|_| ())))?;
            return Ok(false);
        }
        Event::MoveTabId(target, step) => {
            let next = state.borrow().workspace.tab_group(id).and_then(|g| {
                let at = g.members.iter().position(|member| *member == target)?;
                g.members.get(at.checked_add_signed(step as isize)?).copied()
            });
            if let Some(next) = next { tabs::reorder(state, id, target, next)?; }
            return Ok(false);
        }
        Event::NewTab(false) => { tabs::add(state, id, None)?; return Ok(false); }
        Event::NewTab(true) => { tabs::choose_folder(state, id)?; return Ok(false); }
        Event::SelectTab(to) => { tabs::select(state, id, to)?; return Ok(false); }
        Event::CloseTabId(to) => { tabs::close(state, to)?; return Ok(false); }
        Event::CloseTab => { tabs::close(state, id)?; return Ok(false); }
        Event::MoveTab(step) => {
            let destination = state.borrow().workspace.tab_group(id).and_then(|group| {
                let at = group.members.iter().position(|member| *member == id)?;
                let next = at.checked_add_signed(step as isize)?;
                group.members.get(next).copied()
            });
            if let Some(to) = destination { tabs::reorder(state, id, id, to)?; }
            return Ok(false);
        }
        _ => {}
    }
    if let Event::SortFolder(column) = event {
        folder::sort(&mut state.borrow_mut(), id, column)?;
        return Ok(false);
    }
    if let Event::FolderItemCreated(path) = event {
        folder::item_created(&mut state.borrow_mut(), id, path);
        return Ok(false);
    }
    if let Event::SetFolderColumns(widths) = event {
        folder::save_columns(&state.borrow(), id, widths)?;
        return Ok(false);
    }
    if let Event::ToggleFolderColumn(column) = event {
        folder::toggle_column(&state.borrow(), id, column)?;
        return Ok(false);
    }
    if matches!(event, Event::FolderBack) {
        folder::navigate(&mut state.borrow_mut(), id, None)?;
        return Ok(false);
    }
    if matches!(event, Event::FolderHome) {
        folder::home(&mut state.borrow_mut(), id)?;
        return Ok(false);
    }
    if let Event::Activate(index) = event {
        activate_item_with(state, id, index, |owner, identity| {
            open_shell_identity(owner, identity).map_err(|error| error.to_string())
        })?;
        return Ok(false);
    }
    if matches!(
        event,
        Event::ExportBackup
            | Event::CreateBackup
            | Event::RestoreBackupPath(_)
            | Event::ExportBackupPath(_)
            | Event::DeleteBackup(_)
            | Event::RestoreBackup
            | Event::OpenBackups
            | Event::OpenConfigDirectory
            | Event::ReloadConfig
    ) {
        recovery::request(state, &event);
        return Ok(false);
    }
    if matches!(event, Event::RetryDesktop) {
        runtime::maintain(state, true)?;
        if state.borrow().session.is_none() {
            return Err(runtime::status(&state.borrow()));
        }
        return Ok(false);
    }
    if matches!(event, Event::New)
        && state.borrow().runtime.is_some()
        && state.borrow().session.is_none()
    {
        return Err(runtime::status(&state.borrow()));
    }
    if matches!(event, Event::ToggleListView) {
        let mut s = state.borrow_mut();
        let old = s.workspace.clone();
        let panel = s.workspace.panel_mut(id).ok_or(crate::i18n::text("ui-panel-closed"))?;
        if panel.is_search() {
            return Ok(false);
        }
        let enabled = !panel.list_view();
        panel.set_list_view(enabled);
        if let Err(error) = save(&mut s) {
            s.workspace = old;
            return Err(error);
        }
        if let Some(view) = s.views.iter().find(|v| v.id == id) {
            let mut model = view.model.borrow_mut();
            model.list_view = enabled;
            model.scroll = 0;
            model.hovered_item = None;
        }
        refresh_changed_views(&mut s, true);
        return Ok(false);
    }
    if let Event::FileDrag(image) = event {
        let target = {
            let s = state.borrow();
            s.views.iter().find(|v| v.id == id).map(|v| {
                (
                    v.window.hwnd() as isize,
                    v.model.borrow().selected_identities(),
                )
            })
        };
        if let Some((owner, items)) = target {
            window::post_action(owner as _, move || {
                if let Err(error) = desktop_shell::drag_file_items(
                    windows::Win32::Foundation::HWND(owner as _),
                    &items,
                    image.as_ref(),
                ) {
                    window::error(&error.to_string());
                }
            });
        }
        return Ok(false);
    }
    if matches!(event, Event::NewFolder | Event::ChangeFolder) {
        folder::request_picker(state, id, matches!(event, Event::ChangeFolder))?;
        return Ok(false);
    }
    if let Event::SetFolder(path) = &event {
        let mut s = state.borrow_mut();
        let old = s.workspace.clone();
        let panel = s.workspace.panel_mut(id).ok_or(crate::i18n::text("ui-panel-closed"))?;
        if panel.folder().is_none() {
            return Err(crate::i18n::text("ui-this-is-not-a-folder-panel").into());
        }
        panel.set_folder(Some(path.clone()));
        let title = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned();
        panel.set_title(title.clone());
        if let Err(error) = save(&mut s) {
            s.workspace = old;
            return Err(error);
        }
        folder::ensure(&mut s, id)?;
        folder::home(&mut s, id)?;
        if let Some(view) = s.views.iter().find(|v| v.id == id) {
            let mut model = view.model.borrow_mut();
            model.folder = Some(path.clone());
            model.title = title;
            unsafe { InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0); }
            model.clear_selection();
            model.scroll = 0;
        }
        refresh_views(&mut s);
        return Ok(false);
    }
    if matches!(event, Event::OpenFolder) {
        let target = {
            let s = state.borrow();
            s.folders.get(&id).map(|source| &source.path).map(|path| {
                (
                    s.views
                        .iter()
                        .find(|v| v.id == id)
                        .map_or(0, |v| v.window.hwnd() as isize),
                    folder::identity(path.to_path_buf()),
                )
            })
        };
        if let Some((owner, identity)) = target {
            window::defer_action(move || {
                if let Err(error) = open_shell_identity(owner, &identity) {
                    window::error(&error.to_string());
                }
            });
        }
        return Ok(false);
    }
    if let Event::Peek = event {
        let index = {
            let s = state.borrow();
            s.views.iter().find(|view| view.id == id).and_then(|view| {
                let model = view.model.borrow();
                if model.collapsed {
                    return None;
                }
                model
                    .selected
                    .filter(|index| model.selection.contains(index))
                    .or_else(|| model.selection.iter().next().copied())
            })
        };
        if let Some(index) = index {
            let weak = Rc::downgrade(state);
            activate_with(state, id, index, move |owner, identity| {
                let Some(state) = weak.upgrade() else {
                    return Ok(());
                };
                if peek::settings().provider == peek::Provider::QuickLook
                    || state
                        .borrow()
                        .workspace
                        .panel(id)
                        .is_some_and(|p| p.folder().is_some())
                {
                    return peek::open_path(identity);
                }
                hybrid::pause_for_preview(&state.borrow(), true)?;
                let result = peek::open(owner, identity);
                let restored = hybrid::pause_for_preview(&state.borrow(), false);
                result.and(restored)
            })?;
        }
        return Ok(false);
    }
    if let Event::FileCommand(command) = event {
        let target = {
            let s = state.borrow();
            s.views.iter().find(|view| view.id == id).map(|view| {
                let model = view.model.borrow();
                let destination = (command == desktop_shell::FileCommand::Paste)
                    .then(|| s.folders.get(&id).map(|source| source.path.clone()))
                    .flatten();
                (view.window.hwnd(), model.selected_identities(), destination)
            })
        };
        if let Some((owner, identity, destination)) = target {
            if !window::post_action(owner.cast(), move || {
                if unsafe { IsWindow(owner.cast()) } == 0 {
                    return;
                }
                let result = if let Some(path) = destination {
                    desktop_shell::paste_into_folder(
                        windows::Win32::Foundation::HWND(owner.cast()),
                        &path,
                    )
                } else {
                    desktop_shell::invoke_file_commands(
                        windows::Win32::Foundation::HWND(owner.cast()),
                        &identity,
                        command,
                    )
                };
                if let Err(error) = result {
                    window::error(&crate::i18n::format("ui-file-operation-failed", &[("error", format!("{}", error))]));
                }
            }) {
                return Err(crate::i18n::text("ui-could-not-schedule-file-operation").into());
            }
        }
        return Ok(false);
    }
    if let Event::ActivateSelection = event {
        let indices: Vec<_> = {
            let s = state.borrow();
            s.views
                .iter()
                .find(|view| view.id == id)
                .map(|view| view.model.borrow().selection.iter().copied().collect())
                .unwrap_or_default()
        };
        if indices.len() == 1 {
            return handle(state, id, Event::Activate(indices[0]));
        }
        for index in indices {
            activate_with(state, id, index, |owner, identity| {
                open_shell_identity(owner, identity).map_err(|error| error.to_string())
            })?;
        }
        return Ok(false);
    }
    if matches!(event, Event::RenameTitle | Event::SetTitle(_))
        && state
            .borrow()
            .workspace
            .panel(id)
            .is_some_and(|panel| panel.locked())
    {
        return Ok(false);
    }
    if matches!(
        event,
        Event::SetCornerRadius(_)
            | Event::SetIconGrid(_)
            | Event::SetPanelText(_)
            | Event::ToggleTextProtection
            | Event::ToggleBorder
            | Event::ToggleSnap
            | Event::ResetPaneOptions
    ) {
        let mut s = state.borrow_mut();
        let old = s.workspace.pane_options();
        let mut options = old;
        match event {
            Event::SetCornerRadius(radius) => {
                if !radius.is_finite() {
                    return Ok(false);
                }
                options.corner_radius =
                    radius.clamp(0.0, desktop_core::PaneOptions::MAX_CORNER_RADIUS)
            }
            Event::SetIconGrid(value) => {
                if !value.is_finite() {
                    return Ok(false);
                }
                let range = desktop_core::PaneOptions::GRID_SCALE_RANGE;
                let value = value.round().clamp(range.0, range.1);
                options.grid_scale = value;
            }
            Event::ToggleBorder => options.border = !options.border,
            Event::SetPanelText(text) => options.text = text,
            Event::ToggleTextProtection => options.text_protection = !options.text_protection,
            Event::ToggleSnap => options.snap = !options.snap,
            Event::ResetPaneOptions => options = desktop_core::PaneOptions::default(),
            _ => unreachable!(),
        }
        s.workspace.set_pane_options(options);
        if options == old {
            return Ok(false);
        }
        if let Err(error) = s
            .store
            .save_pane_options(options)
            .map_err(|e| e.to_string())
        {
            s.workspace.set_pane_options(old);
            return Err(error);
        }
        for view in &s.views {
            let mut model = view.model.borrow_mut();
            model.options = options;
            if !model.is_list() && (options.grid_scale != old.grid_scale) {
                model.scroll = 0;
                model.hovered_item = None;
            }
            drop(model);
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
    if matches!(event, Event::Settings) {
        settings::show(state, id)?;
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
        let search = s.workspace.panel(id).is_some_and(Panel::is_search);
        let members = s.workspace.tab_group(id).map_or_else(|| vec![id], |g| g.members.clone());
        let result = if search {
            everything_settings::set_enabled(&s.store, false)
        } else {
            for member in &members { remove_panel(&mut s.workspace, *member); }
            save(&mut s)
        };
        if let Err(error) = result {
            s.workspace = old;
            hybrid::sync(&mut s)?;
            return Err(error);
        }
        for member in members {
            s.folders.remove(&member);
            s.tab_models.remove(&member);
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
        let s = state.borrow();
        for view in s.views.iter().filter(|view| view.id != id) {
            if s.workspace.panel(view.id).is_some_and(Panel::is_search) {
                unsafe {
                    PostMessageW(view.window.hwnd().cast(), search::CLEAR_SELECTION, 0, 0);
                }
            }
            let mut model = view.model.borrow_mut();
            if model.selected.is_some() || !model.selection.is_empty() || model.focused {
                model.clear_selection();
                model.focused = false;
                drop(model);
                unsafe {
                    windows_sys::Win32::Graphics::Gdi::InvalidateRect(
                        view.window.hwnd().cast(),
                        std::ptr::null(),
                        0,
                    );
                }
            }
        }
        hybrid::clear_desktop_selection(&s)?;
        return Ok(false);
    }
    if let Event::BeginItemMenu(reply) = event {
        *reply.borrow_mut() = Some(hybrid::begin_item_menu(&state.borrow()));
        return Ok(false);
    }
    if let Event::EndItemMenu = event {
        hybrid::end_item_menu(&state.borrow());
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
            let weak = Rc::downgrade(state);
            rename::show_managed(owner, &identity, &label, model, Rc::new(move |identity, name| {
                let state = weak.upgrade().ok_or(crate::i18n::text("ui-group-closed"))?;
                hybrid::rename_item(&state, owner, identity, name)
            }))?;
        }
        return Ok(false);
    }
    if matches!(event, Event::ToggleSearch) {
        let ids: Vec<_> = state
            .borrow()
            .workspace
            .panels()
            .iter()
            .filter(|p| p.is_search())
            .map(Panel::id)
            .collect();
        let visible = state.borrow().views.iter().any(|v| ids.contains(&v.id));
        if !visible {
            return handle(state, id, Event::EnableSearch);
        }
        for id in ids {
            handle(state, id, Event::ClosePane)?;
        }
        return Ok(false);
    }
    if matches!(
        event,
        Event::New | Event::EnableSearch | Event::MapFolder(_)
    ) {
        let search = matches!(event, Event::EnableSearch);
        if search && everything_settings::resolved(&everything_settings::settings()).is_none() {
            return Ok(false);
        }
        if search {
            let existing = state
                .borrow()
                .workspace
                .panels()
                .iter()
                .find(|p| p.is_search())
                .map(Panel::id);
            if let Some(existing) = existing {
                return enable_search_view(state, existing, create_view);
            }
        }
        let path = if let Event::MapFolder(path) = &event {
            Some(path.clone())
        } else {
            None
        };
        let next = {
            let mut s = state.borrow_mut();
            let old = s.workspace.clone();
            let next = PanelId::new(
                s.workspace
                    .panels()
                    .iter()
                    .map(|p| p.id().get())
                    .max()
                    .unwrap_or(0)
                    + 1,
            );
            let mut panel = Panel::new(
                next,
                path.as_ref()
                    .map(|p| {
                        p.file_name()
                            .unwrap_or(p.as_os_str())
                            .to_string_lossy()
                            .into_owned()
                    })
                    .unwrap_or_else(|| crate::i18n::text("ui-new-group").into()),
                RectDip::new(240.0, 240.0, 480.0, 360.0),
            );
            panel.set_folder(path.clone());
            if path.is_some() {
                folder::Defaults::load(&s.store)?.apply(&s.store, &mut panel)?;
            }
            if search {
                panel.set_search(true);
                panel.set_title(crate::i18n::text("ui-everything-search").to_string());
                panel.set_rect(RectDip::new(240.0, 240.0, 360.0, 200.0));
            }
            s.workspace.add_panel(panel).map_err(|e| e.to_string())?;
            if s.workspace.appearance().is_none() {
                s.workspace
                    .panel_mut(next)
                    .unwrap()
                    .set_backdrop(desktop_core::Backdrop::Acrylic);
            }
            if let Err(error) = save(&mut s) {
                s.workspace = old;
                return Err(error);
            }
            next
        };
        if search {
            // Keep the saved configuration on failure so enabling can retry.
            everything_settings::set_enabled(&state.borrow().store, false)?;
            enable_search_view(state, next, create_view)?;
        } else {
            create_view(state, next)?;
        }
        return Ok(false);
    }
    let mut s = state.borrow_mut();
    match event {
        Event::DetachTab(_) | Event::PreviewPaneMove | Event::FinishPaneMove(_) | Event::RenameTab(_) | Event::MoveTabId(..) | Event::ToggleHeaderDivider | Event::ToggleCompactMenu | Event::SetTitleEmojiColor(_)
        | Event::NewTab(_) | Event::SelectTab(_) | Event::CloseTab | Event::CloseTabId(_)
        | Event::MoveTab(_) => unreachable!("Tabs handled before borrowing PaneApp"),
        Event::SortFolder(_)
        | Event::FolderItemCreated(_)
        | Event::SetFolderColumns(_)
        | Event::ToggleFolderColumn(_)
        | Event::FolderBack
        | Event::FolderHome
        | Event::ExportBackup
        | Event::CreateBackup
        | Event::RestoreBackupPath(_)
        | Event::ExportBackupPath(_)
        | Event::DeleteBackup(_)
        | Event::RestoreBackup
        | Event::OpenBackups
        | Event::OpenConfigDirectory
        | Event::ReloadConfig
        | Event::RetryDesktop
        | Event::ToggleListView
        | Event::ToggleSearch
        | Event::EnableSearch
        | Event::FileDrag(_)
        | Event::NewFolder
        | Event::MapFolder(_)
        | Event::ChangeFolder
        | Event::SetFolder(_)
        | Event::OpenFolder => unreachable!("Handled before borrowing PaneApp"),
        Event::RenameTitle | Event::ClosePane | Event::Settings => {
            unreachable!("Handled before borrowing PaneApp")
        }
        Event::SetTitle(title) => {
            let old = s.workspace.clone();
            s.workspace
                .panel_mut(id)
                .ok_or(crate::i18n::text("ui-group-not-found"))?
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
            refresh_changed_views(&mut s, true);
        }
        Event::PaneItemFocus
        | Event::BeginItemMenu(_)
        | Event::EndItemMenu
        | Event::RenameItem(_) => unreachable!("Handled before borrowing PaneApp"),
        Event::Theme(_)
        | Event::Material(_)
        | Event::SetCornerRadius(_)
        | Event::SetIconGrid(_)
        | Event::SetPanelText(_)
        | Event::ToggleTextProtection
        | Event::ToggleBorder
        | Event::ToggleSnap
        | Event::ResetPaneOptions => {
            unreachable!("Handled before borrowing PaneApp")
        }
        Event::ToggleLocked => {
            let panel = s.workspace.panel_mut(id).ok_or(crate::i18n::text("ui-group-not-found"))?;
            let enabled = !panel.locked();
            panel.set_locked(enabled);
            if let Err(error) = save(&mut s) {
                s.workspace.panel_mut(id).unwrap().set_locked(!enabled);
                return Err(error);
            }
            if let Some(view) = s.views.iter().find(|v| v.id == id) {
                view.model.borrow_mut().locked = enabled;
                unsafe {
                    InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0);
                }
            }
        }
        Event::ToggleTopmost => {
            let panel = s.workspace.panel_mut(id).ok_or(crate::i18n::text("ui-group-not-found"))?;
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
        Event::Refresh => {
            if let Some(source) = s.folders.get(&id) {
                source.refresh();
            } else {
                hybrid::refresh_icons(&mut s);
            }
        }
        Event::Moving(rect) | Event::Sizing(rect, _, _) => {
            if matches!(event, Event::Moving(_)) && tabs::preview_merge(&s, id) {
                return Ok(false);
            }
            if !s.workspace.pane_options().snap {
                return Ok(false);
            }
            let peers: Vec<_> = s
                .views
                .iter()
                .filter(|view| view.id != id)
                .filter(|view| unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(
                        view.window.hwnd().cast(),
                    ) != 0
                })
                .filter_map(|view| {
                    let mut bounds = RECT::default();
                    (unsafe { GetWindowRect(view.window.hwnd().cast(), &raw mut bounds) } != 0)
                        .then_some(bounds)
                })
                .collect();
            if let Some(view) = s.views.iter().find(|view| view.id == id) {
                let scale =
                    unsafe { GetDpiForWindow(view.window.hwnd().cast()) }.max(96) as f32 / 96.0;
                if let Event::Sizing(_, proposal, edge) = event {
                    unsafe {
                        snap::resize(
                            &mut *rect,
                            &proposal,
                            edge,
                            &peers,
                            5,
                            (14.0 * scale).round() as i32,
                        );
                    }
                    return Ok(false);
                }
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
                window::update_auto_hide(view.window.hwnd().cast(), enabled);
            }
        }
        Event::Activate(_) | Event::ActivateSelection | Event::Peek | Event::FileCommand(_) => {
            unreachable!("Handled before borrowing PaneApp")
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
            display_layout::record(&mut s)?;
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
        Event::Drop { index, point } => {
            let source = items_for(&s, id);
            let Some(_) = source.get(index) else {
                return Ok(false);
            };
            let indices: Vec<usize> = s
                .views
                .iter()
                .find(|v| v.id == id)
                .map(|v| {
                    let m = v.model.borrow();
                    if m.selection.contains(&index) {
                        m.selection.iter().copied().collect()
                    } else {
                        vec![index]
                    }
                })
                .unwrap_or_else(|| vec![index]);
            let target = s.views.iter().rev().find_map(|view| {
                if s.workspace.panel(view.id).is_some_and(Panel::is_search) {
                    return None;
                }
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
                if let Some(path) = s.folders.get(&target).map(|source| source.path.clone()) {
                    let identities: Vec<_> = indices
                        .iter()
                        .filter_map(|i| source.get(*i))
                        .map(|i| i.identity.clone())
                        .collect();
                    if !folder::accepts_copy(&identities, &path) {
                        return Ok(false);
                    }
                    let owner = s
                        .views
                        .iter()
                        .find(|v| v.id == target)
                        .unwrap()
                        .window
                        .hwnd() as isize;
                    window::post_action(owner as _, move || {
                        if let Err(error) = desktop_shell::copy_to_folder(
                            windows::Win32::Foundation::HWND(owner as _),
                            &identities,
                            &path,
                        ) {
                            window::error(&error.to_string());
                        }
                    });
                    return Ok(false);
                }
                if s.workspace.panel(id).is_some_and(|p| p.folder().is_some()) {
                    return Ok(false);
                }
                transfer_many(&mut s, id, &indices, target, at)?;
                for view in &s.views {
                    view.model.borrow_mut().clear_selection();
                }
                refresh_views(&mut s);
            } else if hybrid::release(&mut s, id, &indices, point)? {
                refresh_views(&mut s);
            }
        }
        Event::Exit => windows_window::quit(),
        Event::New => unreachable!(),
    }
    Ok(false)
}

/// Preview only; the settings gesture commits its final value separately.
pub(super) fn preview_grid(state: &mut PaneApp, value: f32) {
    if !value.is_finite() {
        return;
    }
    let range = desktop_core::PaneOptions::GRID_SCALE_RANGE;
    let value = value.clamp(range.0, range.1);
    let mut options = state.workspace.pane_options();
    options.grid_scale = value;
    if options == state.workspace.pane_options() {
        return;
    }
    state.workspace.set_pane_options(options);
    for view in &state.views {
        let mut model = view.model.borrow_mut();
        model.options = options;
        if !model.is_list() {
            model.scroll = 0;
            model.hovered_item = None;
        }
        drop(model);
        unsafe {
            InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0);
        }
    }
}

pub(super) fn commit_grid(state: &mut PaneApp, original: f32) -> Result<(), String> {
    let options = state.workspace.pane_options();
    if options.grid_scale == original {
        return Ok(());
    }
    if let Err(error) = state.store.save_pane_options(options) {
        preview_grid(state, original);
        return Err(error.to_string());
    }
    Ok(())
}

pub(super) fn preview_radius(state: &mut PaneApp, radius: f32) {
    if !radius.is_finite() {
        return;
    }
    let radius = radius.clamp(0.0, desktop_core::PaneOptions::MAX_CORNER_RADIUS);
    let mut options = state.workspace.pane_options();
    if options.corner_radius == radius {
        return;
    }
    options.corner_radius = radius;
    state.workspace.set_pane_options(options);
    for view in &state.views {
        view.model.borrow_mut().options = options;
        unsafe {
            InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0);
        }
    }
}

pub(super) fn commit_radius(state: &mut PaneApp, original: f32) -> Result<(), String> {
    let options = state.workspace.pane_options();
    if options.corner_radius == original {
        return Ok(());
    }
    if let Err(error) = state.store.save_pane_options(options) {
        preview_radius(state, original);
        return Err(error.to_string());
    }
    Ok(())
}

/// Update only memory and visuals during a continuous material gesture.
pub(super) fn preview_material(state: &mut PaneApp, backdrop: desktop_core::Backdrop) {
    if state
        .workspace
        .appearance()
        .is_some_and(|(_, old)| old == backdrop)
    {
        return;
    }
    let theme = state
        .workspace
        .appearance()
        .map_or(desktop_core::PanelTheme::System, |v| v.0);
    state.workspace.set_appearance(theme, backdrop);
    for view in &state.views {
        view.model.borrow_mut().backdrop = backdrop;
        unsafe {
            InvalidateRect(view.window.hwnd().cast(), std::ptr::null(), 0);
        }
    }
    if let Some(window) = &state.settings {
        unsafe {
            InvalidateRect(window.hwnd().cast(), std::ptr::null(), 0);
        }
    }
}

pub(super) fn commit_material(
    state: &mut PaneApp,
    original: desktop_core::Backdrop,
) -> Result<(), String> {
    if let Err(error) = save(state) {
        preview_material(state, original);
        return Err(error);
    }
    Ok(())
}
