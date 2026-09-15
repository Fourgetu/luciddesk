//! One Explorer STA per filter session. Reuse idle menu hosts and keep command
//! dialogs alive when changing targets; all Shell objects stay on this thread.
use super::{super::wire::MenuContext, MenuHost, selection::ResolvedTargets};
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use windows::{Win32::Foundation::E_FAIL, core::Result};
use windows_sys::Win32::{
    System::{Com::*, LibraryLoader::*},
    UI::WindowsAndMessaging::*,
};

type Reply<T> = mpsc::SyncSender<std::result::Result<T, i32>>;
enum Request {
    Prepare {
        names: Vec<String>,
        context: MenuContext,
        reply: Reply<isize>,
        cancelled: Arc<AtomicBool>,
    },
    Finish,
    Cancel {
        reply: Reply<()>,
    },
}
pub struct Worker {
    requests: mpsc::Sender<Request>,
    thread: std::thread::JoinHandle<()>,
    active: RefCell<Option<ActiveInvocation>>,
}

struct ActiveInvocation {
    cancelled: Arc<AtomicBool>,
    hwnd: isize,
}
impl ActiveInvocation {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        unsafe {
            PostMessageW(self.hwnd as _, WM_CANCELMODE, 0, 0);
        }
    }
}
impl Worker {
    pub fn create(desktop: windows_sys::Win32::Foundation::HWND) -> Result<Self> {
        let desktop = desktop as isize;
        let (requests, receiver) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("LucidPane native menu".into())
            .spawn(move || unsafe {
                let initialized = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
                if initialized < 0 {
                    return;
                }
                let mut current: Option<MenuHost> = None;
                let mut retired: Vec<MenuHost> = Vec::new();
                let mut last_cleanup = Instant::now();
                'worker: loop {
                    loop {
                        match receiver.try_recv() {
                            Ok(Request::Prepare {
                                names,
                                context,
                                reply,
                                cancelled,
                            }) => {
                                if cancelled.load(Ordering::Acquire) {
                                    continue;
                                }
                                let result = prepare_host(
                                    desktop as _,
                                    &mut current,
                                    &mut retired,
                                    &names,
                                    context,
                                    &cancelled,
                                )
                                .map_err(|error| error.code().0);
                                if cancelled.load(Ordering::Acquire) {
                                    if let Some(host) = &current {
                                        let _ = host.finish(true);
                                    }
                                    continue;
                                }
                                let _ = reply.send(result);
                            }
                            Ok(Request::Finish) => {
                                if let Some(host) = &current {
                                    let _ = host.finish(false);
                                }
                            }
                            Ok(Request::Cancel { reply }) => {
                                let result =
                                    current.as_ref().map_or(Ok(()), |host| host.finish(true));
                                let _ = reply.send(result.map_err(|e| e.code().0));
                            }
                            Err(mpsc::TryRecvError::Disconnected) => break 'worker,
                            Err(mpsc::TryRecvError::Empty) => break,
                        }
                    }
                    if !pump_messages() {
                        break;
                    }
                    if last_cleanup.elapsed() >= Duration::from_secs(1) {
                        retired.retain(MenuHost::has_owned_windows);
                        last_cleanup = Instant::now();
                    }
                    MsgWaitForMultipleObjects(0, std::ptr::null(), 0, 10, QS_ALLINPUT);
                }
                drop(current);
                drop(retired);
                CoUninitialize();
            })
            .map_err(|error| windows::core::Error::new(E_FAIL, error.to_string()))?;
        Ok(Self {
            requests,
            thread,
            active: RefCell::new(None),
        })
    }
    pub fn is_alive(&self) -> bool {
        !self.thread.is_finished()
    }
    pub fn prepare(&self, names: &[String], context: MenuContext) -> Result<isize> {
        let (reply, result) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        *self.active.borrow_mut() = Some(ActiveInvocation {
            cancelled: cancelled.clone(),
            hwnd: 0,
        });
        self.requests
            .send(Request::Prepare {
                names: names.to_vec(),
                context,
                reply,
                cancelled: cancelled.clone(),
            })
            .map_err(|_| windows::core::Error::from_hresult(E_FAIL))?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match result.try_recv() {
                Ok(Ok(hwnd)) => {
                    if let Some(active) = self.active.borrow_mut().as_mut() {
                        active.hwnd = hwnd;
                    }
                    return Ok(hwnd);
                }
                Ok(Err(code)) => return Err(windows::core::HRESULT(code).into()),
                Err(mpsc::TryRecvError::Disconnected) => return Err(E_FAIL.into()),
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if Instant::now() >= deadline {
                cancelled.store(true, Ordering::Release);
                return Err(windows::core::Error::new(E_FAIL, "原生菜单准备超时"));
            }
            wait_for_shell_reply();
        }
    }
    pub fn finish(&self, cancel: bool) -> Result<()> {
        // Properties may still own a modal loop on the Shell STA. Normal
        // dismissal only queues cleanup and never allocates an unused reply.
        if !cancel {
            return self
                .requests
                .send(Request::Finish)
                .map_err(|_| windows::core::Error::from_hresult(E_FAIL));
        }
        if let Some(active) = self.active.borrow().as_ref() {
            active.cancel();
        }
        let (reply, result) = mpsc::sync_channel(1);
        self.requests
            .send(Request::Cancel { reply })
            .map_err(|_| windows::core::Error::from_hresult(E_FAIL))?;
        // Return only after the menu STA has dismissed/closed the presenter.
        // If an extension stalls, its atomic token still prevents later display
        // and command invocation; never forcibly terminate Explorer's thread.
        let start = Instant::now();
        loop {
            match result.try_recv() {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(code)) => return Err(windows::core::HRESULT(code).into()),
                Err(mpsc::TryRecvError::Disconnected) => return Err(E_FAIL.into()),
                _ => {}
            }
            if start.elapsed() >= Duration::from_secs(2) {
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "菜单取消等待超时，已保留取消标记",
                ));
            }
            wait_for_shell_reply();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(active) = self.active.get_mut() {
            active.cancel();
        }
    }
}

