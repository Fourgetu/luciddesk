//! Thread-bound OLE initialization.
use crate::ShellError;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};

/// Initializes COM as an OLE-capable STA for Shell UI, drag/drop, and file operations.
pub struct ShellApartment;

impl ShellApartment {
    /// Initializes OLE on the calling UI thread.
    ///
    /// # Errors
    ///
    /// Returns a COM error when the thread has already been initialized with an incompatible model.
    pub fn initialize_sta() -> Result<Self, ShellError> {
        unsafe { OleInitialize(None) }?;
        Ok(Self)
    }
}

impl Drop for ShellApartment {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}
