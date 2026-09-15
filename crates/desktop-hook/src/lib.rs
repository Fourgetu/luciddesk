//! Explorer member filtering and isolated Shell menu hosting.
mod discovery;
pub mod filter;
pub mod notifications;

pub use discovery::{conflicting_desktop_extension, desktop_view};
