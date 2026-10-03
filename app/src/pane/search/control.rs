//! UI-thread control mailbox. Window messages contain no pointers or external payloads.
use super::*;
use std::{collections::HashMap, rc::Weak};
pub(super) const MESSAGE: u32 = WM_APP + 0x4c5;
#[derive(Clone, Debug)]
pub(in crate::pane) enum Action {
    Query(String),
    Refresh,
    More,
}
#[derive(Default)]
pub(super) struct Mailbox {
    pending: Option<Action>,
    result: Option<Result<(), String>>,
    snapshot: serde_json::Value,
    entries: bool,
}
thread_local! { static MAILBOXES:RefCell<HashMap<isize,Weak<RefCell<Mailbox>>>>=RefCell::new(HashMap::new()); }
pub(super) fn register(hwnd: HWND, mailbox: &Rc<RefCell<Mailbox>>) {
    MAILBOXES.with(|all| {
        all.borrow_mut()
            .insert(hwnd as isize, Rc::downgrade(mailbox));
    });
}
pub(super) fn remove(hwnd: HWND) {
    MAILBOXES.with(|all| {
        all.borrow_mut().remove(&(hwnd as isize));
    });
}
fn mailbox(hwnd: HWND) -> Result<Rc<RefCell<Mailbox>>, String> {
    MAILBOXES
        .with(|all| all.borrow().get(&(hwnd as isize)).and_then(Weak::upgrade))
        .ok_or_else(|| "search window is unavailable".into())
}
pub(in crate::pane) fn snapshot(hwnd: HWND, entries: bool) -> Result<serde_json::Value, String> {
    let mailbox = mailbox(hwnd)?;
    mailbox.borrow_mut().entries = entries;
    unsafe {
        SendMessageW(hwnd, MESSAGE, 0, 0);
    }
    let result = mailbox.borrow().snapshot.clone();
    Ok(result)
}
pub(in crate::pane) fn execute(hwnd: HWND, action: Action) -> Result<serde_json::Value, String> {
    let mailbox = mailbox(hwnd)?;
    // SetWindowText synchronously notifies the parent. Do it before entering its search handler.
    if let Action::Query(query) = &action {
        let wide: Vec<u16> = query.encode_utf16().chain(Some(0)).collect();
        if unsafe { SetWindowTextW(edit(hwnd), wide.as_ptr()) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    {
        let mut m = mailbox.borrow_mut();
        m.pending = Some(action);
        m.result = None;
        m.entries = false;
    }
    unsafe {
        SendMessageW(hwnd, MESSAGE, 0, 0);
    }
    let mut m = mailbox.borrow_mut();
    m.result
        .take()
        .ok_or("search control message was not handled")??;
    Ok(m.snapshot.clone())
}
pub(super) fn receive(hwnd: HWND, state: &mut Search, mailbox: &Rc<RefCell<Mailbox>>) {
    let action = mailbox.borrow_mut().pending.take();
    if let Some(action) = action {
        let result = match action {
            Action::Query(query) => {
                if query.trim() != state.query {
                    state.change(query);
                    resize(hwnd, state);
                    invalidate(hwnd);
                }
                Ok(())
            }
            Action::Refresh => {
                state.change(text(edit(hwnd)));
                resize(hwnd, state);
                invalidate(hwnd);
                Ok(())
            }
            Action::More => {
                if state.busy {
                    Err("search is busy".into())
                } else {
                    if !state.failed
                        && !state.query.is_empty()
                        && state.entries.len() < state.total as usize
                    {
                        state.request(state.entries.len() as u32);
                    }
                    state.wake.notify();
                    Ok(())
                }
            }
        };
        mailbox.borrow_mut().result = Some(result);
    }
    let mut m = mailbox.borrow_mut();
    m.snapshot = serde_json::json!({"query":state.query,"input":text(edit(hwnd)),"generation":state.generation.to_string(),
        "busy":state.busy,"failed":state.failed,"status":state.status,"total":state.total,"loaded_count":state.entries.len(),
        "replacing":state.replacing,"has_more":state.entries.len()<(state.total as usize),
        "entries":if m.entries {Some(state.entries.iter().map(|e|serde_json::json!({"path":e.path,"is_folder":e.folder})).collect::<Vec<_>>())}else{None}});
}
