//! Desktop integration can recover without taking independent panes down.
use super::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub(super) struct State {
    pub path: PathBuf,
    pub desktop_error: Option<String>,
    last_attempt: Instant,
    pub layouts: display_layout::Layouts,
    last_backup: Instant,
    backup_changes: u64,
    backup_error: Option<String>,
}

impl State {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            desktop_error: None,
            last_attempt: Instant::now(),
            layouts: Default::default(),
            last_backup: Instant::now() - Duration::from_secs(300),
            backup_changes: 0,
            backup_error: None,
        }
    }
}

pub(super) fn status(s: &PaneApp) -> String {
    s.runtime
        .as_ref()
        .and_then(|r| r.desktop_error.as_ref())
        .map_or_else(
            || "桌面分组已连接".into(),
            |error| format!("桌面分组暂不可用，文件夹与搜索仍可使用。\n{error}"),
        )
}

pub(super) fn reconnect(state: &Rc<RefCell<PaneApp>>) {
    let path = {
        let mut s = state.borrow_mut();
        if s.session.is_some() {
            return;
        }
        let Some(runtime) = &mut s.runtime else {
            return;
        };
        runtime.last_attempt = Instant::now();
        runtime.path.clone()
    };
    let error = hybrid::connect(state, &path).err();
    let mut s = state.borrow_mut();
    if let Some(runtime) = &mut s.runtime {
        if runtime.desktop_error != error {
            if let Some(error) = &error {
                eprintln!("Desktop integration unavailable: {error}");
            }
        }
        runtime.desktop_error = error;
    }
    if let Some(settings) = &s.settings {
        unsafe {
            InvalidateRect(settings.hwnd().cast(), std::ptr::null(), 0);
        }
    }
}

fn suspend(state: &Rc<RefCell<PaneApp>>) {
    let removed = {
        let mut s = state.borrow_mut();
        s.session.take();
        if let Some(runtime) = &mut s.runtime {
            runtime.desktop_error = Some("Explorer 连接已断开，正在等待恢复".into());
        }
        let mut removed = Vec::new();
        let mut at = 0;
        while at < s.views.len() {
            let view = &s.views[at];
            if s.workspace
                .panel(view.id)
                .is_some_and(|p| p.folder().is_some() || p.is_search())
            {
                at += 1;
            } else {
                let view = s.views.remove(at);
                window::prepare_close(view.window.hwnd().cast());
                hybrid::unregister_drop(&mut s, view.window.hwnd().cast());
                removed.push(view);
            }
        }
        removed
    };
    drop(removed);
}

pub(super) fn maintain(state: &Rc<RefCell<PaneApp>>, force: bool) -> Result<(), String> {
    display_layout::tick(state)?;
    {
        let mut s = state.borrow_mut();
        let changes = s.store.change_count();
        let due = s.runtime.as_ref().is_some_and(|r| {
            r.last_backup.elapsed() >= Duration::from_secs(300) && r.backup_changes != changes
        });
        if due {
            let result = recovery::snapshot(&s, "auto");
            let runtime = s.runtime.as_mut().unwrap();
            runtime.last_backup = Instant::now();
            runtime.backup_error = result.err();
            if runtime.backup_error.is_none() {
                runtime.backup_changes = changes;
            }
            if let Some(error) = &runtime.backup_error {
                eprintln!("Configuration backup failed: {error}");
            }
        }
    }
    if state
        .borrow()
        .session
        .as_ref()
        .is_some_and(|session| !hybrid::is_alive(session))
    {
        suspend(state);
    }
    let due = state
        .borrow()
        .runtime
        .as_ref()
        .is_some_and(|r| r.last_attempt.elapsed() >= Duration::from_secs(10));
    if force || due {
        reconnect(state);
    }
    {
        let removed = {
            let mut s = state.borrow_mut();
            let mut removed = Vec::new();
            let mut at = 0;
            while at < s.views.len() {
                if unsafe { IsWindow(s.views[at].window.hwnd().cast()) } == 0 {
                    let view = s.views.remove(at);
                    hybrid::unregister_drop(&mut s, view.window.hwnd().cast());
                    removed.push(view);
                } else {
                    at += 1;
                }
            }
            removed
        };
        drop(removed);
        let ids: Vec<_> = {
            let s = state.borrow();
            let search_enabled = s
                .workspace
                .panels()
                .iter()
                .any(|p| p.is_search() && !s.views.iter().any(|v| v.id == p.id()))
                && everything_settings::enabled(&s.store)?;
            s.workspace
                .panels()
                .iter()
                .filter(|p| {
                    (if p.is_search() {
                        search_enabled
                    } else {
                        p.folder().is_some() || s.session.is_some()
                    }) && !s.views.iter().any(|v| v.id == p.id())
                })
                .map(Panel::id)
                .collect()
        };
        for id in ids {
            create_view(state, id)?;
        }
    }
    Ok(())
}

