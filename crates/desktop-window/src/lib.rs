mod host;
mod surface;

pub use host::{DesktopHost, DesktopHostError, ShellOwnedDesktopHost};
pub use surface::{
    DesktopItemSurface, DesktopSurfaceEvent, DesktopSurfaceItem, DesktopSurfaceRenderModel,
    MonitorDescriptor, PixelRect, SharedDesktopSurfaceModel, enumerate_monitors,
    post_desktop_surface_changed,
};

use desktop_core::{Backdrop, BackdropKind, RectDip};
use std::cell::{Cell, RefCell};
use std::ffi::{OsString, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use windows_sys::Win32::Foundation::{COLORREF, HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateSolidBrush, DEFAULT_GUI_FONT, DT_CENTER, DT_END_ELLIPSIS,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, DeleteObject, DrawTextW, EndPaint,
    FillRect, GetStockObject, InvalidateRect, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::{EM_SETSEL, ILD_TRANSPARENT, ImageList_Draw};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE, VK_F2, VK_F5, VK_RETURN,
};
use windows_sys::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DI_NORMAL, DestroyIcon, DestroyMenu,
    DestroyWindow, DrawIconEx, EN_KILLFOCUS, ES_AUTOHSCROLL, GetCursorPos, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, HICON, HMENU, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT,
    HTCAPTION, HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, HWND_NOTOPMOST, IMAGE_ICON,
    KillTimer, LR_LOADFROMFILE, LoadImageW, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED,
    PostMessageW, RegisterWindowMessageW, SC_CLOSE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetTimer, SetWindowPos, TPM_LEFTALIGN,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_COMMAND, WM_CONTEXTMENU, WM_DISPLAYCHANGE,
    WM_DPICHANGED, WM_DROPFILES, WM_ENDSESSION, WM_ERASEBKGND, WM_EXITSIZEMOVE, WM_KEYDOWN,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCALCSIZE,
    WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_PAINT, WM_QUERYENDSESSION, WM_SETFONT,
    WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOMMAND, WM_TIMER, WM_USER, WM_WINDOWPOSCHANGED, WS_BORDER,
    WS_CHILD, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_TABSTOP,
    WS_THICKFRAME, WS_VISIBLE,
};
use windows_window::{Result, Window, run};

const RESIZE_BORDER: i32 = 8;
const HEADER_HEIGHT: i32 = 58;
const CONTENT_TOP: i32 = 64;
const HORIZONTAL_PADDING: i32 = 16;
const COLLAPSED_HEIGHT: i32 = 64;
const GRID_PADDING: i32 = 12;
const GRID_MIN_CELL_WIDTH: i32 = 104;
const GRID_CELL_HEIGHT: i32 = 92;
const GRID_ICON_SIZE: i32 = 32;
const HEADER_BUTTON_SIZE: i32 = 28;
const HEADER_BUTTON_GAP: i32 = 4;
const HEADER_BUTTON_TOP: i32 = 8;
const HEADER_BUTTON_RIGHT: i32 = 8;
const WM_PORTAL_CONTENT_CHANGED: u32 = WM_USER + 1;
const REFRESH_TIMER_ID: usize = 0x4c50;
const REFRESH_DEBOUNCE_MS: u32 = 300;

