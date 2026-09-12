//! Dedicated Everything pane: native IME edit, indexed report list and async paging.
#![allow(clippy::wildcard_imports, clippy::too_many_lines)]
use super::{
    Event, GroupModel,
    everything::{self, Entry, PAGE_SIZE, Page},
};
use desktop_core::{RectDip, ShellIdentity};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{Controls::*, HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const EDIT: usize = 101;
const FILTER: usize = 102;
const LIST: usize = 103;
const STATUS: usize = 104;
const PREVIOUS: usize = 105;
const NEXT: usize = 106;
const REFRESH: usize = 107;
const OPEN: usize = 110;
const LOCATION: usize = 111;
const COPY: usize = 112;
const CUT: usize = 113;
const DELETE: usize = 114;
const PEEK: usize = 115;
const FOCUS: usize = 116;
const SELECT_ALL: usize = 117;
const SETTINGS: usize = 118;
const POLL: usize = 38;
pub(super) const CLEAR_SELECTION: u32 = WM_APP + 118;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn control(hwnd: HWND, id: usize) -> HWND {
    unsafe { GetDlgItem(hwnd, id as i32) }
}
fn set_text(hwnd: HWND, text: &str) {
    unsafe {
        SetWindowTextW(hwnd, wide(text).as_ptr());
    }
}
fn text(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    let mut result = vec![0; len + 1];
    let n = unsafe { GetWindowTextW(hwnd, result.as_mut_ptr(), result.len() as i32) };
    String::from_utf16_lossy(&result[..n.max(0) as usize])
}
fn send(hwnd: HWND, message: u32, wp: usize, lp: isize) -> isize {
    unsafe { SendMessageW(hwnd, message, wp, lp) }
}

struct Appearance {
    dark: bool,
    background: HBRUSH,
    surface: HBRUSH,
    font: HFONT,
    dpi: u32,
}
impl Appearance {
    fn new(dark: bool) -> Self {
        Self {
            dark,
            background: unsafe { CreateSolidBrush(if dark { 0x202020 } else { 0xf5f5f5 }) },
            surface: unsafe { CreateSolidBrush(if dark { 0x292929 } else { 0xffffff }) },
            font: std::ptr::null_mut(),
            dpi: 0,
        }
    }
    fn foreground(&self) -> u32 {
        if self.dark { 0xf0f0f0 } else { 0x202020 }
    }
    fn surface_color(&self) -> u32 {
        if self.dark { 0x292929 } else { 0xffffff }
    }
    fn apply(&mut self, hwnd: HWND) {
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        if self.dpi != dpi {
            let font = unsafe {
                CreateFontW(
                    -(((16 * dpi) / 96) as i32),
                    0,
                    0,
                    0,
                    FW_NORMAL as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    CLEARTYPE_QUALITY as u32,
                    DEFAULT_PITCH as u32,
                    windows_sys::w!("Segoe UI"),
                )
            };
            if !font.is_null() {
                for id in [
                    EDIT, FILTER, LIST, STATUS, PREVIOUS, NEXT, REFRESH, SETTINGS,
                ] {
                    send(control(hwnd, id), WM_SETFONT, font as usize, 1);
                }
                if !self.font.is_null() {
                    unsafe {
                        DeleteObject(self.font);
                    }
                }
                self.font = font;
                self.dpi = dpi;
            }
        }
        let theme = wide(if self.dark {
            "DarkMode_Explorer"
        } else {
            "Explorer"
        });
        for id in [EDIT, FILTER, LIST, PREVIOUS, NEXT, REFRESH, SETTINGS] {
            unsafe {
                SetWindowTheme(control(hwnd, id), theme.as_ptr(), std::ptr::null());
            }
        }
        let list = control(hwnd, LIST);
        send(list, LVM_SETTEXTCOLOR, 0, self.foreground() as isize);
        send(list, LVM_SETTEXTBKCOLOR, 0, self.surface_color() as isize);
        send(list, LVM_SETBKCOLOR, 0, self.surface_color() as isize);
        unsafe {
            use windows::Win32::Graphics::Dwm::{
                DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
            };
            let value = i32::from(self.dark);
            let _ = DwmSetWindowAttribute(
                windows::Win32::Foundation::HWND(hwnd),
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const value).cast(),
                4,
            );
            RedrawWindow(
                hwnd,
                std::ptr::null(),
                std::ptr::null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
            );
        }
    }
}
impl Drop for Appearance {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.background);
            DeleteObject(self.surface);
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
        }
    }
}

