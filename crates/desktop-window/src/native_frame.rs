//! Decoration only: the center is absent from the HWND region, including hit testing.
//! Explorer draws and receives input for every icon inside and outside the frame.
use super::{
    DesktopHost, ShellOwnedDesktopHost, TITLE_EDIT_ID, apply_initial_rect, begin_title_edit,
    context_point, dip_to_pixel, draw_text, high_word, low_word, read_window_text, rgb,
    signed_high_word, signed_low_word, taskbar_created_message, wide_null, window_rect,
};
use desktop_core::RectDip;
use std::cell::Cell;
use std::ffi::c_void;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CombineRgn, CreateRectRgn, CreateSolidBrush, DEFAULT_GUI_FONT, DeleteObject,
    EndPaint, FillRect, GetStockObject, InvalidateRect, PAINTSTRUCT, RGN_DIFF, SelectObject,
    SetBkMode, SetTextColor, SetWindowRgn, TRANSPARENT,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_F2};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, DestroyWindow, EN_KILLFOCUS, GetClientRect,
    GetWindowRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTLEFT, HTRIGHT, HTTOP,
    HTTOPLEFT, HTTOPRIGHT, LWA_ALPHA, MB_ICONWARNING, MB_OK, MF_CHECKED, MF_STRING, MF_UNCHECKED,
    MINMAXINFO, MessageBoxW, SetLayeredWindowAttributes, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenuEx, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_ENTERSIZEMOVE,
    WM_ERASEBKGND, WM_EXITSIZEMOVE, WM_GETMINMAXINFO, WM_KEYDOWN, WM_MOVING, WM_NCCALCSIZE,
    WM_NCHITTEST, WM_PAINT, WM_QUERYENDSESSION, WM_SIZE, WM_SIZING, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
};
use windows_window::Window;

const BORDER: i32 = 4;
const HEADER: i32 = 48;

#[derive(Clone, Debug)]
pub enum NativeFrameEvent {
    MenuRequested {
        owner: isize,
        x: i32,
        y: i32,
    },
    MaterialChanged(desktop_core::Backdrop),
    /// Live native move/resize proposal. A rejection keeps the last accepted rectangle.
    GeometryPreview {
        current: RectDip,
    },
    GeometryChanged {
        previous: RectDip,
        current: RectDip,
        move_items: bool,
    },
    TitleChanged(String),
    NewFrame,
    RemoveFrame,
    Exit,
}

pub struct NativeFrame {
    inner: Window,
}

impl NativeFrame {
    /// Creates a frame with a truly empty center; no icon or content surface is created.
    /// The callback returns false to reject a geometry change.
    ///
    /// # Errors
    /// Returns an error if the native window, region, or Shell owner cannot be created.
    #[allow(clippy::too_many_lines)]
    pub fn new<F>(title: String, initial: RectDip, on_event: F) -> Result<Self, String>
    where
        F: FnMut(NativeFrameEvent) -> bool + 'static,
    {
        Self::create(title, initial, false, on_event)
    }

    /// Decoration for the native Hook backend. Layout is always handled by the Hook session.
    pub fn new_hook<F>(title: String, initial: RectDip, on_event: F) -> Result<Self, String>
    where
        F: FnMut(NativeFrameEvent) -> bool + 'static,
    {
        Self::create(title, initial, true, on_event)
    }