const CMD_MICA: i32 = 1_001;
const CMD_MICA_ALT: i32 = 1_002;
const CMD_ACRYLIC: i32 = 1_003;
const CMD_TRANSLUCENT: i32 = 1_004;
const CMD_CLOSE: i32 = 1_100;
const CMD_RENAME: i32 = 1_200;
const CMD_CHOOSE_ICON: i32 = 1_201;
const CMD_RESET_ICON: i32 = 1_202;
const CMD_REFRESH: i32 = 1_203;
const TITLE_EDIT_ID: i32 = 2_001;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderItemKind {
    Directory,
    File,
    Shortcut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderItem {
    pub label: String,
    pub kind: RenderItemKind,
    pub icon: Option<RenderIcon>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderIcon {
    pub image_list: isize,
    pub index: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupRenderModel {
    pub title: String,
    pub subtitle: String,
    pub header_icon: Option<RenderIcon>,
    pub custom_header_icon: Option<PathBuf>,
    pub items: Vec<RenderItem>,
}

pub type SharedRenderModel = Rc<RefCell<GroupRenderModel>>;

#[derive(Clone, Debug, PartialEq)]
pub enum GroupEvent {
    MaterialSelected(Backdrop),
    ActivateItem(usize),
    RefreshRequested,
    GeometryChanged(RectDip),
    CollapsedChanged(bool),
    TitleChanged(String),
    ChooseIconRequested,
    ResetIconRequested,
    ItemsDropped(Vec<PathBuf>),
    MoveItem {
        from: usize,
        to: usize,
    },
    DropItem {
        index: usize,
        screen_x: i32,
        screen_y: i32,
    },
    SelectionChanged(Option<usize>),
    ShellRestarted,
    DisplayConfigurationChanged,
    ShellSettingsChanged,
    SessionEnding,
    CommitRequested,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PanelMenuAction {
    Material(Backdrop),
    Rename,
    ChooseIcon,
    ResetIcon,
    Refresh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeaderButton {
    Menu,
    Close,
}

pub struct GroupWindow {
    inner: Window,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopAttachmentMode {
    ShellOwned,
    Standalone,
}

impl GroupWindow {
    /// Creates and shows the borderless proof-of-concept group window.
    ///
    /// # Errors
    ///
    /// Returns a Windows error when the native window class or HWND cannot be created.
    #[allow(clippy::too_many_lines)]
    pub fn new<F>(
        initial: Backdrop,
        initial_rect: RectDip,
        initial_collapsed: bool,
        model: &SharedRenderModel,
        mut on_event: F,
    ) -> Result<Self>
    where
        F: FnMut(*mut c_void, GroupEvent) + 'static,
    {
        let current = Rc::new(Cell::new(initial.kind()));
        let current_for_messages = Rc::clone(&current);
        let model_for_messages = Rc::clone(model);
        let scroll_row = Rc::new(Cell::new(0_usize));
        let scroll_row_for_messages = Rc::clone(&scroll_row);
        let collapsed = Rc::new(Cell::new(initial_collapsed));
        let collapsed_for_messages = Rc::clone(&collapsed);
        let expanded_height = Rc::new(Cell::new(dip_to_pixel(initial_rect.height)));
        let expanded_height_for_messages = Rc::clone(&expanded_height);
        let title_edit = Rc::new(Cell::new(std::ptr::null_mut::<c_void>()));
        let title_edit_for_messages = Rc::clone(&title_edit);
        let dragged_item = Rc::new(Cell::new(None::<usize>));
        let dragged_item_for_messages = Rc::clone(&dragged_item);
        let selected_item = Rc::new(Cell::new(None::<usize>));
        let selected_item_for_messages = Rc::clone(&selected_item);
        let taskbar_created_message = taskbar_created_message();

        let inner = Window::new("LucidPane")
            .size(
                dip_to_pixel(initial_rect.width),
                dip_to_pixel(initial_rect.height),
            )
            .style(WS_POPUP | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX)
            .ex_style(WS_EX_TOOLWINDOW)
            .on_message(move |raw_hwnd, message, wparam, lparam| {
                let hwnd = raw_hwnd.cast();
                match message {
                    message if message == taskbar_created_message => {
                        on_event(raw_hwnd, GroupEvent::ShellRestarted);
                        Some(0)
                    }
                    WM_DISPLAYCHANGE | WM_DPICHANGED => {
                        on_event(raw_hwnd, GroupEvent::DisplayConfigurationChanged);
                        None
                    }
                    WM_SETTINGCHANGE => {
                        on_event(raw_hwnd, GroupEvent::ShellSettingsChanged);
                        None
                    }
                    WM_QUERYENDSESSION => {
                        on_event(raw_hwnd, GroupEvent::SessionEnding);
                        Some(1)
                    }
                    WM_ENDSESSION if wparam != 0 => {
                        on_event(raw_hwnd, GroupEvent::SessionEnding);
                        Some(0)
                    }
                    WM_NCCALCSIZE => Some(0),
                    WM_COMMAND => {
                        let command = low_word(wparam);
                        let notification = high_word(wparam);
                        if command == u16::try_from(TITLE_EDIT_ID).unwrap_or_default()
                            && u32::from(notification) == EN_KILLFOCUS
                        {
                            let edit: HWND = title_edit_for_messages.replace(std::ptr::null_mut());
                            if !edit.is_null() {
                                if let Some(title) = unsafe { read_window_text(edit) }
                                    && !title.trim().is_empty()
                                {
                                    model_for_messages.borrow_mut().title.clone_from(&title);
                                    on_event(raw_hwnd, GroupEvent::TitleChanged(title));
                                    on_event(raw_hwnd, GroupEvent::CommitRequested);
                                }
                                unsafe {
                                    DestroyWindow(edit);
                                    InvalidateRect(hwnd, std::ptr::null(), 0);
                                }
                            }
                        }
                        Some(0)
                    }
                    WM_ERASEBKGND => Some(1),
                    WM_NCHITTEST => {
                        let hit = unsafe { hit_test(hwnd, lparam) };
                        Some(isize::try_from(hit).unwrap_or_default())
                    }
                    WM_CONTEXTMENU => {
                        let selected =
                            unsafe { show_panel_menu(hwnd, lparam, current_for_messages.get()) };
                        match selected {
                            Some(PanelMenuAction::Material(material)) => {
                                current_for_messages.set(material.kind());
                                on_event(raw_hwnd, GroupEvent::MaterialSelected(material));
                                on_event(raw_hwnd, GroupEvent::CommitRequested);
                            }
                            Some(PanelMenuAction::Rename) => unsafe {
                                begin_title_edit(
                                    hwnd,
                                    &model_for_messages.borrow().title,
                                    &title_edit_for_messages,
                                );
                            },
                            Some(PanelMenuAction::ChooseIcon) => {
                                on_event(raw_hwnd, GroupEvent::ChooseIconRequested);
                            }
                            Some(PanelMenuAction::ResetIcon) => {
                                on_event(raw_hwnd, GroupEvent::ResetIconRequested);
                            }
                            Some(PanelMenuAction::Refresh) => {
                                on_event(raw_hwnd, GroupEvent::RefreshRequested);
                            }
                            None => {}
                        }
                        unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                        Some(0)
                    }
                    WM_PAINT => {
                        unsafe {
                            paint_panel(
                                hwnd,
                                &model_for_messages.borrow(),
                                scroll_row_for_messages.get(),
                                selected_item_for_messages.get(),
                            );
                        }
                        Some(0)
                    }
                    WM_SIZE => {
                        let layout = grid_layout(hwnd);
                        let max_scroll =
                            layout.max_scroll_row(model_for_messages.borrow().items.len());
                        scroll_row_for_messages.set(scroll_row_for_messages.get().min(max_scroll));
                        if selected_item_for_messages
                            .get()
                            .is_some_and(|index| index >= model_for_messages.borrow().items.len())
                        {
                            selected_item_for_messages.set(None);
                        }
                        unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                        Some(0)
                    }
                    WM_WINDOWPOSCHANGED => {
                        if let Some(rect) = unsafe { window_rect(hwnd) } {
                            #[allow(clippy::cast_precision_loss)]
                            let persisted = if collapsed_for_messages.get() {
                                RectDip::new(
                                    rect.x,
                                    rect.y,
                                    rect.width,
                                    expanded_height_for_messages.get() as f32,
                                )
                            } else {
                                expanded_height_for_messages.set(dip_to_pixel(rect.height));
                                rect
                            };
                            on_event(raw_hwnd, GroupEvent::GeometryChanged(persisted));
                        }
                        None
                    }
                    WM_EXITSIZEMOVE => {
                        on_event(raw_hwnd, GroupEvent::CommitRequested);
                        Some(0)
                    }
                    WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
                        unsafe { DestroyWindow(hwnd) };
                        Some(0)
                    }
                    WM_KEYDOWN if wparam == VK_F5 as usize => {
                        on_event(raw_hwnd, GroupEvent::RefreshRequested);
                        scroll_row_for_messages.set(0);
                        unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                        Some(0)
                    }
                    WM_KEYDOWN if wparam == VK_F2 as usize => {
                        unsafe {
                            begin_title_edit(
                                hwnd,
                                &model_for_messages.borrow().title,
                                &title_edit_for_messages,
                            );
                        }
                        Some(0)
                    }
                    WM_KEYDOWN if wparam == VK_RETURN as usize => {
                        if let Some(index) = selected_item_for_messages.get() {
                            on_event(raw_hwnd, GroupEvent::ActivateItem(index));
                        }
                        Some(0)
                    }
                    WM_PORTAL_CONTENT_CHANGED => {
                        if unsafe { SetTimer(hwnd, REFRESH_TIMER_ID, REFRESH_DEBOUNCE_MS, None) }
                            == 0
                        {
                            on_event(raw_hwnd, GroupEvent::RefreshRequested);
                            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                        }
                        Some(0)
                    }
                    WM_TIMER if wparam == REFRESH_TIMER_ID => {
                        unsafe { KillTimer(hwnd, REFRESH_TIMER_ID) };
                        on_event(raw_hwnd, GroupEvent::RefreshRequested);
                        unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                        Some(0)
                    }
                    WM_DROPFILES => {
                        let paths = unsafe { read_drop_paths(wparam as HDROP) };
                        if !paths.is_empty() {
                            on_event(raw_hwnd, GroupEvent::ItemsDropped(paths));
                            scroll_row_for_messages.set(0);
                            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                        }
                        Some(0)
                    }
                    WM_LBUTTONDOWN => {
                        let x = signed_low_word(lparam);
                        let y = signed_high_word(lparam);
                        if y < HEADER_HEIGHT {
                            match header_button_at(hwnd, x, y) {
                                Some(HeaderButton::Close) => unsafe {
                                    DestroyWindow(hwnd);
                                },
                                Some(HeaderButton::Menu) => unsafe {
                                    SendMessageW(
                                        hwnd,
                                        WM_CONTEXTMENU,
                                        hwnd as usize,
                                        header_menu_lparam(hwnd),
                                    );
                                },
                                None => unsafe {
                                    ReleaseCapture();
                                    SendMessageW(
                                        hwnd,
                                        WM_NCLBUTTONDOWN,
                                        usize::try_from(HTCAPTION).unwrap_or_default(),
                                        0,
                                    );
                                },
                            }
                        } else {
                            let item = model_item_index_at(
                                &model_for_messages,
                                grid_layout(hwnd),
                                x,
                                y,
                                scroll_row_for_messages.get(),
                            );
                            if let Some(index) = item {
                                selected_item_for_messages.set(Some(index));
                                on_event(raw_hwnd, GroupEvent::SelectionChanged(Some(index)));
                                dragged_item_for_messages.set(Some(index));
                                unsafe {
                                    SetForegroundWindow(hwnd);
                                    SetFocus(hwnd);
                                    SetCapture(hwnd);
                                    InvalidateRect(hwnd, std::ptr::null(), 0);
                                }
                            } else {
                                selected_item_for_messages.set(None);
                                on_event(raw_hwnd, GroupEvent::SelectionChanged(None));
                                unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                            }
                        }
                        Some(0)
                    }
                    WM_MOUSEMOVE => {
                        let target = model_item_index_at(
                            &model_for_messages,
                            grid_layout(hwnd),
                            signed_low_word(lparam),
                            signed_high_word(lparam),
                            scroll_row_for_messages.get(),
                        );
                        if let Some(from) = dragged_item_for_messages.get()
                            && let Some(to) = target
                            && from != to
                        {
                            dragged_item_for_messages.set(Some(to));
                            selected_item_for_messages.set(Some(to));
                            on_event(raw_hwnd, GroupEvent::MoveItem { from, to });
                            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                        }
                        Some(0)
                    }
                    WM_LBUTTONUP => {
                        if let Some(index) = dragged_item_for_messages.replace(None) {
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
                                GroupEvent::DropItem {
                                    index,
                                    screen_x: point.x,
                                    screen_y: point.y,
                                },
                            );
                            on_event(raw_hwnd, GroupEvent::CommitRequested);
                        }
                        Some(0)
                    }
                    WM_LBUTTONDBLCLK => {
                        if signed_high_word(lparam) < HEADER_HEIGHT
                            && header_button_at(
                                hwnd,
                                signed_low_word(lparam),
                                signed_high_word(lparam),
                            )
                            .is_none()
                        {
                            let next = !collapsed_for_messages.get();
                            unsafe {
                                set_collapsed(
                                    hwnd,
                                    next,
                                    &collapsed_for_messages,
                                    &expanded_height_for_messages,
                                );
                            }
                            on_event(raw_hwnd, GroupEvent::CollapsedChanged(next));
                            on_event(raw_hwnd, GroupEvent::CommitRequested);
                        } else {
                            let item = model_item_index_at(
                                &model_for_messages,
                                grid_layout(hwnd),
                                signed_low_word(lparam),
                                signed_high_word(lparam),
                                scroll_row_for_messages.get(),
                            );
                            if let Some(index) = item {
                                selected_item_for_messages.set(Some(index));
                                on_event(raw_hwnd, GroupEvent::SelectionChanged(Some(index)));
                                unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                                on_event(raw_hwnd, GroupEvent::ActivateItem(index));
                            }
                        }
                        Some(0)
                    }
                    WM_NCLBUTTONDBLCLK if wparam == HTCAPTION as usize => {
                        let next = !collapsed_for_messages.get();
                        unsafe {
                            set_collapsed(
                                hwnd,
                                next,
                                &collapsed_for_messages,
                                &expanded_height_for_messages,
                            );
                        }
                        on_event(raw_hwnd, GroupEvent::CollapsedChanged(next));
                        on_event(raw_hwnd, GroupEvent::CommitRequested);
                        Some(0)
                    }
                    WM_MOUSEWHEEL => {
                        let delta = signed_high_word(isize::try_from(wparam).unwrap_or_default());
                        let item_count = model_for_messages.borrow().items.len();
                        let max_scroll = grid_layout(hwnd).max_scroll_row(item_count);
                        let current_scroll = scroll_row_for_messages.get();
                        let next = if delta < 0 {
                            current_scroll.saturating_add(1).min(max_scroll)
                        } else {
                            current_scroll.saturating_sub(1)
                        };
                        if next != current_scroll {
                            scroll_row_for_messages.set(next);
                            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
                        }
                        Some(0)
                    }
                    WM_SYSCOMMAND if (wparam & 0xfff0) == SC_CLOSE as usize => {
                        unsafe { DestroyWindow(hwnd) };
                        Some(0)
                    }
                    _ => None,
                }
            })
            .create()?;

        unsafe {
            DragAcceptFiles(inner.hwnd().cast(), 1);
            apply_initial_rect(inner.hwnd().cast(), initial_rect);
            if initial_collapsed {
                set_collapsed(inner.hwnd().cast(), true, &collapsed, &expanded_height);
            }
        }

        Ok(Self { inner })
    }

    #[must_use]
    pub fn hwnd(&self) -> *mut c_void {
        self.inner.hwnd()
    }

    #[must_use]
    pub fn hwnd_token(&self) -> isize {
        self.inner.hwnd() as isize
    }

    /// Keeps the interactive MVP pane non-topmost so ordinary windows can cover it.
    #[must_use]
    pub fn attach_to_desktop(&self) -> DesktopAttachmentMode {
        if let Ok(mut host) = ShellOwnedDesktopHost::new()
            && host.attach(self.inner.hwnd()).is_ok()
        {
            DesktopAttachmentMode::ShellOwned
        } else {
            let hwnd: HWND = self.inner.hwnd().cast();
            unsafe {
                SetWindowPos(
                    hwnd,
                    HWND_NOTOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            DesktopAttachmentMode::Standalone
        }
    }

    pub fn run() {
        run();
    }
}

/// Returns the broadcast message Explorer sends after recreating the taskbar and Shell windows.
#[must_use]
pub fn taskbar_created_message() -> u32 {
    let name = wide_null("TaskbarCreated");
    unsafe { RegisterWindowMessageW(name.as_ptr()) }
}

/// Returns the message used to debounce external folder or Shell change notifications.
#[must_use]
pub const fn portal_content_changed_message() -> u32 {
    WM_PORTAL_CONTENT_CHANGED
}

/// Closes one `LucidPane` top-level window on its UI thread.
pub fn close_window(hwnd: isize) {
    unsafe {
        DestroyWindow(hwnd as HWND);
    }
}

unsafe fn begin_title_edit(hwnd: HWND, title: &str, edit_slot: &Cell<HWND>) {
    let existing = edit_slot.get();
    if !existing.is_null() {
        unsafe { SetFocus(existing) };
        return;
    }
    let class = wide_null("EDIT");
    let title = wide_null(title);
    let mut client = RECT::default();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &raw mut client);
    }
    let edit = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_CHILD
                | WS_VISIBLE
                | WS_BORDER
                | WS_TABSTOP
                | u32::try_from(ES_AUTOHSCROLL).unwrap_or_default(),
            HORIZONTAL_PADDING,
            8,
            (header_controls_left(client.right) - HORIZONTAL_PADDING * 2).max(120),
            28,
            hwnd,
            TITLE_EDIT_ID as usize as HMENU,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if edit.is_null() {
        return;
    }
    edit_slot.set(edit);
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    unsafe {
        SendMessageW(edit, WM_SETFONT, font as usize, 1);
        SendMessageW(edit, EM_SETSEL, 0, -1);
        SetFocus(edit);
    }
}

unsafe fn read_window_text(hwnd: HWND) -> Option<String> {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length < 0 {
        return None;
    }
    let capacity = usize::try_from(length).ok()?.saturating_add(1);
    let mut buffer = vec![0_u16; capacity];
    let copied = unsafe {
        GetWindowTextW(
            hwnd,
            buffer.as_mut_ptr(),
            i32::try_from(buffer.len()).unwrap_or(i32::MAX),
        )
    };
    let copied = usize::try_from(copied).ok()?;
    Some(String::from_utf16_lossy(&buffer[..copied]))
}

unsafe fn read_drop_paths(drop: HDROP) -> Vec<PathBuf> {
    let count = unsafe { DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0) };
    let mut paths = Vec::with_capacity(usize::try_from(count).unwrap_or_default());
    for index in 0..count {
        let length = unsafe { DragQueryFileW(drop, index, std::ptr::null_mut(), 0) };
        if length == 0 {
            continue;
        }
        let mut buffer = vec![0_u16; usize::try_from(length).unwrap_or_default() + 1];
        let copied = unsafe {
            DragQueryFileW(
                drop,
                index,
                buffer.as_mut_ptr(),
                u32::try_from(buffer.len()).unwrap_or(u32::MAX),
            )
        };
        if copied > 0 {
            let copied = usize::try_from(copied).unwrap_or_default();
            paths.push(PathBuf::from(OsString::from_wide(&buffer[..copied])));
        }
    }
    unsafe { DragFinish(drop) };
    paths
}

fn low_word(value: usize) -> u16 {
    u16::try_from(value & 0xffff).unwrap_or_default()
}

fn high_word(value: usize) -> u16 {
    u16::try_from((value >> 16) & 0xffff).unwrap_or_default()
}

fn header_button_rect(client_right: i32, button: HeaderButton) -> RECT {
    let close_right = client_right - HEADER_BUTTON_RIGHT;
    let close_left = close_right - HEADER_BUTTON_SIZE;
    let menu_right = close_left - HEADER_BUTTON_GAP;
    let (left, right) = match button {
        HeaderButton::Menu => (menu_right - HEADER_BUTTON_SIZE, menu_right),
        HeaderButton::Close => (close_left, close_right),
    };
    RECT {
        left,
        top: HEADER_BUTTON_TOP,
        right,
        bottom: HEADER_BUTTON_TOP + HEADER_BUTTON_SIZE,
    }
}

fn header_controls_left(client_right: i32) -> i32 {
    header_button_rect(client_right, HeaderButton::Menu).left
}

fn header_button_at(hwnd: HWND, x: i32, y: i32) -> Option<HeaderButton> {
    let mut client = RECT::default();
    if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &raw mut client) }
        == 0
    {
        return None;
    }
    [HeaderButton::Menu, HeaderButton::Close]
        .into_iter()
        .find(|button| point_in_rect(x, y, header_button_rect(client.right, *button)))
}

fn point_in_rect(x: i32, y: i32, rect: RECT) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

unsafe fn header_menu_lparam(hwnd: HWND) -> isize {
    let mut window = RECT::default();
    let mut client = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &raw mut window);
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &raw mut client);
    }
    let menu = header_button_rect(client.right, HeaderButton::Menu);
    pack_point_lparam(window.left + menu.left, window.top + menu.bottom + 2)
}

