//! Modeless settings, drawn with the same native composition pipeline as panes.
#![allow(
    clippy::wildcard_imports,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
use windows_canvas::{ColorF, Ellipse, Rect, RoundedRect, Vector2};

use super::composition::canvas_result;
use super::*;
use desktop_core::{Backdrop, PanelTheme};
use windows_canvas::ID2D1DeviceContext;
use windows_sys::Win32::{
    Graphics::Gdi::*,
    UI::{HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
const SELECT_PANEL: u32 = WM_APP + 95;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;

#[derive(Clone)]
enum Action {
    Window(u32),
    Page(usize),
    Previous,
    Next,
    Change(Event),
}
struct Control {
    bounds: Rect,
    label: String,
    action: Action,
    selected: bool,
    toggle: bool,
}
struct Scene {
    text: Vec<(Rect, String, usize)>,
    cards: Vec<Rect>,
    controls: Vec<Control>,
}

fn contains(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}
impl Scene {
    fn text(&mut self, r: Rect, text: impl Into<String>, size: usize) {
        self.text.push((r, text.into(), size));
    }
    fn button(&mut self, r: Rect, text: &str, action: Action, selected: bool) {
        self.controls.push(Control {
            bounds: r,
            label: text.into(),
            action,
            selected,
            toggle: false,
        });
    }
}
#[path = "settings_layout.rs"]
mod layout;
use layout::scene;

const TOGGLE_TIMER: usize = 0x4c5054;
struct ToggleMotion {
    from: f32,
    to: f32,
    started: std::time::Instant,
}
impl ToggleMotion {
    fn sample(&self, now: std::time::Instant) -> f32 {
        let t = (now.saturating_duration_since(self.started).as_secs_f32() / 0.16).min(1.0);
        self.from + (self.to - self.from) * (1.0 - (1.0 - t).powi(3))
    }
    fn retarget(&mut self, to: f32, now: std::time::Instant, animate: bool) -> f32 {
        if self.to != to {
            self.from = self.sample(now);
            self.to = to;
            self.started = now;
        }
        if !animate {
            self.from = to;
        }
        self.sample(now)
    }
}
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
    for (r, _, _) in &mut scene.text {
        r.top += TITLE_HEIGHT;
        r.bottom += TITLE_HEIGHT;
    }
    for r in &mut scene.cards {
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
        scene.button(
            Rect::from_xywh(width - 138.0 + i as f32 * 46.0, 0.0, 46.0, TITLE_HEIGHT),
            glyph,
            Action::Window(*command),
            false,
        );
    }
    scene
}

fn caption_lines(bounds: Rect, command: u32) -> Vec<(Vector2, Vector2)> {
    let x = (bounds.left + bounds.right) / 2.0 - 5.0;
    let y = (bounds.top + bounds.bottom) / 2.0 - 5.0;
    let segments: &[(f32, f32, f32, f32)] = match command {
        SC_MINIMIZE => &[(0.0, 5.0, 10.0, 5.0)],
        SC_CLOSE => &[(0.0, 0.0, 10.0, 10.0), (10.0, 0.0, 0.0, 10.0)],
        SC_RESTORE => &[
            (2.0, 0.0, 10.0, 0.0),
            (10.0, 0.0, 10.0, 8.0),
            (10.0, 8.0, 8.0, 8.0),
            (2.0, 0.0, 2.0, 2.0),
            (0.0, 2.0, 8.0, 2.0),
            (8.0, 2.0, 8.0, 10.0),
            (8.0, 10.0, 0.0, 10.0),
            (0.0, 10.0, 0.0, 2.0),
        ],
        _ => &[
            (0.0, 0.0, 10.0, 0.0),
            (10.0, 0.0, 10.0, 10.0),
            (10.0, 10.0, 0.0, 10.0),
            (0.0, 10.0, 0.0, 0.0),
        ],
    };
    segments
        .iter()
        .map(|&(x1, y1, x2, y2)| {
            (
                Vector2 {
                    x: x + x1,
                    y: y + y1,
                },
                Vector2 {
                    x: x + x2,
                    y: y + y2,
                },
            )
        })
        .collect()
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

struct Painter {
    formats: Vec<windows_canvas::TextFormat>,
    button_format: windows_canvas::TextFormat,
}
impl Painter {
    fn new() -> windows::core::Result<Self> {
        use windows_canvas::{FontWeight, ParagraphAlignment, TextFormat, WordWrapping};
        let mut formats = vec![];
        for (i, size) in [12.0, 14.0, 20.0, 28.0, 18.0].iter().enumerate() {
            let format = canvas_result(TextFormat::with_weight(
                if i == 4 {
                    "Segoe Fluent Icons"
                } else {
                    super::assets::UI_FONT
                },
                *size,
                FontWeight(if i == 2 || i == 3 { 600 } else { 400 }),
            ))?
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .with_word_wrapping(WordWrapping::NoWrap);
            super::canvas::ellipsis(&format)?;
            formats.push(format);
        }
        let button_format = canvas_result(TextFormat::new(super::assets::UI_FONT, 14.0))?
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .with_word_wrapping(WordWrapping::NoWrap)
            .with_alignment(windows_canvas::TextAlignment::Center);
        super::canvas::ellipsis(&button_format)?;
        Ok(Self {
            formats,
            button_format,
        })
    }
    fn paint(
        &self,
        t: &ID2D1DeviceContext,
        s: &Scene,
        width: f32,
        height: f32,
        scale: f32,
        dark: bool,
        native: bool,
        hover: Option<usize>,
        focus: Option<usize>,
        toggles: &std::collections::HashMap<usize, f32>,
    ) -> windows::core::Result<()> {
        {
            super::canvas::draw(t, scale, |t| {
                let color = |v: u32| ColorF {
                    r: ((v >> 16) & 255) as f32 / 255.0,
                    g: ((v >> 8) & 255) as f32 / 255.0,
                    b: (v & 255) as f32 / 255.0,
                    a: 1.0,
                };
                let bg = color(if dark { 0x202020 } else { 0xf3f3f3 });
                let card = canvas_result(t.create_solid_brush(ColorF {
                    a: if native {
                        if dark { 0.65 } else { 0.72 }
                    } else {
                        1.0
                    },
                    ..color(if dark { 0x2b2b2b } else { 0xffffff })
                }))?;

                let ink = canvas_result(t.create_solid_brush(color(if dark {
                    0xf5f5f5
                } else {
                    0x202020
                })))?;
                let muted = canvas_result(t.create_solid_brush(color(if dark {
                    0xadadad
                } else {
                    0x666666
                })))?;
                let border = canvas_result(t.create_solid_brush(ColorF {
                    a: if native { 0.45 } else { 1.0 },
                    ..color(if dark { 0x424242 } else { 0xdfdfdf })
                }))?;
                let accent = canvas_result(t.create_solid_brush(color(if dark {
                    0x76b9ed
                } else {
                    0x0067c0
                })))?;
                let selected = canvas_result(t.create_solid_brush(color(if dark {
                    0x344656
                } else {
                    0xe2eff9
                })))?;
                let hovered = canvas_result(t.create_solid_brush(color(if dark {
                    0x383838
                } else {
                    0xeeeeee
                })))?;
                let page_background = canvas_result(t.create_solid_brush(ColorF {
                    a: if native {
                        if dark { 0.12 } else { 0.22 }
                    } else {
                        1.0
                    },
                    ..color(if dark { 0x242424 } else { 0xf9f9f9 })
                }))?;

                t.clear(if native {
                    ColorF {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }
                } else {
                    bg
                });
                t.fill_rounded_rect(
                    &RoundedRect {
                        rect: Rect::from_xywh(224.0, 0.0, width - 224.0, height),
                        radius_x: 8.0,
                        radius_y: 8.0,
                    },
                    &page_background,
                );
                for r in &s.cards {
                    let rr = RoundedRect {
                        rect: *r,
                        radius_x: 7.0,
                        radius_y: 7.0,
                    };
                    t.fill_rounded_rect(&rr, &card);
                    t.draw_rounded_rect(&rr, &border, 1.0);
                }
                for (i, c) in s.controls.iter().enumerate() {
                    let navigation = matches!(c.action, Action::Page(_));
                    let caption = matches!(c.action, Action::Window(_));
                    let material = matches!(c.action, Action::Change(Event::Material(_)));
                    let rr = RoundedRect {
                        rect: c.bounds,
                        radius_x: if c.toggle {
                            (c.bounds.bottom - c.bounds.top) / 2.0
                        } else if caption {
                            0.0
                        } else {
                            5.0
                        },
                        radius_y: if c.toggle {
                            (c.bounds.bottom - c.bounds.top) / 2.0
                        } else if caption {
                            0.0
                        } else {
                            5.0
                        },
                    };
                    let progress =
                        toggles
                            .get(&i)
                            .copied()
                            .unwrap_or(if c.selected { 1.0 } else { 0.0 });
                    if c.toggle {
                        let off = color(if dark { 0x383838 } else { 0xffffff });
                        let on = color(if dark { 0x76b9ed } else { 0x0067c0 });
                        let brush = canvas_result(t.create_solid_brush(ColorF {
                            r: off.r + (on.r - off.r) * progress,
                            g: off.g + (on.g - off.g) * progress,
                            b: off.b + (on.b - off.b) * progress,
                            a: 1.0,
                        }))?;
                        t.fill_rounded_rect(&rr, &brush);
                    } else if (!navigation && !caption) || c.selected || hover == Some(i) {
                        t.fill_rounded_rect(
                            &rr,
                            if c.toggle && c.selected {
                                &accent
                            } else if c.selected {
                                &selected
                            } else if hover == Some(i) {
                                &hovered
                            } else {
                                &card
                            },
                        );
                    }
                    if (!navigation && !caption) || focus == Some(i) {
                        t.draw_rounded_rect(
                            &rr,
                            if focus == Some(i) { &accent } else { &border },
                            if focus == Some(i) { 2.0 } else { 1.0 },
                        );
                    }
                    if navigation && c.selected {
                        t.fill_rounded_rect(
                            &RoundedRect {
                                rect: Rect::from_xywh(
                                    c.bounds.left + 3.0,
                                    c.bounds.top + 11.0,
                                    3.0,
                                    18.0,
                                ),
                                radius_x: 1.5,
                                radius_y: 1.5,
                            },
                            &accent,
                        );
                    }
                    if material {
                        let r = c.bounds;
                        let inner = Rect::from_xywh(
                            r.left + 14.0,
                            r.top + 12.0,
                            r.right - r.left - 28.0,
                            72.0,
                        );
                        let tint = match c.action {
                            Action::Change(Event::Material(Backdrop::Acrylic)) => 0x5e819d,
                            Action::Change(Event::Material(Backdrop::Mica)) => 0x646b85,
                            _ => 0x7e718d,
                        };
                        let swatch = canvas_result(t.create_solid_brush(color(tint)))?;
                        t.fill_rounded_rect(
                            &RoundedRect {
                                rect: inner,
                                radius_x: 5.0,
                                radius_y: 5.0,
                            },
                            &swatch,
                        );
                        let glass = canvas_result(t.create_solid_brush(ColorF {
                            a: if matches!(
                                c.action,
                                Action::Change(Event::Material(Backdrop::Acrylic))
                            ) {
                                0.55
                            } else {
                                0.88
                            },
                            ..color(if dark { 0x242832 } else { 0xf1f5fb })
                        }))?;
                        let preview = Rect::from_xywh(
                            inner.left + 10.0,
                            inner.top + 10.0,
                            inner.right - inner.left - 20.0,
                            52.0,
                        );
                        t.fill_rounded_rect(
                            &RoundedRect {
                                rect: preview,
                                radius_x: 5.0,
                                radius_y: 5.0,
                            },
                            &glass,
                        );
                        t.fill_rect(
                            &Rect::from_xywh(preview.left + 10.0, preview.top + 10.0, 42.0, 3.0),
                            &muted,
                        );
                        for j in 0..3 {
                            t.fill_rounded_rect(
                                &RoundedRect {
                                    rect: Rect::from_xywh(
                                        preview.left + 8.0 + j as f32 * 20.0,
                                        preview.top + 29.0,
                                        16.0,
                                        16.0,
                                    ),
                                    radius_x: 4.0,
                                    radius_y: 4.0,
                                },
                                &accent,
                            );
                        }
                        let center = Vector2 {
                            x: r.left + 23.0,
                            y: r.top + 106.0,
                        };
                        t.draw_ellipse(
                            &Ellipse {
                                center,
                                radius_x: 8.0,
                                radius_y: 8.0,
                            },
                            if c.selected { &accent } else { &muted },
                            1.5,
                        );
                        if c.selected {
                            t.fill_ellipse(
                                &Ellipse {
                                    center,
                                    radius_x: 4.0,
                                    radius_y: 4.0,
                                },
                                &accent,
                            );
                        }
                        t.clipped_text(
                            &c.label,
                            &self.formats[1],
                            &Rect::from_xywh(
                                r.left + 42.0,
                                r.top + 89.0,
                                r.right - r.left - 50.0,
                                34.0,
                            ),
                            &ink,
                        );
                    } else if c.toggle {
                        let off = color(if dark { 0xf5f5f5 } else { 0x666666 });
                        let on = color(if dark { 0x202020 } else { 0xffffff });
                        let brush = canvas_result(t.create_solid_brush(ColorF {
                            r: off.r + (on.r - off.r) * progress,
                            g: off.g + (on.g - off.g) * progress,
                            b: off.b + (on.b - off.b) * progress,
                            a: 1.0,
                        }))?;
                        t.fill_ellipse(&toggle_thumb(c.bounds, progress), &brush);
                    } else if caption {
                        if hover == Some(i) && matches!(c.action, Action::Window(SC_CLOSE)) {
                            let red = canvas_result(t.create_solid_brush(color(0xc42b1c)))?;
                            t.fill_rect(&c.bounds, &red);
                        }
                        let white = canvas_result(t.create_solid_brush(color(0xffffff)))?;
                        let brush =
                            if hover == Some(i) && matches!(c.action, Action::Window(SC_CLOSE)) {
                                &white
                            } else {
                                &ink
                            };
                        if let Action::Window(command) = c.action {
                            let stroke = scale.round().max(1.0) / scale;
                            let snap = |v: f32| (v * scale).floor() / scale + stroke / 2.0;
                            for (a, b) in caption_lines(c.bounds, command) {
                                t.draw_line(
                                    Vector2 {
                                        x: snap(a.x),
                                        y: snap(a.y),
                                    },
                                    Vector2 {
                                        x: snap(b.x),
                                        y: snap(b.y),
                                    },
                                    brush,
                                    stroke,
                                );
                            }
                        }
                    } else {
                        let mut bounds = c.bounds;
                        if navigation {
                            bounds.left += 48.0;
                        }
                        t.clipped_text(
                            &c.label,
                            if navigation {
                                &self.formats[1]
                            } else {
                                &self.button_format
                            },
                            &bounds,
                            &ink,
                        );
                    }
                }
                for (r, text, size) in &s.text {
                    t.clipped_text(
                        text,
                        &self.formats[*size],
                        r,
                        if *size == 0 { &muted } else { &ink },
                    );
                }
                t.finish()
            })
        }
    }
}

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
    let painter = Painter::new().map_err(|e| e.to_string())?;
    let mut surface: Option<composition::Surface> = None;
    let mut page = 0;
    let mut selected = id;
    let mut hover = None;
    let mut focus = None;
    let mut keyboard_focus = false;
    let mut pressed = None;
    let mut toggle_motion = std::collections::HashMap::<usize, ToggleMotion>::new();
    let window = windows_window::Window::new("LucidPane 设置")
        .size(900, 520)
        .style(WS_OVERLAPPEDWINDOW)
        .ex_style(WS_EX_APPWINDOW | WS_EX_NOREDIRECTIONBITMAP)
        .on_message(move |raw, msg, wp, lp| {
            let hwnd = raw.cast();
            if msg == WM_NCCALCSIZE || msg == WM_NCPAINT {
                return Some(0);
            }
            if msg == WM_NCACTIVATE {
                return Some(1);
            }
            let Some(state) = weak.upgrade() else {
                return Some(0);
            };
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
                    info.ptMinTrackSize.y = (480.0 * scale) as i32;
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
                    | WM_TIMER
                    | WM_KEYDOWN
            ) {
                return None;
            }
            let available = if let Ok(state) = state.try_borrow() {
                panels = state.workspace.panels().to_vec();
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
            let scene = with_titlebar(
                scene(w, h - TITLE_HEIGHT, page, panel, panels.len(), appearance),
                w,
                unsafe { IsZoomed(hwnd) } != 0,
            );
            let mut activate = None;
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
                    for (i, control) in scene.controls.iter().enumerate().filter(|(_, c)| c.toggle)
                    {
                        let to = if control.selected { 1.0 } else { 0.0 };
                        let motion = toggle_motion.entry(i).or_insert(ToggleMotion {
                            from: to,
                            to,
                            started: now,
                        });
                        let value = motion.retarget(to, now, enabled != 0);
                        animating |= (value - to).abs() > 0.0001;
                        positions.insert(i, value);
                    }
                    unsafe {
                        if animating {
                            SetTimer(hwnd, TOGGLE_TIMER, USER_TIMER_MINIMUM, None);
                        } else {
                            KillTimer(hwnd, TOGGLE_TIMER);
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
                                surface = Some(composition::Surface::new(
                                    windows::Win32::Foundation::HWND(hwnd),
                                )?);
                                composition::Surface::disable_window_shadow(
                                    windows::Win32::Foundation::HWND(hwnd),
                                )?;
                            }
                            let surface = surface.as_mut().unwrap();
                            surface.theme(windows::Win32::Foundation::HWND(hwnd), dark);
                            surface.material(windows::Win32::Foundation::HWND(hwnd), appearance.1);
                            let target =
                                surface.begin_frame(bounds.right as u32, bounds.bottom as u32)?;
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
                        .position(|c| contains(&c.bounds, x, y));
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
                    if msg == WM_LBUTTONUP {
                        if pressed.take() == hit {
                            activate = hit;
                        }
                        unsafe {
                            ReleaseCapture();
                        }
                    }
                }
                WM_CAPTURECHANGED => {
                    pressed = None;
                }
                WM_KEYDOWN => {
                    if wp == VK_ESCAPE as usize {
                        unsafe {
                            PostMessageW(hwnd, WM_CLOSE, 0, 0);
                        }
                        return Some(0);
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
                    } else if wp == VK_SPACE as usize || wp == VK_RETURN as usize {
                        activate = focus;
                    }
                }
                _ => return None,
            }
            if let Some(c) = activate
                .filter(|_| available)
                .and_then(|i| scene.controls.get(i))
            {
                match &c.action {
                    Action::Window(command) => unsafe {
                        let command = if *command == SC_MAXIMIZE && IsZoomed(hwnd) != 0 {
                            SC_RESTORE
                        } else {
                            *command
                        };
                        PostMessageW(hwnd, WM_SYSCOMMAND, command as usize, 0);
                    },
                    Action::Page(value) => {
                        page = *value;
                        toggle_motion.clear();
                        focus = None;
                    }
                    Action::Previous => {
                        selected = panels[(at + panels.len() - 1) % panels.len()].id();
                    }
                    Action::Next => {
                        selected = panels[(at + 1) % panels.len()].id();
                    }
                    Action::Change(event) => {
                        if let Err(e) = handle(&state, selected, event.clone()) {
                            window::error(&e);
                        }
                    }
                }
            }
            if msg != WM_PAINT {
                unsafe {
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
            }
            Some(0)
        })
        .create()
        .map_err(|e| e.to_string())?;
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
            let height = ((520.0 * dpi).round() as i32).min(work.bottom - work.top);
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
        ShowWindow(window.hwnd().cast(), SW_SHOW);
        SetForegroundWindow(window.hwnd().cast());
    }
    state.borrow_mut().settings = Some(window);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn switch_thumb_stays_centered_with_equal_end_insets() {
        for (width, height) in [(42.0, 22.0), (48.0, 24.0)] {
            let bounds = Rect::from_xywh(100.0, 50.0, width, height);
            for position in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let thumb = toggle_thumb(bounds, position);
                assert_eq!(thumb.center.y, 50.0 + height / 2.0);
                assert!(thumb.center.x - thumb.radius_x >= bounds.left + 4.0);
                assert!(thumb.center.x + thumb.radius_x <= bounds.right - 4.0);
            }
            assert_eq!(
                toggle_thumb(bounds, 0.0).center.x - toggle_thumb(bounds, 0.0).radius_x,
                bounds.left + 4.0
            );
            assert_eq!(
                toggle_thumb(bounds, 1.0).center.x + toggle_thumb(bounds, 1.0).radius_x,
                bounds.right - 4.0
            );
        }
    }

    #[test]
    fn switch_motion_reverses_continuously_and_respects_disabled_animation() {
        let now = std::time::Instant::now();
        let mut motion = ToggleMotion {
            from: 0.0,
            to: 0.0,
            started: now,
        };
        assert_eq!(motion.retarget(1.0, now, true), 0.0);
        let halfway = now + std::time::Duration::from_millis(80);
        let value = motion.sample(halfway);
        assert!(value > 0.0 && value < 1.0);
        assert_eq!(motion.retarget(0.0, halfway, true), value);
        assert_eq!(
            motion.sample(halfway + std::time::Duration::from_millis(160)),
            0.0
        );
        assert_eq!(motion.retarget(1.0, halfway, false), 1.0);
    }

    #[test]
    fn custom_frame_keeps_caption_buttons_and_resize_edges_separate() {
        assert_eq!(frame_hit(80.0, 16.0, 1040.0, 760.0, false), HTCAPTION);
        assert_eq!(frame_hit(1020.0, 16.0, 1040.0, 760.0, false), HTCLIENT);
        assert_eq!(frame_hit(2.0, 2.0, 1040.0, 760.0, false), HTTOPLEFT);
        assert_eq!(
            frame_hit(1038.0, 758.0, 1040.0, 760.0, false),
            HTBOTTOMRIGHT
        );
        assert_eq!(frame_hit(80.0, 2.0, 1040.0, 760.0, true), HTCAPTION);
        let s = with_titlebar(
            scene(
                1040.0,
                728.0,
                0,
                None,
                0,
                (PanelTheme::Dark, Backdrop::Mica),
            ),
            1040.0,
            false,
        );
        assert_eq!(
            s.controls
                .iter()
                .filter(|c| matches!(c.action, Action::Window(_)))
                .count(),
            3
        );
    }
    #[test]
    fn settings_layout_and_rendering_at_multiple_scales() {
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let panel = Panel::new(
            PanelId::new(1),
            "工作与灵感",
            PanelSource::DesktopCollection,
            RectDip::default(),
        );
        let painter = Painter::new().unwrap();
        {
            let device = windows_canvas::GpuDevice::new_warp().unwrap();
            for scale in [1.0, 1.5, 2.0] {
                for page in 0..3 {
                    for dark in [false, true] {
                        let s = with_titlebar(
                            scene(
                                940.0,
                                620.0 - TITLE_HEIGHT,
                                page,
                                Some(&panel),
                                2,
                                (PanelTheme::System, Backdrop::Mica),
                            ),
                            940.0,
                            false,
                        );
                        for c in &s.controls {
                            assert!(
                                c.bounds.left >= 0.0
                                    && c.bounds.top >= 0.0
                                    && c.bounds.right <= 940.0
                                    && c.bounds.bottom <= 620.0
                            );
                            assert!(contains(
                                &c.bounds,
                                (c.bounds.left + c.bounds.right) / 2.0,
                                (c.bounds.top + c.bounds.bottom) / 2.0
                            ));
                        }
                        let width = (940.0 * scale) as u32;
                        let height = (620.0 * scale) as u32;
                        let bitmap =
                            super::super::canvas::Offscreen::new(&device, width, height).unwrap();
                        let target = bitmap.target.clone();
                        painter
                            .paint(
                                &target,
                                &s,
                                940.0,
                                620.0,
                                scale,
                                dark,
                                false,
                                None,
                                None,
                                &std::collections::HashMap::new(),
                            )
                            .unwrap();
                        let pixels = bitmap.pixels().unwrap();
                        assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
                        assert_eq!(pixels[0] < 128, dark);
                        if scale == 1.0 && page == 0 {
                            // Standalone raster for visual review, independent of the live desktop.
                            let mut bmp = vec![0u8; 54];
                            bmp[0..2].copy_from_slice(b"BM");
                            bmp[2..6].copy_from_slice(&(54 + pixels.len() as u32).to_le_bytes());
                            bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
                            bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
                            bmp[18..22].copy_from_slice(&(width as i32).to_le_bytes());
                            bmp[22..26].copy_from_slice(&(-(height as i32)).to_le_bytes());
                            bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
                            bmp[28..30].copy_from_slice(&32u16.to_le_bytes());
                            bmp.extend(pixels);
                            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                                .join("../target")
                                .join(if dark {
                                    "settings-dark.bmp"
                                } else {
                                    "settings-light.bmp"
                                });
                            std::fs::write(path, bmp).unwrap();
                        }
                        if page == 0 {
                            painter
                                .paint(
                                    &target,
                                    &s,
                                    940.0,
                                    620.0,
                                    scale,
                                    dark,
                                    true,
                                    None,
                                    None,
                                    &std::collections::HashMap::new(),
                                )
                                .unwrap();
                            let overlay = bitmap.pixels().unwrap();
                            assert_eq!(
                                overlay[3], 0,
                                "the native material must remain visible beneath the sidebar"
                            );
                            let card_at = (((190.0 * scale) as u32 * width
                                + (270.0 * scale) as u32)
                                * 4) as usize;
                            assert!(
                                overlay[card_at + 3] > 0 && overlay[card_at + 3] < 255,
                                "settings cards must retain material transparency"
                            );
                        }
                    }
                }
            }
            let empty = scene(
                960.0,
                650.0,
                1,
                None,
                0,
                (PanelTheme::System, Backdrop::Mica),
            );
            assert!(
                empty
                    .controls
                    .iter()
                    .any(|c| matches!(c.action, Action::Change(Event::New)))
            );
        }
    }
}
