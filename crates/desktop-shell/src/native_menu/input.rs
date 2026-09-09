//! Carry the actual invocation source across the pane/Explorer boundary.
use windows::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::{
    Graphics::Gdi::ScreenToClient,
    UI::WindowsAndMessaging::{
        PostMessageW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_CONTEXTMENU, WM_RBUTTONDOWN,
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuInvocation {
    Mouse,
    Keyboard,
}

pub(super) fn open_mouse(view: HWND, point: POINT) -> windows::core::Result<()> {
    let screen = pack_point(point.x, point.y)?;
    unsafe {
        let mut client = windows_sys::Win32::Foundation::POINT {
            x: point.x,
            y: point.y,
        };
        if ScreenToClient(view.0, &raw mut client) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let packed = pack_point(client.x, client.y)?;
        // CDefView initializes CONTEXT_MENU_PRESENTER_FLAGS on WM_RBUTTONDOWN.
        // DoContextMenuPopup bypasses that input path and can reuse keytip flags
        // from an earlier menu. Forward the pane's mouse-down to the Shell view
        // host, NOT its ListView (which would hit-test/select another item).
        // No button-up is sent: WM_CONTEXTMENU below is the only popup request.
        let mut result = 0;
        if SendMessageTimeoutW(
            view.0,
            WM_RBUTTONDOWN,
            0x0002, // MK_RBUTTON
            packed,
            SMTO_ABORTIFHUNG,
            1000,
            &raw mut result,
        ) == 0
        {
            return Err(windows::core::Error::from_thread());
        }
        // CDefView::_OnContextMenu marks a concrete screen position as a mouse
        // invocation. DoContextMenuPopup alone skips that step, so the XAML
        // presenter may automatically enter access-key display mode. Send to
        // the view host: it uses the already validated selection, not a new
        // ListView hit test at the pane's screen position.
        if PostMessageW(view.0, WM_CONTEXTMENU, view.0 as usize, screen) == 0 {
            return Err(windows::core::Error::from_thread());
        }
    }
    Ok(())
}

fn pack_point(x: i32, y: i32) -> windows::core::Result<isize> {
    let x = i16::try_from(x).map_err(|_| {
        windows::core::Error::from_hresult(windows::Win32::Foundation::E_INVALIDARG)
    })?;
    let y = i16::try_from(y).map_err(|_| {
        windows::core::Error::from_hresult(windows::Win32::Foundation::E_INVALIDARG)
    })?;
    let bits = u32::from(u16::from_ne_bytes(x.to_ne_bytes()))
        | (u32::from(u16::from_ne_bytes(y.to_ne_bytes())) << 16);
    Ok(i32::from_ne_bytes(bits.to_ne_bytes()) as isize)
}
