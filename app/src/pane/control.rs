//! CLI dispatcher. Requests use a dedicated message, not the maintenance wake.
mod plans;
use super::*;
use desktop_api::{Request, Response};
use serde_json::json;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
const READY: u32 = WM_APP + 0x4c3;
struct Pending {
    request: Request,
    reply: mpsc::SyncSender<Response>,
    expires: Instant,
}
struct Snapshot {
    instance: String,
    item_ids: HashMap<String, String>,
    next_item: u64,
    seen: Option<(Workspace, u64, String)>,
    version: u64,
    plans: plans::Plans,
}
impl Snapshot {
    fn new() -> Self {
        Self {
            instance: desktop_api::request_id(),
            item_ids: HashMap::new(),
            next_item: 0,
            seen: None,
            version: 0,
            plans: plans::Plans::default(),
        }
    }
    fn respond(&mut self, state: &mut PaneApp, request: &Request) -> Response {
        let fail = |code, message| Response::failure(&request.request_id, code, message);
        if request.protocol_version != desktop_api::VERSION {
            return fail("PROTOCOL_MISMATCH", "supported protocol version is 1");
        }
        if let Err(error) = request.validate() {
            return fail("INVALID_REQUEST", error);
        }
        let directory = state.runtime.as_ref().and_then(|r| r.path.parent());
        if let Some(expected) = &request.data_dir {
            let actual = directory.and_then(|p| std::fs::canonicalize(p).ok());
            let expected = std::fs::canonicalize(expected).ok();
            if actual.is_none() || expected.is_none() || actual != expected {
                return fail(
                    "DATA_DIR_MISMATCH",
                    "connected GUI uses a different data directory",
                );
            }
        }
        let observed = (
            state.workspace.clone(),
            state.store.change_count(),
            format!("{:?}", desktop_window::enumerate_monitors()),
        );
        if self.seen.as_ref() != Some(&observed) {
            self.version += 1;
            self.seen = Some(observed);
        }
        let version = self.version.to_string();
        let base = desktop_api::Context {
            instance_id: self.instance.clone(),
            state_version: version.clone(),
            inventory_version: version.clone(),
            topology_token: version,
        };
        let context = json!(base);
        if matches!(
            request.command.as_str(),
            "plan.preview" | "plan.apply" | "request.get"
        ) {
            return self.plans.handle(state, &base, &self.item_ids, request);
        }
        let data = match request.command.as_str() {
            "status" => {
                json!({"application_version":env!("CARGO_PKG_VERSION"), "data_dir":directory, "desktop_connected":state.session.as_ref().is_some_and(hybrid::is_alive), "read_only":false})
            }
            "capabilities" => {
                json!({"commands":desktop_api::COMMANDS,"protocol_version":1,"max_frame_bytes":desktop_api::MAX_FRAME,"writes":true,"plans":true,"concurrency_tokens":true,"plan_operations":["pane.create","pane.update","item.assign","item.reorder"],"pane_geometry":false,"item_ids":"opaque-instance-scoped","schema_version":1})
            }
            command => {
                let pane_values: Vec<_> = state.workspace.panels().iter().map(|panel| {
                    let effective = state.views.iter().find(|v| v.id == panel.id()).map(|v| v.model.borrow().collapsed)
                        .or_else(|| state.workspace.tab_group(panel.id()).and_then(|g| state.views.iter().find(|v| v.id == g.active)).map(|v| v.model.borrow().collapsed));
                    json!({"id":panel.id().get().to_string(),"title":panel.title(),
                        "kind":if panel.is_search(){"search"} else if panel.folder().is_some(){"folder"} else {"desktop"},
                        "folder_path":panel.folder(),"locked":panel.locked(),"auto_hide":panel.auto_hide(),
                        "manual_collapsed":panel.collapsed(),"effective_collapsed":effective,
                        "list_view":panel.list_view(),"always_on_top":panel.always_on_top()})
                }).collect();
                if command == "pane.get" {
                    match pane_values
                        .into_iter()
                        .find(|p| Some(p["id"].as_str().unwrap()) == request.id.as_deref())
                    {
                        Some(pane) => pane,
                        None => return fail("NOT_FOUND", "panel does not exist"),
                    }
                } else if command == "pane.list" {
                    json!({"panes":pane_values})
                } else {
                    if let Some(id) = &request.pane {
                        if !state
                            .workspace
                            .panels()
                            .iter()
                            .any(|p| p.id().get().to_string() == *id)
                        {
                            return fail("NOT_FOUND", "panel does not exist");
                        }
                    }
                    let mut live = std::collections::HashSet::new();
                    let mut items = Vec::new();
                    for item in state.workspace.desktop_items() {
                        let key = item.identity().persistent_key();
                        live.insert(key.clone());
                        let id = self.item_ids.entry(key).or_insert_with(|| {
                            self.next_item += 1;
                            format!("{}-item-{}", self.instance, self.next_item)
                        });
                        let (pane, placement) = match item.placement() {
                            desktop_core::DesktopPlacement::Pane { pane_id, position } => (
                                Some(pane_id.get().to_string()),
                                json!({"kind":"pane","pane_id":pane_id.get().to_string(),"column":position.column,"row":position.row}),
                            ),
                            desktop_core::DesktopPlacement::FreeDesktop { .. } => {
                                (None, json!({"kind":"desktop"}))
                            }
                        };
                        if request.unassigned && pane.is_some()
                            || request.pane.is_some() && request.pane != pane
                        {
                            continue;
                        }
                        items.push(json!({"id":id,"display_name":item.display_name(),"path":item.identity().file_system_path(),"placement":placement}));
                    }
                    self.item_ids.retain(|key, _| live.contains(key));
                    if command == "item.list" {
                        json!({"items":items,"inventory_source":"current_app_snapshot"})
                    } else {
                        let tabs:Vec<_> = state.workspace.tab_groups().iter().map(|g| json!({"active":g.active.get().to_string(),"members":g.members.iter().map(|id| id.get().to_string()).collect::<Vec<_>>()})).collect();
                        json!({"panes":pane_values,"items":items,"tabs":tabs,"inventory_source":"current_app_snapshot"})
                    }
                }
            }
        };
        Response::success(&request.request_id, context, data)
    }
}