fn pack_point_lparam(x: i32, y: i32) -> isize {
    fn word(value: i32) -> usize {
        let clamped = value.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
        let signed = i16::try_from(clamped).unwrap_or_default();
        usize::from(u16::from_ne_bytes(signed.to_ne_bytes()))
    }
    isize::try_from(word(x) | (word(y) << 16)).unwrap_or(-1)
}

unsafe fn set_collapsed(
    hwnd: HWND,
    collapse: bool,
    collapsed: &Cell<bool>,
    expanded_height: &Cell<i32>,
) {
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &raw mut rect) } == 0 {
        return;
    }
    if collapse && !collapsed.get() {
        expanded_height.set(rect.bottom - rect.top);
    }
    collapsed.set(collapse);
    let height = if collapse {
        COLLAPSED_HEIGHT
    } else {
        expanded_height.get().max(dip_to_pixel(RectDip::MIN_HEIGHT))
    };
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            rect.right - rect.left,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOMOVE,
        );
    }
}

/// Posts a coalescible Folder Portal refresh request to a live `LucidPane` window.
#[must_use]
pub fn post_portal_content_changed(hwnd: isize) -> bool {
    unsafe { PostMessageW(hwnd as HWND, WM_PORTAL_CONTENT_CHANGED, 0, 0) != 0 }
}

