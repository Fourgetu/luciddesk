//! Carry the actual invocation source across the pane/Explorer boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuInvocation {
    Mouse,
    Keyboard,
}