pub(super) fn start(state: &Rc<RefCell<PaneApp>>) -> Result<windows_window::Window, String> {
    start_at(
        state,
        &desktop_api::transport::endpoint().map_err(|e| e.to_string())?,
    )
}
fn start_at(
    state: &Rc<RefCell<PaneApp>>,
    pipe_name: &str,
) -> Result<windows_window::Window, String> {
    let (send, receive) = mpsc::sync_channel::<Pending>(16);
    let endpoint = Arc::new(Mutex::new(0isize));
    let stopped = Arc::new(AtomicBool::new(false));
    let peer = endpoint.clone();
    let stopping = stopped.clone();
    let server = desktop_api::transport::Server::start_at(pipe_name, move |request| {
        let id = request.request_id.clone();
        let (reply, result) = mpsc::sync_channel(1);
        let expires = Instant::now() + Duration::from_secs(5);
        if send
            .try_send(Pending {
                request,
                reply,
                expires,
            })
            .is_err()
        {
            return Response::failure(&id, "BUSY", "control queue is full");
        }
        {
            let hwnd = peer.lock().unwrap();
            if *hwnd == 0 {
                return Response::failure(&id, "BUSY", "control window is not ready");
            }
            unsafe {
                PostMessageW(*hwnd as _, READY, 0, 0);
            }
        }
        while Instant::now() < expires && !stopping.load(Ordering::Acquire) {
            match result.recv_timeout(Duration::from_millis(50)) {
                Ok(response) => return response,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => (),
            }
        }
        Response::failure(
            &id,
            "TIMEOUT",
            "UI did not answer within the server deadline",
        )
    })
    .map_err(|e| e.to_string())?;
    let mut server = Some(server);
    let owner = endpoint.clone();
    let weak = Rc::downgrade(state);
    let mut snapshot = Snapshot::new();
    let window = windows_window::Window::new("LucidDesk Control")
        .size(1, 1)
        .style(WS_POPUP)
        .ex_style(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)
        .on_message(move |raw, msg, _, lp| {
            if unsafe { crate::window_visibility::defer_show(msg, lp, false) } {
                return Some(0);
            }
            if msg == WM_DESTROY {
                *owner.lock().unwrap() = 0;
                stopped.store(true, Ordering::Release);
                server.take();
                return Some(0);
            }
            if msg != READY && msg != WM_TIMER {
                return None;
            }
            unsafe {
                KillTimer(raw.cast(), 1);
            }
            if let Some(state) = weak.upgrade() {
                if state.try_borrow_mut().is_ok() {
                    while let Ok(pending) = receive.try_recv() {
                        if Instant::now() < pending.expires {
                            let previous = state.borrow().store.change_count();
                            let response =
                                snapshot.respond(&mut state.borrow_mut(), &pending.request);
                            if state.borrow().store.change_count() != previous {
                                present(&state);
                            }
                            let _ = pending.reply.send(response);
                        }
                    }
                } else {
                    unsafe {
                        SetTimer(raw.cast(), 1, 25, None);
                    }
                }
            }
            Some(0)
        })
        .create()
        .map_err(|e| e.to_string())?;
    *endpoint.lock().unwrap() = window.hwnd() as isize;
    Ok(window)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queries_preserve_database_and_config_and_reject_invalid_requests() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.db");
        let mut state = super::super::tests::test_state();
        state.store = WorkspaceStore::open(&path).unwrap();
        state.store.save_workspace(&state.workspace).unwrap();
        state.runtime = Some(runtime::State::new(path.clone()));
        let db = std::fs::read(&path).unwrap();
        let config = std::fs::read(dir.path().join("config.toml")).unwrap();
        let changes = state.store.change_count();
        let mut snapshot = Snapshot::new();
        let mut request = Request {
            protocol_version: 1,
            request_id: "test".into(),
            command: "status".into(),
            id: None,
            pane: None,
            unassigned: false,
            data_dir: None,
            plan: None,
            token: None,
        };
        for command in desktop_api::COMMANDS
            .iter()
            .filter(|c| !matches!(**c, "plan.preview" | "plan.apply" | "request.get"))
        {
            request.command = (*command).into();
            request.id = (*command == "pane.get").then(|| "1".into());
            assert!(snapshot.respond(&mut state, &request).ok, "{command}");
        }
        request.command = "status".into();
        request.id = None;
        request.protocol_version = 2;
        assert_eq!(snapshot.respond(&mut state, &request).exit_code(), 10);
        request.protocol_version = 1;
        request.data_dir = Some(dir.path().join("missing").to_string_lossy().into_owned());
        assert_eq!(snapshot.respond(&mut state, &request).exit_code(), 5);
        assert_eq!(state.store.change_count(), changes);
        assert_eq!(std::fs::read(path).unwrap(), db);
        assert_eq!(
            std::fs::read(dir.path().join("config.toml")).unwrap(),
            config
        );
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    #[test]
    fn pipe_query_runs_on_ui_thread_without_persistence() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let mut app = super::super::tests::test_state();
        app.store.save_workspace(&app.workspace).unwrap();
        let before = app.store.change_count();
        let state = Rc::new(RefCell::new(app));
        let name = format!(
            "{}-ui-{}",
            desktop_api::transport::endpoint().unwrap(),
            desktop_api::request_id()
        );
        let control = start_at(&state, &name).unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let request = Request {
                protocol_version: 1,
                request_id: "ui-integration".into(),
                command: "workspace.get".into(),
                id: None,
                pane: None,
                unassigned: false,
                data_dir: None,
                plan: None,
                token: None,
            };
            tx.send(desktop_api::transport::call_at(
                &name,
                &request,
                Duration::from_secs(3),
            ))
            .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(4);
        let response = loop {
            if let Ok(response) = rx.try_recv() {
                break response.unwrap();
            }
            assert!(Instant::now() < deadline, "CLI response timed out");
            unsafe {
                let mut msg = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(response.ok);
        assert_eq!(
            response.data.unwrap()["panes"].as_array().unwrap().len(),
            state.borrow().workspace.panels().len()
        );
        assert_eq!(state.borrow().store.change_count(), before);
        worker.join().unwrap();
        drop(control);
    }
}

/// Presentation follows durable state. Failures never turn a committed plan into a retryable write.
fn present(state: &Rc<RefCell<PaneApp>>) {
    let missing: Vec<_> = {
        let s = state.borrow();
        s.workspace
            .panels()
            .iter()
            .filter(|p| {
                p.supports_tabs()
                    && s.workspace.tab_visible(p.id())
                    && !s.views.iter().any(|v| v.id == p.id())
            })
            .map(Panel::id)
            .collect()
    };
    for id in missing {
        if let Err(error) = create_view(state, id) {
            crate::diagnostics::log(crate::diagnostics::Level::Error, "cli.presentation", &error);
        }
    }
    let mut s = state.borrow_mut();
    for view in &s.views {
        if let Some(panel) = s.workspace.panel(view.id) {
            view.model.borrow_mut().title = panel.title().to_owned();
            unsafe {
                windows_sys::Win32::Graphics::Gdi::InvalidateRect(
                    view.window.hwnd().cast(),
                    std::ptr::null(),
                    0,
                );
            }
        }
    }
    refresh_views(&mut s);
    if let Err(error) = hybrid::sync(&mut s) {
        crate::diagnostics::log(crate::diagnostics::Level::Error, "cli.presentation", &error);
    }
    s.wake.notify();
}
