//! Opt-in measurement of background membership reconciliation on Explorer's STA.
//! Enable the Perf.Enabled window property only for a bounded measurement, then
//! remove all three properties. No item identities or user data are recorded.
use std::time::Instant;
use windows_sys::Win32::{Foundation::HWND, UI::WindowsAndMessaging::*};

const ENABLED: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.Filter.Perf.Enabled");
const COUNT: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.Filter.Perf.Count");
const MICROS: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.Filter.Perf.Micros");

pub(super) fn clear(hwnd: HWND) {
    unsafe {
        for property in [ENABLED, COUNT, MICROS] {
            RemovePropW(hwnd, property);
        }
    }
}

pub(super) struct Sample(HWND, Instant);
impl Sample {
    pub fn start(hwnd: HWND) -> Option<Self> {
        (!unsafe { GetPropW(hwnd, ENABLED) }.is_null()).then(|| Self(hwnd, Instant::now()))
    }
}
impl Drop for Sample {
    fn drop(&mut self) {
        let elapsed = self.1.elapsed().as_micros().min(usize::MAX as u128) as usize;
        unsafe {
            let count = (GetPropW(self.0, COUNT) as usize).saturating_add(1);
            let micros = (GetPropW(self.0, MICROS) as usize).saturating_add(elapsed);
            SetPropW(self.0, COUNT, count as _);
            SetPropW(self.0, MICROS, micros as _);
        }
    }
}
