//! Deferred callbacks; always release queue borrows before executing or dropping captures.
use super::*;

thread_local! {
    static DEFERRED: RefCell<std::collections::HashMap<usize, Box<dyn FnOnce()>>> = RefCell::new(std::collections::HashMap::new());
    static POSTED: RefCell<std::collections::HashMap<(isize, usize), Box<dyn FnOnce()>>> = RefCell::new(std::collections::HashMap::new());
    static NEXT_ACTION: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Dispatch in the subclass, outside the windows-window callback, so Shell can
// pump messages without detaching the pane's normal event handler.
pub(in crate::pane) fn post_action(hwnd: HWND, action: impl FnOnce() + 'static) -> bool {
    let Some(token) = NEXT_ACTION.with(|next| {
        let token = next.get().checked_add(1)?;
        next.set(token);
        Some(token)
    }) else {
        return false;
    };
    let key = (hwnd as isize, token);
    POSTED.with(|queue| queue.borrow_mut().insert(key, Box::new(action)));
    if unsafe { PostMessageW(hwnd, RUN_POSTED_ACTION, token, 0) } == 0 {
        let action = POSTED.with(|queue| queue.borrow_mut().remove(&key));
        drop(action);
        return false;
    }
    true
}

// A thread timer runs after the current window callback has returned. Modal
// menus then pump pane messages with its windows-window handler installed.
pub(in crate::pane) fn defer_action(action: impl FnOnce() + 'static) -> bool {
    unsafe extern "system" fn dispatch(_: HWND, _: u32, timer: usize, _: u32) {
        unsafe {
            KillTimer(std::ptr::null_mut(), timer);
        }
        let action = DEFERRED.with(|queue| queue.borrow_mut().remove(&timer));
        if let Some(action) = action {
            action();
        }
    }
    let timer = unsafe { SetTimer(std::ptr::null_mut(), 0, USER_TIMER_MINIMUM, Some(dispatch)) };
    if timer == 0 {
        return false;
    }
    DEFERRED.with(|queue| {
        queue.borrow_mut().insert(timer, Box::new(action));
    });
    true
}

pub(super) fn run(hwnd: HWND, wparam: usize) {
    let action = POSTED.with(|queue| queue.borrow_mut().remove(&(hwnd as isize, wparam)));
    if let Some(action) = action {
        action();
    }
}

pub(super) fn cancel(hwnd: HWND) {
    // Release cancelled closures outside the queue borrow; their captured
    // values may themselves destroy windows and reenter this procedure.
    let cancelled = POSTED.with(|queue| {
        let mut queue = queue.borrow_mut();
        let keys: Vec<_> = queue
            .keys()
            .copied()
            .filter(|key| key.0 == hwnd as isize)
            .collect();
        keys.into_iter()
            .filter_map(|key| queue.remove(&key))
            .collect::<Vec<_>>()
    });
    drop(cancelled);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn running_action_releases_queue_and_runs_only_once() {
        let called = Rc::new(std::cell::Cell::new(0));
        let observed = called.clone();
        POSTED.with(|queue| {
            queue.borrow_mut().insert(
                (0, 7),
                Box::new(move || {
                    POSTED.with(|queue| assert!(queue.try_borrow_mut().is_ok()));
                    observed.set(observed.get() + 1);
                }),
            )
        });
        run(std::ptr::null_mut(), 7);
        run(std::ptr::null_mut(), 7);
        assert_eq!(called.get(), 1);
    }

    #[test]
    fn cancelling_releases_queue_before_dropping_captures() {
        struct ReentrantDrop(Rc<std::cell::Cell<bool>>);
        impl Drop for ReentrantDrop {
            fn drop(&mut self) {
                POSTED.with(|queue| assert!(queue.try_borrow_mut().is_ok()));
                self.0.set(true);
            }
        }
        let dropped = Rc::new(std::cell::Cell::new(false));
        let capture = ReentrantDrop(dropped.clone());
        POSTED.with(|queue| {
            queue
                .borrow_mut()
                .insert((0, 8), Box::new(move || drop(capture)))
        });
        cancel(std::ptr::null_mut());
        assert!(dropped.get());
        POSTED.with(|queue| assert!(!queue.borrow().contains_key(&(0, 8))));
    }
}