#[must_use]
pub fn window_contains_screen_point(hwnd: isize, x: i32, y: i32) -> bool {
    let mut rect = RECT::default();
    let valid = unsafe { GetWindowRect(hwnd as HWND, &raw mut rect) != 0 };
    valid && x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

unsafe fn apply_initial_rect(hwnd: HWND, rect: RectDip) {
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            dip_to_pixel(rect.x),
            dip_to_pixel(rect.y),
            dip_to_pixel(rect.width),
            dip_to_pixel(rect.height),
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn dip_to_pixel(value: f32) -> i32 {
    #[allow(clippy::cast_possible_truncation)]
    {
        value.round() as i32
    }
}

unsafe fn hit_test(hwnd: HWND, lparam: isize) -> u32 {
    let x = signed_low_word(lparam);
    let y = signed_high_word(lparam);
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &raw mut rect) } == 0 {
        return HTCLIENT;
    }

    let left = x < rect.left + RESIZE_BORDER;
    let right = x >= rect.right - RESIZE_BORDER;
    let top = y < rect.top + RESIZE_BORDER;
    let bottom = y >= rect.bottom - RESIZE_BORDER;

    match (left, right, top, bottom) {
        (true, _, true, _) => HTTOPLEFT,
        (_, true, true, _) => HTTOPRIGHT,
        (true, _, _, true) => HTBOTTOMLEFT,
        (_, true, _, true) => HTBOTTOMRIGHT,
        (true, _, _, _) => HTLEFT,
        (_, true, _, _) => HTRIGHT,
        (_, _, true, _) => HTTOP,
        (_, _, _, true) => HTBOTTOM,
        _ => HTCLIENT,
    }
}

