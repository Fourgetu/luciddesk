//! Pointer-free notifications shared by the desktop filter and the app.
/// A desktop gesture: wParam is the ListView HWND, lParam is its GetMessageTime
/// tick (u32). Receivers must reject events older than the last Pane input.
pub const DESKTOP_INPUT_MESSAGE: u32 = 0x8000 + 0x4a1;
