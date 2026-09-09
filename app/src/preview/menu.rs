//! WinUI-inspired composition flyout. Uses the same acrylic/content pipeline as panes.
#![allow(
    clippy::wildcard_imports,
    clippy::fn_params_excessive_bools,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
use super::{composition::Surface, render::Renderer};
use desktop_core::Backdrop;
use std::{cell::Cell, rc::Rc, time::Instant};
use windows_sys::Win32::{
    Foundation::{HWND, POINT},
    Graphics::Gdi::*,
    UI::{HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

pub struct Entry {
    pub id: i32,
    pub label: &'static str,
    pub icon: &'static str,
    pub trailing: &'static str,
}
pub const ROW_HEIGHT: f32 = 30.0;
pub fn row_top(rows: &[Entry], index: usize) -> f32 {
    4.0 + rows[..index]
        .iter()
        .map(|row| if row.id == 0 { 7.0 } else { ROW_HEIGHT })
        .sum::<f32>()
}
fn entry(id: i32, label: &'static str, icon: &'static str, trailing: &'static str) -> Entry {
    Entry {
        id,
        label,
        icon,
        trailing,
    }
}

pub fn show(
    owner: HWND,
    anchor: POINT,
    anchored: bool,
    collapsed: bool,
    backdrop: Backdrop,
    auto_hide: bool,
    desktop: bool,
    theme: desktop_core::PanelTheme,
) -> i32 {
    open(
        owner, anchor, anchored, collapsed, backdrop, auto_hide, 0, desktop, false, theme,
    )
}

pub fn show_hook(owner: HWND, anchor: POINT, backdrop: Backdrop)->i32 {
    open(owner,anchor,true,false,backdrop,false,0,false,true,desktop_core::PanelTheme::System)
}

fn open(
    owner: HWND,
    anchor: POINT,
    anchored: bool,
    collapsed: bool,
    backdrop: Backdrop,
    auto_hide: bool,
    submenu: u8,
    desktop: bool,
    hook: bool,
    theme: desktop_core::PanelTheme,
) -> i32 {
    let dark = super::theme::is_dark(theme);
    let rows = if desktop {
        vec![
            entry(1, "新建分组", "+", ""),
            entry(9, "刷新", "↻", "F5"),
            entry(0, "", "", ""),
            entry(4, "退出 LucidPane", "⏻", ""),
        ]
    } else if submenu == 1 {
        vec![
            entry(13, "云母 Alt", if backdrop == Backdrop::MicaAlt { "✓" } else { "" }, ""),
            entry(
                5,
                "亚克力",
                if backdrop == Backdrop::Acrylic {
                    "✓"
                } else {
                    ""
                },
                "",
            ),
            entry(
                6,
                "云母",
                if backdrop == Backdrop::Mica {
                    "✓"
                } else {
                    ""
                },
                "",
            ),
        ]
    } else if submenu == 2 {
        vec![
            entry(14, "跟随系统", if theme == desktop_core::PanelTheme::System { "✓" } else { "" }, ""),
            entry(15, "浅色", if theme == desktop_core::PanelTheme::Light { "✓" } else { "" }, ""),
            entry(16, "深色", if theme == desktop_core::PanelTheme::Dark { "✓" } else { "" }, ""),
        ]
    } else if hook {
        vec![entry(1,"新建分组","+",""),entry(10,"重命名分组","✎","F2"),
            entry(8,"背景材质","◈","›"),entry(0,"","",""),
            entry(11,"移除分组","−",""),entry(4,"退出 LucidPane","⏻","")]
    } else {
        vec![
            entry(1, "新建分组", "+", ""),
            entry(
                2,
                if collapsed {
                    "展开分组"
                } else {
                    "收起分组"
                },
                "⌃",
                "",
            ),
            entry(3, "按名称排序", "↕", ""),
            entry(0, "", "", ""),
            entry(7, "自动收起", if auto_hide { "✓" } else { "" }, ""),
            entry(12, "始终置顶", if unsafe { GetWindowLongW(owner, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST != 0 { "✓" } else { "" }, ""),
            entry(8, "背景材质", "◈", "›"),
            entry(17, "外观主题", "◐", "›"),
            entry(0, "", "", ""),
            entry(4, "退出 LucidPane", "⏻", ""),
        ]
    };
    let scale = unsafe { GetDpiForWindow(owner) }.max(96) as f32 / 96.0;
    let mut animate = 1i32;
    unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut animate).cast(), 0);
    }
    let width = ((if submenu != 0 { 180.0 } else { 216.0 }) * scale).round() as i32;
    let height = ((row_top(&rows, rows.len()) + 4.0) * scale).round() as i32;
    let mut monitor = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetMonitorInfoW(
            MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST),
            &raw mut monitor,
        );
    }
    let work = monitor.rcWork;
    let left = (if anchored { anchor.x - width } else { anchor.x })
        .clamp(work.left, (work.right - width).max(work.left));
    let top = anchor
        .y
        .clamp(work.top, (work.bottom - height).max(work.top));
    let done = Rc::new(Cell::new(false));
    let command = Rc::new(Cell::new(0));
    let done_handler = Rc::clone(&done);
    let command_handler = Rc::clone(&command);
    let Ok(mut renderer) = Renderer::new() else {
        return 0;
    };
    let mut surface: Option<Surface> = None;
    let mut selected: Option<usize> = None;
    let mut down: Option<usize> = None;
    let mut hovered_at = Instant::now();
    let mut opened_submenu = false;
    let started = Instant::now();
    let window = windows_window::Window::new("分组菜单")
        .style(WS_POPUP)
        .ex_style(WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP)
        .size(width, height)
        .on_message(move |raw, message, wparam, lparam| {
            let hwnd = raw.cast();
            let mut activate = None;
            match message {
                WM_DESTROY | WM_ERASEBKGND | WM_NCCALCSIZE => return Some(0),
                WM_CLOSE => {
                    done_handler.set(true);
                    return Some(0);
                }
                WM_ACTIVATE if wparam & 0xffff == WA_INACTIVE as usize => {
                    done_handler.set(true);
                    return Some(0);
                }
                WM_PAINT => {
                    unsafe {
                        let mut paint = PAINTSTRUCT::default();
                        BeginPaint(hwnd, &raw mut paint);
                        EndPaint(hwnd, &raw const paint);
                    }
                    let result = (|| -> windows::core::Result<()> {
                        if surface.is_none() {
                            let mut value = Surface::new(windows::Win32::Foundation::HWND(hwnd))?;
                            value.theme(windows::Win32::Foundation::HWND(hwnd), dark);
                            value.material(
                                windows::Win32::Foundation::HWND(hwnd),
                                Backdrop::Acrylic,
                            );
                            surface = Some(value);
                        }
                        let value = surface.as_mut().unwrap();
                        let pixels = renderer.flyout(
                            width as u32,
                            height as u32,
                            scale,
                            &rows,
                            selected,
                            value.native,
                            dark,
                        )?;
                        value.present(width as u32, height as u32, &pixels)?;
                        value.opacity(if animate != 0 {
                            (started.elapsed().as_secs_f32() / 0.12).min(1.0)
                        } else {
                            1.0
                        })
                    })();
                    if result.is_err() {
                        done_handler.set(true);
                    }
                    return Some(0);
                }
                WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
                    let x = f32::from((lparam as u16).cast_signed()) / scale;
                    let y = f32::from(((lparam >> 16) as u16).cast_signed()) / scale;
                    let hit = rows.iter().enumerate().find_map(|(index, row)| {
                        let top = row_top(&rows, index);
                        (row.id != 0
                            && x >= 5.0
                            && x < width as f32 / scale - 5.0
                            && y >= top
                            && y < top + ROW_HEIGHT)
                            .then_some(index)
                    });
                    if selected != hit {
                        selected = hit;
                        hovered_at = Instant::now();
                        opened_submenu = false;
                        unsafe {
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                    }
                    if message == WM_LBUTTONDOWN {
                        down = hit;
                    }
                    if message == WM_LBUTTONUP && down.take() == hit {
                        activate = hit;
                    }
                }
                WM_KEYDOWN => match wparam as u16 {
                    VK_ESCAPE | VK_LEFT => done_handler.set(true),
                    VK_RETURN | VK_SPACE | VK_RIGHT => {
                        activate = selected
                            .filter(|&index| wparam as u16 != VK_RIGHT || matches!(rows[index].id, 8 | 17));
                    }
                    VK_UP | VK_DOWN => {
                        let indices: Vec<_> = rows
                            .iter()
                            .enumerate()
                            .filter_map(|(i, row)| (row.id != 0).then_some(i))
                            .collect();
                        let current =
                            selected.and_then(|index| indices.iter().position(|&i| i == index));
                        let next = if wparam as u16 == VK_DOWN {
                            current.map_or(0, |i| (i + 1) % indices.len())
                        } else {
                            current.map_or(indices.len() - 1, |i| {
                                (i + indices.len() - 1) % indices.len()
                            })
                        };
                        selected = Some(indices[next]);
                        opened_submenu = true;
                        unsafe {
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                    }
                    _ => {}
                },
                WM_TIMER => {
                    if started.elapsed().as_millis() >= 150 {
                        unsafe {
                            SetTimer(hwnd, 1, 50, None);
                        }
                    }
                    if started.elapsed().as_millis() < 150 {
                        unsafe {
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                    }
                    if !opened_submenu && hovered_at.elapsed().as_millis() >= 250 {
                        activate = selected.filter(|&index| matches!(rows[index].id, 8 | 17));
                    }
                }
                _ => return None,
            }
            if let Some(index) = activate {
                if matches!(rows[index].id, 8 | 17) {
                    opened_submenu = true;
                    let child_width = (180.0 * scale).round() as i32;
                    let x = if left + width + child_width <= work.right {
                        left + width + 4
                    } else {
                        left - child_width - 4
                    };
                    let result = open(
                        hwnd,
                        POINT {
                            x,
                            y: top + (row_top(&rows, index) * scale) as i32,
                        },
                        false,
                        collapsed,
                        backdrop,
                        auto_hide,
                        if rows[index].id == 8 { 1 } else { 2 },
                        false,
                        hook,
                        theme,
                    );
                    if result != 0 {
                        command_handler.set(result);
                        done_handler.set(true);
                    } else if unsafe { GetForegroundWindow() } != hwnd {
                        done_handler.set(true);
                    }
                } else {
                    command_handler.set(rows[index].id);
                    done_handler.set(true);
                }
            }
            Some(0)
        })
        .create();
    let Ok(window) = window else {
        return 0;
    };
    let hwnd = window.hwnd().cast();
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner as isize);
        SetWindowPos(hwnd, HWND_TOP, left, top, width, height, SWP_SHOWWINDOW);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
        SetTimer(hwnd, 1, 16, None);
        InvalidateRect(hwnd, std::ptr::null(), 0);
        let mut message = MSG::default();
        while !done.get() {
            let status = GetMessageW(&raw mut message, std::ptr::null_mut(), 0, 0);
            if status <= 0 {
                if status == 0 {
                    PostQuitMessage(i32::try_from(message.wParam).unwrap_or_default());
                }
                break;
            }
            TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
        KillTimer(hwnd, 1);
    }
    drop(window);
    command.get()
}
