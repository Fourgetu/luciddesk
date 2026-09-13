//! Compact search pane using the same composition backdrop as icon panes.
#![allow(clippy::wildcard_imports, clippy::too_many_lines)]
mod everything;
pub(super) mod everything_settings;
pub(super) mod hotkey;

use super::{Event, GroupModel};
use everything::{Entry, Page};
use desktop_core::{RectDip, ShellIdentity};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{
        Controls::*,
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::*,
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
const OPEN: usize = 110;
const LOCATION: usize = 111;
const COPY: usize = 112;
const CUT: usize = 113;
const DELETE: usize = 114;
const PEEK: usize = 115;
const REFRESH: usize = 107;
pub(super) const INPUT: u32 = WM_APP + 119;
const NAVIGATE: u32 = WM_APP + 120;
pub(super) const FOCUS_INPUT: u32 = WM_APP + 130;
pub(super) const RESTORE_LAYOUT: u32 = WM_APP + 129;
pub(super) const CLEAR_SELECTION: u32 = WM_APP + 118;
const POLL: usize = 38;
const TOP: f32 = 56.0;
const ROW: f32 = 48.0;
const VISIBLE: usize = 8;
const EDIT_PROPERTY: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.SearchInput");
#[cfg(test)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn edit(hwnd: HWND) -> HWND {
    unsafe { GetPropW(hwnd, EDIT_PROPERTY) }
}
fn text(hwnd: HWND) -> String {
    let mut buf = vec![0; unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize + 1];
    let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}
fn invalidate(hwnd: HWND) {
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}
fn scale(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0
}
fn identity(path: std::path::PathBuf) -> ShellIdentity {
    ShellIdentity::FileSystem {
        path,
        volume_id: None,
        file_id: None,
    }
}

struct Request {
    generation: u64,
    query: String,
    offset: u32,
}
struct Search {
    wake: super::wake::Wake,
    sender: mpsc::Sender<Request>,
    receiver: mpsc::Receiver<(u64, Result<Page, String>)>,
    generation: u64,
    query: String,
    due: Option<Instant>,
    busy: bool,
    total: u32,
    entries: Vec<Entry>,
    selection: BTreeSet<usize>,
    focused: Option<usize>,
    anchor: Option<usize>,
    scroll: usize,
    visible_rows: usize,
    status: Option<String>,
}
impl Search {
    fn new() -> Self {
        let wake = super::wake::Wake::default();
        let ready = wake.clone();
        let (sender, requests) = mpsc::channel::<Request>();
        let (responses, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(mut request) = requests.recv() {
                for next in requests.try_iter() {
                    request = next;
                }
                let result = everything::query(&request.query, request.offset);
                if responses.send((request.generation, result)).is_err() {
                    break;
                }
                ready.notify();
            }
        });
        Self {
            wake,
            sender,
            receiver,
            generation: 0,
            query: String::new(),
            due: None,
            busy: false,
            total: 0,
            entries: Vec::new(),
            selection: BTreeSet::new(),
            focused: None,
            anchor: None,
            scroll: 0,
            visible_rows: VISIBLE,
            status: None,
        }
    }
    fn change(&mut self, value: String) {
        self.wake.notify();
        self.generation += 1;
        self.query = value.trim().into();
        self.entries.clear();
        self.selection.clear();
        self.focused = None;
        self.anchor = None;
        self.scroll = 0;
        self.total = 0;
        self.busy = !self.query.is_empty();
        self.status = self.busy.then(|| "正在搜索…".into());
        self.due = self
            .busy
            .then(|| Instant::now() + Duration::from_millis(250));
    }
    fn request(&mut self, offset: u32) {
        if self
            .sender
            .send(Request {
                generation: self.generation,
                query: self.query.clone(),
                offset,
            })
            .is_err()
        {
            self.busy = false;
            self.status = Some("搜索线程已停止，请重新打开面板。".into());
        } else {
            self.busy = true;
        }
    }
    fn accept(&mut self, generation: u64, result: Result<Page, String>) -> bool {
        if generation != self.generation || self.query.is_empty() {
            return false;
        }
        self.busy = false;
        match result {
            Ok(page) => {
                self.total = page.total;
                if page.offset == 0 {
                    self.entries = page.entries;
                } else {
                    self.entries.extend(page.entries);
                }
                self.status = if self.entries.is_empty() {
                    Some("没有找到匹配的文件".into())
                } else {
                    None
                };
            }
            Err(error) => self.status = Some(error),
        }
        true
    }
    fn tick(&mut self) -> bool {
        let mut changed = false;
        if self.due.is_some_and(|t| Instant::now() >= t) {
            self.due = None;
            self.request(0);
            changed = true;
        }
        while let Ok((generation, result)) = self.receiver.try_recv() {
            changed |= self.accept(generation, result);
        }
        changed
    }
    fn more(&mut self) {
        if !self.busy
            && self.status.is_none()
            && self.entries.len() < self.total as usize
            && self.scroll + self.visible_rows >= self.entries.len()
        {
            self.request(self.entries.len() as u32);
        }
    }
    fn height(&self) -> f32 {
        if self.query.is_empty() {
            TOP
        } else if self.entries.is_empty() {
            TOP + 64.0
        } else {
            TOP + ROW * self.entries.len().min(self.visible_rows) as f32 + 12.0
        }
    }
    fn select(&mut self, index: usize, ctrl: bool, shift: bool) {
        if index >= self.entries.len() {
            return;
        }
        if shift {
            let anchor = self.anchor.unwrap_or(index);
            if !ctrl {
                self.selection.clear();
            }
            self.selection.extend(anchor.min(index)..=anchor.max(index));
        } else {
            if ctrl {
                if !self.selection.remove(&index) {
                    self.selection.insert(index);
                }
            } else {
                self.selection.clear();
                self.selection.insert(index);
            }
            self.anchor = Some(index);
        }
        self.focused = Some(index);
        if index < self.scroll {
            self.scroll = index;
        } else if index >= self.scroll + self.visible_rows {
            self.scroll = index + 1 - self.visible_rows;
        }
    }
    fn selected(&self) -> Vec<ShellIdentity> {
        self.selection
            .iter()
            .filter_map(|i| self.entries.get(*i))
            .map(|e| identity(e.path.clone()))
            .collect()
    }
    fn move_focus(&mut self, index: usize, ctrl: bool, shift: bool) {
        let selected = self.selection.clone();
        self.select(index, ctrl && shift, shift);
        if ctrl && !shift {
            self.selection = selected;
        }
    }
}

