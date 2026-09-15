use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// The desktop thread cancels through an atomic token even while a Shell
/// extension blocks the menu STA. Each preparation receives a fresh token.
#[derive(Clone)]
pub(super) struct Invocation(Rc<RefCell<Arc<AtomicBool>>>);
impl Invocation {
    pub fn new(token: Arc<AtomicBool>) -> Self {
        Self(Rc::new(RefCell::new(token)))
    }
    pub fn reset(&self, token: Arc<AtomicBool>) {
        *self.0.borrow_mut() = token;
    }
    pub fn cancelled(&self) -> bool {
        self.0.borrow().load(Ordering::Acquire)
    }
    pub fn check(&self) -> windows::core::Result<()> {
        if self.cancelled() {
            Err(windows::Win32::Foundation::E_ABORT.into())
        } else {
            Ok(())
        }
    }
    pub fn check_fn(&self) -> Rc<dyn Fn() -> bool> {
        let this = self.clone();
        Rc::new(move || this.cancelled())
    }
}
