use crate::{DesktopHost, RenderIcon, ShellOwnedDesktopHost};
use desktop_core::{MonitorId, PointDip};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CombineRgn, CreateRectRgn, CreateSolidBrush, DT_CENTER,
    DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint,
    EnumDisplayMonitors, FillRect, GetMonitorInfoW, HDC, HMONITOR, InvalidateRect, MONITORINFOEXW,
    PAINTSTRUCT, RGN_OR, SetBkMode, SetTextColor, SetWindowRgn, TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::{ILD_TRANSPARENT, ImageList_Draw};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, ReleaseCapture, SetCapture, SetFocus, VK_RETURN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, GetClientRect, GetMessageTime, HTCLIENT, HWND_NOTOPMOST, LWA_COLORKEY,
    MONITORINFOF_PRIMARY, PostMessageW, SC_CLOSE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetForegroundWindow, SetLayeredWindowAttributes, SetWindowPos, WM_DESTROY, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST, WM_PAINT, WM_SYSCOMMAND,
    WM_USER, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows_window::{Result, Window};

const ICON_SIZE_DIP: f32 = 48.0;
const CELL_WIDTH_DIP: f32 = 96.0;
const CELL_HEIGHT_DIP: f32 = 88.0;
const LABEL_TOP_DIP: f32 = 54.0;
const LABEL_HEIGHT_DIP: f32 = 30.0;
const SURFACE_COLOR_KEY: u32 = 0x0003_0201;
const WM_DESKTOP_SURFACE_CHANGED: u32 = WM_USER + 2;

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorDescriptor {
    pub id: MonitorId,
    pub bounds: PixelRect,
    pub work_area: PixelRect,
    pub dpi: u32,
    pub primary: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PixelRect {
    const fn from_rect(rect: RECT) -> Self {
        Self {
            x: rect.left,
            y: rect.top,
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
        }
    }
}

/// Enumerates active display monitors with stable device names and effective DPI.
#[must_use]
pub fn enumerate_monitors() -> Vec<MonitorDescriptor> {
    unsafe extern "system" fn callback(
        monitor: HMONITOR,
        _device_context: HDC,
        _bounds: *mut RECT,
        data: LPARAM,
    ) -> i32 {
        let monitors = unsafe { &mut *(data as *mut Vec<MonitorDescriptor>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = u32::try_from(size_of::<MONITORINFOEXW>()).unwrap_or(u32::MAX);
        if unsafe { GetMonitorInfoW(monitor, (&raw mut info).cast()) } == 0 {
            return 1;
        }
        let length = info
            .szDevice
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(info.szDevice.len());
        let id = String::from_utf16_lossy(&info.szDevice[..length]);
        let mut dpi_x = 96_u32;
        let mut dpi_y = 96_u32;
        let _ =
            unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y) };
        monitors.push(MonitorDescriptor {
            id: MonitorId::new(id),
            bounds: PixelRect::from_rect(info.monitorInfo.rcMonitor),
            work_area: PixelRect::from_rect(info.monitorInfo.rcWork),
            dpi: dpi_x.max(1),
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
        1
    }

    let mut monitors: Vec<MonitorDescriptor> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(callback),
            (&raw mut monitors) as LPARAM,
        );
    }
    monitors.sort_by_key(|monitor| !monitor.primary);
    monitors
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopSurfaceItem {
    pub label: String,
    pub icon: Option<RenderIcon>,
    pub position: PointDip,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesktopSurfaceRenderModel {
    pub items: Vec<DesktopSurfaceItem>,
}

pub type SharedDesktopSurfaceModel = Rc<RefCell<DesktopSurfaceRenderModel>>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesktopSurfaceEvent {
    ActivateItem(usize),
    SelectionChanged(Option<usize>),
    MoveItem {
        index: usize,
        position: PointDip,
    },
    DropItem {
        index: usize,
        screen_x: i32,
        screen_y: i32,
    },
}

pub struct DesktopItemSurface {
    inner: Window,
    dpi: u32,
    model: SharedDesktopSurfaceModel,
}

impl DesktopItemSurface {
    /// Creates one sparse, Shell-owned desktop-item surface for a monitor.
    ///
    /// # Errors
    ///
    /// Returns a Windows error when the HWND cannot be created.
    #[allow(clippy::too_many_lines)]
    pub fn new<F>(
        monitor: &MonitorDescriptor,
        model: &SharedDesktopSurfaceModel,
        mut on_event: F,
    ) -> Result<Self>
    where
        F: FnMut(*mut c_void, DesktopSurfaceEvent) + 'static,
    {
        let model_for_messages = Rc::clone(model);
        let selected = Rc::new(Cell::new(None::<usize>));
        let selected_for_messages = Rc::clone(&selected);
        let drag = Rc::new(Cell::new(None::<(usize, i32, i32)>));
        let drag_for_messages = Rc::clone(&drag);
        let last_click = Rc::new(Cell::new(None::<(usize, u32)>));
        let last_click_for_messages = Rc::clone(&last_click);
        let dpi = monitor.dpi;
        let inner = Window::new("LucidPane Desktop Surface")
            .size(monitor.bounds.width, monitor.bounds.height)
            .style(WS_POPUP)
            .ex_style(WS_EX_TOOLWINDOW | WS_EX_LAYERED)
            .on_message(move |raw_hwnd, message, wparam, lparam| {
                let hwnd = raw_hwnd.cast();
                match message {
                    WM_ERASEBKGND => Some(1),
                    WM_NCHITTEST => Some(isize::try_from(HTCLIENT).unwrap_or_default()),
                    WM_PAINT => {
                        unsafe {
                            paint_surface(
                                hwnd,
                                &model_for_messages.borrow(),
                                selected_for_messages.get(),
                                dpi,
                            );
                        }
                        Some(0)
                    }
                    WM_DESKTOP_SURFACE_CHANGED => {
                        unsafe {
                            update_surface_region(hwnd, &model_for_messages.borrow(), dpi);
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                        Some(0)
                    }
                    WM_LBUTTONDOWN => {
                        let x = signed_low_word(lparam);
                        let y = signed_high_word(lparam);
                        let hit = surface_item_at(&model_for_messages.borrow(), x, y, dpi);
                        selected_for_messages.set(hit);
                        on_event(raw_hwnd, DesktopSurfaceEvent::SelectionChanged(hit));
                        if let Some(index) = hit {
                            let item = &model_for_messages.borrow().items[index];
                            let item_x = dip_to_pixel(item.position.x, dpi);
                            let item_y = dip_to_pixel(item.position.y, dpi);
                            drag_for_messages.set(Some((index, x - item_x, y - item_y)));

                            let now =
                                u32::try_from(unsafe { GetMessageTime() }).unwrap_or_default();
                            if let Some((previous, previous_time)) = last_click_for_messages.get()
                                && previous == index
                                && now.wrapping_sub(previous_time)
                                    <= unsafe { GetDoubleClickTime() }
                            {
                                last_click_for_messages.set(None);
                                on_event(raw_hwnd, DesktopSurfaceEvent::ActivateItem(index));
                            } else {
                                last_click_for_messages.set(Some((index, now)));
                            }
                            unsafe {
                                SetForegroundWindow(hwnd);
                                SetFocus(hwnd);
                                SetCapture(hwnd);
                            }
                        }
                        unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                        Some(0)
                    }
                    WM_MOUSEMOVE => {
                        if let Some((index, offset_x, offset_y)) = drag_for_messages.get() {
                            let position = PointDip::new(
                                pixel_to_dip(signed_low_word(lparam) - offset_x, dpi).max(0.0),
                                pixel_to_dip(signed_high_word(lparam) - offset_y, dpi).max(0.0),
                            );
                            if let Some(item) = model_for_messages.borrow_mut().items.get_mut(index)
                            {
                                item.position = position;
                            }
                            unsafe {
                                update_surface_region(hwnd, &model_for_messages.borrow(), dpi);
                                InvalidateRect(hwnd, std::ptr::null(), 0);
                            }
                            on_event(raw_hwnd, DesktopSurfaceEvent::MoveItem { index, position });
                        }
                        Some(0)
                    }
                    WM_LBUTTONUP => {
                        if let Some((index, _, _)) = drag_for_messages.replace(None) {
                            let mut point = POINT {
                                x: signed_low_word(lparam),
                                y: signed_high_word(lparam),
                            };
                            unsafe {
                                ReleaseCapture();
                                ClientToScreen(hwnd, &raw mut point);
                            }
                            on_event(
                                raw_hwnd,
                                DesktopSurfaceEvent::DropItem {
                                    index,
                                    screen_x: point.x,
                                    screen_y: point.y,
                                },
                            );
                        }
                        Some(0)
                    }
                    WM_KEYDOWN if wparam == VK_RETURN as usize => {
                        if let Some(index) = selected_for_messages.get() {
                            on_event(raw_hwnd, DesktopSurfaceEvent::ActivateItem(index));
                        }
                        Some(0)
                    }
                    WM_SYSCOMMAND if (wparam & 0xfff0) == SC_CLOSE as usize => Some(0),
                    WM_DESTROY => Some(0),
                    _ => None,
                }
            })
            .create()?;

        let hwnd: HWND = inner.hwnd().cast();
        unsafe {
            SetLayeredWindowAttributes(hwnd, SURFACE_COLOR_KEY, 0, LWA_COLORKEY);
            SetWindowPos(
                hwnd,
                HWND_NOTOPMOST,
                monitor.bounds.x,
                monitor.bounds.y,
                monitor.bounds.width,
                monitor.bounds.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            update_surface_region(hwnd, &model.borrow(), dpi);
        }
        if let Ok(mut host) = ShellOwnedDesktopHost::new() {
            let _ = host.attach(inner.hwnd());
        }

        Ok(Self {
            inner,
            dpi,
            model: Rc::clone(model),
        })
    }

    #[must_use]
    pub fn hwnd(&self) -> *mut c_void {
        self.inner.hwnd()
    }

    #[must_use]
    pub fn hwnd_token(&self) -> isize {
        self.inner.hwnd() as isize
    }

    pub fn refresh(&self) {
        let hwnd: HWND = self.inner.hwnd().cast();
        unsafe {
            update_surface_region(hwnd, &self.model.borrow(), self.dpi);
            InvalidateRect(hwnd, std::ptr::null(), 0);
        }
    }
}

#[must_use]
pub fn post_desktop_surface_changed(hwnd: isize) -> bool {
    unsafe { PostMessageW(hwnd as HWND, WM_DESKTOP_SURFACE_CHANGED, 0, 0) != 0 }
}

unsafe fn paint_surface(
    hwnd: HWND,
    model: &DesktopSurfaceRenderModel,
    selected: Option<usize>,
    dpi: u32,
) {
    let mut paint = PAINTSTRUCT::default();
    let device_context = unsafe { BeginPaint(hwnd, &raw mut paint) };
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &raw mut client) };
    let background = unsafe { CreateSolidBrush(SURFACE_COLOR_KEY) };
    unsafe {
        FillRect(device_context, &raw const client, background);
        DeleteObject(background);
        SetBkMode(
            device_context,
            i32::try_from(TRANSPARENT).unwrap_or_default(),
        );
        SetTextColor(device_context, rgb(255, 255, 255));
    }

    for (index, item) in model.items.iter().enumerate() {
        let cell = item_rect(item, dpi);
        if selected == Some(index) {
            let brush = unsafe { CreateSolidBrush(rgb(46, 91, 140)) };
            unsafe {
                FillRect(device_context, &raw const cell, brush);
                DeleteObject(brush);
            }
        }
        if let Some(icon) = item.icon {
            let icon_size = dip_to_pixel(ICON_SIZE_DIP, dpi);
            let icon_x = cell.left + (cell.right - cell.left - icon_size) / 2;
            let icon_y = cell.top + dip_to_pixel(2.0, dpi);
            unsafe {
                ImageList_Draw(
                    icon.image_list,
                    icon.index,
                    device_context,
                    icon_x,
                    icon_y,
                    ILD_TRANSPARENT,
                );
            }
        }
        let mut label_rect = RECT {
            left: cell.left,
            top: cell.top + dip_to_pixel(LABEL_TOP_DIP, dpi),
            right: cell.right,
            bottom: cell.top + dip_to_pixel(LABEL_TOP_DIP + LABEL_HEIGHT_DIP, dpi),
        };
        let label = wide_null(&item.label);
        unsafe {
            DrawTextW(
                device_context,
                label.as_ptr(),
                i32::try_from(label.len().saturating_sub(1)).unwrap_or(i32::MAX),
                &raw mut label_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
        }
    }
    unsafe { EndPaint(hwnd, &raw const paint) };
}

unsafe fn update_surface_region(hwnd: HWND, model: &DesktopSurfaceRenderModel, dpi: u32) {
    let combined = unsafe { CreateRectRgn(0, 0, 0, 0) };
    if combined.is_null() {
        return;
    }
    for item in &model.items {
        let rect = item_rect(item, dpi);
        let cell = unsafe { CreateRectRgn(rect.left, rect.top, rect.right, rect.bottom) };
        if !cell.is_null() {
            unsafe {
                CombineRgn(combined, combined, cell, RGN_OR);
                DeleteObject(cell);
            }
        }
    }
    if unsafe { SetWindowRgn(hwnd, combined, 1) } == 0 {
        unsafe { DeleteObject(combined) };
    }
}

fn surface_item_at(model: &DesktopSurfaceRenderModel, x: i32, y: i32, dpi: u32) -> Option<usize> {
    model.items.iter().position(|item| {
        let rect = item_rect(item, dpi);
        x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
    })
}

fn item_rect(item: &DesktopSurfaceItem, dpi: u32) -> RECT {
    let left = dip_to_pixel(item.position.x, dpi);
    let top = dip_to_pixel(item.position.y, dpi);
    RECT {
        left,
        top,
        right: left + dip_to_pixel(CELL_WIDTH_DIP, dpi),
        bottom: top + dip_to_pixel(CELL_HEIGHT_DIP, dpi),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn dip_to_pixel(value: f32, dpi: u32) -> i32 {
    (value * dpi as f32 / 96.0).round() as i32
}

#[allow(clippy::cast_precision_loss)]
fn pixel_to_dip(value: i32, dpi: u32) -> f32 {
    value as f32 * 96.0 / dpi as f32
}

const fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | ((green as u32) << 8) | ((blue as u32) << 16)
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn signed_low_word(value: isize) -> i32 {
    let bits = value.cast_unsigned();
    let low = u16::try_from(bits & 0xffff).unwrap_or_default();
    i32::from(low.cast_signed())
}

fn signed_high_word(value: isize) -> i32 {
    let bits = value.cast_unsigned();
    let high = u16::try_from((bits >> 16) & 0xffff).unwrap_or_default();
    i32::from(high.cast_signed())
}

impl Drop for DesktopItemSurface {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.inner.hwnd().cast());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DesktopItemSurface, DesktopSurfaceItem, DesktopSurfaceRenderModel, PointDip,
        SharedDesktopSurfaceModel, enumerate_monitors, surface_item_at,
    };
    use std::cell::RefCell;
    use std::rc::Rc;
    use windows_sys::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, ERROR, GetWindowRgn};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWLP_HWNDPARENT, GetShellWindow, GetWindowLongPtrW, IsWindowVisible,
    };

    #[test]
    fn monitor_inventory_contains_a_primary_monitor() {
        let monitors = enumerate_monitors();
        assert!(!monitors.is_empty());
        assert!(monitors.iter().any(|monitor| monitor.primary));
        assert!(monitors.iter().all(|monitor| monitor.dpi > 0));
    }

    #[test]
    fn sparse_surface_hit_testing_only_accepts_icon_cells() {
        let model = DesktopSurfaceRenderModel {
            items: vec![DesktopSurfaceItem {
                label: "Editor".into(),
                icon: None,
                position: PointDip::new(20.0, 30.0),
            }],
        };
        assert_eq!(surface_item_at(&model, 25, 35, 96), Some(0));
        assert_eq!(surface_item_at(&model, 200, 200, 96), None);
    }

    #[test]
    fn surface_is_a_visible_shell_owned_popup_with_a_sparse_region() {
        let monitor = enumerate_monitors().into_iter().next().unwrap();
        let model: SharedDesktopSurfaceModel = Rc::new(RefCell::new(DesktopSurfaceRenderModel {
            items: vec![DesktopSurfaceItem {
                label: "LucidPane Test".into(),
                icon: None,
                position: PointDip::new(20.0, 30.0),
            }],
        }));
        let surface = DesktopItemSurface::new(&monitor, &model, |_, _| {}).unwrap();
        let hwnd = surface.hwnd().cast();
        assert_ne!(unsafe { IsWindowVisible(hwnd) }, 0);
        assert_eq!(
            unsafe { GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) },
            unsafe { GetShellWindow() } as isize
        );

        let region = unsafe { CreateRectRgn(0, 0, 0, 0) };
        assert_ne!(unsafe { GetWindowRgn(hwnd, region) }, ERROR);
        unsafe { DeleteObject(region) };
    }
}