// A color-keyed EDIT exposes the pane backdrop while retaining native IME,
// caret and selection. The owner forwards clicks through transparent pixels.
struct Editor {
    hwnd: HWND,
    font: HFONT,
    dark: bool,
    dpi: u32,
    line_height: i32,
}
impl Editor {
    fn new(owner: HWND) -> Result<Self, String> {
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                windows_sys::w!("EDIT"),
                std::ptr::null(),
                WS_POPUP | ES_AUTOHSCROLL as u32,
                0,
                0,
                1,
                1,
                owner,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err("无法创建搜索框".into());
        }
        unsafe {
            SetPropW(owner, EDIT_PROPERTY, hwnd);
            SetWindowSubclass(owner, Some(input_color_proc), 1, 0);
            SetWindowSubclass(hwnd, Some(input_proc), 1, 0);
            SendMessageW(hwnd, EM_SETLIMITTEXT, 16384, 0);
        }
        Ok(Self {
            hwnd,
            font: std::ptr::null_mut(),
            dark: false,
            dpi: 0,
            line_height: 24,
        })
    }
    fn appearance(&mut self, owner: HWND, dark: bool) {
        let dpi = unsafe { GetDpiForWindow(owner) }.max(96);
        if self.dpi == dpi && self.dark == dark {
            return;
        }
        let font = unsafe {
            CreateFontW(
                -((14 * dpi / 96) as i32),
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                ANTIALIASED_QUALITY as u32,
                0,
                windows_sys::w!("Microsoft YaHei UI"),
            )
        };
        if !font.is_null() {
            unsafe {
                SendMessageW(self.hwnd, WM_SETFONT, font as usize, 0);
                DeleteObject(self.font);
            }
            self.font = font;
        }
        unsafe {
            let background = if dark { 0x202020 } else { 0xf5f5f5 };
            SetLayeredWindowAttributes(self.hwnd, background, 255, LWA_COLORKEY);
        }
        self.dark = dark;
        self.dpi = dpi;
        unsafe {
            let dc = GetDC(self.hwnd);
            let old = SelectObject(dc, self.font);
            let mut metrics = TEXTMETRICW::default();
            if GetTextMetricsW(dc, &raw mut metrics) != 0 {
                self.line_height = metrics.tmHeight.max(1);
            }
            SelectObject(dc, old);
            ReleaseDC(self.hwnd, dc);
        }
        self.position(owner);
        invalidate(self.hwnd);
    }
    fn position(&self, owner: HWND) {
        position_editor(owner, self.hwnd, self.line_height);
    }
}
fn position_editor(owner: HWND, editor: HWND, line_height: i32) {
    let s = scale(owner);
    let mut r = RECT::default();
    let mut p = POINT {
        x: (44.0 * s) as i32,
        y: (((TOP * s).round() as i32 - line_height) / 2).max(0),
    };
    unsafe {
        GetClientRect(owner, &raw mut r);
        ClientToScreen(owner, &raw mut p);
        let mut before = RECT::default();
        GetWindowRect(editor, &raw mut before);
        let width = (r.right - (62.0 * s) as i32).max(1);
        let height = line_height;
        if before.left != p.x
            || before.top != p.y
            || before.right - before.left != width
            || before.bottom - before.top != height
        {
            SetWindowPos(
                editor,
                std::ptr::null_mut(),
                p.x,
                p.y,
                width,
                height,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
    }
}
impl Drop for Editor {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
            DeleteObject(self.font);
        }
    }
}
// Color requests can reenter the owner while its application callback is busy
// forwarding focus/mouse messages. Handle them before that callback, without
// borrowing its state, so EDIT never falls back to the system white brush.
unsafe extern "system" fn input_color_proc(
    owner: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    _: usize,
) -> LRESULT {
    let editor = edit(owner);
    if msg == WM_MOUSEACTIVATE && !editor.is_null() && unsafe { GetFocus() } == editor {
        return MA_NOACTIVATE as isize;
    }
    if matches!(msg, WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC)
        && !editor.is_null()
        && lp == editor as isize
    {
        return unsafe { input_background(editor, wp as HDC) } as isize;
    }
    unsafe { DefSubclassProc(owner, msg, wp, lp) }
}