    #[allow(clippy::too_many_lines)]
    fn create<F>(
        mut title: String,
        initial: RectDip,
        hook_layout: bool,
        mut on_event: F,
    ) -> Result<Self, String>
    where
        F: FnMut(NativeFrameEvent) -> bool + 'static,
    {
        let mut previous = initial;
        let mut accepted = initial;
        let mut move_items = true;
        let mut sizing = false;
        let edit = Cell::new(std::ptr::null_mut::<c_void>());
        let shell_message = taskbar_created_message();
        let inner = Window::new("LucidPane Native Frame")
            .size(dip_to_pixel(initial.width), dip_to_pixel(initial.height))
            .style(WS_POPUP | WS_THICKFRAME | WS_SYSMENU)
            .ex_style(WS_EX_TOOLWINDOW | WS_EX_LAYERED)
            .on_message(move |raw, message, wparam, lparam| {
                let hwnd: HWND = raw.cast();
                match message {
                    // Do not let windows-window quit when just one frame is removed.
                    WM_NCCALCSIZE | WM_ERASEBKGND | WM_DESTROY => Some(0),
                    WM_PAINT => {
                        if hook_layout {
                            let mut ps = PAINTSTRUCT::default();
                            unsafe {
                                BeginPaint(hwnd, &raw mut ps);
                                EndPaint(hwnd, &raw const ps);
                            }
                        } else {
                            unsafe { paint(hwnd, &title) };
                        }
                        Some(0)
                    }
                    WM_SIZE => {
                        unsafe { update_region(hwnd) };
                        Some(0)
                    }
                    WM_GETMINMAXINFO => {
                        let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
                        info.ptMinTrackSize.x = dip_to_pixel(RectDip::MIN_WIDTH);
                        info.ptMinTrackSize.y = dip_to_pixel(RectDip::MIN_HEIGHT);
                        Some(0)
                    }
                    WM_NCHITTEST => {
                        if hook_layout {
                            let mut rect = RECT::default();
                            unsafe {
                                GetWindowRect(hwnd, &raw mut rect);
                            }
                            let x = signed_low_word(lparam);
                            let y = signed_high_word(lparam);
                            if x >= rect.right - 48
                                && x < rect.right - BORDER
                                && y >= rect.top + BORDER
                                && y < rect.top + HEADER
                            {
                                return Some(
                                    windows_sys::Win32::UI::WindowsAndMessaging::HTCLIENT as isize,
                                );
                            }
                        }
                        Some(isize::try_from(unsafe { hit_test(hwnd, lparam) }).unwrap_or_default())
                    }
                    WM_ENTERSIZEMOVE => {
                        previous = unsafe { window_rect(hwnd) }.unwrap_or(previous);
                        accepted = previous;
                        sizing = false;
                        Some(0)
                    }
                    WM_MOVING | WM_SIZING => {
                        sizing |= message == WM_SIZING;
                        if !hook_layout || lparam == 0 {
                            return None;
                        }
                        let proposed = unsafe { &mut *(lparam as *mut RECT) };
                        let current = RectDip::new(
                            proposed.left as f32,
                            proposed.top as f32,
                            (proposed.right - proposed.left) as f32,
                            (proposed.bottom - proposed.top) as f32,
                        );
                        if on_event(NativeFrameEvent::GeometryPreview { current }) {
                            accepted = current;
                        } else {
                            proposed.left = accepted.x.round() as i32;
                            proposed.top = accepted.y.round() as i32;
                            proposed.right = (accepted.x + accepted.width).round() as i32;
                            proposed.bottom = (accepted.y + accepted.height).round() as i32;
                        }
                        Some(1)
                    }
                    WM_EXITSIZEMOVE => {
                        if let Some(current) = unsafe { window_rect(hwnd) }
                            && !on_event(NativeFrameEvent::GeometryChanged {
                                previous,
                                current,
                                move_items: move_items && !sizing,
                            })
                        {
                            unsafe { apply_initial_rect(hwnd, previous) };
                        }
                        Some(0)
                    }
                    WM_CONTEXTMENU => {
                        if hook_layout {
                            let point = context_point(hwnd, lparam);
                            on_event(NativeFrameEvent::MenuRequested {
                                owner: hwnd as isize,
                                x: point.x,
                                y: point.y,
                            });
                            return Some(0);
                        }
                        match unsafe { menu(hwnd, lparam, move_items, hook_layout) } {
                            1 => {
                                on_event(NativeFrameEvent::NewFrame);
                            }
                            2 => unsafe { begin_title_edit(hwnd, &title, &edit) },
                            3 => move_items = !move_items,
                            4 => {
                                on_event(NativeFrameEvent::RemoveFrame);
                                unsafe { DestroyWindow(hwnd) };
                            }
                            5 => {
                                on_event(NativeFrameEvent::Exit);
                            }
                            _ => {}
                        }
                        Some(0)
                    }
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP if hook_layout => {
                        let mut rect = RECT::default();
                        unsafe {
                            GetClientRect(hwnd, &raw mut rect);
                        }
                        if signed_low_word(lparam) >= rect.right - 48
                            && signed_high_word(lparam) < HEADER
                        {
                            let mut point = windows_sys::Win32::Foundation::POINT {
                                x: rect.right - 8,
                                y: HEADER,
                            };
                            unsafe {
                                windows_sys::Win32::Graphics::Gdi::ClientToScreen(
                                    hwnd,
                                    &raw mut point,
                                );
                            }
                            on_event(NativeFrameEvent::MenuRequested {
                                owner: hwnd as isize,
                                x: point.x,
                                y: point.y,
                            });
                        }
                        Some(0)
                    }
                    WM_KEYDOWN if wparam == usize::from(VK_F2) => {
                        unsafe { begin_title_edit(hwnd, &title, &edit) };
                        Some(0)
                    }
                    WM_KEYDOWN if wparam == usize::from(VK_ESCAPE) => {
                        on_event(NativeFrameEvent::Exit);
                        Some(0)
                    }
                    WM_COMMAND
                        if low_word(wparam) == u16::try_from(TITLE_EDIT_ID).unwrap_or_default()
                            && u32::from(high_word(wparam)) == EN_KILLFOCUS =>
                    {
                        let child = edit.replace(std::ptr::null_mut());
                        if !child.is_null() {
                            if let Some(value) = unsafe { read_window_text(child) }
                                && !value.trim().is_empty()
                            {
                                title = value.trim().to_string();
                                on_event(NativeFrameEvent::TitleChanged(title.clone()));
                            }
                            unsafe {
                                DestroyWindow(child);
                                InvalidateRect(hwnd, std::ptr::null(), 0);
                            }
                        }
                        Some(0)
                    }
                    WM_CLOSE => {
                        on_event(NativeFrameEvent::Exit);
                        Some(0)
                    }
                    WM_QUERYENDSESSION => {
                        on_event(NativeFrameEvent::Exit);
                        Some(1)
                    }
                    message if message == shell_message => {
                        if let Ok(mut host) = ShellOwnedDesktopHost::new() {
                            let _ = host.attach(raw);
                        }
                        Some(0)
                    }
                    _ => None,
                }
            })
            .create()
            .map_err(|error| error.to_string())?;
        unsafe {
            apply_initial_rect(inner.hwnd().cast(), initial);
            if SetLayeredWindowAttributes(
                inner.hwnd().cast(),
                0,
                if hook_layout { 1 } else { 220 },
                LWA_ALPHA,
            ) == 0
            {
                return Err(windows_window_error());
            }
            if !update_region(inner.hwnd().cast()) {
                return Err(windows_window_error());
            }
        }
        let mut host = ShellOwnedDesktopHost::new().map_err(|error| error.to_string())?;
        host.attach(inner.hwnd())
            .map_err(|error| error.to_string())?;
        Ok(Self { inner })
    }

