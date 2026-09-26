//! Native hover text keeps complete paths available without widening result rows.
use super::*;

pub(super) struct Tooltip {
    hwnd: HWND,
    tool: TTTOOLINFOW,
    text: Vec<u16>,
}
impl Tooltip {
    pub(super) fn new(owner: HWND) -> Option<Self> {
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                TOOLTIPS_CLASSW,
                std::ptr::null(),
                WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
                0,
                0,
                0,
                0,
                owner,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return None;
            }
            let mut text = vec![0u16];
            let mut tool = TTTOOLINFOW {
                cbSize: size_of::<TTTOOLINFOW>() as u32,
                uFlags: TTF_SUBCLASS,
                hwnd: owner,
                uId: 1,
                lpszText: text.as_mut_ptr(),
                ..Default::default()
            };
            SendMessageW(hwnd, TTM_ADDTOOLW, 0, (&raw mut tool) as isize);
            SendMessageW(hwnd, TTM_SETMAXTIPWIDTH, 0, (560.0 * scale(owner)) as isize);
            Some(Self { hwnd, tool, text })
        }
    }
    pub(super) fn hide(&mut self) {
        unsafe {
            SendMessageW(self.hwnd, TTM_POP, 0, 0);
            SendMessageW(self.hwnd, TTM_ACTIVATE, 0, 0);
        }
    }
    pub(super) fn show_for_row(&mut self, owner: HWND, text: &str, y: f32) {
        self.hide();
        self.text = text.encode_utf16().chain(Some(0)).collect();
        self.tool.lpszText = self.text.as_mut_ptr();
        let s = scale(owner);
        self.tool.rect = RECT {
            left: (8.0 * s) as i32,
            top: (y * s) as i32,
            right: ((client_width(owner) - 8.0) * s) as i32,
            bottom: ((y + ROW) * s) as i32,
        };
        unsafe {
            SendMessageW(
                self.hwnd,
                TTM_NEWTOOLRECTW,
                0,
                (&raw mut self.tool) as isize,
            );
            SendMessageW(
                self.hwnd,
                TTM_UPDATETIPTEXTW,
                0,
                (&raw mut self.tool) as isize,
            );
            SendMessageW(self.hwnd, TTM_ACTIVATE, 1, 0);
        }
    }
}
impl Drop for Tooltip {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}
