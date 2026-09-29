#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
use super::{
    Event, GroupModel,
    composition::Surface,
    layout::{Grid, HEADER},
    render::Renderer,
};
use desktop_core::RectDip;
use std::{cell::RefCell, rc::Rc};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, EndPaint, InvalidateRect, PAINTSTRUCT,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, SetFocus, VK_APPS,
};
#[allow(clippy::wildcard_imports)]
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_window::Window;
mod shape;
pub(super) fn round_flyout(hwnd: HWND, width: i32, height: i32, radius: f32, scale: f32) {
    shape::WindowShape::default().update(hwnd, width, height, radius, scale);
}
pub const ANIMATE_FOLD: u32 = WM_APP + 10;
pub(super) fn update_auto_hide(hwnd: HWND, enabled: bool) {
    unsafe {
        if !enabled {
            KillTimer(hwnd, 3);
        }
        PostMessageW(hwnd, SYNC_POINTER, 1, 0);
    }
}
pub(super) const SYNC_POINTER: u32 = WM_APP + 11;
pub(super) const RUN_POSTED_ACTION: u32 = WM_APP + 12;
pub(super) const TAB_CHANGED: u32 = WM_APP + 13;
pub(super) const RESTORE_MERGE: u32 = WM_APP + 14;
const DESKTOP_LAYER: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.DesktopLayer");
const CLOSING_PANE: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.ClosingPane");

pub(super) fn is_desktop_layer(hwnd: HWND) -> bool {
    unsafe { !GetPropW(hwnd, DESKTOP_LAYER).is_null() }
}

fn clip_drag(hwnd: HWND, hidden: bool, shape: &mut shape::WindowShape) {
    shape.hide(hwnd, hidden);
}

pub(super) fn prepare_close(hwnd: HWND) {
    unsafe {
        SetPropW(hwnd, CLOSING_PANE, 1usize as _);
    }
}

