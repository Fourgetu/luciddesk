//! Counts control geometry queries in the disposable view, without timing assertions.
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::{Foundation::HWND, UI::{Controls::{LVM_GETITEMPOSITION, LVM_GETITEMRECT}, Shell::{DefSubclassProc, SetWindowSubclass, RemoveWindowSubclass}}};
static QUERIES: AtomicUsize = AtomicUsize::new(0);
unsafe extern "system" fn observer(hwnd: HWND, msg: u32, wp: usize, lp: isize, _: usize, _: usize) -> isize {
    if msg == LVM_GETITEMPOSITION || msg == LVM_GETITEMRECT { QUERIES.fetch_add(1, Ordering::Relaxed); }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}
pub struct Meter(HWND);
impl Meter {
    pub fn attach(view: HWND) -> Self {
        unsafe { assert_ne!(SetWindowSubclass(view, Some(observer), 0x0050_4552, 0), 0); }
        Self(view)
    }
    #[allow(clippy::unused_self)] // Scope of the active observer is owned by this guard.
    pub fn reset(&self) { QUERIES.store(0, Ordering::Relaxed); }
    #[allow(clippy::unused_self)]
    pub fn count(&self) -> usize { QUERIES.load(Ordering::Relaxed) }
}
impl Drop for Meter {
    fn drop(&mut self) { unsafe { RemoveWindowSubclass(self.0, Some(observer), 0x0050_4552); } }
}