unsafe fn show_panel_menu(
    hwnd: HWND,
    context_lparam: isize,
    current: BackdropKind,
) -> Option<PanelMenuAction> {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return None;
    }

    for (command, label) in [
        (CMD_RENAME, "Rename Pane (F2)"),
        (CMD_CHOOSE_ICON, "Choose Pane Icon..."),
        (CMD_RESET_ICON, "Use Default Icon"),
        (CMD_REFRESH, "Refresh"),
    ] {
        let wide = wide_null(label);
        unsafe {
            AppendMenuW(
                menu,
                MF_STRING,
                usize::try_from(command).unwrap_or_default(),
                wide.as_ptr(),
            );
        }
    }
    unsafe {
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    }

    for (command, kind, label) in [
        (CMD_MICA, BackdropKind::Mica, "Mica"),
        (CMD_MICA_ALT, BackdropKind::MicaAlt, "Mica Alt"),
        (CMD_ACRYLIC, BackdropKind::Acrylic, "Desktop Acrylic"),
        (
            CMD_TRANSLUCENT,
            BackdropKind::Translucent,
            "Translucent (no blur)",
        ),
    ] {
        let wide = wide_null(label);
        let state = if current == kind {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING | MF_UNCHECKED
        };
        unsafe {
            AppendMenuW(
                menu,
                state,
                usize::try_from(command).unwrap_or_default(),
                wide.as_ptr(),
            );
        }
    }

    let close = wide_null("Close");
    unsafe {
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(
            menu,
            MF_STRING,
            usize::try_from(CMD_CLOSE).unwrap_or_default(),
            close.as_ptr(),
        );
    }

    let point = context_point(hwnd, context_lparam);
    let command = unsafe {
        TrackPopupMenuEx(
            menu,
            TPM_LEFTALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
            point.x,
            point.y,
            hwnd,
            std::ptr::null(),
        )
    };
    unsafe {
        DestroyMenu(menu);
    }

    match command {
        CMD_RENAME => Some(PanelMenuAction::Rename),
        CMD_CHOOSE_ICON => Some(PanelMenuAction::ChooseIcon),
        CMD_RESET_ICON => Some(PanelMenuAction::ResetIcon),
        CMD_REFRESH => Some(PanelMenuAction::Refresh),
        CMD_MICA => Some(PanelMenuAction::Material(Backdrop::Mica)),
        CMD_MICA_ALT => Some(PanelMenuAction::Material(Backdrop::MicaAlt)),
        CMD_ACRYLIC => Some(PanelMenuAction::Material(Backdrop::Acrylic)),
        CMD_TRANSLUCENT => Some(PanelMenuAction::Material(Backdrop::translucent())),
        CMD_CLOSE => {
            unsafe {
                PostMessageW(
                    hwnd,
                    WM_SYSCOMMAND,
                    usize::try_from(SC_CLOSE).unwrap_or_default(),
                    0,
                );
            }
            None
        }
        _ => None,
    }
}

