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
    Foundation::{HWND, POINT, RECT},
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
    auto_hide: bool,
    theme: desktop_core::PanelTheme,
    folder: Option<bool>,
) -> i32 {
    let dark = super::theme::is_dark(theme);
    let mut rows = vec![
        entry(1, "新建分组", "", ""),
        entry(19, "新建文件夹面板…", "", ""),
        entry(23, "新建 Everything 搜索面板", "", ""),
        entry(3, "按名称排序", "", ""),
        entry(0, "", "", ""),
        entry(7, "自动收起", if auto_hide { "✓" } else { "" }, ""),
        entry(
            12,
            "始终置顶",
            if unsafe { GetWindowLongW(owner, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST != 0 {
                "✓"
            } else {
                ""
            },
            "",
        ),
        entry(0, "", "", ""),
        entry(11, "关闭分组", "", ""),
        entry(18, "设置", "", ""),
        entry(4, "退出 LucidPane", "", ""),
    ];
    if let Some(list) = folder {
        rows.splice(
            2..2,
            [
                entry(20, "打开源文件夹", "", ""),
                entry(21, "更换文件夹…", "", ""),
                entry(
                    22,
                    if list {
                        "切换为图标视图"
                    } else {
                        "切换为列表视图"
                    },
                    "",
                    "",
                ),
                entry(9, "刷新", "", "F5"),
            ],
        );
    }
    let scale = unsafe { GetDpiForWindow(owner) }.max(96) as f32 / 96.0;
    let mut animate = 1i32;
    unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut animate).cast(), 0);
    }
    let width = (216.0 * scale).round() as i32;
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
    let Ok(renderer) = Renderer::new() else {
        return 0;
    };
    let mut surface: Option<Surface> = None;
    let mut selected: Option<usize> = None;
    let mut down: Option<usize> = None;
    let mut fade_started: Option<Instant> = None;
    let mut fade: Option<super::animation::Fade> = None;
    let mut fade_finished = animate == 0;
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
                            let mut value = Surface::new_with_opacity(
                                windows::Win32::Foundation::HWND(hwnd),
                                if fade_finished { 1.0 } else { 0.0 },
                            )?;
                            value.theme(windows::Win32::Foundation::HWND(hwnd), dark);
                            value.material(
                                windows::Win32::Foundation::HWND(hwnd),
                                Backdrop::Acrylic,
                            );
                            surface = Some(value);
                        }
                        let value = surface.as_mut().unwrap();
                        let target = value.begin_frame(width as u32, height as u32)?;
                        renderer.paint_flyout(
                            &target,
                            width as u32,
                            height as u32,
                            scale,
                            &rows,
                            selected,
                            value.native,
                            dark,
                        )?;
                        value.end_frame()?;
                        // Start after the first frame is ready: device creation and
                        // rasterization must not consume the animation's time budget.
                        if !fade_finished && fade.is_none() {
                            match super::animation::Fade::new(std::time::Duration::from_millis(120))
                            {
                                Ok(animation) => {
                                    fade = Some(animation);
                                }
                                Err(error) => {
                                    eprintln!("Menu animation unavailable: {error}");
                                    fade_finished = true;
                                }
                            }
                        }
                        let started = *fade_started.get_or_insert_with(Instant::now);
                        let opacity = if fade_finished {
                            1.0
                        } else {
                            match fade.as_ref().unwrap().sample(started.elapsed()) {
                                Ok(opacity) => opacity,
                                Err(error) => {
                                    eprintln!("Menu animation failed: {error}");
                                    fade_finished = true;
                                    1.0
                                }
                            }
                        };
                        value.opacity(opacity)
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
                    VK_ESCAPE | VK_LEFT => {
                        done_handler.set(true);
                    }
                    VK_RETURN | VK_SPACE => {
                        activate = selected;
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
                        unsafe {
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                    }
                    _ => {}
                },
                WM_TIMER => {
                    if !fade_finished {
                        if let (Some(started), Some(value)) = (fade_started, surface.as_mut()) {
                            let opacity = match fade.as_ref().unwrap().sample(started.elapsed()) {
                                Ok(opacity) => opacity,
                                Err(error) => {
                                    eprintln!("Menu animation failed: {error}");
                                    1.0
                                }
                            };
                            // Commit the final opacity even if a busy UI thread skips
                            // every tick in the fade interval. No pixel readback/redraw.
                            if value.opacity(opacity).is_err() {
                                done_handler.set(true);
                            }
                            if opacity >= 1.0 {
                                fade_finished = true;
                                unsafe {
                                    KillTimer(hwnd, 1);
                                }
                            }
                        }
                    }
                }
                _ => return None,
            }
            if let Some(index) = activate {
                command_handler.set(rows[index].id);
                done_handler.set(true);
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
        // Prepare the complete first frame while the popup is still hidden.
        // Neither the content target nor the material starts at full opacity.
        SetWindowPos(hwnd, HWND_TOP, left, top, width, height, SWP_NOACTIVATE);
        SendMessageW(hwnd, WM_PAINT, 0, 0);
        if done.get() {
            return 0;
        }
        SetWindowPos(hwnd, HWND_TOP, left, top, width, height, SWP_SHOWWINDOW);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
        if animate != 0 {
            SetTimer(hwnd, 1, 16, None);
        }
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
            if message.hwnd == owner && message.message == WM_LBUTTONDOWN {
                let mut bounds = RECT::default();
                GetClientRect(owner, &raw mut bounds);
                let owner_scale = GetDpiForWindow(owner).max(96) as f32 / 96.0;
                let x = f32::from((message.lParam as u16).cast_signed()) / owner_scale;
                let y = f32::from(((message.lParam >> 16) as u16).cast_signed()) / owner_scale;
                if super::layout::header_button(bounds.right as f32 / owner_scale, x, y) == Some(1)
                {
                    // Consume the toggle click before the owner can arm another
                    // menu open on mouse-up. Other outside clicks still dispatch.
                    break;
                }
            }
            TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
        KillTimer(hwnd, 1);
    }
    drop(window);
    debug_assert_eq!(Rc::strong_count(&done), 1, "menu callback was not released");
    command.get()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::{ProcessStatus::*, Threading::*};

    #[test]
    #[ignore = "Opens a real menu; run in an interactive desktop session"]
    fn owner_menu_button_dismisses_without_rearming() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let clicks = Rc::new(Cell::new(0));
        let pulses = Rc::new(Cell::new(0));
        let observed_clicks = Rc::clone(&clicks);
        let observed_pulses = Rc::clone(&pulses);
        let owner = windows_window::Window::new("Menu toggle fixture")
            .style(WS_POPUP)
            .size(320, 240)
            .on_message(move |raw, msg, _, _| {
                let hwnd = raw.cast();
                match msg {
                    WM_DESTROY => Some(0),
                    WM_LBUTTONDOWN => {
                        observed_clicks.set(observed_clicks.get() + 1);
                        Some(0)
                    }
                    WM_TIMER => {
                        observed_pulses.set(observed_pulses.get() + 1);
                        unsafe {
                            if observed_pulses.get() == 1 {
                                let mut bounds = RECT::default();
                                GetClientRect(hwnd, &raw mut bounds);
                                let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
                                let x = ((super::super::layout::header_button_x(
                                    bounds.right as f32 / scale,
                                    1,
                                ) + 14.0)
                                    * scale) as isize;
                                let point = (((19.0 * scale) as isize) << 16) | x;
                                PostMessageW(hwnd, WM_LBUTTONDOWN, 0, point);
                            } else {
                                let popup = GetLastActivePopup(hwnd);
                                if popup != hwnd {
                                    PostMessageW(popup, WM_CLOSE, 0, 0);
                                }
                            }
                        }
                        Some(0)
                    }
                    _ => None,
                }
            })
            .create()
            .unwrap();
        let hwnd = owner.hwnd().cast();
        unsafe {
            SetTimer(hwnd, 99, 160, None);
        }
        assert_eq!(
            show(
                hwnd,
                POINT { x: 40, y: 40 },
                true,
                false,
                desktop_core::PanelTheme::Dark,
                None
            ),
            0
        );
        unsafe {
            KillTimer(hwnd, 99);
        }
        assert_eq!(
            pulses.get(),
            1,
            "toggle click must close without the watchdog"
        );
        assert_eq!(clicks.get(), 0, "owner must not arm another menu open");
    }

    #[test]
    #[ignore = "Opens real menus repeatedly; run alone in an interactive desktop session"]
    fn repeated_open_close_releases_resources() {
        unsafe extern "system" fn close(hwnd: HWND, _: isize) -> i32 {
            let mut title = [0u16; 32];
            unsafe {
                let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
                if String::from_utf16_lossy(&title[..len.max(0) as usize]) == "分组菜单" {
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                }
            }
            1
        }
        unsafe extern "system" fn tick(_: HWND, _: u32, _: usize, _: u32) {
            unsafe {
                EnumThreadWindows(GetCurrentThreadId(), Some(close), 0);
            }
        }
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let owner = windows_window::Window::new("Menu lifecycle fixture")
            .style(WS_POPUP)
            .size(320, 240)
            .on_message(|_, msg, _, _| (msg == WM_DESTROY).then_some(0))
            .create()
            .unwrap();
        let owner = owner.hwnd().cast();
        unsafe {
            assert_ne!(SetTimer(owner, 99, 160, Some(tick)), 0);
        }
        let mut samples = Vec::new();
        for batch in 0..6 {
            let started = Instant::now();
            for _ in 0..20 {
                assert_eq!(
                    show(
                        owner,
                        POINT { x: 40, y: 40 },
                        false,
                        false,
                        desktop_core::PanelTheme::Dark,
                        None
                    ),
                    0
                );
            }
            unsafe {
                let process = GetCurrentProcess();
                let mut memory = PROCESS_MEMORY_COUNTERS_EX::default();
                memory.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
                assert_ne!(
                    GetProcessMemoryInfo(process, (&raw mut memory).cast(), memory.cb),
                    0
                );
                let mut handles = 0;
                assert_ne!(GetProcessHandleCount(process, &raw mut handles), 0);
                let gui = (
                    GetGuiResources(process, GR_GDIOBJECTS),
                    GetGuiResources(process, GR_USEROBJECTS),
                );
                println!(
                    "menus={} private_kib={} handles={} gui={gui:?} batch_ms={}",
                    (batch + 1) * 20,
                    memory.PrivateUsage / 1024,
                    handles,
                    started.elapsed().as_millis()
                );
                samples.push((memory.PrivateUsage, handles, gui));
            }
        }
        unsafe {
            KillTimer(owner, 99);
        }
        let warm = samples[1];
        let last = samples[5];
        assert!(
            last.0 <= warm.0 + 16 * 1024 * 1024,
            "private bytes keep growing: {samples:?}"
        );
        assert!(
            last.1 <= warm.1 + 8,
            "process handles keep growing: {samples:?}"
        );
        // DWM/driver lazy initialization may add a small number of helper windows.
        assert!(
            last.2.0 <= warm.2.0 + 2 && last.2.1 <= warm.2.1 + 2,
            "GUI resources keep growing: {samples:?}"
        );
    }
}
