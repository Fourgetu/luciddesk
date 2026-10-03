//! Real Explorer DLL unload regression, with a menu worker and abrupt owner exit.
//! Uses only a test-owned file and never changes desktop membership.
use luciddesk_explorer::filter::FilterSession;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let dll = std::path::PathBuf::from(args.first().ok_or("DLL path required")?);
    let target = args
        .get(1)
        .ok_or("Test file path required")?
        .to_string_lossy()
        .into_owned();
    let abrupt = args.get(2).is_some_and(|arg| arg == "--abrupt");
    let _sta = luciddesk_shell::ShellApartment::initialize_sta()?;
    let owner = windows_window::Window::new("LucidDesk DLL lifecycle test")
        .size(1, 1)
        .style(WS_POPUP)
        .ex_style(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)
        .on_message(|_, msg, _, _| (msg == WM_DESTROY).then_some(0))
        .create()
        .map_err(|e| e.to_string())?;
    let view = luciddesk_explorer::desktop_view()?;
    let hook = FilterSession::connect(view, owner.hwnd() as isize, &dll)?;
    for _ in 0..3 {
        hook.prepare_menu(
            owner.hwnd() as isize,
            std::slice::from_ref(&target),
            300,
            300,
        )?;
        hook.finish_menu()?;
    }
    hook.cancel_menu()?;
    if abrupt {
        // Exercise OwnerWatch restoration without FilterSession's Rust destructor.
        std::process::exit(0);
    }
    drop(hook);
    drop(owner);
    Ok(())
}