pub(super) fn backup_status(s: &PaneApp) -> String {
    s.runtime
        .as_ref()
        .and_then(|r| r.backup_error.as_ref())
        .map_or_else(String::new, |error| format!("自动备份失败：{error}"))
}

pub(super) fn reload(state: &Rc<RefCell<PaneApp>>) -> Result<(), String> {
    let views = {
        let mut s = state.borrow_mut();
        let workspace = s.store.load_workspace().map_err(|e| e.to_string())?;
        peek::load(&s.store)?;
        search_hotkey::load(&s.store)?;
        everything_settings::load(&s.store)?;
        s.session.take();
        s.drops.clear();
        s.folders.clear();
        s.images.clear();
        for v in &s.views {
            window::prepare_close(v.window.hwnd().cast());
        }
        s.workspace = workspace;
        s.runtime.as_mut().unwrap().layouts = Default::default();
        display_layout::initialize(&mut s, desktop_window::enumerate_monitors())?;
        std::mem::take(&mut s.views)
    };
    drop(views);
    reconnect(state);
    let ids: Vec<_> = {
        let s = state.borrow();
        let search_enabled = everything_settings::enabled(&s.store)?;
        s.workspace
            .panels()
            .iter()
            .filter(|p| {
                if p.is_search() {
                    search_enabled
                } else {
                    p.folder().is_some() || s.session.is_some()
                }
            })
            .map(Panel::id)
            .collect()
    };
    for id in ids {
        create_view(state, id)?;
    }
    if let Some(settings) = &state.borrow().settings {
        unsafe {
            InvalidateRect(settings.hwnd().cast(), std::ptr::null(), 0);
        }
    }
    Ok(())
}