unsafe fn input_background(hwnd: HWND, dc: HDC) -> HBRUSH {
    let mut key = 0x202020;
    unsafe {
        GetLayeredWindowAttributes(
            hwnd,
            &raw mut key,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        SetTextColor(dc, if key == 0x202020 { 0xf0f0f0 } else { 0x202020 });
        SetBkColor(dc, key);
        SetDCBrushColor(dc, key);
        GetStockObject(DC_BRUSH) as HBRUSH
    }
}

unsafe extern "system" fn input_proc(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    _: usize,
) -> LRESULT {
    let owner = unsafe { GetParent(hwnd) };
    if msg == WM_MOUSEACTIVATE && unsafe { GetFocus() } == hwnd {
        return MA_NOACTIVATE as isize;
    }
    if msg == WM_SETCURSOR {
        unsafe {
            SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_IBEAM));
        }
        return 1;
    }
    if msg == WM_ERASEBKGND {
        unsafe {
            let mut rect = RECT::default();
            GetClientRect(hwnd, &raw mut rect);
            let dc = wp as HDC;
            FillRect(dc, &rect, input_background(hwnd, dc));
        }
        return 1;
    }
    if msg == WM_MOUSEWHEEL {
        unsafe {
            PostMessageW(owner, msg, wp, lp);
        }
        return 0;
    }
    const COMPOSING: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.SearchComposing");
    if msg == WM_IME_STARTCOMPOSITION {
        unsafe {
            SetPropW(hwnd, COMPOSING, 1usize as _);
        }
    }
    if msg == WM_IME_ENDCOMPOSITION {
        unsafe {
            RemovePropW(hwnd, COMPOSING);
        }
    }
    if msg == WM_KEYDOWN && unsafe { GetPropW(hwnd, COMPOSING) }.is_null() {
        let ctrl = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
        match wp as u16 {
            VK_ESCAPE => {
                unsafe {
                    SetWindowTextW(hwnd, windows_sys::w!(""));
                }
                return 0;
            }
            VK_DOWN => {
                unsafe {
                    SetFocus(owner);
                    PostMessageW(owner, NAVIGATE, VK_DOWN as usize, 0);
                }
                return 0;
            }
            VK_RETURN | VK_F5 => {
                unsafe {
                    PostMessageW(owner, WM_COMMAND, REFRESH, 0);
                }
                return 0;
            }
            0x41 | 0x4c if ctrl => {
                unsafe {
                    SendMessageW(hwnd, EM_SETSEL, 0, -1);
                }
                return 0;
            }
            _ => {}
        }
    }
    let result = unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
    if matches!(
        msg,
        WM_CHAR | WM_PASTE | WM_CUT | WM_CLEAR | WM_UNDO | WM_SETTEXT | WM_IME_ENDCOMPOSITION
    ) {
        unsafe {
            PostMessageW(owner, INPUT, 0, 0);
        }
    }
    result
}

