use std::error::Error;
use std::ffi::c_void;
use std::fmt;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWLP_HWNDPARENT, GetShellWindow, HWND_NOTOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SetWindowLongPtrW, SetWindowPos,
};

pub trait DesktopHost {
    /// Attaches a top-level popup to the current Shell Desktop owner.
    ///
    /// # Errors
    ///
    /// Returns an error when the Shell Desktop window is unavailable.
    fn attach(&mut self, hwnd: *mut c_void) -> Result<(), DesktopHostError>;

    fn detach(&mut self, hwnd: *mut c_void);

    /// Refreshes the Shell HWND after Explorer restarts.
    ///
    /// # Errors
    ///
    /// Returns an error when Explorer has not recreated its Shell window yet.
    fn refresh_shell(&mut self) -> Result<(), DesktopHostError>;
}

#[derive(Default)]
pub struct ShellOwnedDesktopHost {
    shell: HWND,
}

impl ShellOwnedDesktopHost {
    /// Resolves the current Shell Desktop window.
    ///
    /// # Errors
    ///
    /// Returns an error when Explorer's Shell window is unavailable.
    pub fn new() -> Result<Self, DesktopHostError> {
        let mut host = Self::default();
        host.refresh_shell()?;
        Ok(host)
    }

    #[must_use]
    pub const fn shell_hwnd(&self) -> HWND {
        self.shell
    }
}

impl DesktopHost for ShellOwnedDesktopHost {
    fn attach(&mut self, hwnd: *mut c_void) -> Result<(), DesktopHostError> {
        if self.shell.is_null() {
            self.refresh_shell()?;
        }
        let hwnd = hwnd.cast();
        unsafe {
            // For a WS_POPUP top-level window, GWLP_HWNDPARENT changes the owner; it does not
            // turn the popup into a child window.
            SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, self.shell as isize);
            SetWindowPos(
                hwnd,
                HWND_NOTOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        Ok(())
    }

    fn detach(&mut self, hwnd: *mut c_void) {
        unsafe {
            SetWindowLongPtrW(hwnd.cast(), GWLP_HWNDPARENT, 0);
        }
    }

    fn refresh_shell(&mut self) -> Result<(), DesktopHostError> {
        self.shell = unsafe { GetShellWindow() };
        if self.shell.is_null() {
            Err(DesktopHostError::ShellUnavailable)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopHostError {
    ShellUnavailable,
}

impl fmt::Display for DesktopHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShellUnavailable => {
                write!(formatter, "Explorer Shell Desktop window is unavailable")
            }
        }
    }
}

impl Error for DesktopHostError {}
