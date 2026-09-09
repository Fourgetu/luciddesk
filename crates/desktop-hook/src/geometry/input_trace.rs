//! Opt-in, bounded first-press trace. Disk output happens after native press handling.
use std::cell::RefCell;
use std::io::Write;
use windows_sys::Win32::{
    Foundation::HWND,
    UI::{
        Controls::{
            LVM_GETEXTENDEDLISTVIEWSTYLE, LVM_GETNEXTITEM, LVM_GETSELECTEDCOUNT, LVN_BEGINDRAG,
            LVN_GETDISPINFOW, LVN_ITEMCHANGED, LVN_ITEMCHANGING, LVNI_FOCUSED, NM_CUSTOMDRAW,
            NMHDR, NMLISTVIEW,
        },
        Input::KeyboardAndMouse::{GetCapture, GetFocus},
        WindowsAndMessaging::{SendMessageW, WM_LBUTTONDOWN},
    },
};
thread_local! { static EVENTS: RefCell<Option<Vec<String>>> = const { RefCell::new(None) }; }
pub(super) fn event(value: impl FnOnce() -> String) {
    EVENTS.with(|events| {
        if let Ok(mut events) = events.try_borrow_mut()
            && let Some(events) = events.as_mut().filter(|events| events.len() < 160)
        {
            events.push(value());
        }
    });
}
pub(super) struct Press(HWND);
impl Press {
    pub(super) fn begin(view: HWND, message: u32, wp: usize, lp: isize) -> Option<Self> {
        if message != WM_LBUTTONDOWN {
            return None;
        }
        EVENTS.with(|events| *events.borrow_mut() = Some(Vec::with_capacity(160)));
        event(|| {
            format!(
                "PRESS wp={wp} point={},{} exstyle={:#x} focus={:?} capture={:?}",
                (lp as i16),
                ((lp >> 16) as i16),
                unsafe { SendMessageW(view, LVM_GETEXTENDEDLISTVIEWSTYLE, 0, 0) },
                unsafe { GetFocus() },
                unsafe { GetCapture() }
            )
        });
        Some(Self(view))
    }
}
impl Drop for Press {
    fn drop(&mut self) {
        event(|| {
            format!(
                "RETURN selected={} focused={} focus={:?} capture={:?}",
                unsafe { SendMessageW(self.0, LVM_GETSELECTEDCOUNT, 0, 0) },
                unsafe { SendMessageW(self.0, LVM_GETNEXTITEM, usize::MAX, LVNI_FOCUSED as isize) },
                unsafe { GetFocus() },
                unsafe { GetCapture() }
            )
        });
        let events = EVENTS.with(|events| events.borrow_mut().take());
        let Some(events) = events else {
            return;
        };
        let Some(local) = std::env::var_os("LOCALAPPDATA") else {
            return;
        };
        let path = std::path::PathBuf::from(local).join("LucidPane/hook-input.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(
                file,
                "{:?}\n{}",
                std::time::SystemTime::now(),
                events.join("\n")
            );
        }
    }
}
pub(super) unsafe fn notification(header: &NMHDR, lp: isize, result: isize) {
    if [LVN_ITEMCHANGING, LVN_ITEMCHANGED, LVN_BEGINDRAG].contains(&header.code) {
        let change = unsafe { &*(lp as *const NMLISTVIEW) };
        event(|| {
            format!(
                "NOTIFY code={} item={} old={:#x} new={:#x} changed={:#x} point={},{} result={result}",
                header.code.cast_signed(),
                change.iItem,
                change.uOldState,
                change.uNewState,
                change.uChanged,
                change.ptAction.x,
                change.ptAction.y
            )
        });
    } else if header.code != NM_CUSTOMDRAW && header.code != LVN_GETDISPINFOW {
        event(|| format!("NOTIFY code={} result={result}", header.code.cast_signed()));
    }
}