struct Drawing {
    surface: super::composition::Surface,
    name: windows_canvas::TextFormat,
    path: windows_canvas::TextFormat,
    icon: windows_canvas::TextFormat,
    placeholder: windows_canvas::TextFormat,
}
impl Drawing {
    fn new(hwnd: HWND) -> Result<Self, String> {
        use windows_canvas::{ParagraphAlignment, TextFormat, WordWrapping};
        let name = TextFormat::new("Microsoft YaHei UI", 13.0)
            .map_err(|e| e.to_string())?
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .with_word_wrapping(WordWrapping::NoWrap);
        let path = TextFormat::new("Microsoft YaHei UI", 11.0)
            .map_err(|e| e.to_string())?
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .with_word_wrapping(WordWrapping::NoWrap);
        super::canvas::ellipsis(&name).map_err(|e| e.to_string())?;
        super::canvas::ellipsis(&path).map_err(|e| e.to_string())?;
        Ok(Self {
            surface: super::composition::Surface::new_pane(windows::Win32::Foundation::HWND(hwnd))
                .map_err(|e| e.to_string())?,
            name,
            path,
            placeholder: TextFormat::new("Microsoft YaHei UI", 14.0)
                .map_err(|e| e.to_string())?
                .with_paragraph_alignment(ParagraphAlignment::Center)
                .with_word_wrapping(WordWrapping::NoWrap),
            icon: TextFormat::new("Segoe Fluent Icons", 20.0)
                .map_err(|e| e.to_string())?
                .with_alignment(windows_canvas::TextAlignment::Center)
                .with_paragraph_alignment(ParagraphAlignment::Center)
                .with_word_wrapping(WordWrapping::NoWrap),
        })
    }
    fn paint(&mut self, hwnd: HWND, model: &GroupModel, state: &Search) -> Result<(), String> {
        use super::native_graphics::canvas_result;
        use windows_canvas::{ColorF, Rect, RoundedRect};
        let s = scale(hwnd);
        let mut r = RECT::default();
        unsafe {
            GetClientRect(hwnd, &raw mut r);
        }
        self.surface
            .theme(windows::Win32::Foundation::HWND(hwnd), model.dark);
        self.surface
            .material(windows::Win32::Foundation::HWND(hwnd), model.backdrop);
        self.surface.pane_corner_radius = model.options.corner_radius;
        let Some(target) = self
            .surface
            .try_begin_frame(r.right.max(1) as u32, r.bottom.max(1) as u32)
            .map_err(|e| e.to_string())?
        else {
            return Ok(());
        };
        let native = self.surface.native;
        let w = r.right as f32 / s;
        let h = r.bottom as f32 / s;
        super::canvas::draw(&target, s, |target| {
            target.clear(ColorF::new(0.0, 0.0, 0.0, 0.0));
            let contrast = super::theme::panel_contrast(
                model.backdrop,
                model.dark,
                model.options.text,
                native,
            );
            let ink = contrast.ink();
            let base = contrast.base();
            let background = canvas_result(target.create_solid_brush(ColorF::new(
                base,
                base,
                base,
                if !native {
                    1.0
                } else if model.options.text_protection {
                    contrast.scrim
                } else {
                    0.0
                },
            )))?;
            let text = canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 1.0)))?;
            let dim = canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 0.85)))?;
            let line = canvas_result(target.create_solid_brush(ColorF::new(ink, ink, ink, 0.16)))?;
            let outline = canvas_result(
                target.create_solid_brush(super::theme::panel_border(model.dark, model.backdrop)),
            )?;
            let selected =
                canvas_result(target.create_solid_brush(ColorF::new(0.75, 0.8, 0.85, 0.17)))?;
            let shape = RoundedRect {
                rect: Rect::from_xywh(0.5, 0.5, w - 1.0, h - 1.0),
                radius_x: model.options.corner_radius,
                radius_y: model.options.corner_radius,
            };
            target.fill_rounded_rect(&shape, &background);
            if model.options.border {
                target.draw_rounded_rect(&shape, &outline, 1.0);
            }
            target.clipped_text(
                "\u{e721}",
                &self.icon,
                &Rect::from_xywh(12.0, 0.0, 28.0, TOP),
                &dim,
            );
            if unsafe { GetWindowTextLengthW(edit(hwnd)) } == 0 {
                target.clipped_text(
                    "搜索文件…",
                    &self.placeholder,
                    &Rect::from_xywh(44.0, 0.0, (w - 62.0).max(0.0), TOP),
                    &dim,
                );
            }
            if !state.query.is_empty() {
                target.fill_rect(&Rect::from_xywh(12.0, TOP, w - 24.0, 1.0), &line);
                if state.entries.is_empty() {
                    target.clipped_text(
                        state.status.as_deref().unwrap_or("正在搜索…"),
                        &self.path,
                        &Rect::from_xywh(18.0, TOP + 6.0, w - 36.0, 52.0),
                        &dim,
                    );
                }
                for (row, entry) in state
                    .entries
                    .iter()
                    .enumerate()
                    .skip(state.scroll)
                    .take(state.visible_rows)
                {
                    let y = TOP + (row - state.scroll) as f32 * ROW + 4.0;
                    if state.selection.contains(&row) {
                        target.fill_rounded_rect(
                            &RoundedRect {
                                rect: Rect::from_xywh(6.0, y, w - 12.0, ROW - 2.0),
                                radius_x: 4.0,
                                radius_y: 4.0,
                            },
                            &selected,
                        );
                    }
                    let name = entry
                        .path
                        .file_name()
                        .unwrap_or(entry.path.as_os_str())
                        .to_string_lossy();
                    let parent = entry.path.parent().unwrap_or(&entry.path).to_string_lossy();
                    let path = parent;
                    target.clipped_text(
                        if entry.folder { "\u{e8b7}" } else { "\u{e8a5}" },
                        &self.icon,
                        &Rect::from_xywh(16.0, y, 20.0, ROW - 2.0),
                        &dim,
                    );
                    target.clipped_text(
                        &name,
                        &self.name,
                        &Rect::from_xywh(46.0, y + 2.0, (w - 64.0).max(0.0), 22.0),
                        &text,
                    );
                    target.clipped_text(
                        &path,
                        &self.path,
                        &Rect::from_xywh(46.0, y + 24.0, (w - 64.0).max(0.0), 18.0),
                        &dim,
                    );
                }
                if state.entries.len() > state.visible_rows {
                    let track = h - TOP - 10.0;
                    let thumb =
                        (track * state.visible_rows as f32 / state.entries.len() as f32).max(12.0);
                    let y = TOP
                        + 5.0
                        + (track - thumb) * state.scroll as f32
                            / (state.entries.len() - state.visible_rows) as f32;
                    target.fill_rounded_rect(
                        &RoundedRect {
                            rect: Rect::from_xywh(w - 5.0, y, 2.0, thumb),
                            radius_x: 1.0,
                            radius_y: 1.0,
                        },
                        &dim,
                    );
                }
            }
            target.finish()
        })
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn fit_search(work: RECT, current: RECT, scale: f32, state: &mut Search) -> RECT {
    let previous_rows = state.visible_rows;
    state.visible_rows = (((work.bottom - work.top) as f32 / scale - TOP - 12.0) / ROW)
        .floor()
        .clamp(1.0, VISIBLE as f32) as usize;
    state.scroll = state
        .scroll
        .min(state.entries.len().saturating_sub(state.visible_rows));
    if let Some(focused) = state
        .focused
        .filter(|_| previous_rows != state.visible_rows)
    {
        if focused < state.scroll {
            state.scroll = focused;
        }
        if focused >= state.scroll + state.visible_rows {
            state.scroll = focused + 1 - state.visible_rows;
        }
    }
    let height = ((state.height() * scale).round() as i32)
        .min(work.bottom - work.top)
        .max(1);
    let width = (current.right - current.left)
        .min(work.right - work.left)
        .max(1);
    let left = current
        .left
        .clamp(work.left, (work.right - width).max(work.left));
    let top = current
        .top
        .clamp(work.top, (work.bottom - height).max(work.top));
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

fn resize(hwnd: HWND, state: &mut Search) {
    unsafe {
        let mut current = RECT::default();
        GetWindowRect(hwnd, &raw mut current);
        let mut monitor = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let work = if GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &raw mut monitor,
        ) != 0
        {
            monitor.rcWork
        } else {
            current
        };
        let bounds = fit_search(work, current, scale(hwnd), state);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    // SetWindowPos can reenter a busy owner callback, so do not depend on
    // WM_WINDOWPOSCHANGED to move the owned native editor with the pane.
    let editor = edit(hwnd);
    if !editor.is_null() {
        let mut bounds = RECT::default();
        unsafe {
            GetWindowRect(editor, &raw mut bounds);
        }
        position_editor(hwnd, editor, bounds.bottom - bounds.top);
    }
    invalidate(hwnd);
}

fn search_frame_hit(bounds: RECT, point: POINT, scale: f32, locked: bool) -> u32 {
    let x = point.x as f32 / scale;
    let y = point.y as f32 / scale;
    let width = bounds.right as f32 / scale;
    let edge = 5.0_f32.min(width / 2.0);
    if x < edge {
        HTLEFT
    } else if x >= width - edge {
        HTRIGHT
    } else if !locked && y < TOP && (x < 40.0 || x > width - 12.0 || y < 5.0 || y >= TOP - 5.0) {
        HTCAPTION
    } else {
        HTCLIENT
    }
}

pub(super) fn create(
    rect: RectDip,
    model: Rc<RefCell<GroupModel>>,
    event: impl FnMut(Event) -> bool + 'static,
) -> Result<windows_window::Window, String> {
    let initial_dark = {
        let m = model.borrow();
        super::theme::panel_contrast(m.backdrop, m.dark, m.options.text, true).light_text
    };
    let event = Rc::new(RefCell::new(event));
    let callback = Rc::clone(&event);
    let editor: Rc<RefCell<Option<Editor>>> = Rc::new(RefCell::new(None));
    let input = Rc::clone(&editor);
    let mut drawing: Option<Drawing> = None;
    let mut state = Search::new();
    let wake = state.wake.clone();
    let mut last_click: Option<(usize, Instant)> = None;
    let mut move_origin: Option<super::snap::DragOrigin> = None;
    let window = windows_window::Window::new("Everything 搜索")
        .style(WS_POPUP | WS_THICKFRAME)
        .ex_style(WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP)
        .size(rect.width.max(1.0) as i32, TOP as i32)
        .on_message(move |raw, msg, wp, lp| {
            let hwnd = raw.cast();
            match msg {
                WM_NCCALCSIZE | WM_ERASEBKGND => return Some(0),
                WM_DESTROY => {
                    state.wake.unbind();
                    unsafe {
                        KillTimer(hwnd, POLL);
                    }
                    return Some(0);
                }
                WM_CLOSE => {
                    let event = Rc::clone(&callback);
                    super::window::defer_action(move || {
                        (event.borrow_mut())(Event::ClosePane);
                    });
                    return Some(0);
                }
                WM_NCHITTEST => {
                    let mut p = POINT {
                        x: lp as u16 as i16 as i32,
                        y: (lp >> 16) as u16 as i16 as i32,
                    };
                    let mut r = RECT::default();
                    unsafe {
                        ScreenToClient(hwnd, &raw mut p);
                        GetClientRect(hwnd, &raw mut r);
                    }
                    return Some(
                        search_frame_hit(r, p, scale(hwnd), model.borrow().locked) as isize
                    );
                }
                WM_SETCURSOR if lp as u16 as u32 == HTCLIENT => {
                    let mut point = POINT::default();
                    unsafe {
                        GetCursorPos(&raw mut point);
                        ScreenToClient(hwnd, &raw mut point);
                        SetCursor(LoadCursorW(
                            std::ptr::null_mut(),
                            if point.y as f32 / scale(hwnd) < TOP {
                                IDC_IBEAM
                            } else {
                                IDC_ARROW
                            },
                        ));
                    }
                    return Some(1);
                }
                WM_GETMINMAXINFO if lp != 0 => {
                    let info = unsafe { &mut *(lp as *mut MINMAXINFO) };
                    info.ptMinTrackSize.x = 1;
                    info.ptMinTrackSize.y = (TOP * scale(hwnd)).round() as i32;
                    return Some(0);
                }
                WM_ENTERSIZEMOVE => {
                    let mut bounds = RECT::default();
                    let mut pointer = POINT::default();
                    move_origin = if unsafe { GetWindowRect(hwnd, &raw mut bounds) } != 0
                        && unsafe { GetCursorPos(&raw mut pointer) } != 0
                    {
                        Some(super::snap::DragOrigin::new(bounds, pointer))
                    } else {
                        None
                    };
                    return Some(0);
                }
                WM_MOVING if lp != 0 => {
                    if let Some(origin) = &move_origin {
                        let mut pointer = POINT::default();
                        if unsafe { GetCursorPos(&raw mut pointer) } != 0 {
                            unsafe {
                                *(lp as *mut RECT) = origin.proposal(pointer);
                            }
                        }
                    }
                    (callback.borrow_mut())(Event::Moving(lp as *mut RECT));
                    return Some(1);
                }
                WM_SIZING if lp != 0 => {
                    let bounds = unsafe { &mut *(lp as *mut RECT) };
                    let proposal = *bounds;
                    (callback.borrow_mut())(Event::Sizing(bounds, proposal, wp as u32));
                    return Some(1);
                }
                WM_WINDOWPOSCHANGED | WM_SIZE => {
                    if let Some(input) = input.borrow().as_ref() {
                        input.position(hwnd);
                    }
                    if msg == WM_SIZE {
                        invalidate(hwnd);
                    }
                }
                WM_SHOWWINDOW => {
                    if !edit(hwnd).is_null() {
                        unsafe {
                            ShowWindow(
                                edit(hwnd),
                                if wp == 0 { SW_HIDE } else { SW_SHOWNOACTIVATE },
                            );
                        }
                    }
                }
                FOCUS_INPUT => {
                    let editor = edit(hwnd);
                    if !editor.is_null() {
                        unsafe {
                            SetFocus(editor);
                            SendMessageW(editor, EM_SETSEL, 0, -1);
                        }
                    }
                    return Some(0);
                }
                RESTORE_LAYOUT => {
                    resize(hwnd, &mut state);
                    invalidate(hwnd);
                    return Some(0);
                }
                WM_EXITSIZEMOVE => {
                    move_origin = None;
                    resize(hwnd, &mut state);
                    let mut r = RECT::default();
                    unsafe {
                        GetWindowRect(hwnd, &raw mut r);
                    }
                    let s = scale(hwnd);
                    (callback.borrow_mut())(Event::Geometry(RectDip::new(
                        r.left as f32 / s,
                        r.top as f32 / s,
                        (r.right - r.left) as f32 / s,
                        rect.height,
                    )));
                }
                WM_DPICHANGED => {
                    if let Some(input) = input.borrow_mut().as_mut() {
                        let m = model.borrow();
                        input.appearance(
                            hwnd,
                            super::theme::panel_contrast(m.backdrop, m.dark, m.options.text, true)
                                .light_text,
                        );
                    }
                    if lp != 0 {
                        let r = unsafe { &*(lp as *const RECT) };
                        unsafe {
                            SetWindowPos(
                                hwnd,
                                std::ptr::null_mut(),
                                r.left,
                                r.top,
                                r.right - r.left,
                                (state.height() * scale(hwnd)) as i32,
                                SWP_NOZORDER | SWP_NOACTIVATE,
                            );
                        }
                    }
                    resize(hwnd, &mut state);
                }
                INPUT => {
                    let value = text(edit(hwnd));
                    invalidate(hwnd);
                    if value.trim() != state.query {
                        state.change(value);
                        resize(hwnd, &mut state);
                    }
                    return Some(0);
                }
                WM_COMMAND if (wp >> 16) as u32 == EN_CHANGE => {
                    unsafe {
                        PostMessageW(hwnd, INPUT, 0, 0);
                    }
                    return Some(0);
                }
                message if message == super::wake::READY || (message == WM_TIMER && wp == POLL) => {
                    state.wake.received();
                    unsafe {
                        KillTimer(hwnd, POLL);
                    }
                    if state.tick() {
                        resize(hwnd, &mut state);
                    }
                    if let Some(due) = state.due {
                        let delay = due
                            .saturating_duration_since(Instant::now())
                            .as_millis()
                            .clamp(1, 1000) as u32;
                        unsafe {
                            SetTimer(hwnd, POLL, delay, None);
                        }
                    }
                    if let Some(input) = input.borrow_mut().as_mut() {
                        let m = model.borrow();
                        input.appearance(
                            hwnd,
                            super::theme::panel_contrast(m.backdrop, m.dark, m.options.text, true)
                                .light_text,
                        );
                    }
                    return Some(0);
                }
                CLEAR_SELECTION => {
                    state.selection.clear();
                    state.focused = None;
                    invalidate(hwnd);
                    return Some(0);
                }
                WM_MOUSEWHEEL => {
                    let delta = (wp >> 16) as u16 as i16;
                    if delta > 0 {
                        state.scroll = state.scroll.saturating_sub(3);
                    } else {
                        state.scroll = (state.scroll + 3)
                            .min(state.entries.len().saturating_sub(state.visible_rows));
                        state.more();
                    }
                    invalidate(hwnd);
                    return Some(0);
                }
                WM_RBUTTONDOWN => {
                    let y = (lp >> 16) as u16 as i16 as f32 / scale(hwnd);
                    if y >= TOP {
                        let row = state.scroll + ((y - TOP) / ROW) as usize;
                        if row < state.entries.len() {
                            unsafe {
                                SetFocus(hwnd);
                            }
                            if !state.selection.contains(&row) {
                                state.select(row, false, false);
                            }
                            (callback.borrow_mut())(Event::PaneItemFocus);
                            invalidate(hwnd);
                        }
                    }
                    return Some(0);
                }
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                    let y = (lp >> 16) as u16 as i16 as f32 / scale(hwnd);
                    if y < TOP {
                        let editor = edit(hwnd);
                        let mut point = POINT {
                            x: lp as u16 as i16 as i32,
                            y: (lp >> 16) as u16 as i16 as i32,
                        };
                        unsafe {
                            MapWindowPoints(hwnd, editor, &raw mut point, 1);
                            if GetFocus() != editor {
                                SetFocus(editor);
                            }
                            let position =
                                (point.x as u16 as usize) | ((point.y as u16 as usize) << 16);
                            SendMessageW(editor, msg, wp, position as isize);
                        }
                    } else {
                        let row = state.scroll + ((y - TOP) / ROW) as usize;
                        if row < state.entries.len() {
                            unsafe {
                                SetFocus(hwnd);
                            }
                            let ctrl = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
                            let shift = unsafe { GetKeyState(VK_SHIFT as i32) } < 0;
                            state.select(row, ctrl, shift);
                            (callback.borrow_mut())(Event::PaneItemFocus);
                            if !ctrl
                                && !shift
                                && last_click.is_some_and(|(old, time)| {
                                    old == row
                                        && time.elapsed()
                                            < Duration::from_millis(u64::from(unsafe {
                                                GetDoubleClickTime()
                                            }))
                                })
                            {
                                action(hwnd, OPEN, state.selected());
                                last_click = None;
                            } else {
                                last_click = Some((row, Instant::now()));
                            }
                        }
                    }
                    invalidate(hwnd);
                    return Some(0);
                }
                NAVIGATE | WM_KEYDOWN => {
                    let mods = super::keyboard::Modifiers::current();
                    let key = wp as u16;
                    if mods.ctrl && key == 0x4c {
                        unsafe {
                            SetFocus(edit(hwnd));
                            SendMessageW(edit(hwnd), EM_SETSEL, 0, -1);
                        }
                        return Some(0);
                    }
                    if key == VK_ESCAPE {
                        unsafe {
                            SetWindowTextW(edit(hwnd), windows_sys::w!(""));
                            SetFocus(edit(hwnd));
                        }
                        return Some(0);
                    }
                    if mods.ctrl && key == 0x41 {
                        state.selection.extend(0..state.entries.len());
                        (callback.borrow_mut())(Event::PaneItemFocus);
                        invalidate(hwnd);
                        return Some(0);
                    }
                    if matches!(key, VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT) {
                        if !state.entries.is_empty() {
                            let current = state.focused.unwrap_or(0);
                            let index = match key {
                                VK_UP => current.saturating_sub(1),
                                VK_DOWN => {
                                    if state.focused.is_none() {
                                        0
                                    } else {
                                        current + 1
                                    }
                                }
                                VK_HOME => 0,
                                VK_END => state.entries.len() - 1,
                                VK_PRIOR => current.saturating_sub(state.visible_rows),
                                _ => current + state.visible_rows,
                            }
                            .min(state.entries.len() - 1);
                            state.move_focus(index, mods.ctrl, mods.shift);
                            state.more();
                            (callback.borrow_mut())(Event::PaneItemFocus);
                            invalidate(hwnd);
                        }
                        return Some(0);
                    }
                    let command = if super::peek::matches(key, &mods, lp & (1 << 30) != 0) {
                        Some(PEEK)
                    } else {
                        match key {
                            VK_RETURN => Some(if mods.ctrl { LOCATION } else { OPEN }),
                            VK_F5 => Some(REFRESH),
                            VK_DELETE if !mods.shift && !mods.ctrl && !mods.alt => Some(DELETE),
                            0x43 if mods.ctrl => Some(COPY),
                            0x58 if mods.ctrl => Some(CUT),
                            _ => None,
                        }
                    };
                    if let Some(command) = command {
                        if lp & (1 << 30) == 0 {
                            unsafe {
                                PostMessageW(hwnd, WM_COMMAND, command, 0);
                            }
                        }
                        return Some(0);
                    }
                }
                WM_COMMAND => {
                    if wp == REFRESH {
                        state.change(text(edit(hwnd)));
                        resize(hwnd, &mut state);
                    } else {
                        action(hwnd, wp, state.selected());
                    }
                    return Some(0);
                }
                WM_CONTEXTMENU => {
                    let items = state.selected();
                    let (theme, backdrop) = {
                        let m = model.borrow();
                        (m.theme, m.backdrop)
                    };
                    let event = Rc::clone(&callback);
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
                        let mut rows = Vec::new();
                        if !items.is_empty() {
                            for (id, label, shortcut) in [
                                (OPEN, "打开", "Enter"),
                                (LOCATION, "打开文件位置", ""),
                                (COPY, "复制", "Ctrl+C"),
                                (CUT, "剪切", "Ctrl+X"),
                                (DELETE, "删除", "Del"),
                                (PEEK, "预览", ""),
                            ] {
                                rows.push(super::menu::entry(id as i32, label, "", shortcut));
                            }
                            rows.push(super::menu::entry(0, "", "", ""));
                        }
                        rows.push(super::menu::entry(201, "设置", "", ""));
                        rows.push(super::menu::entry(202, "关闭搜索面板", "", ""));
                        let command =
                            super::menu::show_entries(hwnd, point, false, theme, backdrop, rows);
                        match command {
                            201 => {
                                (event.borrow_mut())(Event::Settings);
                            }
                            202 => unsafe {
                                PostMessageW(hwnd, WM_CLOSE, 0, 0);
                            },
                            _ => action(hwnd, command as usize, items),
                        }
                    });
                    return Some(0);
                }
                WM_PAINT => {
                    if let Some(input) = input.borrow_mut().as_mut() {
                        let m = model.borrow();
                        input.appearance(
                            hwnd,
                            super::theme::panel_contrast(m.backdrop, m.dark, m.options.text, true)
                                .light_text,
                        );
                    }
                    let mut ps = PAINTSTRUCT::default();
                    unsafe {
                        BeginPaint(hwnd, &raw mut ps);
                        EndPaint(hwnd, &ps);
                    }
                    if drawing.is_none() {
                        match Drawing::new(hwnd) {
                            Ok(value) => drawing = Some(value),
                            Err(error) => {
                                eprintln!("{error}");
                                return Some(0);
                            }
                        }
                    }
                    if let Some(drawing) = drawing.as_mut() {
                        if let Err(error) = drawing
                            .paint(hwnd, &model.borrow(), &state)
                            .and_then(|()| drawing.surface.end_frame().map_err(|e| e.to_string()))
                        {
                            eprintln!("{error}");
                        }
                    }
                    return Some(0);
                }
                _ => {}
            }
            None
        })
        .create()
        .map_err(|e| e.to_string())?;
    let hwnd = window.hwnd().cast();
    unsafe {
        SetWindowSubclass(hwnd, Some(super::window::borderless_proc), 1, 0);
    }
    let mut native = Editor::new(hwnd)?;
    native.appearance(hwnd, initial_dark);
    *editor.borrow_mut() = Some(native);
    let s = scale(hwnd);
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            (rect.x * s) as i32,
            (rect.y * s) as i32,
            (rect.width.max(1.0) * s) as i32,
            (TOP * s) as i32,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        ShowWindow(edit(hwnd), SW_SHOWNOACTIVATE);
    }
    if let Some(input) = editor.borrow().as_ref() {
        input.position(hwnd);
    }
    invalidate(hwnd);
    wake.bind(hwnd as isize);
    Ok(window)
}

#[cfg(test)]
mod tests;
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
