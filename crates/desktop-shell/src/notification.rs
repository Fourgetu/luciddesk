//! Shell change notification registration and cleanup.
use crate::ShellError;
use std::ptr;
use windows_sys::Win32::{
    Foundation::HWND,
    System::Com::CoTaskMemFree,
    UI::Shell::{
        CSIDL_DESKTOP, Common::ITEMIDLIST, SHCNE_ALLEVENTS, SHCNRF_InterruptLevel,
        SHCNRF_ShellLevel, SHChangeNotifyDeregister, SHChangeNotifyEntry, SHChangeNotifyRegister,
        SHGetSpecialFolderLocation,
    },
};

/// Owns a Shell change registration until dropped.
#[derive(Debug)]
#[must_use = "keep the subscription alive to receive Shell changes"]
pub struct DesktopChangeSubscription {
    registration: u32,
    desktop_pidl: *mut ITEMIDLIST,
}

impl DesktopChangeSubscription {
    /// Registers a window message for recursive Desktop Shell Namespace changes.
    ///
    /// The receiver only needs the notification as an invalidation signal; item identity is
    /// resolved by a debounced namespace reconciliation.
    ///
    /// # Errors
    ///
    /// Returns an error when the Desktop PIDL or Shell registration cannot be created.
    pub fn register(owner: isize, message: u32) -> Result<Self, ShellError> {
        Self::register_folder(owner, message, CSIDL_DESKTOP.cast_signed())
    }

    /// Subscribe directly to Recycle Bin contents, independently of the desktop view.
    /// # Errors
    /// Returns an error if Shell cannot register the namespace notification.
    pub fn register_recycle_bin(owner: isize, message: u32) -> Result<Self, ShellError> {
        Self::register_folder(owner, message, 10) // CSIDL_BITBUCKET
    }

    fn register_folder(owner: isize, message: u32, folder: i32) -> Result<Self, ShellError> {
        let mut desktop_pidl = ptr::null_mut();
        let result =
            unsafe { SHGetSpecialFolderLocation(owner as HWND, folder, &raw mut desktop_pidl) };
        if result < 0 {
            return Err(ShellError::Windows(result));
        }
        let entry = SHChangeNotifyEntry {
            pidl: desktop_pidl,
            fRecursive: 1,
        };
        let registration = unsafe {
            SHChangeNotifyRegister(
                owner as HWND,
                SHCNRF_ShellLevel | SHCNRF_InterruptLevel,
                SHCNE_ALLEVENTS.cast_signed(),
                message,
                1,
                &raw const entry,
            )
        };
        if registration == 0 {
            unsafe { CoTaskMemFree(desktop_pidl.cast()) };
            return Err(ShellError::ChangeNotification);
        }
        Ok(Self {
            registration,
            desktop_pidl,
        })
    }
}

impl Drop for DesktopChangeSubscription {
    fn drop(&mut self) {
        unsafe {
            SHChangeNotifyDeregister(self.registration);
            CoTaskMemFree(self.desktop_pidl.cast());
        }
    }
}