fn context_point(hwnd: HWND, lparam: isize) -> POINT {
    if lparam == -1 {
        let mut rect = RECT::default();
        unsafe {
            GetWindowRect(hwnd, &raw mut rect);
        }
        POINT {
            x: i32::midpoint(rect.left, rect.right),
            y: i32::midpoint(rect.top, rect.bottom),
        }
    } else {
        let mut point = POINT {
            x: signed_low_word(lparam),
            y: signed_high_word(lparam),
        };
        if point.x == -1 && point.y == -1 {
            unsafe {
                GetCursorPos(&raw mut point);
            }
        }
        point
    }
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

fn grid_layout(hwnd: HWND) -> GridLayout {
    let mut rect = RECT::default();
    if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &raw mut rect) }
        == 0
    {
        return GridLayout::new(1, 1);
    }
    GridLayout::new(rect.right - rect.left, rect.bottom - rect.top)
}

fn model_item_index_at(
    model: &SharedRenderModel,
    layout: GridLayout,
    x: i32,
    y: i32,
    scroll_row: usize,
) -> Option<usize> {
    let item_count = model.borrow().items.len();
    layout.item_index_at(x, y, scroll_row, item_count)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GridLayout {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    columns: usize,
}

impl GridLayout {
    fn new(client_width: i32, client_height: i32) -> Self {
        let width = (client_width - GRID_PADDING * 2).max(1);
        let height = (client_height - CONTENT_TOP - GRID_PADDING).max(1);
        let columns = usize::try_from((width / GRID_MIN_CELL_WIDTH).max(1)).unwrap_or(1);
        Self {
            left: GRID_PADDING,
            top: CONTENT_TOP,
            width,
            height,
            columns,
        }
    }

    fn visible_rows(self) -> usize {
        usize::try_from((self.height / GRID_CELL_HEIGHT).max(1)).unwrap_or(1)
    }

    fn columns_for(self, item_count: usize) -> usize {
        self.columns.min(item_count.max(1))
    }

    fn total_rows(self, item_count: usize) -> usize {
        item_count.div_ceil(self.columns_for(item_count))
    }

    fn max_scroll_row(self, item_count: usize) -> usize {
        self.total_rows(item_count)
            .saturating_sub(self.visible_rows())
    }

    fn item_index_at(self, x: i32, y: i32, scroll_row: usize, item_count: usize) -> Option<usize> {
        if x < self.left
            || x >= self.left + self.width
            || y < self.top
            || y >= self.top + self.height
        {
            return None;
        }
        let relative_x = x - self.left;
        let columns = self.columns_for(item_count);
        let column = usize::try_from(relative_x * i32::try_from(columns).ok()? / self.width)
            .ok()?
            .min(columns - 1);
        let row = usize::try_from((y - self.top) / GRID_CELL_HEIGHT).ok()?;
        let absolute_row = scroll_row.checked_add(row)?;
        let index = absolute_row.checked_mul(columns)?.checked_add(column)?;
        (index < item_count).then_some(index)
    }

    fn cell_rect(self, column: usize, visible_row: usize, item_count: usize) -> RECT {
        let columns = i32::try_from(self.columns_for(item_count)).unwrap_or(1);
        let column = i32::try_from(column).unwrap_or_default();
        let row = i32::try_from(visible_row).unwrap_or_default();
        RECT {
            left: self.left + self.width * column / columns,
            top: self.top + row * GRID_CELL_HEIGHT,
            right: self.left + self.width * (column + 1) / columns,
            bottom: self.top + (row + 1) * GRID_CELL_HEIGHT,
        }
    }
}

#[allow(clippy::cast_precision_loss)]
unsafe fn window_rect(hwnd: HWND) -> Option<RectDip> {
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &raw mut rect) } == 0 {
        return None;
    }
    Some(RectDip::new(
        rect.left as f32,
        rect.top as f32,
        (rect.right - rect.left) as f32,
        (rect.bottom - rect.top) as f32,
    ))
}

