//! Thread-bound OLE initialization.
use std::{marker::PhantomData, rc::Rc};
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};

use crate::ShellError;

/// Keeps OLE initialized on the calling thread for Shell UI and drag/drop.
///
/// Keep this guard alive until all OLE-dependent resources have been released.
/// Each successful initialization is balanced by one uninitialization on drop.
/// The guard cannot be constructed directly, cloned, or sent to another thread.
///
/// ```no_run
/// let apartment = desktop_shell::ShellApartment::initialize_sta()?;
/// // Perform Shell operations while `apartment` remains alive.
/// drop(apartment);
/// # Ok::<(), desktop_shell::ShellError>(())
/// ```
///
/// ```compile_fail
/// fn require_send<T: Send>() {}
/// require_send::<desktop_shell::ShellApartment>();
/// ```
///
/// ```compile_fail
/// fn require_sync<T: Sync>() {}
/// require_sync::<desktop_shell::ShellApartment>();
/// ```
///
/// ```compile_fail
/// let apartment = desktop_shell::ShellApartment {};
/// ```
#[derive(Debug)]
#[must_use = "keep the guard alive while using OLE-dependent resources"]
pub struct ShellApartment {
    _thread_bound: PhantomData<Rc<()>>,
}

impl ShellApartment {
    /// Initializes OLE on the calling UI thread.
    ///
    /// # Errors
    /// Returns a COM error if OLE initialization fails, including when this
    /// thread already uses an incompatible apartment model.
    pub fn initialize_sta() -> Result<Self, ShellError> {
        // SAFETY: the reserved argument is null. Only a successful call creates
        // a guard, whose thread-bound ownership balances this call in Drop.
        unsafe { OleInitialize(None) }?;
        Ok(Self {
            _thread_bound: PhantomData,
        })
    }
}

impl Drop for ShellApartment {
    fn drop(&mut self) {
        // SAFETY: safe code cannot move this guard off its initializing thread.
        unsafe { OleUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use super::ShellApartment;
    use windows::Win32::System::Com::{APTTYPE, APTTYPEQUALIFIER, CoGetApartmentType};

    #[test]
    fn nested_guards_balance_each_successful_initialization() {
        std::thread::spawn(|| {
            let mut apartment_type = APTTYPE::default();
            let mut qualifier = APTTYPEQUALIFIER::default();
            let first = ShellApartment::initialize_sta().unwrap();
            let second = ShellApartment::initialize_sta().unwrap();
            drop(first);
            assert!(
                unsafe { CoGetApartmentType(&raw mut apartment_type, &raw mut qualifier) }.is_ok()
            );
            drop(second);
            assert!(
                unsafe { CoGetApartmentType(&raw mut apartment_type, &raw mut qualifier) }.is_err()
            );
        })
        .join()
        .unwrap();
    }
}
