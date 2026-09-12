//! Modeless settings, drawn with the same native composition pipeline as panes.
#![allow(
    clippy::wildcard_imports,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
use windows_canvas::{ColorF, Ellipse, Rect, RoundedRect, Vector2};

use super::native_graphics::canvas_result;
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

struct PendingReveal {
    started: std::time::Instant,
    ready: Box<dyn Fn() -> bool>,
    fade: bool,
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
    ProjectHome,
    Window(u32),
    Page(usize),
    Previous,
    Next,
    Change(Event),
    Radius(u8),
    Opacity(u8),
    Strength(u8),
    StrengthReset,
    Channel(u8, u8),
    ColorPreset(u32),
    SolidColor,
    SolidReset,
    StyleInput(bool),
    PeekEnable,
    PeekBrowse,
    PeekDetect,
    PeekShortcut,
    SearchShortcut,
    SearchReset,
    PeekReset,
    EverythingAutoStart,
    EverythingBrowse,
    EverythingDetect,
    EverythingLaunch,
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
    previews: Vec<(Rect, u32, f32)>,
}

fn radius_from_pointer(bounds: Rect, x: f32) -> u8 {
    let progress =
        ((x - bounds.left - 8.0) / (bounds.right - bounds.left - 16.0).max(1.0)).clamp(0.0, 1.0);
    (progress * f32::from(desktop_core::PaneOptions::MAX_CORNER_RADIUS)).round() as u8
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
                for (r, rgb, opacity) in &s.previews {
                    let light = canvas_result(t.create_solid_brush(color(if dark {
                        0x41454b
                    } else {
                        0xffffff
                    })))?;
                    let shade = canvas_result(t.create_solid_brush(color(if dark {
                        0x30343a
                    } else {
                        0xdfe3e8
                    })))?;
                    let tint = canvas_result(t.create_solid_brush(ColorF {
                        a: *opacity,
                        ..color(*rgb)
                    }))?;
                    let cols = ((r.right - r.left) / 12.0).ceil() as usize;
                    let rows = ((r.bottom - r.top) / 12.0).ceil() as usize;
                    for row in 0..rows {
                        for col in 0..cols {
                            let x = r.left + col as f32 * 12.0;
                            let y = r.top + row as f32 * 12.0;
                            t.fill_rect(
                                &Rect::from_xywh(
                                    x,
                                    y,
                                    12.0f32.min(r.right - x),
                                    12.0f32.min(r.bottom - y),
                                ),
                                if (row + col) % 2 == 0 { &light } else { &shade },
                            );
                        }
                    }
                    t.fill_rect(r, &tint);
                    t.draw_rounded_rect(
                        &RoundedRect {
                            rect: *r,
                            radius_x: 0.0,
                            radius_y: 0.0,
                        },
                        &border,
                        1.0,
                    );
                    for i in 0..if r.bottom - r.top >= 64.0 { 3 } else { 0 } {
                        t.fill_rounded_rect(
                            &RoundedRect {
                                rect: Rect::from_xywh(
                                    r.left + 16.0 + i as f32 * 42.0,
                                    r.top + 18.0,
                                    26.0,
                                    26.0,
                                ),
                                radius_x: 5.0,
                                radius_y: 5.0,
                            },
                            &accent,
                        );
                        t.fill_rect(
                            &Rect::from_xywh(
                                r.left + 18.0 + i as f32 * 42.0,
                                r.top + 51.0,
                                22.0,
                                2.0,
                            ),
                            &ink,
                        );
                    }
                }
                for (i, c) in s.controls.iter().enumerate() {
                    if let Action::Radius(value)
                    | Action::Opacity(value)
                    | Action::Strength(value)
                    | Action::Channel(_, value) = c.action
                    {
                        let r = c.bounds;
                        let cy = (r.top + r.bottom) / 2.0;
                        let left = r.left + 8.0;
                        let right = r.right - 8.0;
                        let max = match c.action {
                            Action::Opacity(_) | Action::Strength(_) => 100.0,
                            Action::Channel(_, _) => 255.0,
                            _ => 24.0,
                        };
                        let cx = left + (right - left) * f32::from(value) / max;
                        let rail = RoundedRect {
                            rect: Rect::from_xywh(left, cy - 2.0, right - left, 4.0),
                            radius_x: 2.0,
                            radius_y: 2.0,
                        };
                        t.fill_rounded_rect(&rail, &border);
                        let (fill_start, fill_width) = if matches!(c.action, Action::Strength(_)) {
                            let middle = (left + right) * 0.5;
                            t.fill_rect(
                                &Rect::from_xywh(middle - 0.5, cy - 5.0, 1.0, 10.0),
                                &muted,
                            );
                            (cx.min(middle), (cx - middle).abs())
                        } else {
                            (left, (cx - left).max(0.0))
                        };
                        let filled = RoundedRect {
                            rect: Rect::from_xywh(fill_start, cy - 2.0, fill_width, 4.0),
                            radius_x: 2.0,
                            radius_y: 2.0,
                        };
                        let channel_brush = if let Action::Channel(channel, _) = c.action {
                            Some(canvas_result(t.create_solid_brush(color(match channel {
                                0 => {
                                    if dark {
                                        0xef8d8d
                                    } else {
                                        0xb83d42
                                    }
                                }
                                1 => {
                                    if dark {
                                        0x8dccaa
                                    } else {
                                        0x287b50
                                    }
                                }
                                _ => {
                                    if dark {
                                        0x88baf0
                                    } else {
                                        0x266eae
                                    }
                                }
                            })))?)
                        } else {
                            None
                        };
                        let slider_ink = channel_brush.as_ref().unwrap_or(&accent);
                        t.fill_rounded_rect(&filled, slider_ink);
                        t.fill_ellipse(
                            &Ellipse {
                                center: Vector2 { x: cx, y: cy },
                                radius_x: 8.0,
                                radius_y: 8.0,
                            },
                            &card,
                        );
                        t.draw_ellipse(
                            &Ellipse {
                                center: Vector2 { x: cx, y: cy },
                                radius_x: 8.0,
                                radius_y: 8.0,
                            },
                            &border,
                            1.0,
                        );
                        t.fill_ellipse(
                            &Ellipse {
                                center: Vector2 { x: cx, y: cy },
                                radius_x: 5.0,
                                radius_y: 5.0,
                            },
                            slider_ink,
                        );
                        if focus == Some(i) {
                            t.draw_rounded_rect(
                                &RoundedRect {
                                    rect: r,
                                    radius_x: 5.0,
                                    radius_y: 5.0,
                                },
                                &accent,
                                2.0,
                            );
                        }
                        continue;
                    }
                    let navigation = matches!(c.action, Action::Page(_)) && c.bounds.left < 224.0;
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
                            if focus == Some(i) || c.selected {
                                &accent
                            } else {
                                &border
                            },
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
                    if let Action::ColorPreset(value) = c.action {
                        if c.selected {
                            t.draw_rounded_rect(&rr, &accent, 2.0);
                        }
                        let chip = canvas_result(t.create_solid_brush(color(value)))?;
                        t.fill_rounded_rect(
                            &RoundedRect {
                                rect: Rect::from_xywh(
                                    c.bounds.left + 5.0,
                                    c.bounds.top + 5.0,
                                    c.bounds.right - c.bounds.left - 10.0,
                                    c.bounds.bottom - c.bounds.top - 10.0,
                                ),
                                radius_x: 3.0,
                                radius_y: 3.0,
                            },
                            &chip,
                        );
                    }
                    if material {
                        let r = c.bounds;
                        let inner = Rect::from_xywh(
                            r.left + 14.0,
                            r.top + 12.0,
                            r.right - r.left - 28.0,
                            56.0,
                        );
                        let tint = match c.action {
                            Action::Change(Event::Material(Backdrop::Acrylic)) => 0x5e819d,
                            Action::Change(Event::Material(Backdrop::Mica)) => 0x646b85,
                            Action::Change(Event::Material(Backdrop::Solid { color, .. })) => color,
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
                            inner.left + 8.0,
                            inner.top + 8.0,
                            inner.right - inner.left - 16.0,
                            40.0,
                        );
                        t.fill_rounded_rect(
                            &RoundedRect {
                                rect: preview,
                                radius_x: 5.0,
                                radius_y: 5.0,
                            },
                            &glass,
                        );
                        let center_x = (preview.left + preview.right) * 0.5;
                        t.fill_rounded_rect(
                            &RoundedRect {
                                rect: Rect::from_xywh(
                                    center_x - 14.0,
                                    preview.top + 8.0,
                                    28.0,
                                    2.0,
                                ),
                                radius_x: 1.0,
                                radius_y: 1.0,
                            },
                            &muted,
                        );
                        for j in 0..3 {
                            t.fill_rounded_rect(
                                &RoundedRect {
                                    rect: Rect::from_xywh(
                                        preview.left + 8.0 + j as f32 * 18.0,
                                        preview.top + 20.0,
                                        12.0,
                                        12.0,
                                    ),
                                    radius_x: 3.0,
                                    radius_y: 3.0,
                                },
                                &accent,
                            );
                        }
                        let center = Vector2 {
                            x: r.left + 23.0,
                            y: r.bottom - 18.0,
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
                                r.bottom - 35.0,
                                r.right - r.left - 50.0,
                                30.0,
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
    let mut options = state.borrow().workspace.pane_options();
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
    let mut material_original = None;
    let mut style_input: Option<(bool, String)> = None;
    let mut cached_scene = None;
    let mut scene_key = None;
    let mut desktop_status = String::new();
    let mut backup_status = String::new();
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
                backup_status = runtime::backup_status(&state);
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
                (desktop_status.clone(), backup_status.clone()),
            );
            let scene_changed = snapshot_changed || scene_key.as_ref() != Some(&key);
            if scene_changed {
                let mut body = scene(
                        w,
                        h - TITLE_HEIGHT,
                        page,
                        panel,
                        panels.len(),
                        search_visible,
                        appearance,
                        options,
                    );
                if page == 6 { body.text(Rect::from_xywh(248.0, 408.0, w - 282.0, 48.0), &backup_status, 0); }
                if page == 5 {
                    body.text(Rect::from_xywh(264.0, 352.0, w - 304.0, 40.0), &desktop_status, 0);
                    body.button(Rect::from_xywh(264.0, 398.0, 150.0, 34.0), "重新连接桌面", Action::Change(Event::RetryDesktop), false);
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
                    if let Some(control) = pressed.and_then(|i| scene.controls.get(i)) {
                        if let Action::Channel(channel, _) = control.action {
                            let fraction = ((x - control.bounds.left - 8.0) / (control.bounds.right - control.bounds.left - 16.0).max(1.0)).clamp(0.0, 1.0);
                            channel_change = Some((channel, (fraction * 255.0).round() as u8));
                        }
                        if matches!(control.action, Action::Strength(_)) {
                            strength_change = Some((((x - control.bounds.left - 8.0) / (control.bounds.right - control.bounds.left - 16.0).max(1.0)).clamp(0.0, 1.0) * 100.0).round() as u8);
                        }
                        if matches!(control.action, Action::Opacity(_)) {
                            opacity_change = Some((((x - control.bounds.left - 8.0) / (control.bounds.right - control.bounds.left - 16.0).max(1.0)).clamp(0.0, 1.0) * 100.0).round() as u8);
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
                        if let Action::Radius(value) = control.action {
                            radius_change = match wp as u16 {
                                VK_LEFT => Some(value.saturating_sub(1)),
                                VK_RIGHT => Some((value + 1).min(24)),
                                VK_HOME => Some(0),
                                VK_END => Some(24),
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
            {
                match &c.action {
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
                    Action::ProjectHome => {
                        if let Err(error) = desktop_shell::open_shell_identity(hwnd as isize, &desktop_core::ShellIdentity::Namespace {
                            parsing_name: "https://git.bbkingdom.fun:30443/yuchen95/LucidPane".into(),
                        }) { window::error(&error.to_string()); }
                    }
                    Action::EverythingAutoStart | Action::EverythingBrowse | Action::EverythingDetect | Action::EverythingLaunch => {
                        let mut value = everything_settings::settings();
                        let result = (|| -> Result<(), String> {
                            match c.action {
                                Action::EverythingAutoStart => value.auto_start = !value.auto_start,
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
                    Action::PeekEnable | Action::PeekBrowse | Action::PeekDetect | Action::PeekReset => {
                        let mut value = peek::settings();
                        let result = (|| -> Result<(), String> {
                            match c.action {
                                Action::PeekEnable => value.enabled = !value.enabled,
                                Action::PeekBrowse => {
                                    let Some(path) = peek::browse(hwnd as isize)? else { return Ok(()); };
                                    value.path = path;
                                }
                                Action::PeekDetect => { value.path.clear(); }
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
                    Action::Radius(_) | Action::Opacity(_) | Action::Strength(_) | Action::Channel(_, _) => {}
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
        // Prepare material and content at the final size before exposing the HWND.
        SendMessageW(hwnd, WM_PAINT, 0, 0);
        // Cloaking keeps the visible HWND in DWM composition without exposing
        // a partial frame. A hidden HWND cannot prepare host backdrop sampling.
        if cloak(hwnd, true).is_ok() {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            PostMessageW(hwnd, PREPARE_REVEAL, 0, 0);
        } else {
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
        }
    }
    state.borrow_mut().settings = Some(window);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_opacity_is_independent_and_rgb_preserves_other_channels() {
        for dark in [false, true] {
            for color in [0x123456, 0x7d4441, 0xffffff] {
                for opacity in [0.0, 0.5, 1.0] {
                    assert_eq!(
                        settings_backdrop(Backdrop::Solid { color, opacity }, dark),
                        Backdrop::Solid {
                            color: if dark { 0x202020 } else { 0xf3f3f3 },
                            opacity: 1.0
                        }
                    );
                }
            }
        }
        assert_eq!(
            settings_backdrop(Backdrop::Acrylic, true),
            Backdrop::Acrylic
        );
        for base in [Backdrop::Acrylic, Backdrop::Mica, Backdrop::MicaAlt] {
            for strength in [0, 50, 100] {
                assert_eq!(settings_backdrop(base.with_strength(strength), true), base);
            }
        }
        assert_eq!(color_channel(0x123456, 0, 255), 0xff3456);
        assert_eq!(color_channel(0x123456, 1, 0), 0x120056);
        assert_eq!(color_channel(0x123456, 2, 255), 0x1234ff);
        let picker = with_titlebar(
            scene(
                800.0,
                480.0 - TITLE_HEIGHT,
                7,
                None,
                0,
                false,
                (
                    PanelTheme::Dark,
                    Backdrop::Solid {
                        color: 0x123456,
                        opacity: 0.5,
                    },
                ),
                Default::default(),
            ),
            800.0,
            false,
        );
        assert!(
            picker
                .controls
                .iter()
                .all(|c| c.bounds.right <= 800.0 && c.bounds.bottom <= 480.0)
        );
        assert_eq!(
            picker
                .controls
                .iter()
                .filter(|c| matches!(c.action, Action::Channel(_, _)))
                .count(),
            3
        );
        assert_eq!(picker.previews.len(), 1);
    }

    #[test]
    fn solid_controls_fit_minimum_settings_size() {
        let s = with_titlebar(
            scene(
                800.0,
                480.0 - TITLE_HEIGHT,
                0,
                None,
                0,
                false,
                (
                    PanelTheme::Dark,
                    Backdrop::Solid {
                        color: 0x24364b,
                        opacity: 0.5,
                    },
                ),
                Default::default(),
            ),
            800.0,
            false,
        );
        for control in &s.controls {
            assert!(control.bounds.right <= 800.0 && control.bounds.bottom <= 480.0);
        }
        assert!(
            s.controls
                .iter()
                .any(|c| matches!(c.action, Action::Opacity(50)))
        );
    }

    #[test]
    fn solid_inputs_validate_color_and_opacity_without_changing_other_channels() {
        let solid = Backdrop::Solid {
            color: 0x123456,
            opacity: 0.85,
        };
        assert_eq!(
            edited_solid(solid, false, "#A1b2C3"),
            Some(Backdrop::Solid {
                color: 0xa1b2c3,
                opacity: 0.85
            })
        );
        for value in ["0", "50%", "100"] {
            assert!(edited_solid(solid, true, value).is_some());
        }
        for value in ["101", "-1", "NaN", ""] {
            assert!(edited_solid(solid, true, value).is_none());
        }
        for value in ["123", "GG0000", "1234567"] {
            assert!(edited_solid(solid, false, value).is_none());
        }
    }

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
                false,
                (PanelTheme::Dark, Backdrop::Mica),
                desktop_core::PaneOptions::default(),
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
        let panel = Panel::new(PanelId::new(1), "工作与灵感", RectDip::default());
        let painter = Painter::new().unwrap();
        {
            let device = windows_canvas::GpuDevice::new_warp().unwrap();
            for scale in [1.0, 1.5, 2.0] {
                for page in 0..8 {
                    for dark in [false, true] {
                        let s = with_titlebar(
                            scene(
                                940.0,
                                620.0 - TITLE_HEIGHT,
                                page,
                                Some(&panel),
                                2,
                                true,
                                (
                                    PanelTheme::System,
                                    if page == 0 {
                                        Backdrop::Acrylic.with_strength(65)
                                    } else if page == 7 {
                                        Backdrop::Solid {
                                            color: 0x24364b,
                                            opacity: 0.85,
                                        }
                                    } else {
                                        Backdrop::Mica
                                    },
                                ),
                                desktop_core::PaneOptions::default(),
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
                        if scale == 1.0 && (page <= 1 || page == 5 || page == 7) {
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
                                .join(match (page, dark) {
                                    (7, true) => "settings-colors-dark.bmp",
                                    (7, false) => "settings-colors-light.bmp",
                                    (5, true) => "settings-about-dark.bmp",
                                    (5, false) => "settings-about-light.bmp",
                                    (1, true) => "settings-pane-dark.bmp",
                                    (1, false) => "settings-pane-light.bmp",
                                    (_, true) => "settings-dark.bmp",
                                    (_, false) => "settings-light.bmp",
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
                2,
                None,
                0,
                false,
                (PanelTheme::System, Backdrop::Mica),
                desktop_core::PaneOptions::default(),
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