    #[must_use]
    pub fn hwnd(&self) -> *mut c_void {
        self.inner.hwnd()
    }

    /// Content bounds in screen pixels, excluding the frame's title and resize border.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn content_bounds(rect: RectDip) -> RectDip {
        RectDip {
            x: rect.x + BORDER as f32,
            y: rect.y + HEADER as f32,
            width: (rect.width - (2 * BORDER) as f32).max(0.0),
            height: (rect.height - (HEADER + BORDER) as f32).max(0.0),
        }
    }

    pub fn run() {
        windows_window::run();
    }
    pub fn quit() {
        windows_window::quit();
    }

    pub fn show_error(message: &str) {
        let message = wide_null(message);
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                wide_null("LucidPane").as_ptr(),
                MB_OK | MB_ICONWARNING,
            );
        }
    }
}

fn windows_window_error() -> String {
    std::io::Error::last_os_error().to_string()
}

unsafe fn update_region(hwnd: HWND) -> bool {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &raw mut rect) };
    let outer = unsafe { CreateRectRgn(0, 0, rect.right, rect.bottom) };
    let center =
        unsafe { CreateRectRgn(BORDER, HEADER, rect.right - BORDER, rect.bottom - BORDER) };
    if outer.is_null() || center.is_null() {
        unsafe {
            DeleteObject(outer);
            DeleteObject(center);
        }
        return false;
    }
    let combined = unsafe { CombineRgn(outer, outer, center, RGN_DIFF) };
    unsafe { DeleteObject(center) };
    if combined == 0 || unsafe { SetWindowRgn(hwnd, outer, 1) } == 0 {
        unsafe { DeleteObject(outer) };
        return false;
    }
    true // Windows now owns outer.
}

