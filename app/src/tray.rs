//! Notification-area entry, owned by the same UI thread as the panes.
use std::{ptr::null_mut, rc::Rc};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::{Shell::*, WindowsAndMessaging::*},
};
use windows_window::Window;

const CALLBACK: u32 = WM_APP + 80;
const ICON_ID: u32 = 1;
// Shell headers define this macro as NIN_SELECT | NINF_KEY.
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;

#[derive(Clone, Copy)]
pub enum Action {
    Settings,
    Show,
    New,
    NewFolder,
    NewSearch,
    Exit,
}

struct Icon(HICON);
impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            DestroyIcon(self.0);
        }
    }
}

pub struct Tray {
    window: Window,
    _icon: Rc<Icon>,
}
impl Tray {
    pub fn new(mut action: impl FnMut(Action) + 'static) -> Result<Self, String> {
        let icon = Rc::new(make_icon()?);
        let callback_icon = Rc::clone(&icon);
        let recreated = unsafe { RegisterWindowMessageW(windows_sys::w!("TaskbarCreated")) };
        if recreated == 0 {
            return Err("无法注册托盘恢复消息".into());
        }
        let window = Window::new("LucidPane Tray")
            .size(1, 1)
            .style(WS_POPUP)
            .ex_style(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)
            .on_message(move |raw, message, wp, lp| {
                let hwnd = raw.cast();
                if message == recreated {
                    // Explorer discards all notification icons when rebuilding
                    // its taskbar. This re-adds ours without restarting Explorer.
                    let _ = add(hwnd, callback_icon.0);
                    return Some(0);
                }
                if message != CALLBACK {
                    return None;
                }
                if ((lp as u32) >> 16) != ICON_ID {
                    return Some(0);
                }
                match lp as u32 & 0xffff {
                    NIN_SELECT | NIN_KEYSELECT => action(Action::Show),
                    WM_CONTEXTMENU => {
                        if let Some(command) = menu(hwnd, wp) {
                            action(command);
                        }
                    }
                    _ => {}
                }
                Some(0)
            })
            .create()
            .map_err(|e| e.to_string())?;
        unsafe {
            ShowWindow(window.hwnd().cast(), SW_HIDE);
        }
        add(window.hwnd().cast(), icon.0)?;
        Ok(Self {
            window,
            _icon: icon,
        })
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data(self.window.hwnd().cast(), self._icon.0));
        }
    }
}

fn data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: CALLBACK,
        hIcon: icon,
        ..Default::default()
    };
    for (slot, ch) in data
        .szTip
        .iter_mut()
        .zip("LucidPane · 桌面分组".encode_utf16())
    {
        *slot = ch;
    }
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    data
}
fn add(hwnd: HWND, icon: HICON) -> Result<(), String> {
    let data = data(hwnd, icon);
    unsafe {
        if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
            return Err("无法添加 LucidPane 托盘图标".into());
        }
        if Shell_NotifyIconW(NIM_SETVERSION, &data) == 0 {
            Shell_NotifyIconW(NIM_DELETE, &data);
            return Err("无法初始化托盘交互".into());
        }
    }
    Ok(())
}

fn menu(hwnd: HWND, anchor: usize) -> Option<Action> {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return None;
        }
        AppendMenuW(menu, MF_STRING, 1, windows_sys::w!("显示分组"));
        AppendMenuW(menu, MF_STRING, 2, windows_sys::w!("新建分组"));
        AppendMenuW(menu, MF_STRING, 5, windows_sys::w!("新建文件夹面板…"));
        AppendMenuW(
            menu,
            MF_STRING,
            6,
            windows_sys::w!("新建 Everything 搜索面板"),
        );
        AppendMenuW(menu, MF_STRING, 4, windows_sys::w!("设置"));
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, 3, windows_sys::w!("退出 LucidPane"));
        SetMenuDefaultItem(menu, 1, 0);
        SetForegroundWindow(hwnd);
        // Version 4 packs signed screen coordinates in wParam.
        let command = TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            (anchor as u16 as i16).into(),
            ((anchor >> 16) as u16 as i16).into(),
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        PostMessageW(hwnd, WM_NULL, 0, 0);
        Shell_NotifyIconW(NIM_SETFOCUS, &data(hwnd, null_mut()));
        match command {
            1 => Some(Action::Show),
            2 => Some(Action::New),
            3 => Some(Action::Exit),
            4 => Some(Action::Settings),
            5 => Some(Action::NewFolder),
            6 => Some(Action::NewSearch),
            _ => None,
        }
    }
}

fn make_icon() -> Result<Icon, String> {
    // Small, opaque blue pane tile with two white groups. No file or Shell icon
    // extraction is needed at startup; the owned HICON lives as long as the tray.
    let mut pixels = [0u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let white = ((6..14).contains(&x) || (18..26).contains(&x)) && (7..25).contains(&y);
            let pixel = if white {
                [255, 255, 255, 255]
            } else {
                [180, 106, 35, 255]
            };
            pixels[(y * 32 + x) * 4..(y * 32 + x + 1) * 4].copy_from_slice(&pixel);
        }
    }
    let mask = [0u8; 128];
    let icon = unsafe { CreateIcon(null_mut(), 32, 32, 1, 32, mask.as_ptr(), pixels.as_ptr()) };
    if icon.is_null() {
        Err("无法创建托盘图标".into())
    } else {
        Ok(Icon(icon))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    #[ignore = "Requires the interactive Windows notification area; run with --ignored"]
    fn tray_registers_handles_keyboard_selection_readds_and_removes() {
        let calls = Rc::new(Cell::new(0));
        let observed = Rc::clone(&calls);
        let tray = Tray::new(move |action| {
            if matches!(action, Action::Show) {
                observed.set(observed.get() + 1);
            }
        })
        .unwrap();
        let hwnd = tray.window.hwnd().cast();
        let id = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: hwnd,
            uID: ICON_ID,
            ..Default::default()
        };
        let mut rect = windows_sys::Win32::Foundation::RECT::default();
        unsafe {
            assert!(Shell_NotifyIconGetRect(&id, &raw mut rect) >= 0);
            SendMessageW(
                hwnd,
                CALLBACK,
                0,
                ((ICON_ID << 16) | NIN_KEYSELECT) as isize,
            );
            assert_eq!(calls.get(), 1);
            SendMessageW(hwnd, CALLBACK, 0, ((2 << 16) | NIN_SELECT) as isize);
            assert_eq!(calls.get(), 1, "Another icon's callback must be ignored");
            Shell_NotifyIconW(NIM_DELETE, &data(hwnd, tray._icon.0));
            let recreated = RegisterWindowMessageW(windows_sys::w!("TaskbarCreated"));
            SendMessageW(hwnd, recreated, 0, 0);
            assert!(Shell_NotifyIconGetRect(&id, &raw mut rect) >= 0);
        }
        drop(tray);
        assert!(unsafe { Shell_NotifyIconGetRect(&id, &raw mut rect) } < 0);
    }
}
