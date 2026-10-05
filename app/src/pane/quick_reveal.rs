//! Raise panes above ordinary windows on demand, and return them to the desktop band afterwards.
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use windows_sys::Win32::{
    Foundation::HWND,
    System::Threading::{AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId},
    UI::WindowsAndMessaging::*,
};

thread_local! {
    /// Whether a reveal currently holds the panes above ordinary windows.
    static REVEALED: Cell<bool> = const { Cell::new(false) };
    /// The window that owned the foreground before the reveal, so dismissing the
    /// panes also returns the user to the application they were working in. A
    /// revealed pane otherwise keeps the foreground, and the desktop band refuses
    /// to keep a foreground pane below ordinary windows.
    static PREVIOUS: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) };
}
pub(super) fn permanent_topmost(hwnd: HWND) -> bool {
    unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0 }
}


pub(super) fn show(hwnd: HWND) {
    super::window::raise_once(hwnd);
}

/// Foreground must be acquired *before* the final raise. Activating a pane while
/// this process is still in the background lets the activation's own z-order
/// change pass through the desktop-band clamp in `WM_WINDOWPOSCHANGING`, which
/// pulls the pane back below ordinary windows. That made a single hotkey press
/// only focus the app and required a second press to actually reveal the panes.
pub(super) fn show_all(state: &Rc<RefCell<super::PaneApp>>) {
    let windows: Vec<_> = state.borrow().views.iter().map(|v| v.window.hwnd()).collect();
    let Some(&first) = windows.first() else { return };
    // Remember where the user came from so dismissing the panes can hand the
    // foreground back instead of stranding them on a pane that sinks below
    // ordinary windows.
    let previous = unsafe { GetForegroundWindow() };
    PREVIOUS.with(|slot| slot.set(previous));
    for &hwnd in &windows {
        show(hwnd.cast());
    }
    activate(first.cast());
    // Raise again now that the process is foreground: no further activation
    // rearrangement will clamp the panes back into the desktop band.
    for &hwnd in &windows {
        show(hwnd.cast());
    }
    REVEALED.with(|slot| slot.set(true));
}

/// True while the panes are being held above ordinary windows by a reveal.
fn revealed() -> bool {
    REVEALED.with(Cell::get)
}

/// Returns every pane to the layer its own preferences describe. Panes the user
/// pinned stay topmost; the rest go back into the desktop band, which is what
/// `set_layer` applied when they were created.
pub(super) fn restore_all(state: &Rc<RefCell<super::PaneApp>>) {
    // Resolve the targets before touching windows: the native calls below can
    // dispatch messages, so no borrow may still be held.
    let targets: Vec<(HWND, bool)> = {
        let s = state.borrow();
        s.views
            .iter()
            .filter_map(|view| {
                s.workspace
                    .panel(view.id)
                    .map(|panel| (view.window.hwnd().cast(), panel.always_on_top()))
            })
            .collect()
    };
    // The foreground must be released *before* sinking. Windows keeps the
    // foreground window at the top of the non-topmost band, so a pane that still
    // holds the foreground is pushed straight back above ordinary windows no
    // matter what position is requested. Return to the application the user came
    // from, and when that was one of our own windows (the shortcut was pressed
    // while a pane already held focus) hand the foreground to the shell, which is
    // what clicking the desktop does.
    let previous = PREVIOUS.with(Cell::get);
    PREVIOUS.with(|slot| slot.set(std::ptr::null_mut()));
    let restore = (!previous.is_null()
        && unsafe { IsWindow(previous) } != 0
        && !belongs_to_self(previous))
    .then_some(previous)
    .unwrap_or_else(|| unsafe { GetShellWindow() });
    if !restore.is_null() && unsafe { GetForegroundWindow() } != restore {
        activate(restore);
    }
    for (hwnd, always_on_top) in targets {
        // A pane the user pinned stays topmost; only the rest rejoin the desktop
        // band, which is exactly what `create` established for them.
        if always_on_top {
            super::window::set_layer(hwnd, true);
        } else {
            super::window::sink_to_desktop(hwnd);
        }
    }
    REVEALED.with(|slot| slot.set(false));
}

/// Reveals when the panes are on the desktop, and returns them when they are
/// already revealed, so one shortcut both opens and dismisses them.

/// Whether `hwnd` belongs to this process, so returning to it would just leave a
/// pane holding the foreground.
fn belongs_to_self(hwnd: HWND) -> bool {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &raw mut pid) };
    pid == unsafe { GetCurrentProcessId() }
}
pub(super) fn toggle(state: &Rc<RefCell<super::PaneApp>>) {
    if revealed() {
        restore_all(state);
    } else {
        show_all(state);
    }
}