unsafe fn hit_test(hwnd: HWND, lparam: isize) -> u32 {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut rect) };
    let x = signed_low_word(lparam) - rect.left;
    let y = signed_high_word(lparam) - rect.top;
    let right = x >= rect.right - rect.left - BORDER;
    let bottom = y >= rect.bottom - rect.top - BORDER;
    match (x < BORDER, right, y < BORDER, bottom) {
        (true, _, true, _) => HTTOPLEFT,
        (_, true, true, _) => HTTOPRIGHT,
        (true, _, _, true) => HTBOTTOMLEFT,
        (_, true, _, true) => HTBOTTOMRIGHT,
        (true, _, _, _) => HTLEFT,
        (_, true, _, _) => HTRIGHT,
        (_, _, true, _) => HTTOP,
        (_, _, _, true) => HTBOTTOM,
        _ => HTCAPTION,
    }
}

unsafe fn paint(hwnd: HWND, title: &str) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &raw mut ps) };
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &raw mut rect) };
    let brush = unsafe { CreateSolidBrush(rgb(36, 36, 36)) };
    unsafe {
        FillRect(hdc, &raw const rect, brush);
        DeleteObject(brush);
        SetBkMode(hdc, TRANSPARENT.cast_signed());
        SetTextColor(hdc, rgb(242, 242, 242));
    }
    let font = unsafe { SelectObject(hdc, GetStockObject(DEFAULT_GUI_FONT)) };
    let mut title_rect = RECT {
        left: 12,
        top: 8,
        right: rect.right - 12,
        bottom: HEADER - 4,
    };
    draw_text(hdc, title, &mut title_rect);
    unsafe {
        SelectObject(hdc, font);
        EndPaint(hwnd, &raw const ps);
    }
}

unsafe fn menu(hwnd: HWND, lparam: isize, move_items: bool, hook_layout: bool) -> u32 {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return 0;
    }
    for (id, label) in [
        (1, "新建分组"),
        (2, "重命名分组"),
        (3, "移动框内图标"),
        (4, "移除分组框"),
        (5, "退出 LucidPane"),
    ] {
        if hook_layout && id == 3 {
            continue;
        }
        let flags = MF_STRING
            | if id == 3 && move_items {
                MF_CHECKED
            } else {
                MF_UNCHECKED
            };
        unsafe { AppendMenuW(menu, flags, id, wide_null(label).as_ptr()) };
    }
    let point = context_point(hwnd, lparam);
    let result = unsafe {
        TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            hwnd,
            std::ptr::null(),
        )
    };
    unsafe { DestroyMenu(menu) };
    result.cast_unsigned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Graphics::Gdi::{GetWindowRgn, PtInRegion};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWLP_HWNDPARENT, GetShellWindow, GetWindowLongPtrW, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOZORDER, SetWindowPos, WS_EX_TOPMOST,
    };

    #[test]
    fn native_frame_leaves_center_out_of_visual_and_input_region_after_resize() {
        let frame =
            NativeFrame::new("Native frame test".into(), RectDip::default(), |_| true).unwrap();
        let hwnd = frame.hwnd().cast();
        assert_eq!(
            unsafe { GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) },
            unsafe { GetShellWindow() } as isize
        );
        assert_eq!(
            unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) }
                & isize::try_from(WS_EX_TOPMOST).unwrap(),
            0
        );
        for (width, height) in [(420, 360), (600, 240)] {
            unsafe {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER,
                );
            }
            let region = unsafe { CreateRectRgn(0, 0, 0, 0) };
            assert_ne!(unsafe { GetWindowRgn(hwnd, region) }, 0);
            assert_ne!(
                unsafe { PtInRegion(region, 20, 20) },
                0,
                "title remains interactive"
            );
            assert_ne!(
                unsafe { PtInRegion(region, 1, height / 2) },
                0,
                "border remains resizable"
            );
            assert_eq!(
                unsafe { PtInRegion(region, width / 2, height / 2) },
                0,
                "Explorer owns the entire center"
            );
            unsafe {
                DeleteObject(region);
            }
        }
    }
}