unsafe fn paint_panel(
    hwnd: HWND,
    model: &GroupRenderModel,
    scroll_row: usize,
    selected_item: Option<usize>,
) {
    let mut paint = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &raw mut paint) };
    if hdc.is_null() {
        return;
    }

    let mut client = RECT::default();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &raw mut client);
    }
    let brush = unsafe { CreateSolidBrush(rgb(28, 34, 46)) };
    if !brush.is_null() {
        unsafe {
            FillRect(hdc, &raw const client, brush);
            DeleteObject(brush.cast());
        }
    }

    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    let old_font = if font.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe { SelectObject(hdc, font) }
    };
    unsafe {
        SetBkMode(hdc, i32::try_from(TRANSPARENT).unwrap_or(1));
        SetTextColor(hdc, rgb(245, 247, 250));
    }

    let header_has_icon = draw_header_icon(hdc, model);
    let header_text_left = if header_has_icon {
        HORIZONTAL_PADDING + 42
    } else {
        HORIZONTAL_PADDING
    };
    let mut title_rect = RECT {
        left: header_text_left,
        top: 8,
        right: header_controls_left(client.right) - 8,
        bottom: 32,
    };
    draw_text(hdc, &model.title, &mut title_rect);

    unsafe { SetTextColor(hdc, rgb(176, 184, 196)) };
    let mut subtitle_rect = RECT {
        left: header_text_left,
        top: 31,
        right: header_controls_left(client.right) - 8,
        bottom: HEADER_HEIGHT,
    };
    draw_text(hdc, &model.subtitle, &mut subtitle_rect);
    draw_header_buttons(hdc, client.right);

    let layout = GridLayout::new(client.right - client.left, client.bottom - client.top);
    if model.items.is_empty() {
        draw_empty_hint(hdc, layout);
    }
    draw_items(hdc, client, layout, model, scroll_row, selected_item);

    if !old_font.is_null() {
        unsafe { SelectObject(hdc, old_font) };
    }
    unsafe { EndPaint(hwnd, &raw const paint) };
}

fn draw_items(
    hdc: *mut c_void,
    client: RECT,
    layout: GridLayout,
    model: &GroupRenderModel,
    scroll_row: usize,
    selected_item: Option<usize>,
) {
    let columns = layout.columns_for(model.items.len());
    let first_index = scroll_row.saturating_mul(columns);
    for (visible_index, item) in model.items.iter().skip(first_index).enumerate() {
        let item_index = first_index + visible_index;
        let visible_row = visible_index / columns;
        let column = visible_index % columns;
        let cell = layout.cell_rect(column, visible_row, model.items.len());
        if cell.top >= client.bottom {
            break;
        }
        if selected_item == Some(item_index) {
            draw_selection(hdc, cell);
        }
        let label = if item.icon.is_some() {
            item.label.clone()
        } else {
            let marker = match item.kind {
                RenderItemKind::Directory => "[DIR]",
                RenderItemKind::File => "[FILE]",
                RenderItemKind::Shortcut => "[LNK]",
            };
            format!("{marker}  {}", item.label)
        };
        if let Some(icon) = item.icon {
            let icon_x = i32::midpoint(cell.left, cell.right) - GRID_ICON_SIZE / 2;
            unsafe {
                ImageList_Draw(
                    icon.image_list,
                    icon.index,
                    hdc,
                    icon_x,
                    cell.top + 6,
                    ILD_TRANSPARENT,
                );
            }
        }
        let mut label_rect = RECT {
            left: cell.left + 5,
            top: cell.top + 42,
            right: cell.right - 5,
            bottom: cell.bottom.min(client.bottom),
        };
        unsafe { SetTextColor(hdc, rgb(232, 236, 242)) };
        draw_grid_text(hdc, &label, &mut label_rect);
    }
}

fn draw_selection(hdc: *mut c_void, cell: RECT) {
    let rect = RECT {
        left: cell.left + 4,
        top: cell.top + 2,
        right: cell.right - 4,
        bottom: cell.bottom - 2,
    };
    let brush = unsafe { CreateSolidBrush(rgb(54, 91, 138)) };
    if !brush.is_null() {
        unsafe {
            FillRect(hdc, &raw const rect, brush);
            DeleteObject(brush.cast());
        }
    }
}