/// Raise within the desktop-pane band, without jumping above ordinary application windows.
fn desktop_insert_after(hwnd: HWND) -> Option<HWND> {
    unsafe {
        let mut peer = GetTopWindow(std::ptr::null_mut());
        while !peer.is_null() {
            if peer != hwnd && !GetPropW(peer, DESKTOP_LAYER).is_null()
                && GetPropW(peer, CLOSING_PANE).is_null() && IsWindowVisible(peer) != 0 {
                let mut previous = GetWindow(peer, GW_HWNDPREV);
                if previous == hwnd { previous = GetWindow(hwnd, GW_HWNDPREV); }
                // HWND_TOP preserves the non-topmost band; inserting after a topmost HWND would not.
                if previous.is_null() || GetWindowLongW(previous, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0 {
                    return Some(HWND_TOP);
                }
                return Some(previous);
            }
            peer = GetWindow(peer, GW_HWNDNEXT);
        }
        None
    }
}

/// Raise only within the desktop-pane band, including clicks on owned controls.
pub(super) fn raise_among_peers(hwnd: HWND) {
    unsafe {
        if !GetPropW(hwnd, DESKTOP_LAYER).is_null()
            && let Some(after) = desktop_insert_after(hwnd)
        {
            let previous = GetWindow(hwnd, GW_HWNDPREV);
            let in_place = if after == HWND_TOP {
                previous.is_null() || GetWindowLongW(previous, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0
            } else {
                previous == after
            };
            if !in_place {
                crate::diagnostics::render_trace(format_args!("hwnd={hwnd:?} raise among peers after={after:?} previous={previous:?}"));
                SetWindowPos(hwnd, after, 0, 0, 0, 0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
            }
        }
    }
}

/// Raise once while preserving the topmost bit and subsequent desktop behavior.
pub(super) fn raise_once(hwnd: HWND) {
    unsafe {
        let desktop = RemovePropW(hwnd, DESKTOP_LAYER);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        if !desktop.is_null() { SetPropW(hwnd, DESKTOP_LAYER, desktop); }
    }
}

pub fn set_layer(hwnd: HWND, always_on_top: bool) {
    unsafe {
        // Let Windows change the actual topmost bit before applying desktop-band constraints.
        RemovePropW(hwnd, DESKTOP_LAYER);
        SetWindowPos(hwnd, if always_on_top { HWND_TOPMOST } else { HWND_NOTOPMOST },
            0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        if !always_on_top {
            SetPropW(hwnd, DESKTOP_LAYER, 1usize as _);
            SetWindowPos(hwnd, desktop_insert_after(hwnd).unwrap_or(HWND_BOTTOM), 0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        }
    }
}

// Shell menus pump messages while open. Keep auto-hide and repeated menu requests
// suspended until that nested interaction returns, including error paths.
struct MenuActivity(Rc<std::cell::Cell<bool>>, HWND);
impl MenuActivity {
    fn begin(active: Rc<std::cell::Cell<bool>>, hwnd: HWND) -> Self {
        active.set(true);
        Self(active, hwnd)
    }
}
impl Drop for MenuActivity {
    fn drop(&mut self) {
        self.0.set(false);
        unsafe {
            PostMessageW(self.1, SYNC_POINTER, 0, 0);
        }
    }
}

// Nested menu loops can consume mouse-leave notifications while our callback
// is suspended. Reconcile against the actual, unobscured pointer on return.
fn sync_pointer(hwnd: HWND, model: &RefCell<GroupModel>) {
    let mut p = POINT::default();
    let over = unsafe { GetCursorPos(&raw mut p) != 0 && WindowFromPoint(p) == hwnd };
    if over {
        unsafe {
            windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &raw mut p);
        }
        let m = model.borrow();
        let in_client = pane_hit(client(hwnd), p, scale(hwnd), &m) == HTCLIENT;
        drop(m);
        if in_client {
            update_pointer(hwnd, model, Some(p));
            track_client_leave(hwnd);
            return;
        }
    }
    update_pointer(hwnd, model, None);
}

fn track_client_leave(hwnd: HWND) {
    unsafe {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        TrackMouseEvent(&raw mut track);
    }
}

fn update_pointer(hwnd: HWND, model: &RefCell<GroupModel>, pointer: Option<POINT>) {
    let mut m = model.borrow_mut();
    let hovered = pointer.is_some_and(|p| {
        scrollbar(hwnd, &m)
            .is_some_and(|bar| bar.contains(p.x as f32 / scale(hwnd), p.y as f32 / scale(hwnd)))
    });
    let (button, item) = if let Some(p) = pointer {
        let s = scale(hwnd);
        let button = {
            m.header_button(
                client(hwnd).right as f32 / s,
                p.x as f32 / s,
                p.y as f32 / s,
            )
        };
        (
            button,
            if hovered {
                None
            } else {
                m.hit(grid(hwnd, &m), p.x as f32 / s, p.y as f32 / s, s)
            },
        )
    } else {
        (None, None)
    };
    let tab = pointer.and_then(|p| {
        let s = scale(hwnd);
        super::tabs::hit(&m, client(hwnd).right as f32 / s, p.x as f32 / s, p.y as f32 / s)
    });
    if (m.hovered_button, m.hovered_item, m.scrollbar.hovered, m.hovered_tab) != (button, item, hovered, tab) {
        m.hovered_tab = tab;
        m.hovered_button = button;
        m.hovered_item = item;
        m.scrollbar.hovered = hovered;
        drop(m);
        invalidate(hwnd);
    }
}

fn tab_drag_threshold(origin: POINT, current: POINT, dpi: u32) -> bool {
    use windows_sys::Win32::UI::HiDpi::GetSystemMetricsForDpi;
    let dx = unsafe { GetSystemMetricsForDpi(SM_CXDRAG, dpi) }.max(1) as u32;
    let dy = unsafe { GetSystemMetricsForDpi(SM_CYDRAG, dpi) }.max(1) as u32;
    origin.x.abs_diff(current.x) >= dx || origin.y.abs_diff(current.y) >= dy
}

fn pane_hit(r: RECT, p: POINT, scale: f32, model: &GroupModel) -> u32 {
    if super::tabs::hit(model, r.right as f32 / scale, p.x as f32 / scale, p.y as f32 / scale).is_some() {
        return HTCLIENT;
    }
    if super::scrollbar::Bar::for_model(model, r.right as f32 / scale, r.bottom as f32 / scale)
        .is_some_and(|bar| bar.contains(p.x as f32 / scale, p.y as f32 / scale))
    {
        return HTCLIENT;
    }
    if model
        .header_button(
            r.right as f32 / scale,
            p.x as f32 / scale,
            p.y as f32 / scale,
        )
        .is_some()
    {
        HTCLIENT
    } else {
        let hit = frame_hit(r, p, scale, model.collapsed, model.locked);
        if hit == HTCLIENT && model.tabs.len() > 1 && !model.locked && (p.y as f32 / scale) < HEADER { HTCAPTION } else { hit }
    }
}

fn frame_hit(r: RECT, p: POINT, scale: f32, collapsed: bool, locked: bool) -> u32 {
    let border = (5.0 * scale) as i32;
    match (
        p.x < border,
        p.x >= r.right - border,
        !collapsed && p.y < border,
        !collapsed && p.y >= r.bottom - border,
    ) {
        (true, _, true, _) => HTTOPLEFT,
        (_, true, true, _) => HTTOPRIGHT,
        (true, _, _, true) => HTBOTTOMLEFT,
        (_, true, _, true) => HTBOTTOMRIGHT,
        (true, _, _, _) => HTLEFT,
        (_, true, _, _) => HTRIGHT,
        (_, _, true, _) => HTTOP,
        (_, _, _, true) => HTBOTTOM,
        _ if !locked
            && p.y as f32 / scale < HEADER
            && p.x as f32 / scale
                < r.right as f32 / scale - super::layout::HEADER_BUTTONS_WIDTH =>
        {
            HTCAPTION
        }
        _ => HTCLIENT,
    }
}

#[cfg(test)]
mod hit_tests {
    use super::*;

    #[test]
    fn desktop_panes_raise_among_peers_without_covering_apps() {
        unsafe extern "system" fn count_positions(
            hwnd: HWND, message: u32, wparam: usize, lparam: isize, _: usize, data: usize,
        ) -> isize {
            unsafe {
                if message == WM_WINDOWPOSCHANGING {
                    let count = &*(data as *const std::cell::Cell<u32>);
                    count.set(count.get() + 1);
                }
                windows_sys::Win32::UI::Shell::DefSubclassProc(hwnd, message, wparam, lparam)
            }
        }
        unsafe {
            let make = || CreateWindowExW(WS_EX_TOOLWINDOW, windows_sys::w!("STATIC"), std::ptr::null(),
                WS_POPUP | WS_VISIBLE, 20, 20, 240, 120, std::ptr::null_mut(), std::ptr::null_mut(),
                std::ptr::null_mut(), std::ptr::null());
            let first = make();
            let second = make();
            let app = make();
            let owner = make();
            ShowWindow(owner, SW_HIDE);
            for hwnd in [first, second] {
                assert!(!hwnd.is_null());
                SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner as isize);
                assert_ne!(windows_sys::Win32::UI::Shell::SetWindowSubclass(hwnd, Some(borderless_proc), 1, 0), 0);
            }
            let above = |a, b| {
                let mut current = GetWindow(b, GW_HWNDPREV);
                while !current.is_null() {
                    if current == a { return true; }
                    current = GetWindow(current, GW_HWNDPREV);
                }
                false
            };
            set_layer(first, false);
            SetWindowPos(app, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
            set_layer(second, false);
            assert!(above(second, first), "new pane must be above its peers");
            assert!(above(app, second), "desktop panes must remain below applications");
            SetWindowPos(first, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
            assert!(above(first, second), "activation must raise the old pane");
            assert!(above(app, first));
            for _ in 0..3 {
                let mut menu_owner = first;
                let mut popups = Vec::new();
                for _ in 0..2 {
                    let popup = make();
                    ShowWindow(popup, SW_HIDE);
                    SetWindowLongPtrW(popup, GWLP_HWNDPARENT, menu_owner as isize);
                    SetWindowPos(popup, HWND_TOPMOST, 20, 20, 100, 100, SWP_NOACTIVATE | SWP_NOOWNERZORDER);
                    SetWindowPos(popup, HWND_TOPMOST, 20, 20, 100, 100, SWP_SHOWWINDOW | SWP_NOOWNERZORDER);
                    SetForegroundWindow(popup);
                    assert!(above(first, second), "opening an owned menu must preserve pane order");
                    assert!(above(app, first), "opening a menu must preserve the application band");
                    assert!(above(popup, first));
                    assert_eq!(GetWindowLongW(first, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST, 0);
                    menu_owner = popup;
                    popups.push(popup);
                }
                for popup in popups.into_iter().rev() {
                    DestroyWindow(popup);
                    assert!(above(first, second), "closing an owned menu must preserve pane order");
                    assert!(above(app, first), "closing a menu must preserve the application band");
                }
            }
            let positions = std::cell::Cell::new(0u32);
            assert_ne!(windows_sys::Win32::UI::Shell::SetWindowSubclass(
                first, Some(count_positions), 2, (&raw const positions) as usize,
            ), 0);
            for interaction in [WM_LBUTTONDOWN, WM_NCLBUTTONDOWN, WM_ENTERSIZEMOVE] {
                // Simulate a peer covering an already-active pane: there is no
                // WM_MOUSEACTIVATE before the next click or native move loop.
                SetWindowPos(second, HWND_TOP, 0, 0, 0, 0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
                assert!(above(second, first));
                SendMessageW(first, interaction, 0, 0);
                assert!(above(first, second), "interaction {interaction:#x} must raise the pane");
                assert!(above(app, first), "interaction must preserve the application band");
                positions.set(0);
                SendMessageW(first, interaction, 0, 0);
                assert_eq!(positions.get(), 0, "repeated interaction must not reset z-order");
            }
            // With no visible peer the pane still cannot go below its owner.
            // Repeated HWND_BOTTOM requests must not be mistaken for useful raises.
            ShowWindow(second, SW_HIDE);
            set_layer(first, false);
            for interaction in [WM_MOUSEACTIVATE, WM_LBUTTONDOWN, WM_NCLBUTTONDOWN, WM_ENTERSIZEMOVE] {
                positions.set(0);
                SendMessageW(first, interaction, 0, 0);
                assert_eq!(positions.get(), 0, "single owned pane must not reorder on {interaction:#x}");
            }
            let mut activation = WINDOWPOS {
                hwnd: first, hwndInsertAfter: HWND_TOP,
                flags: SWP_NOMOVE | SWP_NOSIZE, ..Default::default()
            };
            SendMessageW(first, WM_WINDOWPOSCHANGING, 0, (&raw mut activation) as isize);
            assert_ne!(activation.flags & SWP_NOZORDER, 0,
                "activation of a single pane must preserve its position, not move to HWND_BOTTOM");
            windows_sys::Win32::UI::Shell::RemoveWindowSubclass(first, Some(count_positions), 2);
            set_layer(first, true);
            assert_ne!(GetWindowLongW(first, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST, 0);
            set_layer(first, false);
            assert_eq!(GetWindowLongW(first, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST, 0);
            assert!(above(app, first));
            for hwnd in [first, second, app, owner] { DestroyWindow(hwnd); }
        }
    }

    #[test]
    fn borderless_activation_keeps_default_state_without_frame_painting() {
        let activations = Rc::new(RefCell::new(Vec::new()));
        let received = Rc::clone(&activations);
        let window = Window::new("Borderless activation")
            .size(240, 160).style(WS_POPUP | WS_THICKFRAME)
            .on_message(move |_, message, wparam, lparam| {
                if message == WM_NCACTIVATE { received.borrow_mut().push((wparam, lparam)); }
                None
            }).create().unwrap();
        let hwnd = window.hwnd().cast();
        unsafe {
            assert_ne!(windows_sys::Win32::UI::Shell::SetWindowSubclass(hwnd, Some(borderless_proc), 1, 0), 0);
            activations.borrow_mut().clear();
            SendMessageW(hwnd, WM_NCACTIVATE, 1, 0);
            assert_ne!(SendMessageW(hwnd, WM_NCACTIVATE, 0, 0), 0);
            assert_eq!(*activations.borrow(), [(1, -1), (0, -1)]);
            assert_eq!(SendMessageW(hwnd, WM_NCPAINT, 1, 0), 0);
            assert_eq!(SendMessageW(hwnd, WM_ERASEBKGND, 0, 0), 1);
        }
    }

    #[test]
    fn preview_clip_preserves_window_capture_and_restores_region() {
        use windows_sys::Win32::Graphics::Gdi::*;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
        unsafe {
            let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, windows_sys::w!("STATIC"), std::ptr::null(),
                WS_POPUP | WS_VISIBLE, 20, 20, 240, 120, std::ptr::null_mut(), std::ptr::null_mut(),
                std::ptr::null_mut(), std::ptr::null());
            assert!(!hwnd.is_null());
            SetCapture(hwnd);
            let mut clipped = shape::WindowShape::default();
            clip_drag(hwnd, true, &mut clipped);
            assert!(clipped.hidden);
            assert_ne!(IsWindowVisible(hwnd), 0);
            assert_eq!(GetCapture(), hwnd);
            let region = CreateRectRgn(0, 0, 0, 0);
            assert_eq!(GetWindowRgn(hwnd, region), NULLREGION);
            clip_drag(hwnd, false, &mut clipped);
            assert!(!clipped.hidden);
            assert_eq!(GetWindowRgn(hwnd, region), 0);
            assert_eq!(GetCapture(), hwnd);
            DeleteObject(region);
            ReleaseCapture();
            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn tab_click_jitter_does_not_start_a_pane_drag() {
        for dpi in [96, 144, 192] {
            let origin = POINT { x: 120, y: 18 };
            assert!(!tab_drag_threshold(origin, origin, dpi));
            assert!(!tab_drag_threshold(origin, POINT { x: 121, y: 19 }, dpi));
            assert!(tab_drag_threshold(origin, POINT { x: 160, y: 18 }, dpi));
            assert!(tab_drag_threshold(origin, POINT { x: 120, y: -30 }, dpi));
        }
    }

    #[test]
    fn locked_pane_blocks_moving_but_preserves_resize_hit_targets() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for collapsed in [false, true] {
                let height = if collapsed { HEADER } else { 300.0 };
                let r = RECT {
                    left: 0,
                    top: 0,
                    right: (240.0 * scale) as i32,
                    bottom: (height * scale) as i32,
                };
                for x in [1.0, 100.0, 216.0, 239.0] {
                    for y in [1.0, 19.0, height - 1.0] {
                        let p = POINT {
                            x: (x * scale) as i32,
                            y: (y * scale) as i32,
                        };
                        let unlocked = frame_hit(r, p, scale, collapsed, false);
                        assert_eq!(
                            frame_hit(r, p, scale, collapsed, true),
                            if unlocked == HTCAPTION {
                                HTCLIENT
                            } else {
                                unlocked
                            }
                        );
                    }
                }
                assert_eq!(
                    frame_hit(
                        r,
                        POINT {
                            x: (100.0 * scale) as i32,
                            y: (19.0 * scale) as i32
                        },
                        scale,
                        collapsed,
                        false
                    ),
                    HTCAPTION
                );
            }
        }
    }

    #[test]
    fn collapsed_header_has_buttons_and_dragging_but_no_vertical_resize() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let r = RECT {
                left: 0,
                top: 0,
                right: (240.0 * scale) as i32,
                bottom: (HEADER * scale) as i32,
            };
            let at = |x: f32, y: f32, collapsed| {
                frame_hit(
                    r,
                    POINT {
                        x: (x * scale) as i32,
                        y: (y * scale) as i32,
                    },
                    scale,
                    collapsed,
                    false,
                )
            };
            for y in [1.0, 19.0, 37.0] {
                assert_eq!(at(100.0, y, true), HTCAPTION);
                assert_eq!(at(216.0, y, true), HTCLIENT);
                assert_eq!(at(184.0, y, true), HTCLIENT);
                assert_eq!(at(1.0, y, true), HTLEFT);
                assert_eq!(at(239.0, y, true), HTRIGHT);
            }
            assert_eq!(at(100.0, 1.0, false), HTTOP);
            assert_eq!(at(100.0, 37.0, false), HTBOTTOM);
            assert_eq!(at(1.0, 1.0, false), HTTOPLEFT);
            assert_eq!(at(239.0, 37.0, false), HTBOTTOMRIGHT);
        }
    }
}

thread_local! {
    static DEFERRED: RefCell<std::collections::HashMap<usize, Box<dyn FnOnce()>>> = RefCell::new(std::collections::HashMap::new());
    static POSTED: RefCell<std::collections::HashMap<(isize, usize), Box<dyn FnOnce()>>> = RefCell::new(std::collections::HashMap::new());
    static NEXT_ACTION: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Dispatch in the subclass, outside the windows-window callback, so Shell can
// pump messages without detaching the pane's normal event handler.
pub(super) fn post_action(hwnd: HWND, action: impl FnOnce() + 'static) -> bool {
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
pub(super) fn defer_action(action: impl FnOnce() + 'static) -> bool {
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

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn point(value: isize) -> POINT {
    POINT {
        x: (value as u16).cast_signed().into(),
        y: ((value >> 16) as u16).cast_signed().into(),
    }
}
fn scale(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0
}
fn client(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        GetClientRect(hwnd, &raw mut r);
    }
    r
}
fn grid(hwnd: HWND, model: &GroupModel) -> Grid {
    let r = client(hwnd);
    let s = scale(hwnd);
    model.grid(r.right as f32 / s, r.bottom as f32 / s)
}

struct ColumnDrag {
    divider: usize,
    bounds: [f32; 5],
    original: Option<[f32; 4]>,
    proportions: [f32; 4],
    visible: u8,
}

fn scrollbar(hwnd: HWND, model: &GroupModel) -> Option<super::scrollbar::Bar> {
    let r = client(hwnd);
    let s = scale(hwnd);
    super::scrollbar::Bar::for_model(model, r.right as f32 / s, r.bottom as f32 / s)
}

fn update_scrollbar_animation(
    hwnd: HWND,
    model: &RefCell<GroupModel>,
    motion: &mut super::animation::Motion,
    enabled: bool,
    timer_running: &mut bool,
) {
    let mut m = model.borrow_mut();
    let visible = scrollbar(hwnd, &m).is_some();
    let animating = m
        .scrollbar
        .animate(motion, std::time::Instant::now(), enabled, visible);
    if animating != *timer_running {
        unsafe {
            *timer_running = animating && SetTimer(hwnd, 4, USER_TIMER_MINIMUM, None) != 0;
            if !*timer_running {
                KillTimer(hwnd, 4);
            }
        }
    }
}

fn column_divider(hwnd: HWND, model: &GroupModel, point: POINT) -> Option<usize> {
    if !model.is_list() || model.folder.is_none() || model.collapsed {
        return None;
    }
    let scale = scale(hwnd);
    let grid = grid(hwnd, model);
    let top = grid.content_top - super::layout::LIST_HEADER;
    let y = point.y as f32 / scale;
    if !(top..grid.content_top).contains(&y) {
        return None;
    }
    let x = point.x as f32 / scale - super::layout::PADDING;
    let columns = model.list_columns(grid.cell_width);
    (1..4).find(|&divider| {
        model.folder_visible_columns & (1 << divider) != 0 && (x - columns[divider]).abs() <= 4.0
    })
}
fn invalidate(hwnd: HWND) {
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

// windows-window temporarily detaches its callback during reentrant dispatch. Non-client
// sizing must still be handled while DWM or collapse/resize calls synchronously reenter.
pub(super) unsafe extern "system" fn borderless_proc(
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    id: usize,
    _data: usize,
) -> isize {
    if matches!(message, WM_ACTIVATE | WM_NCACTIVATE | WM_LBUTTONDOWN | WM_NCLBUTTONDOWN | WM_ENTERSIZEMOVE | WM_EXITSIZEMOVE) {
        crate::diagnostics::render_trace(format_args!("hwnd={hwnd:?} message={message:#x} wparam={wparam:#x}"));
    }
    if message == WM_WINDOWPOSCHANGING && lparam != 0 {
        let flags = unsafe { (*(lparam as *const WINDOWPOS)).flags };
        if flags & (SWP_SHOWWINDOW | SWP_HIDEWINDOW) != 0 {
            crate::diagnostics::render_trace(format_args!("hwnd={hwnd:?} windowpos flags={flags:#x} visible={}", unsafe { IsWindowVisible(hwnd) }));
        }
    }
    if message == RUN_POSTED_ACTION {
        let action = POSTED.with(|queue| queue.borrow_mut().remove(&(hwnd as isize, wparam)));
        if let Some(action) = action {
            action();
        }
        return 0;
    }
    if message == WM_NCDESTROY {
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
    if message == WM_DESTROY && unsafe { !GetPropW(hwnd, CLOSING_PANE).is_null() } {
        return 0;
    }
    if message == WM_NCCALCSIZE {
        return 0;
    }
    // The composition surface owns the entire frame. DefWindowProc's classic
    // activation painting can flash over it on Windows 10.
    if message == WM_NCACTIVATE {
        return unsafe { windows_sys::Win32::UI::Shell::DefSubclassProc(hwnd, message, wparam, -1) };
    }
    if message == WM_NCPAINT { return 0; }
    if message == WM_ERASEBKGND { return 1; }
    unsafe {
        // An already-active pane can be covered by another pane without losing
        // activation. Its next click/drag need not send WM_MOUSEACTIVATE.
        // Handle interaction here, outside the model callback's borrow guards.
        if matches!(message, WM_MOUSEACTIVATE | WM_LBUTTONDOWN | WM_NCLBUTTONDOWN | WM_ENTERSIZEMOVE)
        {
            raise_among_peers(hwnd);
        }
        if message == WM_WINDOWPOSCHANGING && !GetPropW(hwnd, DESKTOP_LAYER).is_null() {
            let position = &mut *(lparam as *mut WINDOWPOS);
            // Activating/destroying an owned popup can reposition every sibling
            // pane in the shared owner group. Those passive moves must not be
            // treated as requests to raise each sibling above the active pane.
            // Our explicit peer raises already specify SWP_NOOWNERZORDER.
            if position.flags & (SWP_NOACTIVATE | SWP_NOOWNERZORDER) == SWP_NOACTIVATE {
                position.flags |= SWP_NOZORDER;
            }
            if position.flags & SWP_NOZORDER == 0 {
                // All desktop panes share a Shell owner. Raising a pane must
                // not reorder that owner and indirectly raise its other panes.
                position.flags |= SWP_NOOWNERZORDER;
                if let Some(after) = desktop_insert_after(hwnd) {
                    let previous = GetWindow(hwnd, GW_HWNDPREV);
                    if previous == after || (after == HWND_TOP && !previous.is_null()
                        && GetWindowLongW(previous, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0) {
                        position.flags |= SWP_NOZORDER;
                    } else {
                        position.hwndInsertAfter = after;
                    }
                } else if position.hwndInsertAfter != HWND_BOTTOM {
                    // The initial set_layer placement may use HWND_BOTTOM once.
                    // A lone pane has no peer to raise above on activation; keep
                    // its existing position instead of repeatedly sinking it.
                    position.flags |= SWP_NOZORDER;
                }
                crate::diagnostics::render_trace(format_args!("hwnd={hwnd:?} desktop windowpos after={:?} flags={:#x}", position.hwndInsertAfter, position.flags));
            }
        }
        if message == WM_NCDESTROY {
            RemovePropW(hwnd, DESKTOP_LAYER);
            windows_sys::Win32::UI::Shell::RemoveWindowSubclass(hwnd, Some(borderless_proc), id);
        }
        windows_sys::Win32::UI::Shell::DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

#[allow(clippy::too_many_lines)]
pub fn create<F>(
    bounds: RectDip,
    model: Rc<RefCell<GroupModel>>,
    event: F,
) -> Result<Window, String>
where
    F: FnMut(Event) -> bool + 'static,
{
    let events = Rc::new(RefCell::new(event));
    let mut renderer = Renderer::new().map_err(|e| e.to_string())?;
    let mut surface: Option<Surface> = None;
    let mut visibility = super::visibility::Transition::default();
    let mut drag: Option<(usize, POINT, bool)> = None;
    let mut column_drag: Option<ColumnDrag> = None;
    let mut scrollbar_drag: Option<f32> = None;
    let mut scrollbar_motion = super::animation::Motion::settled(0.0, std::time::Instant::now());
    let mut scrollbar_timer = false;
    let mut scrollbar_animated = super::scrollbar::animations_enabled();
    let mut drag_identity = None;
    let mut drag_image: Option<super::drag_drop::image::DragImage> = None;
    let mut fold: Option<super::animation::Fold> = None;
    let mut pane_moved = false;
    let mut drag_clipped = shape::WindowShape::default();
    let mut move_origin: Option<super::snap::DragOrigin> = None;
    let mut auto_hide = super::auto_hide::AutoHide::default();
    let mut tab_press: Option<(desktop_core::PanelId, POINT)> = None;
    let menu_active = Rc::new(std::cell::Cell::new(false));
    let mut paint_error = false;
    let model_init = Rc::clone(&model);
    let inspect = std::env::var_os("LUCIDPANE_INSPECT").is_some();
    let window_title = format!("LucidDesk — {}", model.borrow().title);
    let prepared = Rc::new(std::cell::Cell::new(false));
    let show_prepared = Rc::clone(&prepared);
    let window = Window::new(&window_title)
        .size(bounds.width as i32, bounds.height as i32)
        .style(WS_POPUP | WS_THICKFRAME | WS_SYSMENU)
        .ex_style(WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW)
        .on_message(move |raw, message, wparam, lparam| {
            if unsafe { crate::window_visibility::defer_show(message, lparam, show_prepared.get()) } {
                return Some(0);
            }
            let hwnd: HWND = raw.cast();
            #[cfg(test)]
            if message == WM_APP + 199 {
                return Some(surface.as_ref().map_or(-1, |s| (s.current_opacity() * 1000.0) as isize));
            }
            if visibility.message(hwnd, message, wparam, lparam, surface.as_ref(), |_| {}) {
                return Some(0);
            }
            if message == super::visibility::CLOSED {
                let events = Rc::clone(&events);
                defer_action(move || { (events.borrow_mut())(Event::ClosePane); });
                return Some(0);
            }
            let event = |value| (events.borrow_mut())(value);
            if message == SYNC_POINTER && wparam != 0 {
                auto_hide.reset();
            }
            if matches!(message, WM_MOUSEMOVE | WM_MOUSELEAVE | WM_NCMOUSEMOVE
                | WM_NCMOUSELEAVE
                | WM_ACTIVATE | WM_WINDOWPOSCHANGED | WM_EXITSIZEMOVE | SYNC_POINTER)
                || (message == WM_TIMER && wparam == 3)
            {
                unsafe { KillTimer(hwnd, 3); }
                let (enabled, collapsed) = {
                    let m = model.borrow();
                    (m.auto_hide, m.collapsed)
                };
                let suspended = menu_active.get() || super::rename::active(hwnd)
                    || drag.is_some()
                    || unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture() } == hwnd;
                let mut cursor = POINT::default();
                let hovered = unsafe { GetCursorPos(&raw mut cursor) != 0 && WindowFromPoint(cursor) == hwnd };
                let (delay, collapse) = auto_hide.update(enabled, suspended, collapsed, hovered, std::time::Instant::now());
                if let Some(delay) = delay {
                    unsafe { SetTimer(hwnd, 3, delay.as_millis().max(1) as u32, None); }
                }
                if let Some(collapse) = collapse {
                    if message == WM_TIMER {
                        event(Event::SetCollapsed(collapse));
                    } else {
                        // Position/activation messages can arrive synchronously
                        // while the owner holds PaneApp. Commit on a later tick.
                        unsafe { SetTimer(hwnd, 3, 1, None); }
                    }
                }
                if enabled && message == WM_NCMOUSEMOVE {
                    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe { TrackMouseEvent(&raw mut track); }
                }
            }
            match message {
                TAB_CHANGED => {
                    tab_press = None;
                    drag = None;
                    drag_image = None;
                    column_drag = None;
                    scrollbar_drag = None;
                    fold = None;
                    unsafe {
                        KillTimer(hwnd, 2);
                        if windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture() == hwnd { ReleaseCapture(); }
                        PostMessageW(hwnd, SYNC_POINTER, 0, 0);
                    }
                    invalidate(hwnd);
                    Some(0)
                }
                WM_KEYDOWN if !menu_active.get() && unsafe {
                    windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x11) < 0
                } && matches!(wparam, 0x09 | 0x54 | 0x57) => {
                    match wparam {
                        0x09 => {
                            let step = if unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x10) } < 0 { -1 } else { 1 };
                            let next = super::tabs::adjacent(&model.borrow(), step);
                            if let Some(next) = next { event(Event::SelectTab(next)); }
                        }
                        0x54 => { if model.borrow().folder.is_none() { event(Event::NewTab(false)); } }
                        _ => { event(Event::CloseTab); }
                    }
                    Some(0)
                }
                WM_DISPLAYCHANGE => {
                    // The runtime supervisor restores the layout after displays settle.
                    invalidate(hwnd);
                    Some(0)
                }
                WM_SETFOCUS | WM_KILLFOCUS => {
                    model.borrow_mut().focused = message == WM_SETFOCUS;
                    if message == WM_SETFOCUS && model.borrow().selected.is_some() {
                        event(Event::PaneItemFocus);
                    }
                    invalidate(hwnd);
                    Some(0)
                }
                SYNC_POINTER => {
                    sync_pointer(hwnd, &model);
                    // The nested menu loop may have validated WM_PAINT while
                    // this window's callback was suspended.
                    invalidate(hwnd);
                    Some(0)
                }
                WM_NCMOUSEMOVE => {
                    // Registering client leave tracking from the non-client
                    // border immediately posts WM_MOUSELEAVE. Never re-arm it
                    // here or in WM_MOUSELEAVE, which would flood the queue.
                    update_pointer(hwnd, &model, None);
                    None
                }
                WM_MOUSEACTIVATE => {
                    Some(MA_ACTIVATE as isize)
                }
                WM_ACTIVATE => {
                    unsafe {
                        PostMessageW(hwnd, SYNC_POINTER, 0, 0);
                    }
                    None
                }
                WM_POWERBROADCAST => {
                    invalidate(hwnd);
                    Some(1)
                }
                WM_SETTINGCHANGE | WM_THEMECHANGED => {
                    scrollbar_animated = super::scrollbar::animations_enabled();
                    {
                        let mut m = model.borrow_mut();
                        m.dark = super::theme::is_dark(m.theme);
                    }
                    if let Ok(new_renderer) = Renderer::new() {
                        renderer = new_renderer;
                    }
                    // Appearance changes invalidate rendering resources, not Shell image inventory.
                    invalidate(hwnd);
                    Some(0)
                }
                WM_KEYDOWN if wparam == 0x1b && scrollbar_drag.is_some() => {
                    scrollbar_drag = None;
                    model.borrow_mut().scrollbar.dragging = false;
                    unsafe {
                        ReleaseCapture();
                    }
                    sync_pointer(hwnd, &model);
                    invalidate(hwnd);
                    Some(0)
                }
                WM_KEYDOWN if wparam == 0x1b && column_drag.is_some() => {
                    model.borrow_mut().folder_columns = column_drag.take().unwrap().original;
                    unsafe {
                        ReleaseCapture();
                    }
                    invalidate(hwnd);
                    Some(0)
                }
                WM_SETCURSOR if lparam as u16 == HTCLIENT as u16 => {
                    let mut pointer = POINT::default();
                    unsafe {
                        GetCursorPos(&raw mut pointer);
                        windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &raw mut pointer);
                    }
                    if scrollbar_drag.is_some()
                        || scrollbar(hwnd, &model.borrow()).is_some_and(|bar| {
                            bar.contains(
                                pointer.x as f32 / scale(hwnd),
                                pointer.y as f32 / scale(hwnd),
                            )
                        })
                    {
                        unsafe {
                            SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_ARROW));
                        }
                        Some(1)
                    } else if column_drag.is_some()
                        || column_divider(hwnd, &model.borrow(), pointer).is_some()
                    {
                        unsafe {
                            SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_SIZEWE));
                        }
                        Some(1)
                    } else {
                        None
                    }
                }
                WM_KEYDOWN if wparam == 0x74 => {
                    event(Event::Refresh);
                    Some(0)
                }
                WM_RBUTTONUP => {
                    let mut p = point(lparam);
                    unsafe {
                        ClientToScreen(hwnd, &raw mut p);
                        let packed = (usize::from(p.x as u16) | (usize::from(p.y as u16) << 16))
                            .cast_signed();
                        PostMessageW(hwnd, WM_CONTEXTMENU, hwnd as usize, packed);
                    }
                    Some(0)
                }
                WM_KEYDOWN
                    if wparam == 0x5d
                        || (wparam == 0x79
                            && unsafe {
                                windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x10)
                            } < 0) =>
                {
                    unsafe {
                        PostMessageW(hwnd, WM_CONTEXTMENU, hwnd as usize, -1);
                    }
                    Some(0)
                }
                WM_MOVING => {
                    pane_moved |= unsafe {
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x01)
                    } < 0;
                    if let Some(origin) = &move_origin {
                        let mut pointer = POINT::default();
                        if unsafe { GetCursorPos(&raw mut pointer) } != 0 {
                            unsafe {
                                *(lparam as *mut RECT) = origin.proposal(pointer);
                            }
                        }
                    }
                    event(Event::Moving(lparam as *mut RECT));
                    if pane_moved {
                        unsafe { SetTimer(hwnd, 5, 40, None); }
                        let hidden = model.borrow().merge_occluded;
                        clip_drag(hwnd, hidden, &mut drag_clipped);
                    }
                    Some(1)
                }
                RESTORE_MERGE => {
                    model.borrow_mut().merge_occluded = false;
                    clip_drag(hwnd, false, &mut drag_clipped);
                    Some(0)
                }
                WM_TIMER if wparam == 5 => {
                    if pane_moved {
                        event(Event::PreviewPaneMove);
                        let hidden = model.borrow().merge_occluded;
                        clip_drag(hwnd, hidden, &mut drag_clipped);
                    }
                    Some(0)
                }
                WM_TIMER if wparam == 4 => {
                    update_scrollbar_animation(
                        hwnd,
                        &model,
                        &mut scrollbar_motion,
                        scrollbar_animated,
                        &mut scrollbar_timer,
                    );
                    invalidate(hwnd);
                    Some(0)
                }
                WM_TIMER if wparam == 3 => Some(0),
                ANIMATE_FOLD => {
                    let mut enabled = 1i32;
                    unsafe {
                        SystemParametersInfoW(
                            SPI_GETCLIENTAREAANIMATION,
                            0,
                            (&raw mut enabled).cast(),
                            0,
                        );
                    }
                    let m = model.borrow();
                    fold = Some(super::animation::Fold::new(
                        client(hwnd).bottom as f32 / scale(hwnd),
                        lparam as f32,
                        m.reveal,
                        if m.collapsed { 0.0 } else { 1.0 },
                        std::time::Instant::now(),
                        if enabled != 0 {
                            super::animation::FOLD_DURATION
                        } else {
                            std::time::Duration::ZERO
                        },
                    ));
                    drop(m);
                    unsafe {
                        SetTimer(hwnd, 2, USER_TIMER_MINIMUM, None);
                        PostMessageW(hwnd, WM_TIMER, 2, 0);
                    }
                    Some(0)
                }
                WM_TIMER if wparam == 2 => {
                    if let Some(animation) = &fold {
                        let (height, reveal, done) = animation.sample(std::time::Instant::now());
                        model.borrow_mut().reveal = reveal;
                        let bounds = client(hwnd);
                        let height = (height * scale(hwnd)).round() as i32;
                        if height != bounds.bottom {
                            unsafe {
                                SetWindowPos(
                                    hwnd,
                                    std::ptr::null_mut(),
                                    0,
                                    0,
                                    bounds.right,
                                    height,
                                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                                );
                            }
                        }
                        invalidate(hwnd);
                        if done {
                            fold = None;
                            unsafe {
                                KillTimer(hwnd, 2);
                            }
                        }
                    }
                    Some(0)
                }
                WM_DESTROY => {
                    unsafe { KillTimer(hwnd, 5); }
                    if pane_moved { event(Event::FinishPaneMove(false)); }
                    // The supervisor can recreate a surface destroyed with Explorer.
                    Some(0)
                }
                WM_NCCALCSIZE | WM_ERASEBKGND => Some(0),
                WM_NCHITTEST => {
                    let mut p = point(lparam);
                    unsafe {
                        windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &raw mut p);
                    }
                    let r = client(hwnd);
                    let m = model.borrow();
                    let hit = pane_hit(r, p, scale(hwnd), &m);
                    Some(isize::try_from(hit).unwrap_or_default())
                }
                WM_SYSCOMMAND if model.borrow().locked && wparam as u32 & 0xfff0 == SC_MOVE => {
                    Some(0)
                }
                WM_GETMINMAXINFO => {
                    let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
                    let m = model.borrow();
                    let cell = m.resize_cell();
                    let s = scale(hwnd);
                    info.ptMinTrackSize.x =
                        ((cell.0 + super::layout::PADDING * 2.0).max(if m.tabs.len() > 1 { desktop_core::RectDip::MIN_WIDTH } else { 0.0 }) * s).ceil() as i32;
                    info.ptMinTrackSize.y = ((if m.collapsed {
                        HEADER
                    } else {
                        let rows = { m.row_contents(grid(hwnd, &m)) };
                        m.content_header()
                            + super::layout::PADDING * 2.0
                            + rows.get(m.scroll).copied().unwrap_or(cell.1)
                    }) * s)
                        .ceil() as i32;
                    Some(0)
                }
                WM_SIZING => {
                    let rect = unsafe { &mut *(lparam as *mut RECT) };
                    let proposal = *rect;
                    let m = model.borrow();
                    if m.is_list() || m.items.is_empty() {
                        drop(m);
                        event(Event::Sizing(rect, proposal, wparam as u32));
                        return Some(1);
                    }
                    let s = scale(hwnd);
                    // Resolve columns first on corner drags, then measure the final row.
                    let horizontal = match wparam as u32 {
                        WMSZ_LEFT | WMSZ_TOPLEFT | WMSZ_BOTTOMLEFT => Some(WMSZ_LEFT),
                        WMSZ_RIGHT | WMSZ_TOPRIGHT | WMSZ_BOTTOMRIGHT => Some(WMSZ_RIGHT),
                        _ => None,
                    };
                    if let Some(edge) = horizontal {
                        super::layout::resize_pane(
                            rect,
                            edge,
                            m.resize_cell(),
                            s,
                            m.collapsed,
                            &[],
                        );
                    }
                    let proposed_grid = m.grid(
                        (rect.right - rect.left) as f32 / s,
                        (rect.bottom - rect.top) as f32 / s,
                    );
                    let rows = { m.row_contents(proposed_grid) };
                    let start = m.scroll.min(rows.len().saturating_sub(1));
                    super::layout::resize_pane(
                        rect,
                        wparam as u32,
                        m.resize_cell(),
                        s,
                        m.collapsed,
                        &rows[start..],
                    );
                    drop(m);
                    // Peer alignment wins over content-grid snapping, using the
                    // unsnapped Windows proposal to preserve the release distance.
                    event(Event::Sizing(rect, proposal, wparam as u32));
                    Some(1)
                }
                WM_PAINT => {
                    update_scrollbar_animation(
                        hwnd,
                        &model,
                        &mut scrollbar_motion,
                        scrollbar_animated,
                        &mut scrollbar_timer,
                    );
                    let mut ps = PAINTSTRUCT::default();
                    unsafe {
                        BeginPaint(hwnd, &raw mut ps);
                        EndPaint(hwnd, &raw const ps);
                    }
                    let r = client(hwnd);
                    if r.right > 0 && r.bottom > 0 {
                        let s = scale(hwnd);
                        let radius = model.borrow().options.corner_radius;
                        drag_clipped.update(hwnd, r.right, r.bottom, radius, s);
                        let result = (|| {
                            if surface.is_none() {
                                surface = Some(Surface::new_pane(
                                    windows::Win32::Foundation::HWND(hwnd),
                                )?);
                            }
                            let surface = surface.as_mut().unwrap();
                            surface.pane_corner_radius = model.borrow().options.corner_radius;
                            surface
                                .theme(windows::Win32::Foundation::HWND(hwnd), model.borrow().dark);
                            {
                                surface.material(
                                    windows::Win32::Foundation::HWND(hwnd),
                                    model.borrow().backdrop,
                                );
                            }
                            model.borrow_mut().native_material = surface.native;
                            let Some(target) =
                                surface.try_begin_frame(r.right as u32, r.bottom as u32)?
                            else {
                                return Ok(());
                            };
                            renderer.paint(
                                &target,
                                r.right as u32,
                                r.bottom as u32,
                                s,
                                &model.borrow(),
                            )?;
                            surface.end_frame()
                        })();
                        if let Err(error) = result {
                            if !paint_error {
                                eprintln!("面板渲染失败：{error}");
                            }
                            paint_error = true;
                            {
                                event(Event::Exit);
                            }
                        } else {
                            paint_error = false;
                        }
                    }
                    Some(0)
                }
                WM_SIZE => {
                    let mut m = model.borrow_mut();
                    if fold.is_none() && !m.collapsed {
                        m.scroll = m.scroll.min(grid(hwnd, &m).max_scroll(m.items.len()));
                    }
                    drop(m);
                    invalidate(hwnd);
                    Some(0)
                }
                WM_DPICHANGED => {
                    let _ = crate::app_icon::apply(hwnd);
                    let r = unsafe { &*(lparam as *const RECT) };
                    unsafe {
                        SetWindowPos(
                            hwnd,
                            std::ptr::null_mut(),
                            r.left,
                            r.top,
                            r.right - r.left,
                            r.bottom - r.top,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                    invalidate(hwnd);
                    Some(0)
                }
                WM_EXITSIZEMOVE => {
                    unsafe { KillTimer(hwnd, 5); }
                    if pane_moved {
                        let commit = unsafe {
                            windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x1b)
                        } >= 0;
                        event(Event::FinishPaneMove(commit));
                    }
                    let hidden = model.borrow().merge_occluded;
                    clip_drag(hwnd, hidden, &mut drag_clipped);
                    pane_moved = false;
                    move_origin = None;
                    let mut r = RECT::default();
                    unsafe {
                        GetWindowRect(hwnd, &raw mut r);
                    }
                    let s = scale(hwnd);
                    event(Event::Geometry(RectDip::new(
                        r.left as f32 / s,
                        r.top as f32 / s,
                        (r.right - r.left) as f32 / s,
                        (r.bottom - r.top) as f32 / s,
                    )));
                    Some(0)
                }
                WM_ENTERSIZEMOVE => {
                    pane_moved = false;
                    // Finish the fold before manual geometry changes so an intermediate height
                    // cannot replace the persisted expanded size.
                    if let Some(animation) = fold.take() {
                        model.borrow_mut().reveal = animation.to_reveal;
                        unsafe {
                            KillTimer(hwnd, 2);
                            SetWindowPos(
                                hwnd,
                                std::ptr::null_mut(),
                                0,
                                0,
                                client(hwnd).right,
                                (animation.to * scale(hwnd)).round() as i32,
                                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                            );
                        }
                        invalidate(hwnd);
                    }
                    let mut bounds = RECT::default();
                    let mut pointer = POINT::default();
                    if unsafe { GetWindowRect(hwnd, &raw mut bounds) } != 0
                        && unsafe { GetCursorPos(&raw mut pointer) } != 0
                    {
                        move_origin = Some(super::snap::DragOrigin::new(bounds, pointer));
                    }
                    Some(0)
                }
                WM_LBUTTONDOWN => {
                    let p = point(lparam);
                    let s = scale(hwnd);
                    let r = client(hwnd);
                    let hit = super::tabs::hit(&model.borrow(), r.right as f32 / s, p.x as f32 / s, p.y as f32 / s);
                    if let Some(hit) = hit {
                        tab_press = Some((hit, p));
                        unsafe { SetFocus(hwnd); SetCapture(hwnd); }
                        return Some(0);
                    }
                    let bar = scrollbar(hwnd, &model.borrow())
                        .filter(|bar| bar.contains(p.x as f32 / s, p.y as f32 / s));
                    if let Some(bar) = bar {
                        let y = p.y as f32 / s;
                        let dragging = bar.on_thumb(y);
                        {
                            let mut m = model.borrow_mut();
                            m.hovered_item = None;
                            m.scrollbar.hovered = true;
                            m.scrollbar.dragging = dragging;
                            if dragging {
                                scrollbar_drag = Some(y - bar.thumb_top);
                            } else {
                                m.scroll = bar.page_to(m.scroll, y);
                            }
                        }
                        unsafe {
                            SetFocus(hwnd);
                            if dragging {
                                SetCapture(hwnd);
                            }
                        }
                        track_client_leave(hwnd);
                        invalidate(hwnd);
                        return Some(0);
                    }
                    let divider = column_divider(hwnd, &model.borrow(), p);
                    if let Some(divider) = divider {
                        let m = model.borrow();
                        let full =
                            super::columns::bounds(grid(hwnd, &m).cell_width, m.folder_columns);
                        column_drag = Some(ColumnDrag {
                            divider,
                            bounds: m.list_columns(grid(hwnd, &m).cell_width),
                            original: m.folder_columns,
                            proportions: std::array::from_fn(|i| {
                                (full[i + 1] - full[i]) / (full[4] - full[0])
                            }),
                            visible: m.folder_visible_columns,
                        });
                        drop(m);
                        unsafe {
                            SetFocus(hwnd);
                            SetCapture(hwnd);
                        }
                        return Some(0);
                    }
                    if model.borrow().is_list()
                        && model.borrow().folder.is_some()
                        && p.y as f32 / s >= model.borrow().content_header()
                        && p.y as f32 / s < model.borrow().content_header() + super::layout::LIST_HEADER
                    {
                        let columns = model
                            .borrow()
                            .list_columns(grid(hwnd, &model.borrow()).cell_width);
                        let x = p.x as f32 / s - super::layout::PADDING;
                        if let Some(column) =
                            (0..4).find(|i| x >= columns[*i] && x < columns[*i + 1])
                        {
                            event(Event::SortFolder(column as u8));
                        }
                        return Some(0);
                    }
                    if p.y as f32 / s < HEADER {
                        let button = model.borrow().header_button(
                            r.right as f32 / s,
                            p.x as f32 / s,
                            p.y as f32 / s,
                        );
                        model.borrow_mut().pressed_button = button;
                        update_pointer(hwnd, &model, Some(p));
                        if button.is_some() {
                            unsafe {
                                SetCapture(hwnd);
                            }
                        }
                        invalidate(hwnd);
                    } else {
                        let selected = {
                            let m = model.borrow();
                            m.hit(grid(hwnd, &m), p.x as f32 / s, p.y as f32 / s, s)
                        };
                        let modifiers = super::keyboard::Modifiers::current();
                        {
                            let mut m = model.borrow_mut();
                            if let Some(index) = selected {
                                if modifiers.ctrl
                                    || modifiers.shift
                                    || !m.selection.contains(&index)
                                {
                                    m.select_item(index, modifiers.ctrl, modifiers.shift);
                                }
                            } else if !modifiers.ctrl && !modifiers.shift {
                                m.clear_selection();
                            }
                        }
                        if selected.is_some() {
                            event(Event::PaneItemFocus);
                        }
                        drag = selected
                            .filter(|_| !modifiers.ctrl && !modifiers.shift)
                            .map(|index| (index, p, false));
                        drag_identity =
                            selected.map(|index| model.borrow().items[index].identity.clone());
                        unsafe {
                            SetFocus(hwnd);
                            if drag.is_some() {
                                SetCapture(hwnd);
                            }
                        }
                        invalidate(hwnd);
                    }
                    Some(0)
                }
                WM_MOUSEMOVE => {
                    if let Some((_, origin)) = tab_press {
                        let current = point(lparam);
                        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
                        let crossed = tab_drag_threshold(origin, current, dpi);
                        if crossed && !model.borrow().locked && wparam as u32 & 1 != 0 {
                            // Clear the pending click before handing off to the native move loop.
                            // Posting avoids re-entering this window callback while it is borrowed.
                            tab_press = None;
                            unsafe {
                                ReleaseCapture();
                                let mut screen = current;
                                ClientToScreen(hwnd, &raw mut screen);
                                let position = (screen.x as u16 as usize | ((screen.y as u16 as usize) << 16)) as isize;
                                PostMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, position);
                            }
                        }
                        return Some(0);
                    }
                    if let Some(offset) = scrollbar_drag {
                        let mut m = model.borrow_mut();
                        if let Some(bar) = scrollbar(hwnd, &m) {
                            m.scroll = bar.drag_to(point(lparam).y as f32 / scale(hwnd), offset);
                        }
                        drop(m);
                        invalidate(hwnd);
                        return Some(0);
                    }
                    if let Some(drag) = &column_drag {
                        let x = point(lparam).x as f32 / scale(hwnd) - super::layout::PADDING;
                        model.borrow_mut().folder_columns = Some(super::columns::resize_visible(
                            drag.bounds,
                            drag.divider,
                            x,
                            drag.proportions,
                            drag.visible,
                        ));
                        unsafe {
                            SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_SIZEWE));
                        }
                        invalidate(hwnd);
                        return Some(0);
                    }
                    update_pointer(hwnd, &model, Some(point(lparam)));
                    track_client_leave(hwnd);
                    let s = scale(hwnd);
                    if let Some((index, start, moved)) = drag.as_mut() {
                        let current = drag_identity.as_ref().and_then(|identity| {
                            model
                                .borrow()
                                .items
                                .iter()
                                .position(|item| &item.identity == identity)
                        });
                        let Some(current) = current else {
                            return Some(0);
                        };
                        *index = current;
                        let p = point(lparam);
                        *moved |= (p.x - start.x).abs() > unsafe { GetSystemMetrics(SM_CXDRAG) }
                            || (p.y - start.y).abs() > unsafe { GetSystemMetrics(SM_CYDRAG) };
                        if *moved {
                            if model.borrow().folder.is_some() {
                                drag = None;
                                drag_identity = None;
                                unsafe {
                                    ReleaseCapture();
                                }
                                event(Event::FileDrag);
                                return Some(0);
                            }
                            let mut screen = p;
                            unsafe {
                                ClientToScreen(hwnd, &raw mut screen);
                            }
                            if drag_image.is_none() {
                                let m = model.borrow();
                                let grid = grid(hwnd, &m);
                                let selected = if m.selection.contains(index) {
                                    m.selection.iter().copied().collect::<Vec<_>>()
                                } else {
                                    vec![*index]
                                };
                                let placeholder = super::assets::Pixels {
                                    width: 1,
                                    height: 1,
                                    data: vec![0; 4],
                                };
                                let cells = selected
                                    .into_iter()
                                    .filter_map(|index| {
                                        let item = m.items.get(index)?;
                                        let render = if m.is_list() {
                                            super::drag_drop::image::list_item_pixels
                                        } else {
                                            super::drag_drop::image::item_pixels
                                        };
                                        let pixels = render(
                                            item.image.as_deref().unwrap_or(&placeholder),
                                            &item.label,
                                            grid,
                                            s,
                                        )?;
                                        let (x, y) = m.cell(grid, index);
                                        Some((
                                            pixels,
                                            POINT {
                                                x: (x * s).round() as i32,
                                                y: (y * s).round() as i32,
                                            },
                                        ))
                                    })
                                    .collect::<Vec<_>>();
                                if let Some((pixels, origin)) =
                                    super::drag_drop::image::selection_pixels(&cells)
                                {
                                    let hotspot = POINT {
                                        x: start.x - origin.x,
                                        y: start.y - origin.y,
                                    };
                                    let size = windows_sys::Win32::Foundation::SIZE {
                                        cx: pixels.width as i32,
                                        cy: pixels.height as i32,
                                    };
                                    drop(m);
                                    drag_image = super::drag_drop::image::DragImage::new(
                                        hwnd, &pixels, screen, hotspot, size,
                                    );
                                }
                            }
                            if let Some(image) = &drag_image {
                                image.move_to(screen);
                            }
                        }
                    }
                    Some(0)
                }
                WM_MOUSELEAVE => {
                    update_pointer(hwnd, &model, None);
                    Some(0)
                }
                WM_LBUTTONUP => {
                    if let Some((pressed, _)) = tab_press.take() {
                        unsafe { ReleaseCapture(); }
                        let p = point(lparam);
                        let scale = scale(hwnd);
                        let hit = super::tabs::hit(&model.borrow(), client(hwnd).right as f32 / scale, p.x as f32 / scale, p.y as f32 / scale);
                        if hit == Some(pressed) { event(Event::SelectTab(pressed)); }
                        return Some(0);
                    }
                    if scrollbar_drag.take().is_some() {
                        model.borrow_mut().scrollbar.dragging = false;
                        unsafe {
                            ReleaseCapture();
                        }
                        update_pointer(hwnd, &model, Some(point(lparam)));
                        invalidate(hwnd);
                        return Some(0);
                    }
                    if let Some(drag) = column_drag.take() {
                        let widths = model.borrow().folder_columns;
                        unsafe {
                            ReleaseCapture();
                        }
                        invalidate(hwnd);
                        if widths != drag.original {
                            if let Some(widths) = widths {
                                event(Event::SetFolderColumns(widths));
                            }
                        }
                        return Some(0);
                    }
                    let pressed = model.borrow_mut().pressed_button.take();
                    if let Some(button) = pressed {
                        let p = point(lparam);
                        let s = scale(hwnd);
                        let released = model.borrow().header_button(
                            client(hwnd).right as f32 / s,
                            p.x as f32 / s,
                            p.y as f32 / s,
                        );
                        unsafe {
                            ReleaseCapture();
                        }
                        update_pointer(hwnd, &model, Some(p));
                        invalidate(hwnd);
                        if released == Some(button) && model.borrow().header_button_enabled(button)
                        {
                            match button {
                                0 => {
                                    event(Event::Collapse);
                                }
                                1 => unsafe {
                                    PostMessageW(hwnd, WM_CONTEXTMENU, 0, -1);
                                },
                                2 => {
                                    event(Event::FolderBack);
                                }
                                3 => {
                                    event(Event::FolderHome);
                                }
                                _ => {}
                            }
                        }
                        return Some(0);
                    }
                    let old = drag.take();
                    drag_image = None;
                    unsafe {
                        ReleaseCapture();
                    }
                    if let Some((_, _, false)) = old {
                        let index = drag_identity.take().and_then(|identity| {
                            model
                                .borrow()
                                .items
                                .iter()
                                .position(|item| item.identity == identity)
                        });
                        if let Some(index) = index {
                            model.borrow_mut().select_item(index, false, false);
                        }
                        invalidate(hwnd);
                    }
                    if let Some((_, _, true)) = old {
                        let index = drag_identity.take().and_then(|identity| {
                            model
                                .borrow()
                                .items
                                .iter()
                                .position(|item| item.identity == identity)
                        });
                        let Some(index) = index else {
                            return Some(0);
                        };
                        let mut p = point(lparam);
                        unsafe {
                            ClientToScreen(hwnd, &raw mut p);
                        }
                        event(Event::Drop { index, point: p });
                    }
                    Some(0)
                }
                WM_CAPTURECHANGED | WM_CANCELMODE => {
                    tab_press = None;
                    scrollbar_drag = None;
                    model.borrow_mut().scrollbar.dragging = false;
                    if let Some(drag) = column_drag.take() {
                        model.borrow_mut().folder_columns = drag.original;
                    }
                    model.borrow_mut().pressed_button = None;
                    if message == WM_CANCELMODE {
                        unsafe { KillTimer(hwnd, 5); }
                    clip_drag(hwnd, false, &mut drag_clipped);
                        if pane_moved {
                            event(Event::FinishPaneMove(false));
                            pane_moved = false;
                        }
                        unsafe {
                            ReleaseCapture();
                        }
                    }
                    invalidate(hwnd);
                    drag = None;
                    drag_image = None;
                    unsafe { PostMessageW(hwnd, SYNC_POINTER, 0, 0); }
                    Some(0)
                }
                WM_LBUTTONDBLCLK => {
                    let p = point(lparam);
                    let dpi = scale(hwnd);
                    let hit = super::tabs::hit(&model.borrow(), client(hwnd).right as f32 / dpi, p.x as f32 / dpi, p.y as f32 / dpi);
                    if hit.is_some() { return Some(0); }
                    let p = point(lparam);
                    let s = scale(hwnd);
                    if scrollbar(hwnd, &model.borrow())
                        .is_some_and(|bar| bar.contains(p.x as f32 / s, p.y as f32 / s))
                    {
                        return Some(0);
                    }
                    let selected = {
                        let m = model.borrow();
                        m.hit(grid(hwnd, &m), p.x as f32 / s, p.y as f32 / s, s)
                    };
                    if let Some(index) = selected {
                        event(Event::Activate(index));
                    }
                    Some(0)
                }
                WM_NCLBUTTONDBLCLK => {
                    if wparam == HTCAPTION as usize {
                        event(Event::RenameTitle);
                    }
                    Some(0)
                }
                WM_MOUSEWHEEL => {
                    let delta = point(wparam.cast_signed()).y;
                    let mut m = model.borrow_mut();
                    let max = grid(hwnd, &m).max_scroll(m.items.len());
                    m.scroll = if delta < 0 {
                        (m.scroll + 1).min(max)
                    } else {
                        m.scroll.saturating_sub(1)
                    };
                    drop(m);
                    invalidate(hwnd);
                    Some(0)
                }
                WM_KEYUP if wparam == usize::from(VK_APPS) => Some(0),
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    use super::keyboard::{self, Command, Modifiers};
                    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_LEFT;
                    let navigation_mods = Modifiers::current();
                    if model.borrow().folder.is_some()
                        && model.borrow().renaming.is_none()
                        && navigation_mods.alt
                        && !navigation_mods.ctrl
                        && !navigation_mods.shift
                        && wparam as u16 == VK_LEFT
                    {
                        event(Event::FolderBack);
                        return Some(0);
                    }
                    if model.borrow().renaming.is_some() {
                        return None;
                    }
                    let modifiers = Modifiers::current();
                    let repeated = lparam & (1 << 30) != 0;
                    let Some(command) = u16::try_from(wparam).ok().and_then(|key| {
                        if super::peek::matches(key, &modifiers, repeated) {
                            Some(Command::Peek)
                        } else {
                            keyboard::command(key, &modifiers, repeated)
                        }
                    }) else {
                        return None;
                    };
                    match command {
                        Command::Cancel => {
                            if drag.take().is_some() {
                                drag_identity = None;
                                drag_image = None;
                                unsafe {
                                    ReleaseCapture();
                                }
                            } else {
                                model.borrow_mut().clear_selection();
                            }
                            invalidate(hwnd);
                        }
                        Command::Open => {
                            event(Event::ActivateSelection);
                        }
                        Command::Peek => {
                            event(Event::Peek);
                        }
                        Command::Rename => {
                            let identity = {
                                let m = model.borrow();
                                if m.selection.len() == 1 {
                                    m.selection
                                        .first()
                                        .and_then(|i| m.items.get(*i))
                                        .map(|i| i.identity.clone())
                                } else {
                                    None
                                }
                            };
                            if let Some(identity) = identity {
                                event(Event::RenameItem(identity));
                            }
                        }
                        Command::SelectAll => {
                            model.borrow_mut().select_all();
                            if !model.borrow().selection.is_empty() {
                                event(Event::PaneItemFocus);
                            }
                            invalidate(hwnd);
                        }
                        Command::ToggleSelection => {
                            let mut m = model.borrow_mut();
                            let index = m.selected.or_else(|| (!m.items.is_empty()).then_some(0));
                            if let Some(index) = index {
                                m.select_item(index, true, false);
                            }
                            drop(m);
                            if index.is_some() {
                                event(Event::PaneItemFocus);
                            }
                            invalidate(hwnd);
                        }
                        Command::Refresh => {
                            event(Event::Refresh);
                        }
                        Command::File(command) => {
                            event(Event::FileCommand(command));
                        }
                        Command::Menu => unsafe {
                            PostMessageW(hwnd, WM_CONTEXTMENU, hwnd as usize, -1);
                        },
                        Command::Navigate(key) => {
                            let mut m = model.borrow_mut();
                            if m.collapsed {
                                return Some(0);
                            }
                            let grid = grid(hwnd, &m);
                            let next = keyboard::next_selection(
                                key,
                                m.selected,
                                m.items.len(),
                                grid.columns,
                                grid.visible_rows,
                            );
                            if let Some(next) = next {
                                if modifiers.ctrl && !modifiers.shift {
                                    if m.selection_anchor.is_none() {
                                        m.selection_anchor = m.selected;
                                    }
                                    m.selected = Some(next);
                                } else {
                                    m.select_item(next, modifiers.ctrl, modifiers.shift);
                                }
                                let row = next / grid.columns.max(1);
                                let rows = grid.visible_rows.max(1);
                                if row < m.scroll {
                                    m.scroll = row;
                                } else if row >= m.scroll + rows {
                                    m.scroll = row - rows + 1;
                                }
                                m.scroll = m.scroll.min(grid.max_scroll(m.items.len()));
                            }
                            drop(m);
                            if next.is_some() {
                                event(Event::PaneItemFocus);
                            }
                            invalidate(hwnd);
                        }
                    }
                    Some(0)
                }
                WM_CONTEXTMENU => {
                    if menu_active.get() {
                        return Some(0);
                    }
                    let activity = MenuActivity::begin(Rc::clone(&menu_active), hwnd);
                    auto_hide.reset();
                    let model = Rc::clone(&model);
                    let events = Rc::clone(&events);
                    let window_state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
                    if !defer_action(move || {
                        let _activity = activity;
                        if unsafe { IsWindow(hwnd) } == 0
                            || unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } != window_state
                        {
                            return;
                        }
                        let event = |value| (events.borrow_mut())(value);
                        let mut anchor = point(lparam);
                        let mut tab_context = None;
                        if lparam != -1 {
                            let mut p = anchor;
                            unsafe {
                                windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &raw mut p);
                            }
                            let tab = super::tabs::hit(&model.borrow(), client(hwnd).right as f32 / scale(hwnd), p.x as f32 / scale(hwnd), p.y as f32 / scale(hwnd));
                            if let Some(tab) = tab {
                                tab_context = Some(tab);
                            }
                            let column_menu = {
                                let m = model.borrow();
                                (m.folder.is_some()
                                    && m.is_list()
                                    && !m.collapsed
                                    && (m.content_header()..m.content_header() + super::layout::LIST_HEADER)
                                        .contains(&(p.y as f32 / scale(hwnd))))
                                .then_some((m.theme, m.backdrop, m.folder_visible_columns))
                            };
                            if let Some((theme, backdrop, visible)) = column_menu {
                                let command = super::menu::show_entries(
                                    hwnd,
                                    anchor,
                                    false,
                                    theme,
                                    backdrop,
                                    super::menu::column_entries(visible),
                                );
                                if (31..=33).contains(&command) {
                                    event(Event::ToggleFolderColumn((command - 30) as u8));
                                }
                                return;
                            }
                        }
                        let index = if lparam == -1 {
                            if wparam != 0 {
                                let m = model.borrow();
                                m.selected
                                    .filter(|i| m.selection.contains(i))
                                    .or_else(|| m.selection.first().copied())
                            } else {
                                None
                            }
                        } else {
                            let mut p = anchor;
                            unsafe {
                                windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &raw mut p);
                            }
                            let m = model.borrow();
                            m.hit(
                                grid(hwnd, &m),
                                p.x as f32 / scale(hwnd),
                                p.y as f32 / scale(hwnd),
                                scale(hwnd),
                            )
                        };
                        if let Some(index) = index {
                            let (identity, identities) = {
                                let mut m = model.borrow_mut();
                                if !m.selection.contains(&index) {
                                    m.select_item(index, false, false);
                                }
                                m.selected = Some(index);
                                (m.items[index].identity.clone(), m.selected_identities())
                            };
                            if lparam == -1 {
                                let m = model.borrow();
                                let g = grid(hwnd, &m);
                                let (x, y) = m.cell(g, index);
                                let s = scale(hwnd);
                                let mut rect = RECT::default();
                                unsafe {
                                    GetClientRect(hwnd, &raw mut rect);
                                    anchor = POINT {
                                        x: ((x + g.cell_width * 0.5) * s).round().clamp(0.0, rect.right.max(0) as f32) as i32,
                                        y: ((y + g.icon_size) * s).round().clamp(0.0, rect.bottom.max(0) as f32) as i32,
                                    };
                                    windows_sys::Win32::Graphics::Gdi::ClientToScreen(hwnd, &raw mut anchor);
                                }
                            }
                            invalidate(hwnd);
                            if model.borrow().folder.is_some() || !super::compact_menu::enabled() {
                                let result = desktop_shell::show_file_items_menu(
                                    windows::Win32::Foundation::HWND(hwnd),
                                    &identities,
                                    windows::Win32::Foundation::POINT {
                                        x: anchor.x,
                                        y: anchor.y,
                                    },
                                );
                                match result {
                                    Ok(true) => {
                                        event(Event::RenameItem(identity));
                                    }
                                    Ok(false) => {
                                        event(Event::Refresh);
                                    }
                                    Err(message) => error(&message.to_string()),
                                }
                                return;
                            }
                            let reply = Rc::new(RefCell::new(None));
                            event(Event::BeginItemMenu(Rc::clone(&reply)));
                            let hook = reply
                                .borrow_mut()
                                .take()
                                .unwrap_or_else(|| Err(crate::i18n::text("ui-menu-is-not-ready").into()));
                            let hook = match hook {
                                Ok(hook) => hook,
                                Err(message) => {
                                    error(&message);
                                    return;
                                }
                            };
                            let result = super::shell_menu::show_many(
                                hwnd, &hook, &identities, anchor, lparam == -1,
                            );
                            event(Event::EndItemMenu);
                            match result {
                                Ok(true) => {
                                    event(Event::RenameItem(identity));
                                }
                                Ok(false) => {}
                                Err(message) => error(&message),
                            }
                            // Inventory polling detects rename/delete; dismissing a menu must not
                            // discard every image and trigger a visible reload.
                            return;
                        }
                        let (auto_hide, locked, theme, backdrop, collapsed) = {
                            let m = model.borrow();
                            (m.auto_hide, m.locked, m.theme, m.backdrop, m.collapsed)
                        };
                        update_pointer(hwnd, &model, None);
                        invalidate(hwnd);
                        let is_folder = {
                            let model = model.borrow();
                            (model.folder.is_some(), model.is_list())
                        };
                        let visible_columns = model.borrow().folder_visible_columns;
                        let command = if tab_context.is_some() {
                            let entries = super::menu::tab_context_entries(&model.borrow(), super::quick_reveal::permanent_topmost(hwnd));
                            super::menu::show_entries(hwnd, anchor, false, theme, backdrop, entries)
                        } else { menu(
                            hwnd,
                            lparam,
                            auto_hide,
                            locked,
                            theme,
                            backdrop,
                            is_folder,
                            visible_columns,
                            collapsed,
                        ) };
                        update_pointer(hwnd, &model, None);
                        invalidate(hwnd);
                        match command {
                            49 => { let id = tab_context.unwrap_or(model.borrow().active_tab); event(Event::DetachTab(id)); }
                            48 => { event(Event::Collapse); }
                            40 => { event(Event::NewTab(false)); }
                            41 => { event(Event::NewTab(true)); }
                            42 => { let id = tab_context.unwrap_or(model.borrow().active_tab); event(Event::CloseTabId(id)); }
                            43 => { if let Some(id) = tab_context { event(Event::RenameTab(id)); } else { event(Event::RenameTitle); } }
                            44 | 45 => {
                                let step = if command == 44 { -1 } else { 1 };
                                if let Some(id) = tab_context { event(Event::MoveTabId(id, step)); }
                                else { event(Event::MoveTab(step)); }
                            }
                            46 | 47 => {
                                let next = super::tabs::adjacent(&model.borrow(), if command == 46 { -1 } else { 1 });
                                if let Some(next) = next { event(Event::SelectTab(next)); }
                            }
                            31..=33 => {
                                event(Event::ToggleFolderColumn((command - 30) as u8));
                            }
                            10 => {
                                event(Event::ToggleLocked);
                            }
                            25 | 26 => {
                                if model.borrow().is_list() != (command == 26) {
                                    event(Event::ToggleListView);
                                }
                            }
                            19 => {
                                event(Event::NewFolder);
                            }
                            20 => {
                                event(Event::OpenFolder);
                            }
                            21 => {
                                event(Event::ChangeFolder);
                            }
                            1 => {
                                event(Event::New);
                            }
                            4 => {
                                event(Event::Exit);
                            }
                            7 => {
                                event(Event::ToggleAutoHide);
                            }
                            12 => {
                                event(Event::ToggleTopmost);
                            }
                            9 => {
                                event(Event::Refresh);
                            }
                            11 => {
                                event(Event::ClosePane);
                            }
                            18 => {
                                event(Event::Settings);
                            }
                            _ => {}
                        }
                    }) {
                        error("Could not schedule the pane menu");
                    }
                    Some(0)
                }
                WM_CLOSE => {
                    let events = Rc::clone(&events);
                    defer_action(move || { (events.borrow_mut())(Event::ClosePane); });
                    Some(0)
                }
                WM_SYSCOMMAND if wparam & 0xfff0 == SC_CLOSE as usize => {
                    unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0); }
                    Some(0)
                }
                _ => None,
            }
        })
        .create()
        .map_err(|e| e.to_string())?;
    let hwnd = window.hwnd().cast();
    crate::app_icon::apply(hwnd)?;
    let s = scale(hwnd);
    unsafe {
        if !inspect {
            SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, GetShellWindow() as isize);
            SetWindowPos(
                hwnd,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        if windows_sys::Win32::UI::Shell::SetWindowSubclass(hwnd, Some(borderless_proc), 1, 0) == 0
        {
            return Err(crate::i18n::text("ui-could-not-initialize-borderless-group-window").into());
        }
        let class_style = GetClassLongPtrW(hwnd, GCL_STYLE);
        SetClassLongPtrW(
            hwnd,
            GCL_STYLE,
            (class_style | CS_DBLCLKS as usize).cast_signed(),
        );
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            (bounds.x * s) as i32,
            (bounds.y * s) as i32,
            (bounds.width * s) as i32,
            ((if model_init.borrow().collapsed {
                HEADER
            } else {
                bounds.height
            }) * s) as i32,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        if model_init.borrow().auto_hide {
            PostMessageW(hwnd, SYNC_POINTER, 0, 0);
        }
    }
    invalidate(hwnd);
    // Bootstrap the transparent composition content before the first visible frame.
    unsafe {
        SendMessageW(hwnd, WM_PAINT, 0, 0);
        prepared.set(true);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Ok(window)
}

fn menu(
    hwnd: HWND,
    lparam: isize,
    auto_hide: bool,
    locked: bool,
    theme: desktop_core::PanelTheme,
    backdrop: desktop_core::Backdrop,
    folder: (bool, bool),
    visible_columns: u8,
    collapsed: bool,
) -> i32 {
    let anchored = lparam == -1;
    let mut anchor = point(lparam);
    if anchored {
        let dpi = scale(hwnd);
        anchor = POINT {
            x: client(hwnd).right - (10.0 * dpi) as i32,
            y: ((HEADER + 4.0) * dpi) as i32,
        };
        unsafe {
            ClientToScreen(hwnd, &raw mut anchor);
        }
    }
    super::menu::show(
        hwnd,
        anchor,
        anchored,
        auto_hide,
        locked,
        theme,
        backdrop,
        folder,
        visible_columns,
        collapsed,
    )
}
pub fn error(message: &str) {
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(message).as_ptr(),
            wide("LucidDesk").as_ptr(),
            MB_OK | MB_ICONWARNING,
        );
    }
}
