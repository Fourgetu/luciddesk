//! Shared native popup appearance for application and Explorer menu hosts.
mod frame;
mod theme;

pub use frame::MenuFrame;
pub use theme::{ScopedTheme, apply as apply_theme, apply_scoped as apply_scoped_theme};
