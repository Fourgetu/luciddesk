use luciddesk_core::MonitorId;
use windows_sys::Win32::Foundation::{LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorDescriptor {
    pub id: MonitorId,
    pub bounds: PixelRect,
    pub work_area: PixelRect,
    pub dpi: u32,
    pub primary: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PixelRect {
    const fn from_rect(rect: RECT) -> Self {
        Self {
            x: rect.left,
            y: rect.top,
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
        }
    }
}

/// Enumerates active display monitors with stable device names and effective DPI.
#[must_use]
pub fn enumerate_monitors() -> Vec<MonitorDescriptor> {
    unsafe extern "system" fn callback(
        monitor: HMONITOR,
        _device_context: HDC,
        _bounds: *mut RECT,
        data: LPARAM,
    ) -> i32 {
        let monitors = unsafe { &mut *(data as *mut Vec<MonitorDescriptor>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = u32::try_from(size_of::<MONITORINFOEXW>()).unwrap_or(u32::MAX);
        if unsafe { GetMonitorInfoW(monitor, (&raw mut info).cast()) } == 0 {
            return 1;
        }
        let length = info
            .szDevice
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(info.szDevice.len());
        let id = String::from_utf16_lossy(&info.szDevice[..length]);
        let mut dpi_x = 96_u32;
        let mut dpi_y = 96_u32;
        let _ =
            unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y) };
        monitors.push(MonitorDescriptor {
            id: MonitorId::new(id),
            bounds: PixelRect::from_rect(info.monitorInfo.rcMonitor),
            work_area: PixelRect::from_rect(info.monitorInfo.rcWork),
            dpi: dpi_x.max(1),
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
        1
    }

    let mut monitors: Vec<MonitorDescriptor> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(callback),
            (&raw mut monitors) as LPARAM,
        );
    }
    monitors.sort_by_key(|monitor| !monitor.primary);
    monitors
}