/// Runs only on the menu STA; resolve before replacing a useful host and keep
/// hosts with outstanding command dialogs alive until the periodic cleanup.
fn prepare_host(
    desktop: windows_sys::Win32::Foundation::HWND,
    current: &mut Option<MenuHost>,
    retired: &mut Vec<MenuHost>,
    names: &[String],
    context: MenuContext,
    cancelled: &Arc<AtomicBool>,
) -> Result<isize> {
    if current.as_ref().is_some_and(MenuHost::is_busy) {
        return Err(
            windows::core::HRESULT::from_win32(windows::Win32::Foundation::ERROR_BUSY.0).into(),
        );
    }
    let targets = ResolvedTargets::resolve(names)?;
    if let Some(host) = current.as_ref() {
        if host.reprepare(&targets, context, cancelled.clone())? {
            return host.view_hwnd();
        }
    }
    // Tear down an idle presenter before initializing its replacement.
    if let Some(previous) = current.take() {
        if previous.has_owned_windows() {
            retired.push(previous);
        }
    }
    let next = MenuHost::create(desktop, &targets, context, cancelled.clone())?;
    let hwnd = next.view_hwnd()?;
    *current = Some(next);
    Ok(hwnd)
}

/// Wait on the desktop thread while filter state is borrowed. Service only
/// synchronous COM calls; dispatching posted filter requests would reenter it.
fn wait_for_shell_reply() {
    unsafe {
        let mut msg = MSG::default();
        PeekMessageW(
            &raw mut msg,
            std::ptr::null_mut(),
            WM_NULL,
            WM_NULL,
            PM_NOREMOVE,
        );
        MsgWaitForMultipleObjectsEx(0, std::ptr::null(), 5, QS_SENDMESSAGE, MWMO_INPUTAVAILABLE);
    }
}

// Runs on the independent menu STA, where WinUI and posted input must dispatch.
fn pump_messages() -> bool {
    unsafe {
        let mut msg = MSG::default();
        for _ in 0..32 {
            if PeekMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) == 0 {
                break;
            }
            if msg.message == WM_QUIT {
                return false;
            }
            let module = GetModuleHandleW(windows_sys::w!("Microsoft.UI.Windowing.Core.dll"));
            let consumed = GetProcAddress(module, windows_sys::s!("ContentPreTranslateMessage"))
                .is_some_and(|proc| {
                    let translate: unsafe extern "system" fn(*const MSG) -> i32 =
                        std::mem::transmute(proc);
                    translate(&msg) != 0
                });
            if !consumed {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_worker_does_not_block_normal_finish_and_cancel_propagates_close_failure() {
        let (requests, receiver) = mpsc::channel();
        let (unblock, modal) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            // Simulate a Properties dialog keeping the Shell worker occupied.
            modal.recv().unwrap();
            assert!(matches!(receiver.recv().unwrap(), Request::Finish));
            let Request::Cancel { reply } = receiver.recv().unwrap() else {
                panic!("Expected a cancellation acknowledgement request");
            };
            reply
                .send(Err(windows::Win32::Foundation::E_ACCESSDENIED.0))
                .unwrap();
        });
        let worker = Worker {
            requests,
            thread,
            active: RefCell::new(None),
        };
        worker.finish(false).unwrap();
        unblock.send(()).unwrap();
        assert_eq!(
            worker.finish(true).unwrap_err().code(),
            windows::Win32::Foundation::E_ACCESSDENIED
        );
    }
}