/// The desktop keeps the foreground events it asked for. Anything else means the
/// user moved on to another application, so a reveal has served its purpose.
pub(super) fn retract_if_revealed(state: &Rc<RefCell<super::PaneApp>>) {
    if revealed() {
        restore_all(state);
    }
}

/// Acquires the foreground for `hwnd`, borrowing the current foreground thread's
/// input state when `SetForegroundWindow` would otherwise be refused.
///
/// `SetForegroundWindow` only succeeds for a process the system has granted that
/// right. A `WM_HOTKEY` produced by `RegisterHotKey` carries the grant, but the
/// notification this app posts from its own keyboard hook does not, so a
/// listen-only fallback binding would raise nothing and the panes stayed behind
/// ordinary windows. Attaching to the foreground thread for the duration of the
/// call is the documented way to borrow that right, and the attachment is always
/// released so no input state is shared afterwards.
fn activate(hwnd: HWND) {
    unsafe {
        if GetForegroundWindow() == hwnd {
            return;
        }
        let own = GetCurrentThreadId();
        let foreground = GetForegroundWindow();
        let foreign = if foreground.is_null() {
            0
        } else {
            GetWindowThreadProcessId(foreground, std::ptr::null_mut())
        };
        let attached = foreign != 0
            && foreign != own
            && AttachThreadInput(own, foreign, 1) != 0;
        SetForegroundWindow(hwnd);
        if attached {
            AttachThreadInput(own, foreign, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reveal_preserves_topmost_and_allows_other_windows_to_cover_it() {
        let make = || windows_window::Window::new("LucidDesk reveal test")
            .size(100, 100).style(WS_POPUP).ex_style(WS_EX_TOOLWINDOW)
            .create().unwrap();
        let pane = make();
        let other = make();
        let hwnd = pane.hwnd().cast();
        let other_hwnd = other.hwnd().cast();
        super::super::window::set_layer(hwnd, false);
        show(hwnd);
        assert!(!permanent_topmost(hwnd));
        unsafe {
            ShowWindow(other_hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(other_hwnd, HWND_TOP, 0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let mut next = GetWindow(other_hwnd, GW_HWNDNEXT);
            while !next.is_null() && next != hwnd { next = GetWindow(next, GW_HWNDNEXT); }
            assert_eq!(next, hwnd, "another ordinary window can cover the revealed pane");
        }
        show(hwnd);
        assert!(!permanent_topmost(hwnd));
        super::super::window::set_layer(hwnd, true);
        show(hwnd);
        assert!(permanent_topmost(hwnd));
        super::super::window::set_layer(hwnd, false);
    }

    #[test]
    fn toggle_reveals_then_returns_panes_to_their_own_layer() {
        let _apartment = luciddesk_shell::ShellApartment::initialize_sta().unwrap();
        let mut app = super::super::tests::test_state();
        let mut expected = Vec::new();
        for (n, always_on_top) in [(10, false), (11, true)] {
            let id = luciddesk_core::PanelId::new(n);
            let mut panel = luciddesk_core::Panel::new(id, format!("Panel {n}"), Default::default());
            panel.set_always_on_top(always_on_top);
            app.workspace.add_panel(panel).unwrap();
            let model = Rc::new(RefCell::new(
                super::super::create_model(&app, id).unwrap(),
            ));
            let window = windows_window::Window::new("Reveal toggle view")
                .style(WS_POPUP)
                .size(100, 100)
                .create()
                .unwrap();
            super::super::window::set_layer(window.hwnd().cast(), always_on_top);
            unsafe { ShowWindow(window.hwnd().cast(), SW_HIDE); }
            expected.push((id, always_on_top));
            app.views.push(super::super::View {
                id,
                target: Rc::new(std::cell::Cell::new(id)),
                window,
                model,
            });
        }
        let state = Rc::new(RefCell::new(app));

        toggle(&state);
        assert!(revealed(), "first toggle reveals");
        for view in &state.borrow().views {
            assert_ne!(
                unsafe { IsWindowVisible(view.window.hwnd().cast()) },
                0,
                "a revealed pane is visible"
            );
        }

        toggle(&state);
        assert!(!revealed(), "second toggle retracts");
        for (view, (id, always_on_top)) in state.borrow().views.iter().zip(&expected) {
            assert_eq!(&view.id, id);
            assert_eq!(
                permanent_topmost(view.window.hwnd().cast()),
                *always_on_top,
                "each pane returns to its own topmost preference"
            );
        }
    }

    #[test]
    fn retract_is_a_noop_while_the_panes_are_on_the_desktop() {
        let state = Rc::new(RefCell::new(super::super::tests::test_state()));
        assert!(!revealed());
        retract_if_revealed(&state);
        assert!(!revealed());
    }
}
