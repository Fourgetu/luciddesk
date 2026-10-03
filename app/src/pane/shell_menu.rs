//! Explorer hosts the compact menu against an isolated, validated Shell selection.
use desktop_core::ShellIdentity;
use luciddesk_explorer::filter::FilterSession;
use windows_sys::Win32::Foundation::{HWND, POINT};

/// Preparation, popup and cancellation may pump messages. The caller must hold
/// the session independently and release all app, model and event borrows.
/// Returns whether the selected item should enter the Pane rename editor.
pub fn show_many(
    owner: HWND,
    hook: &FilterSession,
    identities: &[ShellIdentity],
    point: POINT,
    keyboard: bool,
) -> Result<bool, String> {
    let names: Vec<_> = identities
        .iter()
        .map(|item| item.activation_name().to_string_lossy().into_owned())
        .collect();
    #[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
    let started = std::time::Instant::now();
    let shown = hook
        .prepare_menu(owner as isize, &names, point.x, point.y)
        .and_then(|host| {

            #[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
            {
                eprintln!("menu_prepare_us={}", started.elapsed().as_micros());
            }
            let result = desktop_shell::show_isolated_item_menu(
                windows::Win32::Foundation::HWND(owner),
                windows::Win32::Foundation::HWND(host as _),
                if keyboard {
                    desktop_shell::MenuInvocation::Keyboard
                } else {
                    desktop_shell::MenuInvocation::Mouse
                },
            )
            .map_err(|error| crate::i18n::format("ui-could-not-open-explorer-icon-menu", &[("error", format!("{}", error))]));
            #[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
            record_presenter(host);
            result
        });
    // Always finish/cancel, including when preparation timed out. Keep the
    // original opening error if cleanup also fails; the Hook retains its token.
    let finished = if shown.is_err() {
        hook.cancel_menu().map(|()| false)
    } else {
        hook.finish_menu()
    };
    shown.and(finished)
}

#[cfg(any(debug_assertions, feature = "menu-diagnostics"))]
fn record_presenter(host: isize) {
    use std::io::Write;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetPropW;
    let (mode, error) = unsafe {
        (GetPropW(host as _, windows_sys::w!("LucidDesk.Menu.Presenter")) as usize,
         GetPropW(host as _, windows_sys::w!("LucidDesk.Menu.PresenterError")) as usize as u32)
    };
    let Ok(base) = desktop_shell::local_app_data_path() else { return; };
    if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true)
        .open(base.join("LucidDesk").join("menu-presenter.log")) {
        let values = unsafe { ["PrepareCalled", "PrepareResult", "ReadyCalled", "ReadyResult", "ShowCalled"].map(|name| {
            let key: Vec<u16> = format!("LucidDesk.Menu.{name}").encode_utf16().chain(Some(0)).collect();
            GetPropW(host as _, key.as_ptr()) as usize as u32
        }) };
        let _ = writeln!(log, "{:?} host={host:x} presenter={mode} initialization_hresult=0x{error:08X} prepare_called={} prepare_hresult=0x{:08X} ready_called={} ready={} show={}", std::time::SystemTime::now(), values[0], values[1], values[2], values[3], values[4]);
    }
}
