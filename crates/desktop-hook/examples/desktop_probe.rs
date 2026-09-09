//! Read-only live Explorer handshake. No work-area, item-position or visibility writes.
use desktop_hook::{
    HookSession, desktop_view,
    protocol::{QUERY_AREA_COUNT, QUERY_AUTOARRANGE, QUERY_ITEM_COUNT, Request},
};
use std::ptr::{null, null_mut};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow};

fn main() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let folder = exe.parent().unwrap().parent().unwrap();
    let source = folder.join("desktop_hook.dll");
    let data = std::fs::read(&source).map_err(|e| e.to_string())?;
    let hash = data.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)
    });
    let version = folder.join("hook-runtime").join(format!("{hash:016x}"));
    std::fs::create_dir_all(&version).map_err(|e| e.to_string())?;
    let dll = version.join("desktop_hook.dll");
    if !dll.exists() {
        std::fs::write(&dll, data).map_err(|e| e.to_string())?;
    }
    let view = desktop_view()?;
    let owner = unsafe {
        CreateWindowExW(
            0,
            windows_sys::w!("STATIC"),
            windows_sys::w!("LucidPane Read-only Hook Probe"),
            0,
            0,
            0,
            1,
            1,
            null_mut(),
            null_mut(),
            null_mut(),
            null(),
        )
    };
    let result = (|| {
        let session = HookSession::connect(view, owner as isize, &dll)?;
        println!(
            "Explorer handshake OK; view={view}; auto_arrange={}; work_areas={}; items={}; no layout writes",
            session.request(&Request::new(QUERY_AUTOARRANGE))?,
            session.request(&Request::new(QUERY_AREA_COUNT))?,
            session.request(&Request::new(QUERY_ITEM_COUNT))?
        );
        drop(session);
        Ok(())
    })();
    unsafe {
        DestroyWindow(owner);
    }
    result
}