// Control painting can be synchronous while the window's FnMut handler is
// detached. A separate subclass keeps colors available during those redraws.
unsafe extern "system" fn colors(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    data: usize,
) -> LRESULT {
    if matches!(
        msg,
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC
    ) {
        let appearance = unsafe { &*(data as *const RefCell<Appearance>) };
        if let Ok(paint) = appearance.try_borrow() {
            let background = if msg == WM_CTLCOLORSTATIC {
                if paint.dark { 0x202020 } else { 0xf5f5f5 }
            } else {
                paint.surface_color()
            };
            unsafe {
                SetTextColor(wp as HDC, paint.foreground());
                SetBkColor(wp as HDC, background);
            }
            return if msg == WM_CTLCOLORSTATIC {
                paint.background
            } else {
                paint.surface
            } as isize;
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

unsafe extern "system" fn header_paint(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    data: usize,
) -> LRESULT {
    if msg == WM_PAINT {
        let appearance = unsafe { &*(data as *const RefCell<Appearance>) };
        if let Ok(paint) = appearance.try_borrow() {
            let mut ps = PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(hwnd, &raw mut ps) };
            let mut bounds = RECT::default();
            unsafe {
                GetClientRect(hwnd, &raw mut bounds);
                FillRect(dc, &bounds, paint.background);
                SetTextColor(dc, paint.foreground());
                SetBkMode(dc, TRANSPARENT as i32);
            }
            let old = unsafe { SelectObject(dc, paint.font) };
            let count = send(hwnd, HDM_GETITEMCOUNT, 0, 0);
            for i in 0..count {
                let mut rect = RECT::default();
                let mut buffer = [0u16; 128];
                let mut item = HDITEMW {
                    mask: HDI_TEXT,
                    pszText: buffer.as_mut_ptr(),
                    cchTextMax: buffer.len() as i32,
                    ..Default::default()
                };
                send(hwnd, HDM_GETITEMRECT, i as usize, (&raw mut rect) as isize);
                send(hwnd, HDM_GETITEMW, i as usize, (&raw mut item) as isize);
                rect.left += 10;
                rect.right -= 8;
                unsafe {
                    DrawTextW(
                        dc,
                        buffer.as_ptr(),
                        -1,
                        &raw mut rect,
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                    );
                }
            }
            unsafe {
                SelectObject(dc, old);
                EndPaint(hwnd, &ps);
            }
            return 0;
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

#[derive(Clone)]
struct Request {
    generation: u64,
    query: String,
    offset: u32,
}
struct Search {
    sender: mpsc::Sender<Request>,
    receiver: mpsc::Receiver<(u64, Result<Page, String>)>,
    generation: u64,
    due: Option<Instant>,
    offset: u32,
    total: u32,
    entries: Vec<Entry>,
    busy: bool,
}
impl Search {
    fn new() -> Self {
        let (sender, requests) = mpsc::channel::<Request>();
        let (responses, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(mut request) = requests.recv() {
                // Superseded requests never enter Everything's IPC queue.
                for newer in requests.try_iter() {
                    request = newer;
                }
                let result = everything::query(&request.query, request.offset);
                if responses.send((request.generation, result)).is_err() {
                    break;
                }
            }
        });
        Self {
            sender,
            receiver,
            generation: 0,
            due: None,
            offset: 0,
            total: 0,
            entries: Vec::new(),
            busy: false,
        }
    }
    fn schedule(&mut self, hwnd: HWND, offset: u32, delay: bool) {
        self.generation += 1;
        self.offset = offset;
        self.busy = true;
        self.due = Some(
            Instant::now()
                + if delay {
                    Duration::from_millis(300)
                } else {
                    Duration::ZERO
                },
        );
        self.entries.clear();
        send(control(hwnd, LIST), LVM_DELETEALLITEMS, 0, 0);
        set_text(control(hwnd, STATUS), "正在搜索…");
        self.buttons(hwnd);
    }
    fn buttons(&self, hwnd: HWND) {
        unsafe {
            EnableWindow(
                control(hwnd, PREVIOUS),
                i32::from(!self.busy && self.offset > 0),
            );
            EnableWindow(
                control(hwnd, NEXT),
                i32::from(!self.busy && self.offset.saturating_add(PAGE_SIZE) < self.total),
            );
        }
    }
    fn tick(&mut self, hwnd: HWND) {
        if self.due.is_some_and(|due| Instant::now() >= due) {
            self.due = None;
            let query = filtered(
                &text(control(hwnd, EDIT)),
                send(control(hwnd, FILTER), CB_GETCURSEL, 0, 0),
            );
            if self
                .sender
                .send(Request {
                    generation: self.generation,
                    query,
                    offset: self.offset,
                })
                .is_err()
            {
                self.busy = false;
                set_text(control(hwnd, STATUS), "搜索线程已停止，请重新打开面板。");
            }
        }
        while let Ok((generation, result)) = self.receiver.try_recv() {
            if generation != self.generation {
                continue;
            }
            self.busy = false;
            match result {
                Ok(page) => {
                    self.total = page.total;
                    self.offset = page.offset;
                    if self.offset >= self.total && self.offset > 0 {
                        self.schedule(hwnd, 0, false);
                        continue;
                    }
                    self.entries = page.entries;
                    fill(hwnd, &self.entries);
                    let status = if self.total == 0 {
                        "没有匹配的结果".to_string()
                    } else {
                        format!(
                            "{} 个结果 · 显示 {}–{} · Enter 打开 / Ctrl+Enter 打开位置",
                            self.total,
                            self.offset + 1,
                            self.offset + self.entries.len() as u32
                        )
                    };
                    set_text(control(hwnd, STATUS), &status);
                }
                Err(error) => {
                    self.total = 0;
                    set_text(control(hwnd, STATUS), &error);
                }
            }
            self.buttons(hwnd);
        }
    }
    fn selected(&self, hwnd: HWND) -> Vec<ShellIdentity> {
        let list = control(hwnd, LIST);
        let mut result = Vec::new();
        let mut index = send(list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
        while index >= 0 {
            if let Some(entry) = self.entries.get(index as usize) {
                result.push(identity(entry.path.clone()));
            }
            index = send(
                list,
                LVM_GETNEXTITEM,
                index as usize,
                LVNI_SELECTED as isize,
            );
        }
        result
    }
}

fn filtered(query: &str, filter: isize) -> String {
    let prefix = match filter {
        1 => "file:",
        2 => "folder:",
        _ => return query.trim().to_string(),
    };
    if query.trim().is_empty() {
        prefix.into()
    } else {
        format!("{prefix} <{}>", query.trim())
    }
}
fn identity(path: std::path::PathBuf) -> ShellIdentity {
    ShellIdentity::FileSystem {
        path,
        volume_id: None,
        file_id: None,
    }
}

fn modified(ticks: u64) -> String {
    use windows_sys::Win32::System::Time::{
        FileTimeToSystemTime, SystemTimeToTzSpecificLocalTimeEx,
    };
    let file = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    if ticks == 0
        || unsafe { FileTimeToSystemTime(&file, &raw mut utc) } == 0
        || unsafe { SystemTimeToTzSpecificLocalTimeEx(std::ptr::null(), &utc, &raw mut local) } == 0
    {
        return "—".into();
    }
    format!(
        "{:04}/{:02}/{:02} {:02}:{:02}",
        local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute
    )
}
fn kind(entry: &Entry) -> String {
    if entry.folder {
        return "文件夹".into();
    }
    entry.path.extension().map_or_else(
        || "文件".into(),
        |ext| format!("{} 文件", ext.to_string_lossy().to_uppercase()),
    )
}
fn fill(hwnd: HWND, entries: &[Entry]) {
    let list = control(hwnd, LIST);
    send(list, WM_SETREDRAW, 0, 0);
    send(list, LVM_DELETEALLITEMS, 0, 0);
    for (i, entry) in entries.iter().enumerate() {
        let name = entry
            .path
            .file_name()
            .unwrap_or(entry.path.as_os_str())
            .to_string_lossy()
            .into_owned();
        let path = entry
            .path
            .parent()
            .unwrap_or(&entry.path)
            .to_string_lossy()
            .into_owned();
        for (column, value) in [name, path, kind(entry), modified(entry.modified)]
            .iter()
            .enumerate()
        {
            let mut value = wide(value);
            let item = LVITEMW {
                mask: LVIF_TEXT,
                iItem: i as i32,
                iSubItem: column as i32,
                pszText: value.as_mut_ptr(),
                ..Default::default()
            };
            send(
                list,
                if column == 0 {
                    LVM_INSERTITEMW
                } else {
                    LVM_SETITEMW
                },
                0,
                (&raw const item) as isize,
            );
        }
    }
    send(list, WM_SETREDRAW, 1, 0);
    unsafe {
        InvalidateRect(list, std::ptr::null(), 1);
    }
}

unsafe extern "system" fn keys(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    _: usize,
) -> LRESULT {
    const COMPOSING: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.SearchComposing");
    if msg == WM_IME_STARTCOMPOSITION {
        unsafe {
            SetPropW(hwnd, COMPOSING, 1usize as _);
        }
    }
    if msg == WM_IME_ENDCOMPOSITION || msg == WM_NCDESTROY {
        unsafe {
            RemovePropW(hwnd, COMPOSING);
        }
    }
    if msg == WM_KEYDOWN && !unsafe { GetPropW(hwnd, COMPOSING) }.is_null() {
        return unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
    }
    if msg == WM_KEYDOWN {
        let ctrl = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT as i32) } < 0;
        let alt = unsafe { GetKeyState(VK_MENU as i32) } < 0;
        let owner = unsafe { GetParent(hwnd) };
        let editing = unsafe { GetDlgCtrlID(hwnd) } == EDIT as i32;
        let command = if ctrl && !alt && !shift && wp == 0x4c {
            Some(FOCUS)
        } else if wp == VK_F5 as usize && !ctrl && !alt && !shift {
            Some(REFRESH)
        } else if wp == VK_RETURN as usize && !alt && !shift {
            Some(if editing {
                REFRESH
            } else if ctrl {
                LOCATION
            } else {
                OPEN
            })
        } else if !editing && !alt {
            let mods = super::keyboard::Modifiers {
                ctrl,
                shift,
                alt,
                windows: unsafe {
                    GetKeyState(VK_LWIN as i32) < 0 || GetKeyState(VK_RWIN as i32) < 0
                },
            };
            if super::peek::matches(wp as u16, &mods, lp & (1 << 30) != 0) {
                Some(PEEK)
            } else if ctrl && !shift {
                match wp {
                    0x41 => Some(SELECT_ALL),
                    0x43 => Some(COPY),
                    0x58 => Some(CUT),
                    _ => None,
                }
            } else if !ctrl && !shift && wp == VK_DELETE as usize {
                Some(DELETE)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(command) = command {
            if lp & (1 << 30) != 0 {
                return 0;
            }
            unsafe {
                PostMessageW(owner, WM_COMMAND, command, 0);
            }
            return 0;
        }
        if editing && wp == VK_ESCAPE as usize {
            set_text(hwnd, "");
            return 0;
        }
        if editing && ctrl && wp == 0x41 {
            send(hwnd, EM_SETSEL, 0, -1);
            return 0;
        }
        if editing && wp == VK_DOWN as usize {
            let list = control(owner, LIST);
            unsafe {
                SetFocus(list);
            }
            if send(list, LVM_GETSELECTEDCOUNT, 0, 0) == 0 && send(list, LVM_GETITEMCOUNT, 0, 0) > 0
            {
                let item = LVITEMW {
                    state: LVIS_SELECTED | LVIS_FOCUSED,
                    stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                    ..Default::default()
                };
                send(list, LVM_SETITEMSTATE, 0, (&raw const item) as isize);
            }
            return 0;
        }
        if wp == VK_TAB as usize {
            unsafe {
                SetFocus(GetNextDlgTabItem(owner, hwnd, i32::from(shift)));
            }
            return 0;
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

fn layout(hwnd: HWND) {
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(hwnd, &raw mut rect);
    }
    let s = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
    let w = (rect.right as f32 / s) as i32;
    let h = (rect.bottom as f32 / s) as i32;
    for (id, x, y, width, height) in [
        (EDIT, 16, 16, w - 326, 32),
        (FILTER, w - 298, 16, 110, 240),
        (REFRESH, w - 176, 16, 76, 32),
        (SETTINGS, w - 88, 16, 72, 32),
        (LIST, 16, 64, w - 32, h - 128),
        (STATUS, 16, h - 53, w - 215, 44),
        (PREVIOUS, w - 186, h - 42, 80, 30),
        (NEXT, w - 96, h - 42, 80, 30),
    ] {
        unsafe {
            MoveWindow(
                control(hwnd, id),
                (x as f32 * s) as i32,
                (y as f32 * s) as i32,
                (width.max(1) as f32 * s) as i32,
                (height.max(1) as f32 * s) as i32,
                1,
            );
        }
    }
    for (i, width) in [205, (w - 32 - 205 - 105 - 150).max(100), 105, 150]
        .iter()
        .enumerate()
    {
        send(
            control(hwnd, LIST),
            LVM_SETCOLUMNWIDTH,
            i,
            (*width as f32 * s) as isize,
        );
    }
}

fn action(hwnd: HWND, command: usize, items: Vec<ShellIdentity>) {
    if items.is_empty() {
        return;
    }
    let owner = hwnd as isize;
    super::window::defer_action(move || {
        if unsafe { IsWindow(owner as HWND) } == 0 {
            return;
        }
        let result = match command {
            OPEN => desktop_shell::open_shell_identity(owner, &items[0]).map_err(|e| e.to_string()),
            LOCATION => show_location(&items[0]),
            PEEK => super::peek::open_path(&items[0]),
            COPY | CUT | DELETE => desktop_shell::invoke_file_commands(
                windows::Win32::Foundation::HWND(owner as _),
                &items,
                match command {
                    COPY => desktop_shell::FileCommand::Copy,
                    CUT => desktop_shell::FileCommand::Cut,
                    _ => desktop_shell::FileCommand::Delete,
                },
            )
            .map(|_| ())
            .map_err(|e| e.to_string()),
            _ => Ok(()),
        };
        if let Err(error) = result {
            super::window::error(&error);
        }
        if command == DELETE {
            unsafe {
                PostMessageW(owner as HWND, WM_COMMAND, REFRESH, 0);
            }
        }
    });
}
fn show_location(item: &ShellIdentity) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{SHOpenFolderAndSelectItems, SHParseDisplayName},
    };
    let path = item.file_system_path().ok_or("搜索结果没有文件路径")?;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let mut pidl = std::ptr::null_mut();
        SHParseDisplayName(
            windows_core::PCWSTR(path.as_ptr()),
            None,
            &raw mut pidl,
            0,
            None,
        )
        .map_err(|e| e.to_string())?;
        let result = SHOpenFolderAndSelectItems(pidl, None, 0).map_err(|e| e.to_string());
        CoTaskMemFree(Some(pidl.cast()));
        result
    }
}

pub(super) fn create(
    rect: RectDip,
    model: Rc<RefCell<GroupModel>>,
    event: impl FnMut(Event) -> bool + 'static,
) -> Result<windows_window::Window, String> {
    let event = Rc::new(RefCell::new(event));
    let mut state = Search::new();
    let appearance = Rc::new(RefCell::new(Appearance::new(model.borrow().dark)));
    let paint = Rc::clone(&appearance);
    let handler = Rc::clone(&event);
    let window = windows_window::Window::new("Everything 搜索 · LucidPane")
        .style(WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN)
        .ex_style(WS_EX_APPWINDOW)
        .size(rect.width as i32, rect.height as i32)
        .on_message(move |raw, message, wp, lp| {
            let hwnd = raw.cast();
            match message {
                WM_SETFOCUS => {
                    unsafe {
                        SetFocus(control(hwnd, EDIT));
                    }
                    return Some(0);
                }
                CLEAR_SELECTION => {
                    let item = LVITEMW {
                        state: 0,
                        stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                        ..Default::default()
                    };
                    send(
                        control(hwnd, LIST),
                        LVM_SETITEMSTATE,
                        usize::MAX,
                        (&raw const item) as isize,
                    );
                    return Some(0);
                }
                WM_CLOSE => {
                    let handler = Rc::clone(&handler);
                    super::window::defer_action(move || {
                        (handler.borrow_mut())(Event::ClosePane);
                    });
                    return Some(0);
                }
                WM_DESTROY => {
                    unsafe {
                        KillTimer(hwnd, POLL);
                    }
                    return Some(0);
                }
                WM_SIZE => {
                    layout(hwnd);
                    return Some(0);
                }
                WM_GETMINMAXINFO => {
                    if lp != 0 {
                        unsafe {
                            let dpi = GetDpiForWindow(hwnd).max(96);
                            (*(lp as *mut MINMAXINFO)).ptMinTrackSize = POINT {
                                x: (680 * dpi / 96) as i32,
                                y: (400 * dpi / 96) as i32,
                            };
                        }
                    }
                    return Some(0);
                }
                WM_EXITSIZEMOVE => {
                    let mut bounds = RECT::default();
                    unsafe {
                        GetWindowRect(hwnd, &raw mut bounds);
                    }
                    let s = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
                    (handler.borrow_mut())(Event::Geometry(RectDip::new(
                        bounds.left as f32 / s,
                        bounds.top as f32 / s,
                        (bounds.right - bounds.left) as f32 / s,
                        (bounds.bottom - bounds.top) as f32 / s,
                    )));
                }
                WM_DPICHANGED => {
                    if lp != 0 {
                        let r = unsafe { &*(lp as *const RECT) };
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
                    }
                    paint.borrow_mut().apply(hwnd);
                    layout(hwnd);
                }
                WM_TIMER if wp == POLL => {
                    let dark = model.borrow().dark;
                    if paint.borrow().dark != dark {
                        *paint.borrow_mut() = Appearance::new(dark);
                        paint.borrow_mut().apply(hwnd);
                    }
                    state.tick(hwnd);
                    return Some(0);
                }
                WM_TIMER => {
                    (handler.borrow_mut())(Event::Tick);
                    return Some(0);
                }
                WM_COMMAND => {
                    let id = wp & 0xffff;
                    let notification = (wp >> 16) as u32;
                    match id {
                        EDIT if notification == EN_CHANGE => state.schedule(hwnd, 0, true),
                        FILTER if notification == CBN_SELCHANGE => state.schedule(hwnd, 0, false),
                        REFRESH => state.schedule(hwnd, 0, false),
                        PREVIOUS if !state.busy => {
                            state.schedule(hwnd, state.offset.saturating_sub(PAGE_SIZE), false)
                        }
                        NEXT if !state.busy
                            && state.offset.saturating_add(PAGE_SIZE) < state.total =>
                        {
                            state.schedule(hwnd, state.offset + PAGE_SIZE, false)
                        }
                        FOCUS => {
                            unsafe {
                                SetFocus(control(hwnd, EDIT));
                            }
                            send(control(hwnd, EDIT), EM_SETSEL, 0, -1);
                        }
                        SELECT_ALL => {
                            (handler.borrow_mut())(Event::PaneItemFocus);
                            let item = LVITEMW {
                                state: LVIS_SELECTED,
                                stateMask: LVIS_SELECTED,
                                ..Default::default()
                            };
                            send(
                                control(hwnd, LIST),
                                LVM_SETITEMSTATE,
                                usize::MAX,
                                (&raw const item) as isize,
                            );
                        }
                        OPEN | LOCATION | COPY | CUT | DELETE | PEEK => {
                            action(hwnd, id, state.selected(hwnd))
                        }
                        SETTINGS => {
                            let handler = Rc::clone(&handler);
                            super::window::defer_action(move || {
                                (handler.borrow_mut())(Event::Settings);
                            });
                        }
                        _ => {}
                    }
                    return Some(0);
                }
                WM_NOTIFY if lp != 0 => {
                    let header = unsafe { &*(lp as *const NMHDR) };
                    if header.idFrom == LIST && header.code == LVN_ITEMCHANGED {
                        let change = unsafe { &*(lp as *const NMLISTVIEW) };
                        if change.uNewState & LVIS_SELECTED != 0
                            && change.uOldState & LVIS_SELECTED == 0
                        {
                            (handler.borrow_mut())(Event::PaneItemFocus);
                        }
                    }
                    if header.idFrom == LIST && header.code == NM_DBLCLK {
                        action(hwnd, OPEN, state.selected(hwnd));
                        return Some(0);
                    }
                }
                WM_CONTEXTMENU if wp == control(hwnd, LIST) as usize => {
                    let items = state.selected(hwnd);
                    if !items.is_empty() {
                        let owner = hwnd as isize;
                        super::window::defer_action(move || {
                            let hwnd = owner as HWND;
                            if unsafe { IsWindow(hwnd) } == 0 {
                                return;
                            }
                            let mut point = POINT::default();
                            unsafe {
                                GetCursorPos(&raw mut point);
                            }
                            let menu = unsafe { CreatePopupMenu() };
                            if menu.is_null() {
                                return;
                            }
                            for (id, label) in [
                                (OPEN, "打开\tEnter"),
                                (LOCATION, "打开文件位置\tCtrl+Enter"),
                                (COPY, "复制\tCtrl+C"),
                                (CUT, "剪切\tCtrl+X"),
                                (DELETE, "删除\tDelete"),
                                (PEEK, "Peek 预览"),
                            ] {
                                unsafe {
                                    AppendMenuW(menu, MF_STRING, id, wide(label).as_ptr());
                                }
                            }
                            let command = unsafe {
                                TrackPopupMenuEx(
                                    menu,
                                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                                    point.x,
                                    point.y,
                                    hwnd,
                                    std::ptr::null(),
                                )
                            };
                            unsafe {
                                DestroyMenu(menu);
                            }
                            action(hwnd, command as usize, items);
                        });
                    }
                    return Some(0);
                }
                WM_ERASEBKGND => {
                    let Ok(paint) = paint.try_borrow() else {
                        return None;
                    };
                    let mut r = RECT::default();
                    unsafe {
                        GetClientRect(hwnd, &raw mut r);
                        FillRect(wp as HDC, &r, paint.background);
                    }
                    return Some(1);
                }
                _ => {}
            }
            None
        })
        .create()
        .map_err(|e| e.to_string())?;
    let hwnd = window.hwnd().cast();
    unsafe {
        SetWindowSubclass(hwnd, Some(colors), 2, Rc::as_ptr(&appearance) as usize);
    }
    unsafe {
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        });
    }
    for (id, class, label, style) in [
        (
            EDIT,
            "EDIT",
            "",
            ES_AUTOHSCROLL as u32 | WS_BORDER | WS_TABSTOP,
        ),
        (
            FILTER,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP,
        ),
        (REFRESH, "BUTTON", "刷新", BS_PUSHBUTTON as u32 | WS_TABSTOP),
        (
            SETTINGS,
            "BUTTON",
            "设置",
            BS_PUSHBUTTON as u32 | WS_TABSTOP,
        ),
        (
            LIST,
            "SysListView32",
            "",
            LVS_REPORT | LVS_SHOWSELALWAYS | WS_TABSTOP | WS_BORDER,
        ),
        (
            STATUS,
            "STATIC",
            "输入名称或 Everything 语法（例如 ext:pdf）",
            0,
        ),
        (
            PREVIOUS,
            "BUTTON",
            "上一页",
            BS_PUSHBUTTON as u32 | WS_TABSTOP,
        ),
        (NEXT, "BUTTON", "下一页", BS_PUSHBUTTON as u32 | WS_TABSTOP),
    ] {
        let child = unsafe {
            CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(label).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                0,
                0,
                1,
                1,
                hwnd,
                id as _,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if child.is_null() {
            return Err("无法创建 Everything 搜索控件".into());
        }
        send(
            child,
            WM_SETFONT,
            unsafe { GetStockObject(DEFAULT_GUI_FONT) } as usize,
            1,
        );
        unsafe {
            SetWindowSubclass(child, Some(keys), 1, 0);
        }
    }
    send(
        control(hwnd, EDIT),
        EM_SETCUEBANNER,
        0,
        wide("搜索全部文件 · 支持 ext:pdf、path: 等语法").as_ptr() as isize,
    );
    send(control(hwnd, EDIT), EM_SETLIMITTEXT, 16_384, 0);
    for label in ["全部", "文件", "文件夹"] {
        send(
            control(hwnd, FILTER),
            CB_ADDSTRING,
            0,
            wide(label).as_ptr() as isize,
        );
    }
    send(control(hwnd, FILTER), CB_SETCURSEL, 0, 0);
    send(
        control(hwnd, LIST),
        LVM_SETEXTENDEDLISTVIEWSTYLE,
        0,
        (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP) as isize,
    );
    for (i, name) in ["文件名", "所在路径", "类型", "修改时间"]
        .iter()
        .enumerate()
    {
        let mut name = wide(name);
        let column = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH,
            cx: 150,
            pszText: name.as_mut_ptr(),
            ..Default::default()
        };
        send(
            control(hwnd, LIST),
            LVM_INSERTCOLUMNW,
            i,
            (&raw const column) as isize,
        );
    }
    let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            (rect.x * scale) as i32,
            (rect.y * scale) as i32,
            (rect.width * scale) as i32,
            (rect.height * scale) as i32,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        SetTimer(hwnd, POLL, 50, None);
        PostMessageW(hwnd, WM_COMMAND, REFRESH, 0);
    }
    appearance.borrow_mut().apply(hwnd);
    unsafe {
        SetWindowSubclass(
            send(control(hwnd, LIST), LVM_GETHEADER, 0, 0) as HWND,
            Some(header_paint),
            2,
            Rc::as_ptr(&appearance) as usize,
        );
        RedrawWindow(
            hwnd,
            std::ptr::null(),
            std::ptr::null_mut(),
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
        );
    }
    layout(hwnd);
    Ok(window)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_group_boolean_expressions() {
        assert_eq!(filtered(" a | b ", 1), "file: <a | b>");
        assert_eq!(filtered("", 2), "folder:");
        assert_eq!(filtered("ext:pdf", 0), "ext:pdf");
    }
}