fn draw_empty_hint(hdc: *mut c_void, layout: GridLayout) {
    unsafe { SetTextColor(hdc, rgb(156, 166, 180)) };
    let mut rect = RECT {
        left: layout.left,
        top: layout.top + 40,
        right: layout.left + layout.width,
        bottom: layout.top + 100,
    };
    draw_grid_text(hdc, "Drop desktop icons here", &mut rect);
}

fn draw_header_buttons(hdc: *mut c_void, client_right: i32) {
    for (button, label, color) in [
        (HeaderButton::Menu, "...", rgb(72, 82, 98)),
        (HeaderButton::Close, "X", rgb(132, 58, 64)),
    ] {
        let mut rect = header_button_rect(client_right, button);
        let brush = unsafe { CreateSolidBrush(color) };
        if !brush.is_null() {
            unsafe {
                FillRect(hdc, &raw const rect, brush);
                DeleteObject(brush.cast());
            }
        }
        unsafe { SetTextColor(hdc, rgb(245, 247, 250)) };
        draw_button_text(hdc, label, &mut rect);
    }
}

fn draw_header_icon(hdc: *mut c_void, model: &GroupRenderModel) -> bool {
    if let Some(path) = model.custom_header_icon.as_deref()
        && draw_custom_icon(hdc, path)
    {
        return true;
    }
    if let Some(icon) = model.header_icon {
        unsafe {
            ImageList_Draw(
                icon.image_list,
                icon.index,
                hdc,
                HORIZONTAL_PADDING,
                13,
                ILD_TRANSPARENT,
            );
        }
        return true;
    }
    false
}

fn draw_custom_icon(hdc: *mut c_void, path: &Path) -> bool {
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ico"))
    {
        return false;
    }
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        LoadImageW(
            std::ptr::null_mut(),
            wide.as_ptr(),
            IMAGE_ICON,
            GRID_ICON_SIZE,
            GRID_ICON_SIZE,
            LR_LOADFROMFILE,
        )
    };
    if handle.is_null() {
        return false;
    }
    let icon = handle as HICON;
    unsafe {
        DrawIconEx(
            hdc,
            HORIZONTAL_PADDING,
            13,
            icon,
            GRID_ICON_SIZE,
            GRID_ICON_SIZE,
            0,
            std::ptr::null_mut(),
            DI_NORMAL,
        );
        DestroyIcon(icon);
    }
    true
}

fn draw_text(hdc: *mut c_void, text: &str, rect: &mut RECT) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let length = i32::try_from(wide.len()).unwrap_or(i32::MAX);
    unsafe {
        DrawTextW(
            hdc,
            wide.as_ptr(),
            length,
            rect,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}

fn draw_grid_text(hdc: *mut c_void, text: &str, rect: &mut RECT) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let length = i32::try_from(wide.len()).unwrap_or(i32::MAX);
    unsafe {
        DrawTextW(
            hdc,
            wide.as_ptr(),
            length,
            rect,
            DT_CENTER | DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}

fn draw_button_text(hdc: *mut c_void, text: &str, rect: &mut RECT) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let length = i32::try_from(wide.len()).unwrap_or(i32::MAX);
    unsafe {
        DrawTextW(
            hdc,
            wide.as_ptr(),
            length,
            rect,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
    }
}

fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16)
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::{GridLayout, GroupRenderModel, RenderItem, RenderItemKind, model_item_index_at};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn grid_flows_left_to_right_then_down() {
        let grid = GridLayout::new(420, 360);
        assert_eq!(grid.columns, 3);
        assert_eq!(grid.item_index_at(20, 70, 0, 8), Some(0));
        assert_eq!(grid.item_index_at(160, 70, 0, 8), Some(1));
        assert_eq!(grid.item_index_at(300, 70, 0, 8), Some(2));
        assert_eq!(grid.item_index_at(20, 160, 0, 8), Some(3));
    }

    #[test]
    fn grid_redistributes_columns_after_resize() {
        assert_eq!(GridLayout::new(300, 360).columns, 2);
        assert_eq!(GridLayout::new(600, 360).columns, 5);
    }

    #[test]
    fn grid_cells_fill_available_width_without_a_trailing_gap() {
        let grid = GridLayout::new(421, 360);
        let last = grid.cell_rect(grid.columns_for(3) - 1, 0, 3);
        assert_eq!(last.right, grid.left + grid.width);
    }

    #[test]
    fn sparse_first_row_spreads_items_across_the_pane() {
        let grid = GridLayout::new(600, 360);
        assert_eq!(grid.columns, 5);
        assert_eq!(grid.columns_for(3), 3);
        let last = grid.cell_rect(2, 0, 3);
        assert_eq!(last.right, grid.left + grid.width);
    }

    #[test]
    fn hit_testing_releases_the_render_model_before_event_updates() {
        let model = Rc::new(RefCell::new(GroupRenderModel {
            items: vec![RenderItem {
                label: "Editor".into(),
                kind: RenderItemKind::Shortcut,
                icon: None,
            }],
            ..GroupRenderModel::default()
        }));

        assert_eq!(
            model_item_index_at(&model, GridLayout::new(420, 360), 20, 70, 0),
            Some(0)
        );
        model.borrow_mut().subtitle = "Selected: Editor".into();
        assert_eq!(model.borrow().subtitle, "Selected: Editor");
    }

    #[test]
    fn grid_hit_test_includes_scroll_rows() {
        let grid = GridLayout::new(420, 360);
        assert_eq!(grid.item_index_at(20, 70, 2, 20), Some(6));
    }

    #[test]
    fn grid_rejects_header_and_empty_cells() {
        let grid = GridLayout::new(420, 360);
        assert_eq!(grid.item_index_at(20, 20, 0, 4), None);
        assert_eq!(grid.item_index_at(300, 160, 0, 4), None);
    }
}
