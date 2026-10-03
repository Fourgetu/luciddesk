//! Copies a diagnostic report to the Windows clipboard.
pub fn copy(owner: isize, text: &str) -> windows::core::Result<()> {
    use windows::Win32::{
        Foundation::*,
        System::{DataExchange::*, Memory::*},
    };
    struct Memory(HGLOBAL);
    impl Drop for Memory {
        fn drop(&mut self) {
            unsafe {
                let _ = GlobalFree(Some(self.0));
            }
        }
    }
    struct Clipboard;
    impl Drop for Clipboard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }
    let wide: Vec<_> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let memory = Memory(GlobalAlloc(GMEM_MOVEABLE, wide.len() * size_of::<u16>())?);
        let target = GlobalLock(memory.0).cast::<u16>();
        if target.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
        let _ = GlobalUnlock(memory.0);
        OpenClipboard(Some(HWND(owner as _)))?;
        let _clipboard = Clipboard;
        EmptyClipboard()?;
        SetClipboardData(13, Some(HANDLE(memory.0.0)))?; // CF_UNICODETEXT
        std::mem::forget(memory); // Clipboard owns the allocation after success.
    }
    Ok(())
}