pub(super) fn supervisor(state: &Rc<RefCell<PaneApp>>) -> Result<windows_window::Window, String> {
    let weak = Rc::downgrade(state);
    let wake = state.borrow().wake.clone();
    let received = wake.clone();
    let mut hotkey = search_hotkey::Registration::default();
    let show_message = unsafe { RegisterWindowMessageW(windows_sys::w!("LucidPane.ShowExisting")) };
    let window = windows_window::Window::new("LucidPane Runtime")
        .size(1, 1)
        .style(WS_POPUP)
        .ex_style(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)
        .on_message(move |raw, msg, wp, _| {
            if msg == WM_DESTROY {
                received.unbind();
                hotkey.update(raw as isize, None);
                return Some(0);
            }
            if msg == WM_HOTKEY && wp == search_hotkey::ID as usize {
                if let Some(state) = weak.upgrade() {
                    search_hotkey::activate(&state);
                }
                return Some(0);
            }
            if msg == show_message {
                if let Some(state) = weak.upgrade() {
                    let s = state.borrow();
                    for view in &s.views {
                        unsafe {
                            ShowWindow(view.window.hwnd().cast(), SW_SHOWNOACTIVATE);
                        }
                    }
                    if let Some(view) = s.views.first() {
                        unsafe {
                            SetForegroundWindow(view.window.hwnd().cast());
                        }
                    } else if let Some(settings) = &s.settings {
                        unsafe {
                            ShowWindow(settings.hwnd().cast(), SW_RESTORE);
                            SetForegroundWindow(settings.hwnd().cast());
                        }
                    }
                }
                return Some(0);
            }
            if msg != WM_TIMER && msg != super::wake::READY {
                return None;
            }
            if msg == super::wake::READY {
                received.received();
            }
            if let Some(state) = weak.upgrade() {
                if state.try_borrow_mut().is_ok() {
                    {
                        let mut s = state.borrow_mut();
                        folder::poll(&mut s);
                        if s.session.is_some() {
                            if let Err(error) = hybrid::tick(&mut s) {
                                eprintln!("Desktop synchronization: {error}");
                            }
                        }
                        unsafe {
                            KillTimer(raw.cast(), 2);
                            if let Some(delay) = hybrid::next_work(&s) {
                                SetTimer(raw.cast(), 2, delay, None);
                            }
                        }
                    }
                    if msg != WM_TIMER || wp != 1 {
                        return Some(0);
                    }
                    let previous = {
                        let s = state.borrow();
                        (status(&s), backup_status(&s), search_hotkey::status())
                    };
                    if let Err(error) = maintain(&state, false) {
                        eprintln!("Runtime recovery: {error}");
                    }
                    let s = state.borrow();
                    let enabled = s
                        .views
                        .iter()
                        .any(|v| s.workspace.panel(v.id).is_some_and(Panel::is_search))
                        && everything_settings::enabled(&s.store).unwrap_or(false);
                    hotkey.update(raw as isize, enabled.then(search_hotkey::settings));
                    if previous != (status(&s), backup_status(&s), search_hotkey::status())
                        && let Some(settings) = &s.settings
                    {
                        unsafe {
                            InvalidateRect(settings.hwnd().cast(), std::ptr::null(), 0);
                        }
                    }
                } else {
                    // Nested COM/menu callbacks can temporarily hold PaneApp.
                    // Retry the coalesced notification instead of dropping it.
                    unsafe {
                        SetTimer(raw.cast(), 2, 25, None);
                    }
                }
            }
            Some(0)
        })
        .create()
        .map_err(|e| e.to_string())?;
    unsafe {
        ShowWindow(window.hwnd().cast(), SW_HIDE);
        SetTimer(window.hwnd().cast(), 1, 1000, None);
    }
    wake.bind(window.hwnd() as isize);
    Ok(window)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_completion_reaches_supervisor_without_timer_polling() {
        let root = std::env::temp_dir().join(format!(
            "lucidpane-wake-folder-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let state = Rc::new(RefCell::new(super::super::tests::test_state()));
        let id = PanelId::new(2);
        state
            .borrow_mut()
            .workspace
            .panel_mut(id)
            .unwrap()
            .set_folder(Some(root.clone()));
        folder::ensure(&mut state.borrow_mut(), id).unwrap();
        let supervisor = supervisor(&state).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while state.borrow().folders[&id].loading && Instant::now() < deadline {
            unsafe {
                let mut msg = MSG::default();
                // Deliberately do not dispatch WM_TIMER: the worker notification
                // must deliver the initial snapshot even if it finished pre-bind.
                while PeekMessageW(
                    &raw mut msg,
                    supervisor.hwnd().cast(),
                    super::super::wake::READY,
                    super::super::wake::READY,
                    PM_REMOVE,
                ) != 0
                {
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!state.borrow().folders[&id].loading);
        assert!(state.borrow().folders[&id].status.is_none());
        drop(supervisor);
        state.borrow_mut().folders.clear();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn suspended_desktop_preserves_membership_and_independent_panes() {
        let _sta = ShellApartment::initialize_sta().unwrap();
        let state = Rc::new(RefCell::new(super::super::tests::test_state()));
        create_view(&state, PanelId::new(1)).unwrap();
        handle(
            &state,
            PanelId::new(0),
            Event::MapFolder(std::env::temp_dir()),
        )
        .unwrap();
        handle(&state, PanelId::new(0), Event::EnableSearch).unwrap();
        let before = state.borrow().workspace.clone();
        let independent: Vec<_> = state
            .borrow()
            .views
            .iter()
            .filter(|v| v.id != PanelId::new(1))
            .map(|v| (v.id, v.window.hwnd()))
            .collect();
        assert_eq!(
            state.borrow().drops.len(),
            1,
            "folder drops must register without a desktop session"
        );
        suspend(&state);
        assert_eq!(state.borrow().workspace, before);
        assert_eq!(state.borrow().views.len(), 2);
        for (id, hwnd) in independent {
            assert!(
                state
                    .borrow()
                    .views
                    .iter()
                    .any(|v| v.id == id && v.window.hwnd() == hwnd)
            );
            unsafe {
                SendMessageW(hwnd.cast(), WM_DISPLAYCHANGE, 0, 0);
            }
        }
        let mut msg = MSG::default();
        assert_eq!(
            unsafe {
                PeekMessageW(
                    &raw mut msg,
                    std::ptr::null_mut(),
                    WM_QUIT,
                    WM_QUIT,
                    PM_REMOVE,
                )
            },
            0
        );
        let mut s = state.borrow_mut();
        s.drops.clear();
        for view in &s.views {
            window::prepare_close(view.window.hwnd().cast());
        }
    }
}
