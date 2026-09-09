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
    ReleaseCapture, SetCapture, SetFocus, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_UP,
};
#[allow(clippy::wildcard_imports)]
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_window::Window;
pub const ANIMATE_FOLD: u32 = WM_APP + 10;
const DESKTOP_LAYER: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.DesktopLayer");

pub fn set_layer(hwnd: HWND, always_on_top: bool) {
    unsafe {
        if always_on_top { RemovePropW(hwnd, DESKTOP_LAYER); }
        else { SetPropW(hwnd, DESKTOP_LAYER, 1usize as _); }
        SetWindowPos(hwnd, if always_on_top { HWND_TOPMOST } else { HWND_NOTOPMOST },
            0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        if !always_on_top {
            SetWindowPos(hwnd, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

// Shell menus pump messages while open. Keep auto-hide and repeated menu requests
// suspended until that nested interaction returns, including error paths.
struct MenuActivity(Rc<std::cell::Cell<bool>>);
impl MenuActivity {
    fn begin(active: Rc<std::cell::Cell<bool>>) -> Self {
        active.set(true);
        Self(active)
    }
}
impl Drop for MenuActivity {
    fn drop(&mut self) {
        self.0.set(false);
    }
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
unsafe extern "system" fn borderless_proc(
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    id: usize,
    _data: usize,
) -> isize {
    if message == WM_NCCALCSIZE {
        return 0;
    }
    unsafe {
        if message == WM_WINDOWPOSCHANGING && !GetPropW(hwnd, DESKTOP_LAYER).is_null() {
            let position = &mut *(lparam as *mut WINDOWPOS);
            if position.flags & SWP_NOZORDER == 0 { position.hwndInsertAfter = HWND_BOTTOM; }
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
    mut event: F,
) -> Result<Window, String>
where
    F: FnMut(Event) -> bool + 'static,
{
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
    let mut region_cells = None;
    let model_init = Rc::clone(&model);
    let desktop = model.borrow().desktop;
    let managed = model.borrow().managed;
    let inspect = std::env::var_os("LUCIDPANE_INSPECT").is_some();
    let window_title = format!("LucidPane — {}", model.borrow().title);
    let window = Window::new(&window_title)
        .size(bounds.width as i32, bounds.height as i32)
        .style(if desktop { WS_POPUP } else { WS_POPUP | WS_THICKFRAME | WS_SYSMENU })
        .ex_style(WS_EX_NOREDIRECTIONBITMAP | if managed && !inspect { WS_EX_TOOLWINDOW } else { WS_EX_APPWINDOW })
        .on_message(move |raw, message, wparam, lparam| {
            let hwnd: HWND = raw.cast();
            match message {
                WM_WINDOWPOSCHANGING if desktop => {
                    let position=unsafe {&mut *(lparam as *mut WINDOWPOS)};
                    if position.flags & SWP_NOZORDER==0 {position.hwndInsertAfter=HWND_BOTTOM;}
                    Some(0)
                }
                WM_DISPLAYCHANGE if managed => {
                    // Restore Explorer if display topology changes; stale surfaces must never strand icons.
                    event(Event::Exit); Some(0)
                }
                WM_SETFOCUS | WM_KILLFOCUS => {
                    model.borrow_mut().focused = message == WM_SETFOCUS;
                    invalidate(hwnd); Some(0)
                }
                WM_SETTINGCHANGE | WM_THEMECHANGED => {
                    { let mut m = model.borrow_mut(); m.dark = super::theme::is_dark(m.theme); }
                    if let Ok(new_renderer) = Renderer::new() { renderer = new_renderer; }
                    // Appearance changes invalidate rendering resources, not Shell image inventory.
                    invalidate(hwnd); Some(0)
                }
                WM_KEYDOWN if wparam == 0x74 => { event(Event::Refresh); Some(0) }
                WM_RBUTTONUP => {
                    let mut p=point(lparam);
                    unsafe {
                        ClientToScreen(hwnd,&raw mut p);
                        let packed=(usize::from(p.x as u16)|(usize::from(p.y as u16)<<16)).cast_signed();
                        PostMessageW(hwnd,WM_CONTEXTMENU,hwnd as usize,packed);
                    }
                    Some(0)
                }
                WM_KEYDOWN if wparam==0x5d || (wparam==0x79 && unsafe {windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x10)}<0) => {
                    unsafe {PostMessageW(hwnd,WM_CONTEXTMENU,hwnd as usize,-1);}
                    Some(0)
                }
                WM_MOVING => {
                    if let Some(origin) = &move_origin {
                        let mut pointer = POINT::default();
                        if unsafe { GetCursorPos(&raw mut pointer) } != 0 {
                            unsafe { *(lparam as *mut RECT) = origin.proposal(pointer); }
                        }
                    }
                    event(Event::Moving(lparam as *mut RECT));
                    Some(1)
                }
                WM_TIMER if wparam == 3 => {
                    let (enabled, collapsed) = { let m = model.borrow(); (m.auto_hide, m.collapsed) };
                    if !enabled || menu_active.get() { hover_state = None; return Some(0); }
                    // Do not fold a pane while it owns a drag or a resize operation.
                    if drag.is_some() || unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture() } == hwnd {
                        hover_state = None; return Some(0);
                    }
                    let mut cursor = POINT::default();
                    unsafe { GetCursorPos(&raw mut cursor); }
                    let hovered = unsafe { WindowFromPoint(cursor) } == hwnd;
                    let now = std::time::Instant::now();
                    let (previous, since) = *hover_state.get_or_insert((hovered, now));
                    if previous != hovered { hover_state = Some((hovered, now)); }
                    else if since.elapsed() >= std::time::Duration::from_millis(if hovered { 120 } else { 600 })
                        && collapsed == hovered {
                        event(Event::SetCollapsed(!hovered));
                    }
                    Some(0)
                }
                ANIMATE_FOLD => {
                    let mut enabled = 1i32;
                    unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0); }
                    let m = model.borrow();
                    fold = Some(super::animation::Fold {
                        from: client(hwnd).bottom as f32 / scale(hwnd), to: lparam as f32,
                        from_reveal: m.reveal, to_reveal: if m.collapsed { 0.0 } else { 1.0 },
                        started: std::time::Instant::now(),
                        duration: std::time::Duration::from_millis(if enabled != 0 { 200 } else { 0 }),
                    });
                    drop(m);
                    unsafe { SetTimer(hwnd, 2, 16, None); PostMessageW(hwnd, WM_TIMER, 2, 0); }
                    Some(0)
                }
                WM_TIMER if wparam == 2 => {
                    if let Some(animation) = &fold {
                        let (height, reveal, done) = animation.sample(std::time::Instant::now());
                        model.borrow_mut().reveal = reveal;
                        let bounds = client(hwnd);
                        unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, bounds.right,
                            (height * scale(hwnd)).round() as i32,
                            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE); }
                        invalidate(hwnd);
                        if done { fold = None; unsafe { KillTimer(hwnd, 2); } }
                    }
                    Some(0)
                }
                WM_DESTROY if managed => {event(Event::Exit);Some(0)}
                WM_NCCALCSIZE | WM_ERASEBKGND | WM_DESTROY => Some(0),
                WM_NCHITTEST => {
                    if desktop { return Some(isize::try_from(HTCLIENT).unwrap()); }
                    let mut p = point(lparam);
                    unsafe {
                        windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &raw mut p);
                    }
                    let r = client(hwnd);
                    let border = (5.0 * scale(hwnd)) as i32;
                    let hit = match (
                        p.x < border,
                        p.x >= r.right - border,
                        p.y < border,
                        p.y >= r.bottom - border,
                    ) {
                        (true, _, true, _) => HTTOPLEFT,
                        (_, true, true, _) => HTTOPRIGHT,
                        (true, _, _, true) => HTBOTTOMLEFT,
                        (_, true, _, true) => HTBOTTOMRIGHT,
                        (true, _, _, _) => HTLEFT,
                        (_, true, _, _) => HTRIGHT,
                        (_, _, true, _) => HTTOP,
                        (_, _, _, true) => HTBOTTOM,
                        _ if p.y as f32 / scale(hwnd) < HEADER
                            && p.x as f32 / scale(hwnd) < r.right as f32 / scale(hwnd) - 70.0 =>
                        {
                            HTCAPTION
                        }
                        _ => HTCLIENT,
                    };
                    Some(isize::try_from(hit).unwrap_or_default())
                }
                WM_GETMINMAXINFO => {
                    let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
                    info.ptMinTrackSize.x = (240.0 * scale(hwnd)) as i32;
                    info.ptMinTrackSize.y = ((if model.borrow().collapsed {
                        HEADER
                    } else {
                        180.0
                    }) * scale(hwnd)) as i32;
                    Some(0)
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
                                surface =
                                    Some(Surface::new(windows::Win32::Foundation::HWND(hwnd))?);
                                if !desktop {
                                    Surface::disable_window_shadow(windows::Win32::Foundation::HWND(hwnd))?;
                                }
                            }
                            let surface = surface.as_mut().unwrap();
                            surface.theme(windows::Win32::Foundation::HWND(hwnd), model.borrow().dark);
                            if !desktop { surface.material(
                                windows::Win32::Foundation::HWND(hwnd),
                                model.borrow().backdrop,
                            ); }
                            model.borrow_mut().native_material = desktop || surface.native;
                            let target = surface.begin_frame(r.right as u32, r.bottom as u32)?;
                            renderer.paint(&target,
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
                            if managed { event(Event::Exit); }
                        } else {
                            paint_error = false;
                            if desktop { update_region(hwnd, &model.borrow(), &mut region_cells); }
                            event(Event::Ready);
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
                    if desktop {event(Event::Exit);return Some(0);}
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
                            SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, client(hwnd).right,
                                (animation.to * scale(hwnd)).round() as i32,
                                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                        }
                        invalidate(hwnd);
                    }
                    let mut bounds = RECT::default();
                    let mut pointer = POINT::default();
                    if unsafe { GetWindowRect(hwnd, &raw mut bounds) } != 0
                        && unsafe { GetCursorPos(&raw mut pointer) } != 0 {
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
                    if !desktop && p.y as f32 / s < HEADER {
                        match super::layout::header_button(r.right as f32 / s, p.x as f32 / s, p.y as f32 / s) {
                            Some(1) => unsafe {
                                PostMessageW(hwnd, WM_CONTEXTMENU, 0, -1);
                            },
                            Some(0) => { event(Event::Collapse); }
                            _ => {}
                        }
                    } else {
                        let selected = {
                            let m = model.borrow();
                            m.hit(grid(hwnd, &m),
                                p.x as f32 / s,
                                p.y as f32 / s,
                            )
                        };
                        model.borrow_mut().selected = selected;
                        drag = selected.map(|index| (index, p, false));
                        drag_identity=selected.map(|index|model.borrow().items[index].identity.clone());
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
                    let p = point(lparam);
                    let s = scale(hwnd);
                    let hovered = super::layout::header_button(client(hwnd).right as f32 / s, p.x as f32 / s, p.y as f32 / s);
                    let item = { let m = model.borrow(); m.hit(grid(hwnd,&m),p.x as f32/s,p.y as f32/s) };
                    if model.borrow().hovered_item != item { model.borrow_mut().hovered_item=item; invalidate(hwnd); }
                    if model.borrow().hovered_button != hovered {
                        model.borrow_mut().hovered_button = hovered;
                        invalidate(hwnd);
                    }
                    unsafe {
                        let mut track = windows_sys::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT {
                            cbSize: size_of::<windows_sys::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT>() as u32,
                            dwFlags: windows_sys::Win32::UI::Input::KeyboardAndMouse::TME_LEAVE,
                            hwndTrack: hwnd, dwHoverTime: 0,
                        };
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::TrackMouseEvent(&raw mut track);
                    }
                    if let Some((index, start, moved)) = drag.as_mut() {
                        let current=drag_identity.as_ref().and_then(|identity|model.borrow().items.iter().position(|item|&item.identity==identity));
                        let Some(current)=current else {return Some(0);};
                        *index=current;
                        let p = point(lparam);
                        *moved |= (p.x - start.x).abs() > unsafe { GetSystemMetrics(SM_CXDRAG) }
                            || (p.y - start.y).abs() > unsafe { GetSystemMetrics(SM_CYDRAG) };
                        if *moved {
                            let mut screen = p;
                            unsafe { ClientToScreen(hwnd, &raw mut screen); }
                            if drag_image.is_none() {
                                let image = model.borrow().items.get(*index).and_then(|item| item.image.clone());
                                if let Some(image) = image {
                                    let m = model.borrow();
                                    let grid = grid(hwnd, &m);
                                    let (cell_x, cell_y) = m.cell(grid, *index);
                                    let ratio = grid.icon_size / image.width.max(image.height) as f32;
                                    let (width, height) = (image.width as f32 * ratio, image.height as f32 * ratio);
                                    let hotspot = POINT {
                                        x: start.x - ((cell_x + (grid.cell_width - width) / 2.0) * s).round() as i32,
                                        y: start.y - ((cell_y + if managed {2.0}else{4.0} + (grid.icon_size - height) / 2.0) * s).round() as i32,
                                    };
                                    let size = windows_sys::Win32::Foundation::SIZE {
                                        cx: (width * s).round().max(1.0) as i32,
                                        cy: (height * s).round().max(1.0) as i32,
                                    };
                                    drop(m);
                                    drag_image = super::drag_image::DragImage::new(hwnd, &image, screen, hotspot, size);
                                }
                            }
                            if let Some(image) = &drag_image { image.move_to(screen); }
                        }
                    }
                    Some(0)
                }
                WM_MOUSELEAVE => {
                    model.borrow_mut().hovered_button = None;
                    model.borrow_mut().hovered_item = None;
                    invalidate(hwnd);
                    Some(0)
                }
                WM_LBUTTONUP => {
                    let old = drag.take();
                    drag_image = None;
                    unsafe {
                        ReleaseCapture();
                    }
                    if let Some((_, _, true)) = old {
                        let index=drag_identity.take().and_then(|identity|model.borrow().items.iter().position(|item|item.identity==identity));
                        let Some(index)=index else {return Some(0);};
                        let mut p = point(lparam);
                        unsafe {
                            ClientToScreen(hwnd, &raw mut p);
                        }
                        event(Event::Drop { index, point: p });
                    }
                    Some(0)
                }
                WM_CAPTURECHANGED | WM_CANCELMODE => {
                    drag = None;
                    drag_image = None;
                    Some(0)
                }
                WM_LBUTTONDBLCLK => {
                    let p = point(lparam);
                    let s = scale(hwnd);
                    let selected = {
                        let m = model.borrow();
                        m.hit(grid(hwnd, &m),p.x as f32 / s, p.y as f32 / s)
                    };
                    if let Some(index) = selected {
                        event(Event::Activate(index));
                    }
                    Some(0)
                }
                WM_NCLBUTTONDBLCLK => {
                    if !desktop { event(Event::Collapse); }
                    Some(0)
                }
                WM_MOUSEWHEEL => {
                    if desktop { return Some(0); }
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
                WM_KEYDOWN if wparam == usize::from(VK_ESCAPE) => {
                    if drag.take().is_some() {
                        drag_image = None;
                        unsafe { ReleaseCapture(); }
                    } else if managed { model.borrow_mut().selected=None; invalidate(hwnd); } else { event(Event::Exit); }
                    Some(0)
                }
                WM_KEYDOWN if wparam == usize::from(VK_RETURN) => {
                    let selected = model.borrow().selected;
                    if let Some(index) = selected {
                        event(Event::Activate(index));
                    }
                    Some(0)
                }
                WM_KEYDOWN => {
                    let mut m = model.borrow_mut();
                    let grid = grid(hwnd, &m);
                    if !m.items.is_empty() {
                        let at = m.selected.unwrap_or(0);
                        let next = match wparam as u16 {
                            VK_LEFT => at.saturating_sub(if desktop {grid.visible_rows} else {1}),
                            VK_RIGHT => (at + if desktop {grid.visible_rows} else {1}).min(m.items.len() - 1),
                            VK_UP => at.saturating_sub(if desktop {1} else {grid.columns}),
                            VK_DOWN => (at + if desktop {1} else {grid.columns}).min(m.items.len() - 1),
                            _ => at,
                        };
                        m.selected = Some(next);
                        let row = next / grid.columns;
                        if desktop { m.scroll=0; } else if row < m.scroll {
                            m.scroll = row;
                        } else if row >= m.scroll + grid.visible_rows {
                            m.scroll = row - grid.visible_rows + 1;
                        }
                    }
                    drop(m);
                    invalidate(hwnd);
                    Some(0)
                }
                WM_CONTEXTMENU => {
                    if menu_active.get() { return Some(0); }
                    let _menu_activity = MenuActivity::begin(Rc::clone(&menu_active));
                    hover_state = None;
                    let mut anchor=point(lparam);
                    let index=if lparam == -1 {
                        if desktop || wparam!=0 {model.borrow().selected}else{None}
                    } else {
                        let mut p=anchor;
                        unsafe {windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd,&raw mut p);}
                        let m=model.borrow(); m.hit(grid(hwnd,&m),p.x as f32/scale(hwnd),p.y as f32/scale(hwnd))
                    };
                    if let Some(index)=index {
                        let identity={let mut m=model.borrow_mut();m.selected=Some(index);m.items[index].identity.clone()};
                        if lparam == -1 {unsafe {GetCursorPos(&raw mut anchor);}}
                        invalidate(hwnd);
                        event(Event::MenuSelection(true));
                        let result=super::shell_menu::show(hwnd,&identity,anchor);
                        event(Event::MenuSelection(false));
                        if let Err(message)=result {error(&message);}
                        // Inventory polling detects rename/delete; dismissing a menu must not
                        // discard every image and trigger a visible reload.
                        return Some(0);
                    }
                    let (collapsed, backdrop, auto_hide, theme) = { let m = model.borrow(); (m.collapsed, m.backdrop, m.auto_hide, m.theme) };
                    let command = menu(
                        hwnd,
                        lparam,
                        collapsed,
                        backdrop,
                        auto_hide,
                        desktop,
                        theme,
                    );
                    match command {
                        1 => {
                            event(Event::New);
                        }
                        2 => {
                            event(Event::Collapse);
                        }
                        3 => {
                            event(Event::Sort);
                        }
                        4 => {
                            event(Event::Exit);
                        }
                        5 => {
                            event(Event::Material(desktop_core::Backdrop::Acrylic));
                        }
                        6 => {
                            event(Event::Material(desktop_core::Backdrop::Mica));
                        }
                        7 => { hover_state = None; event(Event::ToggleAutoHide); }
                        12 => { event(Event::ToggleTopmost); }
                        13 => { event(Event::Material(desktop_core::Backdrop::MicaAlt)); }
                        14 => { event(Event::Theme(desktop_core::PanelTheme::System)); }
                        15 => { event(Event::Theme(desktop_core::PanelTheme::Light)); }
                        16 => { event(Event::Theme(desktop_core::PanelTheme::Dark)); }
                        9 => {event(Event::Refresh);}
                        _ => {}
                    }
                    Some(0)
                }
                WM_CLOSE => {
                    event(Event::Exit);
                    Some(0)
                }
                WM_SYSCOMMAND if wparam & 0xfff0 == SC_CLOSE as usize => {
                    event(Event::Exit); Some(0)
                }
                _ => None,
            }
        })
        .create()
        .map_err(|e| e.to_string())?;
    let hwnd = window.hwnd().cast();
    let s = if desktop { 1.0 } else { scale(hwnd) };
    unsafe {
        if managed && !inspect {
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
        if !desktop {
            SetTimer(hwnd, 3, 60, None);
        }
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

fn update_region(hwnd: HWND, model: &GroupModel, previous: &mut Option<Vec<(i32, i32, i32, i32)>>) {
    use windows_sys::Win32::Graphics::Gdi::{
        CombineRgn, CreateRectRgn, DeleteObject, RGN_OR, SetWindowRgn,
    };
    let s = scale(hwnd);
    let grid = grid(hwnd, model);
    let cells: Vec<_> = model
        .items
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let (x, y) = model.cell(grid, index);
            (
                (x * s).floor() as i32,
                (y * s).floor() as i32,
                ((x + grid.cell_width) * s).ceil() as i32,
                ((y + grid.cell_height) * s).ceil() as i32,
            )
        })
        .collect();
    if previous.as_ref() == Some(&cells) {
        return;
    }
    unsafe {
        let region = CreateRectRgn(0, 0, 0, 0);
        for &(left, top, right, bottom) in &cells {
            let cell = CreateRectRgn(left, top, right, bottom);
            CombineRgn(region, region, cell, RGN_OR);
            DeleteObject(cell);
        }
        if SetWindowRgn(hwnd, region, 0) == 0 {
            DeleteObject(region);
        } else {
            *previous = Some(cells);
        }
    }
}

fn menu(
    hwnd: HWND,
    lparam: isize,
    collapsed: bool,
    material: desktop_core::Backdrop,
    auto_hide: bool,
    desktop: bool,
    theme: desktop_core::PanelTheme,
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
        hwnd, anchor, anchored, collapsed, material, auto_hide, desktop, theme,
    )
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
