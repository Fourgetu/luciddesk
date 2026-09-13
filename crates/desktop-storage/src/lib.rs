//! SQLite workspace persistence and TOML application preferences.
//!
//! Open a [`WorkspaceStore`] to load and save domain values from `desktop-core`.
//! Failures are reported through [`StoreError`].
mod error;
mod store;

pub use error::StoreError;
pub use store::WorkspaceStore;
