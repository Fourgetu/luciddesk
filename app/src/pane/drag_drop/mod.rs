//! OLE drop registration, drag previews, and temporary drop descriptions.
mod description;
pub(super) mod image;
pub(super) mod target;

/// Only hand group drags to OLE over Explorer. Pane sorting and dropping back
/// onto the desktop continue through the internal membership drag path.
pub(super) fn over_explorer(point: windows_sys::Win32::Foundation::POINT) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GA_ROOT, GetAncestor, GetClassNameW, WindowFromPoint,
    };
    unsafe {
        let target = GetAncestor(WindowFromPoint(point), GA_ROOT);
        let mut class = [0u16; 64];
        let length = GetClassNameW(target, class.as_mut_ptr(), class.len() as i32);
        class[..length.max(0) as usize]
            .iter()
            .copied()
            .eq("CabinetWClass".encode_utf16())
    }
}
