//! Modeless settings, drawn with the same native composition pipeline as panes.
#![allow(
    clippy::wildcard_imports,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
use windows_canvas::{ColorF, Ellipse, Rect, RoundedRect, Vector2};

use super::native_graphics::canvas_result;
use super::search::{everything_settings, hotkey as search_hotkey};
use super::*;
use desktop_core::{Backdrop, PanelTheme};
use windows_canvas::ID2D1DeviceContext;
use windows_sys::Win32::{
    Graphics::Gdi::*,
    UI::{HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
const SELECT_PANEL: u32 = WM_APP + 95;
const PREPARE_REVEAL: u32 = WM_APP + 96;
const REVEAL_TIMER: usize = 0x4c5055;
pub(super) const DEFAULT_HEIGHT: i32 = 600;
const MIN_HEIGHT: f32 = 560.0;

struct PendingReveal {
    started: std::time::Instant,
    ready: Box<dyn Fn() -> bool>,
    fade: bool,
}

// windows-window::create calls ShowWindow before returning the HWND. Suppress
// that first show until placement, custom frame and composition are prepared.
unsafe fn defer_initial_show(msg: u32, lp: isize, prepared: bool) -> bool {
    if msg != WM_WINDOWPOSCHANGING || prepared {
        return false;
    }
    unsafe {
        let position = &mut *(lp as *mut WINDOWPOS);
        position.flags = (position.flags & !SWP_SHOWWINDOW) | SWP_NOACTIVATE;
    }
    true
}

unsafe fn cloak(
    hwnd: windows_sys::Win32::Foundation::HWND,
    hidden: bool,
) -> windows::core::Result<()> {
    unsafe {
        super::native_graphics::set_attribute(
            windows::Win32::Foundation::HWND(hwnd),
            windows::Win32::Graphics::Dwm::DWMWA_CLOAK.0,
            &i32::from(hidden),
        )
    }
}
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;

#[derive(Clone)]
enum Action {
    FolderDefaults(folder::Defaults),
    BackupPolicy(u8),
    BackupRecord(std::path::PathBuf),
    BackupPage(isize),
    BackupAdvanced,
    BackupStatus,
    ProjectHome,
    CopyDiagnostics,
    Window(u32),
    Page(usize),
    Change(Event),
    Radius(f32),
    GridSize(f32),
    Opacity(u8),
    Strength(u8),
    StrengthReset,
    Channel(u8, u8),
    ColorPreset(u32),
    SolidColor,
    SolidReset,
    StyleInput(bool),
    PeekEnable,
    PreviewProvider(peek::Provider),
    PeekBrowse,
    PeekDetect,
    PeekShortcut,
    SearchShortcut,
    SearchReset,
    PeekReset,
    EverythingBrowse,
    EverythingDetect,
    EverythingLaunch,
}
mod controls;
use controls::{Control, ControlKind, Slider, Style};
struct Scene {
    text: Vec<(Rect, String, usize)>,
    cards: Vec<Rect>,
    separators: Vec<Rect>,
    controls: Vec<Control>,
    previews: Vec<(Rect, u32, f32)>,
    app_icon: Option<Rect>,
}

fn grid_range() -> (f32, f32) {
    desktop_core::PaneOptions::GRID_SCALE_RANGE
}

fn grid_slider_position(value: f32) -> f32 {
    let (min, max) = grid_range();
    let value = value.clamp(min, max);
    if value <= 100.0 {
        0.5 * (value - min) / (100.0 - min)
    } else {
        0.5 + 0.5 * (value - 100.0) / (max - 100.0)
    }
}

fn grid_slider_value(position: f32) -> f32 {
    let (min, max) = grid_range();
    let position = position.clamp(0.0, 1.0);
    if position <= 0.5 {
        (min + position * 2.0 * (100.0 - min)).round()
    } else {
        (100.0 + (position - 0.5) * 2.0 * (max - 100.0)).round()
    }
}

fn radius_from_pointer(bounds: Rect, x: f32) -> f32 {
    let progress = controls::slider_fraction(bounds, x);
    progress * desktop_core::PaneOptions::MAX_CORNER_RADIUS
}

fn solid_style(store: &desktop_storage::WorkspaceStore, dark: bool) -> Backdrop {
    if let Ok(Some(value)) = store.preference("solid_style") {
        if let Some((color, opacity)) = value.split_once('|') {
            if let (Ok(color), Ok(opacity)) = (color.parse::<u32>(), opacity.parse::<f32>()) {
                if color <= 0xffffff && opacity.is_finite() && (0.0..=1.0).contains(&opacity) {
                    return Backdrop::Solid { color, opacity };
                }
            }
        }
    }
    Backdrop::Solid {
        color: if dark { 0x181b20 } else { 0xf5f6f8 },
        opacity: 0.85,
    }
}

fn edited_solid(backdrop: Backdrop, percentage: bool, text: &str) -> Option<Backdrop> {
    let Backdrop::Solid {
        mut color,
        mut opacity,
    } = backdrop
    else {
        return None;
    };
    if percentage {
        let value = text.trim().trim_end_matches('%').parse::<u8>().ok()?;
        if value > 100 {
            return None;
        }
        opacity = f32::from(value) / 100.0;
    } else {
        let text = text.trim();
        let text = text.strip_prefix('#').unwrap_or(text);
        if text.len() != 6 || !text.is_ascii() {
            return None;
        }
        color = u32::from_str_radix(text, 16).ok()?;
    }
    Some(Backdrop::Solid { color, opacity })
}

fn settings_backdrop(backdrop: Backdrop, dark: bool) -> Backdrop {
    match backdrop {
        Backdrop::Solid { .. } => Backdrop::Solid {
            color: if dark { 0x202020 } else { 0xf3f3f3 },
            opacity: 1.0,
        },
        other => other.base(),
    }
}

fn material_style(store: &desktop_storage::WorkspaceStore, backdrop: Backdrop) -> Backdrop {
    backdrop
        .strength_key()
        .and_then(|key| store.preference(key).ok().flatten())
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|value| *value <= 100)
        .map_or(backdrop, |value| backdrop.with_strength(value))
}

fn color_channel(color: u32, channel: u8, value: u8) -> u32 {
    let shift = (2 - u32::from(channel.min(2))) * 8;
    (color & !(255 << shift)) | (u32::from(value) << shift)
}

fn contains(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}
mod layout;
use layout::scene;

const TOGGLE_TIMER: usize = 0x4c5054;
use super::animation::Motion as ToggleMotion;
fn toggle_thumb(bounds: Rect, progress: f32) -> Ellipse {
    let height = bounds.bottom - bounds.top;
    let radius = (height / 2.0 - 4.0).max(1.0);
    let left = bounds.left + height / 2.0;
    let right = bounds.right - height / 2.0;
    Ellipse {
        center: Vector2 {
            x: left + (right - left) * progress.clamp(0.0, 1.0),
            y: (bounds.top + bounds.bottom) / 2.0,
        },
        radius_x: radius,
        radius_y: radius,
    }
}

const TITLE_HEIGHT: f32 = 32.0;
fn frame_hit(x: f32, y: f32, width: f32, height: f32, maximized: bool) -> u32 {
    if !maximized {
        let (left, right, top, bottom) = (x < 6.0, x >= width - 6.0, y < 6.0, y >= height - 6.0);
        match (left, right, top, bottom) {
            (true, _, true, _) => return HTTOPLEFT,
            (_, true, true, _) => return HTTOPRIGHT,
            (true, _, _, true) => return HTBOTTOMLEFT,
            (_, true, _, true) => return HTBOTTOMRIGHT,
            (true, _, _, _) => return HTLEFT,
            (_, true, _, _) => return HTRIGHT,
            (_, _, true, _) => return HTTOP,
            (_, _, _, true) => return HTBOTTOM,
            _ => {}
        }
    }
    if y < TITLE_HEIGHT && x < width - 138.0 {
        HTCAPTION
    } else {
        HTCLIENT
    }
}
fn with_titlebar(mut scene: Scene, width: f32, maximized: bool) -> Scene {
    if let Some(r) = &mut scene.app_icon {
        r.top += TITLE_HEIGHT;
        r.bottom += TITLE_HEIGHT;
    }
    for (r, _, _) in &mut scene.text {
        r.top += TITLE_HEIGHT;
        r.bottom += TITLE_HEIGHT;
    }
    for r in scene.cards.iter_mut().chain(&mut scene.separators) {
        r.top += TITLE_HEIGHT;
        r.bottom += TITLE_HEIGHT;
    }
    for (r, _, _) in &mut scene.previews {
        r.top += TITLE_HEIGHT;
        r.bottom += TITLE_HEIGHT;
    }
    for c in &mut scene.controls {
        c.bounds.top += TITLE_HEIGHT;
        c.bounds.bottom += TITLE_HEIGHT;
    }
    scene.text(
        Rect::from_xywh(16.0, 0.0, 220.0, TITLE_HEIGHT),
        "LucidPane 设置",
        0,
    );
    for (i, (glyph, command)) in [
        ("最小化", SC_MINIMIZE),
        (
            if maximized { "还原" } else { "最大化" },
            if maximized { SC_RESTORE } else { SC_MAXIMIZE },
        ),
        ("关闭", SC_CLOSE),
    ]
    .iter()
    .enumerate()
    {
        scene.control(
            ControlKind::Caption,
            Rect::from_xywh(width - 138.0 + i as f32 * 46.0, 0.0, 46.0, TITLE_HEIGHT),
            glyph,
            Action::Window(*command),
            false,
        );
    }
    scene
}

// Windows may calculate the frame synchronously while the main handler is
// detached. Keep the client-area decision independent of application state.
unsafe extern "system" fn frame_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    id: usize,
    _: usize,
) -> isize {
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    // Destruction may be nested inside WM_CLOSE while windows-window has
    // detached its callback. Closing settings must never post WM_QUIT.
    if msg == WM_DESTROY {
        return 0;
    }
    if msg == WM_NCCALCSIZE || msg == WM_NCPAINT {
        return 0;
    }
    if msg == WM_NCACTIVATE {
        return 1;
    }
    if msg == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(hwnd, Some(frame_proc), id);
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

mod painter;
use painter::Painter;

pub(super) fn show(state: &Rc<RefCell<PaneApp>>, id: PanelId) -> Result<(), String> {
    // Native activation synchronously dispatches messages to other panes. Do not
    // hold a shared-state borrow while showing or activating this window.
    let existing = state.borrow().settings.as_ref().map(|window| window.hwnd());
    if let Some(hwnd) = existing {
        unsafe {
            SendMessageW(hwnd.cast(), SELECT_PANEL, id.get() as usize, 0);
            ShowWindow(hwnd.cast(), SW_RESTORE);
            SetForegroundWindow(hwnd.cast());
        }
        return Ok(());
    }
    let weak = Rc::downgrade(state);
    let (mut panels, mut appearance) = {
        let state = state.borrow();
        (
            state.workspace.panels().to_vec(),
            state
                .workspace
                .appearance()
                .or_else(|| {
                    state
                        .workspace
                        .panels()
                        .first()
                        .map(|p| (p.theme(), p.backdrop()))
                })
                .unwrap_or((PanelTheme::System, Backdrop::Mica)),
        )
    };
    let mut options = state.borrow().workspace.pane_options();
    let mut folder_defaults = folder::Defaults::load(&state.borrow().store)?;
    let painter = Painter::new().map_err(|e| e.to_string())?;
    let mut surface: Option<composition::Surface> = None;
    let mut reveal: Option<PendingReveal> = None;
    let mut page = if state
        .borrow()
        .runtime
        .as_ref()
        .is_some_and(|r| r.desktop_error.is_some())
    {
        5
    } else {
        0
    };
    let mut recording_peek = false;
    let mut recording_search = false;
    let mut search_visible = false;
    let mut selected = id;
    let mut hover = None;
    let mut focus = None;
    let mut keyboard_focus = false;
    let mut pressed = None;
    let mut radius_original = None;
    let mut grid_original = None;
    let mut material_original = None;
    let mut style_input: Option<(bool, String)> = None;
    let mut cached_scene = None;
    let mut scene_key = None;
    let mut desktop_status = String::new();
    let mut diagnostics_copied = false;
    let mut backup_view = recovery::View::default();
    let mut backup_policy = recovery::Policy::default();
    let mut backup_offset = 0usize;
    let mut toggle_timer_running = false;
    let mut toggle_motion = std::collections::HashMap::<usize, ToggleMotion>::new();
    let prepared = Rc::new(std::cell::Cell::new(false));
    let show_prepared = Rc::clone(&prepared);
    let window = windows_window::Window::new("LucidPane 设置")
        .size(900, DEFAULT_HEIGHT)
        .style(WS_OVERLAPPEDWINDOW)
        .ex_style(WS_EX_APPWINDOW | WS_EX_NOREDIRECTIONBITMAP)
        .on_message(move |raw, msg, wp, lp| {
            let hwnd = raw.cast();
            if unsafe { defer_initial_show(msg, lp, show_prepared.get()) } {
                return Some(0);
            }
            if msg == WM_NCCALCSIZE || msg == WM_NCPAINT {
                return Some(0);
            }
            if msg == WM_NCACTIVATE {
                return Some(1);
            }
            let Some(state) = weak.upgrade() else {
                return Some(0);
            };
            if let Some((percentage, text)) = &mut style_input {
                if msg == WM_CHAR {
                    if wp == 8 { text.pop(); }
                    else if let Some(c) = char::from_u32(wp as u32) {
                        if (c.is_ascii_hexdigit() || c == '#' || c == '%') && text.len() < 8 { text.push(c); }
                    }
                    scene_key = None;
                    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0); }
                    return Some(0);
                }
                if msg == WM_KEYDOWN {
                    if wp == VK_ESCAPE as usize { style_input = None; }
                    else if wp == VK_RETURN as usize {
                        if let Some(value) = edited_solid(appearance.1, *percentage, text) {
                            if let Err(error) = handle(&state, selected, Event::Material(value)) { window::error(&error); }
                            style_input = None;
                        } else { window::error("请输入 6 位 HEX 颜色或 0–100 的百分比。"); }
                    } else { return Some(0); }
                    scene_key = None;
                    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0); }
                    return Some(0);
                }
                if msg == WM_LBUTTONDOWN { style_input = None; scene_key = None; }
            }
            if msg == PREPARE_REVEAL {
                let ready = surface.as_ref().and_then(|s| s.commit_ready().ok())
                    .unwrap_or_else(|| Box::new(|| true));
                let mut animations = 1i32;
                unsafe {
                    SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut animations).cast(), 0);
                }
                reveal = Some(PendingReveal {
                    started: std::time::Instant::now(), ready, fade: animations != 0,
                });
                unsafe {
                    if SetTimer(hwnd, REVEAL_TIMER, USER_TIMER_MINIMUM, None) == 0 {
                        reveal = None;
                        let _ = cloak(hwnd, false);
                        SetForegroundWindow(hwnd);
                    }
                }
                return Some(0);
            }
            if msg == WM_TIMER && wp == REVEAL_TIMER {
                if let Some(pending) = &mut reveal {
                    let timed_out = pending.started.elapsed().as_secs() >= 1;
                    if !(pending.ready)() && !timed_out { return Some(0); }
                    if pending.fade && !timed_out {
                        pending.fade = false;
                        // Commit the initial animation frame while still cloaked,
                        // so uncloaking cannot briefly expose full-opacity content.
                        if let Some(surface) = &surface
                            && let Ok(ready) = surface.fade_in().and_then(|()| surface.commit_ready())
                        {
                            pending.ready = ready;
                            return Some(0);
                        }
                    }
                    reveal = None;
                    unsafe {
                        KillTimer(hwnd, REVEAL_TIMER);
                        let _ = windows::Win32::Graphics::Dwm::DwmFlush();
                        let _ = cloak(hwnd, false);
                        SetForegroundWindow(hwnd);
                    }
                }
                return Some(0);
            }
            if msg == SELECT_PANEL {
                if wp != 0 {
                    selected = PanelId::new(wp as u64);
                }
                unsafe {
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
                return Some(0);
            }
            if msg == WM_CLOSE {
                let Ok(mut owner) = state.try_borrow_mut() else {
                    unsafe {
                        PostMessageW(hwnd, WM_CLOSE, 0, 0);
                    }
                    return Some(0);
                };
                if let Some(original) = material_original.take() {
                    if let Err(error) = events::commit_material(&mut owner, original) { window::error(&error); }
                }
                if let Some(original) = radius_original.take() {
                    if let Err(error) = events::commit_radius(&mut owner, original) {
                        window::error(&error);
                    }
                }
                if let Some(original) = grid_original.take() {
                    if let Err(error) = events::commit_grid(&mut owner, original) { window::error(&error); }
                }
                let window = if owner
                    .settings
                    .as_ref()
                    .is_some_and(|window| window.hwnd() == raw)
                {
                    owner.settings.take()
                } else {
                    None
                };
                drop(owner);
                unsafe {
                    KillTimer(hwnd, TOGGLE_TIMER);
                }
                // Drop outside the PaneApp borrow: native destruction can send
                // focus messages to other panes. The callback's render resources
                // are released when this invocation returns.
                drop(window);
                return Some(0);
            }
            if msg == WM_DESTROY {
                return Some(0);
            }
            let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
            if msg == WM_NCHITTEST {
                let mut point = windows_sys::Win32::Foundation::POINT {
                    x: (lp as u16 as i16).into(),
                    y: ((lp >> 16) as u16 as i16).into(),
                };
                let mut bounds = RECT::default();
                unsafe {
                    ScreenToClient(hwnd, &raw mut point);
                    GetClientRect(hwnd, &raw mut bounds);
                }
                return Some(frame_hit(
                    point.x as f32 / scale,
                    point.y as f32 / scale,
                    bounds.right as f32 / scale,
                    bounds.bottom as f32 / scale,
                    unsafe { IsZoomed(hwnd) } != 0,
                ) as isize);
            }
            if msg == WM_GETMINMAXINFO {
                unsafe {
                    let info = &mut *(lp as *mut MINMAXINFO);
                    info.ptMinTrackSize.x = (800.0 * scale) as i32;
                    info.ptMinTrackSize.y = (MIN_HEIGHT * scale) as i32;
                    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                    let mut metrics = MONITORINFO {
                        cbSize: size_of::<MONITORINFO>() as u32,
                        ..Default::default()
                    };
                    if GetMonitorInfoW(monitor, &raw mut metrics) != 0 {
                        info.ptMinTrackSize.x = info
                            .ptMinTrackSize
                            .x
                            .min(metrics.rcWork.right - metrics.rcWork.left);
                        info.ptMinTrackSize.y = info
                            .ptMinTrackSize
                            .y
                            .min(metrics.rcWork.bottom - metrics.rcWork.top);
                        info.ptMaxPosition.x = metrics.rcWork.left - metrics.rcMonitor.left;
                        info.ptMaxPosition.y = metrics.rcWork.top - metrics.rcMonitor.top;
                        info.ptMaxSize.x = metrics.rcWork.right - metrics.rcWork.left;
                        info.ptMaxSize.y = metrics.rcWork.bottom - metrics.rcWork.top;
                    }
                }
                return Some(0);
            }
            if msg == WM_DPICHANGED {
                let _ = crate::app_icon::apply(hwnd);
                unsafe {
                    let r = &*(lp as *const RECT);
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
                return Some(0);
            }
            // Hook synchronization can repaint/activate this window while the
            // workspace is mutably borrowed. Render the last complete snapshot
            // during that reentry; unrelated native messages need no snapshot.
            if !matches!(
                msg,
                WM_PAINT
                    | WM_ERASEBKGND
                    | WM_SIZE
                    | WM_ACTIVATE
                    | WM_MOUSEMOVE
                    | WM_MOUSELEAVE
                    | WM_NCMOUSEMOVE
                    | WM_LBUTTONDOWN
                    | WM_LBUTTONUP
                    | WM_CAPTURECHANGED
                    | WM_CANCELMODE
                    | WM_TIMER
                    | WM_KEYDOWN
                    | WM_SYSKEYDOWN
            ) {
                return None;
            }
            let mut snapshot_changed = false;
            let available = if let Ok(state) = state.try_borrow() {
                if panels != state.workspace.panels() {
                    panels = state.workspace.panels().to_vec();
                    snapshot_changed = true;
                }
                search_visible = state.views.iter().any(|v| state.workspace.panel(v.id).is_some_and(Panel::is_search));
                desktop_status = runtime::status(&state);
                let fresh = recovery::view(&state);
                let policy = recovery::Policy::load(&state.store);
                snapshot_changed |= fresh != backup_view || policy != backup_policy;
                backup_view = fresh; backup_policy = policy;
                if backup_offset >= backup_view.records.len() {backup_offset = 0;}
                let defaults=folder::Defaults::load(&state.store).unwrap_or_default();
                snapshot_changed |= defaults != folder_defaults;
                folder_defaults=defaults;
                options = state.workspace.pane_options();
                appearance = state
                    .workspace
                    .appearance()
                    .or_else(|| panels.first().map(|p| (p.theme(), p.backdrop())))
                    .unwrap_or((PanelTheme::System, Backdrop::Mica));
                true
            } else {
                false
            };
            let at = panels.iter().position(|p| p.id() == selected).unwrap_or(0);
            let panel = panels.get(at);
            if let Some(p) = panel {
                selected = p.id();
            }
            let dark = theme::is_dark(appearance.0);
            let mut bounds = RECT::default();
            unsafe {
                GetClientRect(hwnd, &raw mut bounds);
            }
            let (w, h) = (bounds.right as f32 / scale, bounds.bottom as f32 / scale);
            let key = (
                w.to_bits(),
                h.to_bits(),
                page,
                selected,
                appearance,
                options,
                peek::settings(),
                everything_settings::settings(),
                (recording_peek, recording_search, search_hotkey::settings(), search_hotkey::status()),
                unsafe { IsZoomed(hwnd) } != 0,
                search_visible,
                desktop_status.clone(),
            );
            let scene_changed = snapshot_changed || scene_key.as_ref() != Some(&key);
            if scene_changed {
                let mut body = scene(
                        w,
                        h - TITLE_HEIGHT,
                        page,
                        search_visible,
                        appearance,
                        options,
                    );
                if page == 8 { layout::folder_defaults(&mut body, w, folder_defaults); }
                if matches!(page,6|9|10) {
                    if page==9 {layout::backup_history(&mut body,w,&backup_view,backup_offset);} else {layout::backup_page(&mut body,w,&backup_view,backup_policy,page==10);}
                }
                if page == 5 {
                    layout::about_status(&mut body, w, &desktop_status, diagnostics_copied);
                }
                cached_scene = Some(with_titlebar(body, w, key.9));
                scene_key = Some(key);
            }
            if recording_peek || recording_search {
                for control in &mut cached_scene.as_mut().unwrap().controls {
                    if (recording_peek && matches!(control.action, Action::PeekShortcut)) || (recording_search && matches!(control.action, Action::SearchShortcut)) { control.label = "按下快捷键…".into(); }
                }
            }
            if let Some((percentage, text)) = &style_input {
                for control in &mut cached_scene.as_mut().unwrap().controls {
                    if matches!(control.action, Action::StyleInput(p) if p == *percentage) { control.label = format!("{text}|"); }
                }
            }
            let scene = cached_scene.as_ref().unwrap();
            let interaction_before = (hover, focus, keyboard_focus, pressed);
            let mut activate = None;
            let mut radius_change = None;
            let mut grid_change = None;
            let mut opacity_change = None;
            let mut strength_change = None;
            let mut channel_change = None;
            match msg {
                WM_PAINT => {
                    let now = std::time::Instant::now();
                    let mut enabled = 1i32;
                    unsafe {
                        SystemParametersInfoW(
                            SPI_GETCLIENTAREAANIMATION,
                            0,
                            (&raw mut enabled).cast(),
                            0,
                        );
                    }
                    let mut positions = std::collections::HashMap::new();
                    let mut animating = false;
                    for (i, control) in scene.controls.iter().enumerate().filter(|(_, c)| c.is_toggle())
                    {
                        let to = if control.selected { 1.0 } else { 0.0 };
                        let motion = toggle_motion.entry(i).or_insert_with(|| ToggleMotion::settled(to, now));
                        let value = motion.retarget(to, now, enabled != 0);
                        animating |= (value - to).abs() > 0.0001;
                        positions.insert(i, value);
                    }
                    unsafe {
                        if animating != toggle_timer_running {
                            toggle_timer_running = animating && SetTimer(hwnd, TOGGLE_TIMER, USER_TIMER_MINIMUM, None) != 0;
                            if !toggle_timer_running { KillTimer(hwnd, TOGGLE_TIMER); }
                        }
                    }
                    unsafe {
                        let mut ps = PAINTSTRUCT::default();
                        BeginPaint(hwnd, &raw mut ps);
                        EndPaint(hwnd, &raw const ps);
                    }
                    if bounds.right > 0 && bounds.bottom > 0 {
                        let result = (|| -> windows::core::Result<()> {
                            if surface.is_none() {
                                surface = Some(composition::Surface::new_settings(
                                    windows::Win32::Foundation::HWND(hwnd),
                                )?);
                                composition::Surface::disable_window_shadow(
                                    windows::Win32::Foundation::HWND(hwnd),
                                )?;
                            }
                            let surface = surface.as_mut().unwrap();
                            surface.theme(windows::Win32::Foundation::HWND(hwnd), dark);
                            surface.material(windows::Win32::Foundation::HWND(hwnd), settings_backdrop(appearance.1, dark));
                            let Some(target) =
                                surface.try_begin_frame(bounds.right as u32, bounds.bottom as u32)? else {
                                    return Ok(());
                                };
                            painter.paint(
                                &target,
                                &scene,
                                w,
                                h,
                                scale,
                                dark,
                                surface.native,
                                hover,
                                if keyboard_focus { focus } else { None },
                                &positions,
                            )?;
                            surface.end_frame()
                        })();
                        if let Err(e) = result {
                            surface = None;
                            eprintln!("Settings paint: {e}");
                        }
                    }
                }
                WM_TIMER if wp == TOGGLE_TIMER => {}
                WM_ERASEBKGND => return Some(1),
                WM_SIZE => {
                    hover = None;
                }
                WM_ACTIVATE => {
                    if wp & 0xffff == WA_INACTIVE as usize {
                        hover = None;
                        pressed = None;
                        keyboard_focus = false;
                    }
                }
                WM_MOUSELEAVE | WM_NCMOUSEMOVE => {
                    hover = None;
                }
                WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
                    if msg == WM_MOUSEMOVE {
                        unsafe {
                            let mut tracking = TRACKMOUSEEVENT {
                                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                                dwFlags: TME_LEAVE,
                                hwndTrack: hwnd,
                                dwHoverTime: 0,
                            };
                            TrackMouseEvent(&raw mut tracking);
                        }
                    }
                    let x = (lp as u16 as i16) as f32 / scale;
                    let y = ((lp >> 16) as u16 as i16) as f32 / scale;
                    let hit = scene
                        .controls
                        .iter()
                        .position(|c| c.enabled && contains(&c.bounds, x, y));
                    hover = hit;
                    if msg == WM_LBUTTONDOWN {
                        pressed = hit;
                        focus = hit;
                        keyboard_focus = false;
                        unsafe {
                            SetFocus(hwnd);
                            SetCapture(hwnd);
                        }
                    }
                    if let Some(control) = pressed.and_then(|i| scene.controls.get(i)) {
                        if let Action::Channel(channel, _) = control.action {
                            let fraction = controls::slider_fraction(control.bounds, x);
                            channel_change = Some((channel, (fraction * 255.0).round() as u8));
                        }
                        if matches!(control.action, Action::Strength(_)) {
                            strength_change = Some((controls::slider_fraction(control.bounds, x) * 100.0).round() as u8);
                        }
                        if matches!(control.action, Action::Opacity(_)) {
                            opacity_change = Some((controls::slider_fraction(control.bounds, x) * 100.0).round() as u8);
                        }
                        if let Action::GridSize(_) = control.action {
                            let fraction = controls::slider_fraction(control.bounds, x);
                            grid_change = Some(grid_slider_value(fraction));
                        }
                        if matches!(control.action, Action::Radius(_)) {
                            radius_change = Some(radius_from_pointer(control.bounds, x));
                        }
                    }
                    if msg == WM_LBUTTONUP {
                        if pressed.take() == hit {
                            activate = hit;
                        }
                        unsafe {
                            ReleaseCapture();
                        }
                    }
                }
                WM_CAPTURECHANGED | WM_CANCELMODE => {
                    pressed = None;
                }
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    if recording_search {
                        if lp & (1 << 30) != 0 { return Some(0); }
                        let key = wp as u16;
                        if matches!(key, VK_CONTROL | VK_SHIFT | VK_MENU | VK_LWIN | VK_RWIN) { return Some(0); }
                        if key != VK_ESCAPE {
                            let value = search_hotkey::Shortcut { key, modifiers: peek::modifier_bits(&keyboard::Modifiers::current()) };
                            if let Err(error) = search_hotkey::save(&state.borrow().store, value) { window::error(&error); return Some(0); }
                        }
                        recording_search = false;
                        unsafe { InvalidateRect(hwnd, std::ptr::null(), 0); }
                        return Some(0);
                    }
                    if recording_peek {
                        if lp & (1 << 30) != 0 { return Some(0); }
                        let key = wp as u16;
                        if matches!(key, VK_CONTROL | VK_SHIFT | VK_MENU | VK_LWIN | VK_RWIN) { return Some(0); }
                        if key != VK_ESCAPE {
                            let bits = peek::modifier_bits(&keyboard::Modifiers::current());
                            if !peek::valid_shortcut(key, bits) {
                                window::error("此快捷键与现有操作冲突或不受支持，请使用字母、数字、功能键或空格，可搭配 Ctrl、Shift、Alt。");
                                return Some(0);
                            }
                            let mut value = peek::settings(); value.key = key; value.modifiers = bits;
                            if let Err(error) = peek::save(&state.borrow().store, value) { window::error(&error); }
                        }
                        recording_peek = false;
                        unsafe { InvalidateRect(hwnd, std::ptr::null(), 0); }
                        return Some(0);
                    }
                    if msg == WM_SYSKEYDOWN { return None; }
                    if wp == VK_ESCAPE as usize {
                        if page == 7 {
                            page = 0; scene_key = None; focus = None;
                            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0); }
                            return Some(0);
                        }
                        unsafe {
                            PostMessageW(hwnd, WM_CLOSE, 0, 0);
                        }
                        return Some(0);
                    }
                    if let Some(control) = focus.and_then(|i| scene.controls.get(i)) {
                        if let Action::Channel(channel, value) = control.action {
                            channel_change = match wp as u16 {
                                VK_LEFT => Some(value.saturating_sub(1)), VK_RIGHT => Some(value.saturating_add(1)),
                                VK_HOME => Some(0), VK_END => Some(255), _ => None,
                            }.map(|value| (channel, value));
                        }
                        if let Action::Strength(value) = control.action {
                            strength_change = match wp as u16 {
                                VK_LEFT => Some(value.saturating_sub(1)), VK_RIGHT => Some((value + 1).min(100)),
                                VK_HOME => Some(0), VK_END => Some(100), _ => None,
                            };
                        }
                        if let Action::Opacity(value) = control.action {
                            opacity_change = match wp as u16 {
                                VK_LEFT => Some(value.saturating_sub(1)), VK_RIGHT => Some((value + 1).min(100)),
                                VK_HOME => Some(0), VK_END => Some(100), _ => None,
                            };
                        }
                        if let Action::GridSize(value) = control.action {
                            let range = grid_range();
                            grid_change = match wp as u16 {
                                VK_LEFT => Some((value - 1.0).max(range.0)),
                                VK_RIGHT => Some((value + 1.0).min(range.1)),
                                VK_HOME => Some(range.0), VK_END => Some(range.1), _ => None,
                            };
                        }
                        if let Action::Radius(value) = control.action {
                            radius_change = match wp as u16 {
                                VK_LEFT => Some((value - 0.1).max(0.0)),
                                VK_RIGHT => Some((value + 0.1).min(24.0)),
                                VK_HOME => Some(0.0),
                                VK_END => Some(24.0),
                                _ => None,
                            };
                        }
                    }
                    if wp == VK_TAB as usize || wp == VK_DOWN as usize || wp == VK_UP as usize {
                        keyboard_focus = true;
                        let n = scene.controls.len();
                        let backwards = wp == VK_UP as usize
                            || (wp == VK_TAB as usize
                                && unsafe { GetKeyState(VK_SHIFT as i32) } < 0);
                        focus = Some(match focus {
                            Some(i) if backwards => (i + n - 1) % n,
                            Some(i) => (i + 1) % n,
                            None => {
                                if backwards {
                                    n - 1
                                } else {
                                    0
                                }
                            }
                        });
                        for _ in 0..n {
                            if scene.controls[focus.unwrap()].enabled { break; }
                            let i = focus.unwrap();
                            focus = Some(if backwards { (i + n - 1) % n } else { (i + 1) % n });
                        }
                    } else if wp == VK_SPACE as usize || wp == VK_RETURN as usize {
                        activate = focus;
                    }
                }
                _ => return None,
            }
            if let Some((channel, value)) = channel_change.filter(|_| available) {
                if let Backdrop::Solid { color, opacity } = appearance.1 {
                    let backdrop = Backdrop::Solid { color: color_channel(color, channel, value), opacity };
                    if msg == WM_KEYDOWN {
                        if let Err(error) = handle(&state, selected, Event::Material(backdrop)) { window::error(&error); }
                    } else {
                        material_original.get_or_insert(appearance.1);
                        events::preview_material(&mut state.borrow_mut(), backdrop);
                    }
                }
            }
            if let Some(value) = strength_change.filter(|_| available) {
                let backdrop = appearance.1.with_strength(value);
                if msg == WM_KEYDOWN {
                    if let Err(error) = handle(&state, selected, Event::Material(backdrop)) { window::error(&error); }
                } else {
                    material_original.get_or_insert(appearance.1);
                    events::preview_material(&mut state.borrow_mut(), backdrop);
                }
            }
            if let Some(value) = opacity_change.filter(|_| available) {
                if let Backdrop::Solid { color, .. } = appearance.1 {
                    let backdrop = Backdrop::Solid { color, opacity: f32::from(value) / 100.0 };
                    if msg == WM_KEYDOWN {
                        if let Err(error) = handle(&state, selected, Event::Material(backdrop)) { window::error(&error); }
                    } else {
                        material_original.get_or_insert(appearance.1);
                        events::preview_material(&mut state.borrow_mut(), backdrop);
                    }
                }
            }
            if available && matches!(msg, WM_LBUTTONUP | WM_CAPTURECHANGED | WM_CANCELMODE) {
                if let Some(original) = material_original.take() {
                    if let Err(error) = events::commit_material(&mut state.borrow_mut(), original) { window::error(&error); }
                }
            }
            if let Some(value) = grid_change.filter(|_| available) {
                if msg == WM_KEYDOWN {
                    if let Err(error) = handle(&state, selected, Event::SetIconGrid(value)) { window::error(&error); }
                } else {
                    let mut owner = state.borrow_mut();
                    let options = owner.workspace.pane_options();
                    grid_original.get_or_insert(options.grid_scale);
                    events::preview_grid(&mut owner, value);
                }
            }
            if available && matches!(msg, WM_LBUTTONUP | WM_CAPTURECHANGED | WM_CANCELMODE) {
                if let Some(original) = grid_original.take() {
                    if let Err(error) = events::commit_grid(&mut state.borrow_mut(), original) { window::error(&error); }
                }
            }
            if let Some(radius) = radius_change.filter(|_| available) {
                if msg == WM_KEYDOWN {
                    if let Err(error) = handle(&state, selected, Event::SetCornerRadius(radius)) {
                        window::error(&error);
                    }
                } else {
                    let mut owner = state.borrow_mut();
                    radius_original.get_or_insert(owner.workspace.pane_options().corner_radius);
                    events::preview_radius(&mut owner, radius);
                }
            }
            if available && matches!(msg, WM_LBUTTONUP | WM_CAPTURECHANGED | WM_CANCELMODE) {
                if let Some(original) = radius_original.take() {
                    if let Err(error) = events::commit_radius(&mut state.borrow_mut(), original) {
                        window::error(&error);
                    }
                }
            }
            if let Some(c) = activate
                .filter(|_| available)
                .and_then(|i| scene.controls.get(i))
                .filter(|c| c.enabled)
            {
                match &c.action {
                    Action::BackupStatus => {
                        let message=backup_view.status.clone();
                        window::defer_action(move || {window::error(&message);});
                    }
                    Action::BackupAdvanced => {page=10;scene_key=None;focus=None;}
                    Action::BackupPage(direction) => {
                        let count=6;
                        backup_offset=if *direction<0{backup_offset.saturating_sub(count)}else{backup_offset+count};scene_key=None;
                    }
                    Action::BackupPolicy(kind) => {
                        let state=Rc::clone(&state);let kind=*kind;
                        let mut point=windows_sys::Win32::Foundation::POINT {
                            x:(c.bounds.left*scale).round() as i32,
                            y:((c.bounds.bottom+4.0)*scale).round() as i32,
                        };
                        unsafe{ClientToScreen(hwnd,&raw mut point);}
                        window::defer_action(move || {
                            let mut policy=recovery::Policy::load(&state.borrow().store);
                            if kind==0 {policy.enabled=!policy.enabled;} else {
                                let options: &[(u64, &'static str)] = if kind == 1 {
                                    &[(5, "5 分钟"), (15, "15 分钟"), (30, "30 分钟"), (60, "60 分钟")]
                                } else {
                                    &[(10, "最近 10 份"), (20, "最近 20 份"), (50, "最近 50 份")]
                                };
                                let selected = if kind == 1 { policy.minutes } else { policy.keep as u64 };
                                if let Some(value) = controls::choose(hwnd, point, appearance, options, selected) {
                                    if kind == 1 { policy.minutes = value; } else { policy.keep = value as usize; }
                                }
                            }
                            if let Err(e)=policy.save(&state.borrow().store){window::error(&e);}
                            unsafe{InvalidateRect(hwnd,std::ptr::null(),0);}
                        });
                    }
                    Action::BackupRecord(path) => {
                        let state=Rc::clone(&state);let path=path.clone();
                        let mut point=windows_sys::Win32::Foundation::POINT::default();unsafe{GetCursorPos(&raw mut point);}
                        window::defer_action(move || {
                            let rows=vec![super::menu::entry(1,"恢复…","",""),super::menu::entry(2,"导出…","",""),super::menu::entry(3,"删除…","","")];
                            let result=super::menu::show_entries(hwnd,point,false,appearance.0,appearance.1,rows);
                            let event=match result{1=>Some(Event::RestoreBackupPath(path)),2=>Some(Event::ExportBackupPath(path)),3=>Some(Event::DeleteBackup(path)),_=>None};
                            if let Some(event)=event{recovery::request(&state,&event);}
                        });
                    }
                    Action::FolderDefaults(value) => {
                        match value.save(&state.borrow().store) {
                            Ok(()) => { folder_defaults = *value; scene_key = None; }
                            Err(error) => window::error(&error),
                        }
                    }
                    Action::StyleInput(percentage) => {
                        style_input = Some((*percentage, String::new()));
                        scene_key = None;
                    }
                    Action::SolidColor => { page = 7; focus = None; hover = None; scene_key = None; }
                    Action::ColorPreset(color) => {
                        if let Backdrop::Solid { opacity, .. } = appearance.1 {
                            if let Err(error) = handle(&state, selected, Event::Material(Backdrop::Solid { color: *color, opacity })) { window::error(&error); }
                        }
                    }
                    Action::StrengthReset => {
                        if let Err(error) = handle(&state, selected, Event::Material(appearance.1.base())) { window::error(&error); }
                    }
                    Action::SolidReset => {
                        let value = Backdrop::Solid { color: if dark { 0x181b20 } else { 0xf5f6f8 }, opacity: 0.85 };
                        if let Err(error) = handle(&state, selected, Event::Material(value)) { window::error(&error); }
                    }
                    Action::Change(Event::Material(Backdrop::Solid { .. })) => {
                        let value = solid_style(&state.borrow().store, dark);
                        if let Err(error) = handle(&state, selected, Event::Material(value)) { window::error(&error); }
                    }
                    Action::CopyDiagnostics => {
                        let report = format!("{}Desktop: {}\r\n", crate::diagnostics::report(), desktop_status);
                        match crate::diagnostics::copy(hwnd as isize, &report) {
                            Ok(()) => { diagnostics_copied = true; scene_key = None; }
                            Err(error) => window::error(&error.to_string()),
                        }
                    }
                    Action::ProjectHome => {
                        if let Err(error) = desktop_shell::open_shell_identity(hwnd as isize, &desktop_core::ShellIdentity::Namespace {
                            parsing_name: "https://git.bbkingdom.fun:30443/yuchen95/LucidPane".into(),
                        }) { window::error(&error.to_string()); }
                    }
                    Action::EverythingBrowse | Action::EverythingDetect | Action::EverythingLaunch => {
                        let mut value = everything_settings::settings();
                        let result = (|| -> Result<(), String> {
                            match c.action {
                                Action::EverythingBrowse => {
                                    let Some(path) = everything_settings::browse(hwnd as isize)? else { return Ok(()); };
                                    value.path = path;
                                }
                                Action::EverythingDetect => value.path.clear(),
                                Action::EverythingLaunch => return everything_settings::launch(),
                                _ => unreachable!(),
                            }
                            everything_settings::save(&state.borrow().store, value)
                        })();
                        if let Err(error) = result { window::error(&error); }
                        scene_key = None;
                    }
                    Action::SearchShortcut => { recording_search = true; recording_peek = false; }
                    Action::SearchReset => {
                        if let Err(error) = search_hotkey::save(&state.borrow().store, Default::default()) { window::error(&error); }
                        recording_search = false;
                    }
                    Action::PeekShortcut => { recording_peek = true; recording_search = false; }
                    Action::PreviewProvider(_) | Action::PeekEnable | Action::PeekBrowse | Action::PeekDetect | Action::PeekReset => {
                        let mut value = peek::settings();
                        let result = (|| -> Result<(), String> {
                            match c.action {
                                Action::PreviewProvider(provider) => value.provider = provider,
                                Action::PeekEnable => value.enabled = !value.enabled,
                                Action::PeekBrowse => {
                                    let Some(path) = peek::browse(hwnd as isize, value.provider)? else { return Ok(()); };
                                    value.set_path(path);
                                }
                                Action::PeekDetect => { value.set_path(String::new()); }
                                Action::PeekReset => { value.key = VK_SPACE; value.modifiers = 0; }
                                _ => unreachable!(),
                            }
                            peek::save(&state.borrow().store, value)
                        })();
                        if let Err(error) = result { window::error(&error); }
                        scene_key = None;
                    }
                    Action::Window(command) => unsafe {
                        let command = if *command == SC_MAXIMIZE && IsZoomed(hwnd) != 0 {
                            SC_RESTORE
                        } else {
                            *command
                        };
                        PostMessageW(hwnd, WM_SYSCOMMAND, command as usize, 0);
                    },
                    Action::Page(value) => {
                        recording_peek = false;
                        recording_search = false;
                        diagnostics_copied = false;
                        page = *value;
                        toggle_motion.clear();
                        focus = None;
                    }
                    Action::GridSize(_) | Action::Radius(_) | Action::Opacity(_) | Action::Strength(_) | Action::Channel(_, _) => {}
                    Action::Change(Event::Material(value)) if value.strength().is_some() => {
                        let backdrop = material_style(&state.borrow().store, *value);
                        if let Err(error) = handle(&state, selected, Event::Material(backdrop)) { window::error(&error); }
                    }
                    Action::Change(event) => {
                        if let Err(e) = handle(&state, selected, event.clone()) {
                            window::error(&e);
                        }
                    }
                }
            }
            if msg != WM_PAINT
                && (scene_changed
                    || interaction_before != (hover, focus, keyboard_focus, pressed)
                    || activate.is_some()
                    || grid_change.is_some()
                    || radius_change.is_some_and(|radius| radius != options.corner_radius)
                    || matches!(msg, WM_SIZE | WM_ACTIVATE | WM_TIMER))
            {
                unsafe {
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
            }
            Some(0)
        })
        .create()
        .map_err(|e| e.to_string())?;
    crate::app_icon::apply(window.hwnd().cast())?;
    unsafe {
        if windows_sys::Win32::UI::Shell::SetWindowSubclass(
            window.hwnd().cast(),
            Some(frame_proc),
            0x4c505346,
            0,
        ) == 0
        {
            return Err("无法初始化设置窗口边框".into());
        }
        let hwnd = window.hwnd().cast();
        let dpi = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let mut monitor = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &raw mut monitor,
        ) != 0
        {
            let work = monitor.rcWork;
            let width = ((900.0 * dpi).round() as i32).min(work.right - work.left);
            let height = ((DEFAULT_HEIGHT as f32 * dpi).round() as i32).min(work.bottom - work.top);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                work.left + (work.right - work.left - width) / 2,
                work.top + (work.bottom - work.top - height) / 2,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
        // Prepare material and content at the final size before exposing the HWND.
        SendMessageW(hwnd, WM_PAINT, 0, 0);
        // Cloaking keeps the visible HWND in DWM composition without exposing
        // a partial frame. A hidden HWND cannot prepare host backdrop sampling.
        if cloak(hwnd, true).is_ok() {
            prepared.set(true);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            PostMessageW(hwnd, PREPARE_REVEAL, 0, 0);
        } else {
            prepared.set(true);
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
        }
    }
    state.borrow_mut().settings = Some(window);
    Ok(())
}

#[cfg(test)]
mod tests;
