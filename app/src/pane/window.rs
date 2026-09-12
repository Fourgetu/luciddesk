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
pub const ANIMATE_FOLD: u32 = WM_APP + 10;
const SYNC_POINTER: u32 = WM_APP + 11;
pub(super) const RUN_POSTED_ACTION: u32 = WM_APP + 12;
const DESKTOP_LAYER: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.DesktopLayer");
const CLOSING_PANE: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.ClosingPane");

pub(super) fn prepare_close(hwnd: HWND) {
    unsafe {
        SetPropW(hwnd, CLOSING_PANE, 1usize as _);
    }
}

pub fn set_layer(hwnd: HWND, always_on_top: bool) {
    unsafe {
        if always_on_top {
            RemovePropW(hwnd, DESKTOP_LAYER);
        } else {
            SetPropW(hwnd, DESKTOP_LAYER, 1usize as _);
        }
        SetWindowPos(
            hwnd,
            if always_on_top {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            },
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        if !always_on_top {
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
        let in_client = frame_hit(client(hwnd), p, scale(hwnd), m.collapsed, m.locked) == HTCLIENT;
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
    let (button, item) = if let Some(p) = pointer {
        let s = scale(hwnd);
        let button = {
            super::layout::header_button(
                client(hwnd).right as f32 / s,
                p.x as f32 / s,
                p.y as f32 / s,
            )
        };
        (
            button,
            m.hit(grid(hwnd, &m), p.x as f32 / s, p.y as f32 / s, s),
        )
    } else {
        (None, None)
    };
    if (m.hovered_button, m.hovered_item) != (button, item) {
        m.hovered_button = button;
        m.hovered_item = item;
        drop(m);
        invalidate(hwnd);
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
    unsafe {
        if message == WM_WINDOWPOSCHANGING && !GetPropW(hwnd, DESKTOP_LAYER).is_null() {
            let position = &mut *(lparam as *mut WINDOWPOS);
            if position.flags & SWP_NOZORDER == 0 {
                position.hwndInsertAfter = HWND_BOTTOM;
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
    let mut drag: Option<(usize, POINT, bool)> = None;
    let mut drag_identity = None;
    let mut drag_image: Option<super::drag_image::DragImage> = None;
    let mut fold: Option<super::animation::Fold> = None;
    let mut move_origin: Option<super::snap::DragOrigin> = None;
    let mut hover_state: Option<(bool, std::time::Instant)> = None;
    let menu_active = Rc::new(std::cell::Cell::new(false));
    let mut paint_error = false;
    let model_init = Rc::clone(&model);
    let inspect = std::env::var_os("LUCIDPANE_INSPECT").is_some();
    let window_title = format!("LucidPane — {}", model.borrow().title);
    let window = Window::new(&window_title)
        .size(bounds.width as i32, bounds.height as i32)
        .style(WS_POPUP | WS_THICKFRAME | WS_SYSMENU)
        .ex_style(
            WS_EX_NOREDIRECTIONBITMAP
                | if !inspect {
                    WS_EX_TOOLWINDOW
                } else {
                    WS_EX_APPWINDOW
                },
        )
        .on_message(move |raw, message, wparam, lparam| {
            let hwnd: HWND = raw.cast();
            let event = |value| (events.borrow_mut())(value);
            match message {
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
                WM_ACTIVATE => {
                    unsafe {
                        PostMessageW(hwnd, SYNC_POINTER, 0, 0);
                    }
                    None
                }
                WM_SETTINGCHANGE | WM_THEMECHANGED => {
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
                    if let Some(origin) = &move_origin {
                        let mut pointer = POINT::default();
                        if unsafe { GetCursorPos(&raw mut pointer) } != 0 {
                            unsafe {
                                *(lparam as *mut RECT) = origin.proposal(pointer);
                            }
                        }
                    }
                    event(Event::Moving(lparam as *mut RECT));
                    Some(1)
                }
                WM_TIMER if wparam == 3 => {
                    let (enabled, collapsed) = {
                        let m = model.borrow();
                        (m.auto_hide, m.collapsed)
                    };
                    if !enabled || menu_active.get() || super::rename::active(hwnd) {
                        hover_state = None;
                        return Some(0);
                    }
                    // Do not fold a pane while it owns a drag or a resize operation.
                    if drag.is_some()
                        || unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture() }
                            == hwnd
                    {
                        hover_state = None;
                        return Some(0);
                    }
                    let mut cursor = POINT::default();
                    unsafe {
                        GetCursorPos(&raw mut cursor);
                    }
                    let hovered = unsafe { WindowFromPoint(cursor) } == hwnd;
                    let now = std::time::Instant::now();
                    let (previous, since) = *hover_state.get_or_insert((hovered, now));
                    if previous != hovered {
                        hover_state = Some((hovered, now));
                    } else if since.elapsed()
                        >= std::time::Duration::from_millis(if hovered { 120 } else { 600 })
                        && collapsed == hovered
                    {
                        event(Event::SetCollapsed(!hovered));
                    }
                    Some(0)
                }
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
                    fold = Some(super::animation::Fold {
                        from: client(hwnd).bottom as f32 / scale(hwnd),
                        to: lparam as f32,
                        from_reveal: m.reveal,
                        to_reveal: if m.collapsed { 0.0 } else { 1.0 },
                        started: std::time::Instant::now(),
                        duration: std::time::Duration::from_millis(if enabled != 0 {
                            200
                        } else {
                            0
                        }),
                    });
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
                    let hit = frame_hit(r, p, scale(hwnd), m.collapsed, m.locked);
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
                        ((cell.0 + super::layout::PADDING * 2.0) * s).ceil() as i32;
                    info.ptMinTrackSize.y = ((if m.collapsed {
                        HEADER
                    } else {
                        let rows = { m.row_contents(grid(hwnd, &m)) };
                        HEADER
                            + super::layout::PADDING * 2.0
                            + rows.get(m.scroll).copied().unwrap_or(cell.1)
                    }) * s)
                        .ceil() as i32;
                    Some(0)
                }
                WM_SIZING => {
                    if model.borrow().is_list() {
                        return None;
                    }
                    let rect = unsafe { &mut *(lparam as *mut RECT) };
                    let m = model.borrow();
                    if m.items.is_empty() {
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
                    Some(1)
                }
                WM_PAINT => {
                    let mut ps = PAINTSTRUCT::default();
                    unsafe {
                        BeginPaint(hwnd, &raw mut ps);
                        EndPaint(hwnd, &raw const ps);
                    }
                    let r = client(hwnd);
                    if r.right > 0 && r.bottom > 0 {
                        let s = scale(hwnd);
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
                                eprintln!("分组渲染失败：{error}");
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
                WM_TIMER => {
                    if event(Event::Tick) {
                        unsafe {
                            KillTimer(hwnd, 1);
                        }
                    }
                    Some(0)
                }
                WM_LBUTTONDOWN => {
                    let p = point(lparam);
                    let s = scale(hwnd);
                    let r = client(hwnd);
                    if model.borrow().is_list()
                        && p.y as f32 / s >= HEADER
                        && p.y as f32 / s < HEADER + super::layout::LIST_HEADER
                    {
                        let columns =
                            super::layout::list_columns(grid(hwnd, &model.borrow()).cell_width);
                        let x = p.x as f32 / s - super::layout::PADDING;
                        if let Some(column) =
                            (0..3).find(|i| x >= columns[*i] && x < columns[*i + 1])
                        {
                            event(Event::SortFolder(column as u8));
                        }
                        return Some(0);
                    }
                    if p.y as f32 / s < HEADER {
                        let button = super::layout::header_button(
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
                                let image = model
                                    .borrow()
                                    .items
                                    .get(*index)
                                    .and_then(|item| item.image.clone());
                                if let Some(image) = image {
                                    let m = model.borrow();
                                    let grid = grid(hwnd, &m);
                                    let (cell_x, cell_y) = m.cell(grid, *index);
                                    let Some(pixels) = super::drag_image::item_pixels(
                                        &image,
                                        &m.items[*index].label,
                                        grid,
                                        s,
                                    ) else {
                                        return Some(0);
                                    };
                                    let hotspot = POINT {
                                        x: start.x - (cell_x * s).round() as i32,
                                        y: start.y - (cell_y * s).round() as i32,
                                    };
                                    let size = windows_sys::Win32::Foundation::SIZE {
                                        cx: pixels.width as i32,
                                        cy: pixels.height as i32,
                                    };
                                    drop(m);
                                    drag_image = super::drag_image::DragImage::new(
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
                    let pressed = model.borrow_mut().pressed_button.take();
                    if let Some(button) = pressed {
                        let p = point(lparam);
                        let s = scale(hwnd);
                        let released = super::layout::header_button(
                            client(hwnd).right as f32 / s,
                            p.x as f32 / s,
                            p.y as f32 / s,
                        );
                        unsafe {
                            ReleaseCapture();
                        }
                        update_pointer(hwnd, &model, Some(p));
                        invalidate(hwnd);
                        if released == Some(button) {
                            match button {
                                0 => {
                                    event(Event::Collapse);
                                }
                                1 => unsafe {
                                    PostMessageW(hwnd, WM_CONTEXTMENU, 0, -1);
                                },
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
                    model.borrow_mut().pressed_button = None;
                    if message == WM_CANCELMODE {
                        unsafe {
                            ReleaseCapture();
                        }
                    }
                    invalidate(hwnd);
                    drag = None;
                    drag_image = None;
                    Some(0)
                }
                WM_LBUTTONDBLCLK => {
                    let p = point(lparam);
                    let s = scale(hwnd);
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
                    hover_state = None;
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
                                unsafe {
                                    GetCursorPos(&raw mut anchor);
                                }
                            }
                            invalidate(hwnd);
                            if model.borrow().folder.is_some() {
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
                            event(Event::MenuSelection(true));
                            let result = super::shell_menu::show_many(
                                hwnd,
                                &identities,
                                anchor,
                                lparam == -1,
                            );
                            event(Event::ItemMenuEnded(identity));
                            if let Err(message) = result {
                                error(&message);
                            }
                            // Inventory polling detects rename/delete; dismissing a menu must not
                            // discard every image and trigger a visible reload.
                            return;
                        }
                        let (auto_hide, locked, theme) = {
                            let m = model.borrow();
                            (m.auto_hide, m.locked, m.theme)
                        };
                        update_pointer(hwnd, &model, None);
                        invalidate(hwnd);
                        let is_folder = {
                            let model = model.borrow();
                            model.folder.as_ref().map(|_| model.folder_list)
                        };
                        let command = menu(hwnd, lparam, auto_hide, locked, theme, is_folder);
                        update_pointer(hwnd, &model, None);
                        invalidate(hwnd);
                        match command {
                            23 => {
                                event(Event::FolderBack);
                            }
                            10 => {
                                event(Event::ToggleLocked);
                            }
                            22 => {
                                event(Event::ToggleFolderView);
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
                            3 => {
                                event(Event::Sort);
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
                    event(Event::Exit);
                    Some(0)
                }
                WM_SYSCOMMAND if wparam & 0xfff0 == SC_CLOSE as usize => {
                    event(Event::Exit);
                    Some(0)
                }
                _ => None,
            }
        })
        .create()
        .map_err(|e| e.to_string())?;
    let hwnd = window.hwnd().cast();
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
            return Err("无法初始化无边框分组窗口".into());
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
        SetTimer(hwnd, 1, 100, None);
        SetTimer(hwnd, 3, 60, None);
    }
    invalidate(hwnd);
    // Bootstrap the transparent composition content before the first visible frame.
    unsafe {
        SendMessageW(hwnd, WM_PAINT, 0, 0);
        // A hidden launcher can override the first ShowWindow in windows-window::create.
        // Explicitly reveal only after transparent content has been initialized.
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
    folder: Option<bool>,
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
    super::menu::show(hwnd, anchor, anchored, auto_hide, locked, theme, folder)
}
pub fn error(message: &str) {
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(message).as_ptr(),
            wide("LucidPane").as_ptr(),
            MB_OK | MB_ICONWARNING,
        );
    }
}
