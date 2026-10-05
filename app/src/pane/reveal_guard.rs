//! Return revealed panes to the desktop band when another application takes focus.
//!
//! `quick_reveal` raises the panes above ordinary windows on demand. Without this
//! module they stayed there until something else happened to move them, so they
//! kept covering the window the user clicked next. A system foreground event is
//! the signal that the user moved on; the callback is deliberately minimal and
//! only posts a notification, because it runs inside the shell's event dispatch
//! and must not borrow application state.
//!
//! The hook is registered out of context, so Windows delivers callbacks on the
//! thread that installed it — the thread already running the pane message loop.
//! `WINEVENT_SKIPOWNPROCESS` keeps the reveal's own activation from being read as
//! "the user left", and clicking another pane of this same process therefore
//! keeps the panes revealed.

use std::cell::Cell;
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::Accessibility::{
    HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, PostMessageW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

thread_local! {
    static HOOK: Cell<HWINEVENTHOOK> = const { Cell::new(std::ptr::null_mut()) };
    /// Window that receives the notification, and the message to post to it.
    static TARGET: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) };
    static NOTIFY: Cell<u32> = const { Cell::new(0) };
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    event: u32,
    _hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    // Only a top-level window becoming foreground is of interest; the window-level
    // and object-level variants would fire for sub-elements of the same switch.
    if event != EVENT_SYSTEM_FOREGROUND || object != 0 || child != 0 {
        return;
    }
    let target = TARGET.with(Cell::get);
    let notify = NOTIFY.with(Cell::get);
    if target.is_null() || notify == 0 {
        return;
    }
    // SAFETY: posting is inert. `target` is the runtime window of this thread's
    // message loop, and the notification carries no payload.
    unsafe {
        PostMessageW(target, notify, 0, 0 as LPARAM);
    }
}

/// Watches foreground changes, notifying `target` with `notify`. Reinstalling
/// replaces the previous target; the hook itself is shared and installed once.
pub(super) fn install(target: HWND, notify: u32) {
    if target.is_null() || notify == 0 {
        return;
    }
    TARGET.with(|slot| slot.set(target));
    NOTIFY.with(|slot| slot.set(notify));
    if !HOOK.with(Cell::get).is_null() {
        return;
    }
    // SAFETY: an out-of-context hook requires a null module handle, and the
    // callback outlives it because the hook is removed by `remove`.
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    if !hook.is_null() {
        HOOK.with(|slot| slot.set(hook));
    }
}

/// Stops watching. State is cleared first so a racing callback cannot post to a
/// window that is being destroyed.
pub(super) fn remove() {
    TARGET.with(|slot| slot.set(std::ptr::null_mut()));
    NOTIFY.with(|slot| slot.set(0));
    let hook = HOOK.with(Cell::get);
    if hook.is_null() {
        return;
    }
    // SAFETY: the handle came from `install` and is cleared immediately after, so
    // it is unhooked exactly once.
    unsafe {
        UnhookWinEvent(hook);
    }
    HOOK.with(|slot| slot.set(std::ptr::null_mut()));
}
