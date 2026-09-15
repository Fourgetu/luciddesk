//! Explorer hosts the compact menu against an isolated, validated Shell selection.
use desktop_core::ShellIdentity;
use desktop_hook::filter::FilterSession;
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
            desktop_shell::show_isolated_item_menu(
                windows::Win32::Foundation::HWND(owner),
                windows::Win32::Foundation::HWND(host as _),
                if keyboard {
                    desktop_shell::MenuInvocation::Keyboard
                } else {
                    desktop_shell::MenuInvocation::Mouse
                },
            )
            .map_err(|error| format!("无法打开 Explorer 图标菜单：{error}"))
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
