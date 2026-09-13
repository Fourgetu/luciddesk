//! Shared rendering for the explicit settings control types.
use super::*;

pub(super) struct Painter {
    app_icon: super::super::assets::Pixels,
    formats: Vec<windows_canvas::TextFormat>,
    button_format: windows_canvas::TextFormat,
}
impl Painter {
    pub(super) fn new() -> windows::core::Result<Self> {
        use windows_canvas::{FontWeight, ParagraphAlignment, TextFormat, WordWrapping};
        let mut formats = vec![];
        for (i, size) in [12.0, 14.0, 20.0, 28.0, Style::ICON, Style::NAV_ICON]
            .iter()
            .enumerate()
        {
            let format = canvas_result(TextFormat::with_weight(
                if i >= 4 {
                    "Segoe Fluent Icons"
                } else {
                    super::super::assets::UI_FONT
                },
                *size,
                FontWeight(if i == 2 || i == 3 { 600 } else { 400 }),
            ))?
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .with_word_wrapping(WordWrapping::NoWrap);
            let format = if i >= 4 {
                format.with_alignment(windows_canvas::TextAlignment::Center)
            } else {
                format
            };
            super::super::canvas::ellipsis(&format)?;
            formats.push(format);
        }
        let button_format = canvas_result(TextFormat::new(super::super::assets::UI_FONT, 14.0))?
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .with_word_wrapping(WordWrapping::NoWrap)
            .with_alignment(windows_canvas::TextAlignment::Center);
        super::super::canvas::ellipsis(&button_format)?;
        Ok(Self {
            app_icon: {
                let icon = crate::app_icon::load(128, 128).map_err(|message| {
                    windows::core::Error::new(windows::Win32::Foundation::E_FAIL, message)
                })?;
                super::super::assets::icon_pixels(windows::Win32::UI::WindowsAndMessaging::HICON(
                    icon.0,
                ))?
            },
            formats,
            button_format,
        })
    }
    fn label_width(&self, text: &str) -> windows::core::Result<f32> {
        let layout = canvas_result(windows_canvas::TextLayout::new(
            text,
            &self.formats[1],
            4096.0,
            64.0,
        ))?;
        Ok(layout.metrics().width)
    }
    pub(super) fn paint(
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
            super::super::canvas::draw(t, scale, |t| {
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
                if let Some(bounds) = &s.app_icon {
                    let pixels = &self.app_icon;
                    let bitmap =
                        canvas_result(t.create_bitmap(&pixels.data, pixels.width, pixels.height))?;
                    t.draw_bitmap(&bitmap, bounds, 1.0);
                }
                for r in &s.separators {
                    t.fill_rect(r, &border);
                }
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
                    if !c.enabled && c.is_toggle() {
                        let r = c.bounds;
                        t.draw_rounded_rect(
                            &RoundedRect {
                                rect: r,
                                radius_x: 12.0,
                                radius_y: 12.0,
                            },
                            &border,
                            1.0,
                        );
                        t.fill_ellipse(
                            &toggle_thumb(r, if c.selected { 1.0 } else { 0.0 }),
                            &muted,
                        );
                        continue;
                    }
                    if let ControlKind::Slider(slider) = c.kind {
                        let r = c.bounds;
                        let cy = (r.top + r.bottom) / 2.0;
                        let left = r.left + Style::SLIDER_INSET;
                        let right = r.right - Style::SLIDER_INSET;
                        let value = slider.value;
                        let max = slider.max;
                        let cx = left + (right - left) * value / max;
                        let rail = RoundedRect {
                            rect: Rect::from_xywh(left, cy - 2.0, right - left, 4.0),
                            radius_x: 2.0,
                            radius_y: 2.0,
                        };
                        t.fill_rounded_rect(&rail, &border);
                        let (fill_start, fill_width) = if slider.centered {
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
                        let channel_brush = if let Some(channel) = slider.channel {
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
                    if matches!(c.kind, ControlKind::Combo) {
                        let rounded = RoundedRect {
                            rect: c.bounds,
                            radius_x: Style::COMBO_RADIUS,
                            radius_y: Style::COMBO_RADIUS,
                        };
                        t.fill_rounded_rect(
                            &rounded,
                            if c.enabled && hover == Some(i) {
                                &hovered
                            } else {
                                &card
                            },
                        );
                        t.draw_rounded_rect(&rounded, &border, 1.0);
                        if c.enabled {
                            t.draw_line(
                                Vector2 {
                                    x: c.bounds.left + 4.0,
                                    y: c.bounds.bottom - 1.0,
                                },
                                Vector2 {
                                    x: c.bounds.right - 4.0,
                                    y: c.bounds.bottom - 1.0,
                                },
                                if focus == Some(i) { &accent } else { &muted },
                                if focus == Some(i) { 2.0 } else { 1.0 },
                            );
                        }
                        let text = if c.enabled { &ink } else { &muted };
                        t.clipped_text(
                            &c.label,
                            &self.formats[1],
                            &Rect::from_xywh(
                                c.bounds.left + 12.0,
                                c.bounds.top,
                                c.bounds.right - c.bounds.left - 44.0,
                                Style::COMBO_HEIGHT,
                            ),
                            text,
                        );
                        t.clipped_text(
                            "\u{e70d}",
                            &self.formats[4],
                            &Rect::from_xywh(
                                c.bounds.right - 28.0,
                                c.bounds.top,
                                Style::ICON_SLOT,
                                Style::COMBO_HEIGHT,
                            ),
                            text,
                        );
                        continue;
                    }
                    let navigation = matches!(c.kind, ControlKind::Navigation);
                    let caption = matches!(c.kind, ControlKind::Caption);
                    let plain = c.kind.is_row();

                    let material = matches!(c.action, Action::Change(Event::Material(_)));
                    let rr = RoundedRect {
                        rect: c.bounds,
                        radius_x: if c.is_toggle() {
                            (c.bounds.bottom - c.bounds.top) / 2.0
                        } else if caption {
                            0.0
                        } else {
                            Style::RADIUS
                        },
                        radius_y: if c.is_toggle() {
                            (c.bounds.bottom - c.bounds.top) / 2.0
                        } else if caption {
                            0.0
                        } else {
                            Style::RADIUS
                        },
                    };
                    let progress =
                        toggles
                            .get(&i)
                            .copied()
                            .unwrap_or(if c.selected { 1.0 } else { 0.0 });
                    if c.is_toggle() {
                        let off = color(if dark { 0x383838 } else { 0xffffff });
                        let on = color(if dark { 0x76b9ed } else { 0x0067c0 });
                        let brush = canvas_result(t.create_solid_brush(ColorF {
                            r: off.r + (on.r - off.r) * progress,
                            g: off.g + (on.g - off.g) * progress,
                            b: off.b + (on.b - off.b) * progress,
                            a: 1.0,
                        }))?;
                        t.fill_rounded_rect(&rr, &brush);
                    } else if (!navigation && !caption && !plain) || c.selected || hover == Some(i)
                    {
                        t.fill_rounded_rect(
                            &rr,
                            if c.selected {
                                &selected
                            } else if hover == Some(i) {
                                &hovered
                            } else {
                                &card
                            },
                        );
                    }
                    if (!navigation && !caption && !plain) || focus == Some(i) {
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
                    } else if c.is_toggle() {
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
                            let glyph = match command {
                                SC_MINIMIZE => "\u{e921}",
                                SC_MAXIMIZE => "\u{e922}",
                                SC_RESTORE => "\u{e923}",
                                SC_CLOSE => "\u{e8bb}",
                                _ => "",
                            };
                            t.clipped_text(glyph, &self.formats[4], &c.bounds, brush);
                        }
                    } else {
                        let mut bounds = c.bounds;
                        if navigation {
                            bounds.left += Style::NAV_TEXT_INSET;
                        } else if plain {
                            bounds.left += Style::ROW_INSET;
                        }
                        if !navigation {
                            let back = c.kind.is_back();
                            let forward = matches!(c.kind, ControlKind::ForwardRow);
                            if back || forward {
                                // Center the compact back button's icon and label as one group.
                                let left = if back && !plain {
                                    controls::centered_icon_left(
                                        c.bounds,
                                        self.label_width(&c.label)?,
                                    )
                                } else if back {
                                    bounds.left
                                } else {
                                    c.bounds.right - 28.0
                                };
                                t.clipped_text(
                                    if back { "\u{e72b}" } else { "\u{e76c}" },
                                    &self.formats[4],
                                    &Rect::from_xywh(
                                        left,
                                        c.bounds.top,
                                        16.0,
                                        c.bounds.bottom - c.bounds.top,
                                    ),
                                    &ink,
                                );
                                if back {
                                    bounds.left = left + Style::ICON_SLOT + Style::ICON_GAP;
                                } else {
                                    bounds.right -= 40.0;
                                }
                            }
                        }
                        t.clipped_text(
                            &c.label,
                            if navigation || plain || c.kind.is_back() {
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
