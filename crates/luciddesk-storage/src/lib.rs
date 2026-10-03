//! SQLite workspace persistence and TOML application preferences.
//!
//! Open a [`WorkspaceStore`] to load and save domain values from `luciddesk-core`.
//! Failures are reported through [`StoreError`].
mod error;
mod store;

pub use error::StoreError;
pub use store::{FolderPreferences, SettingValue, WorkspaceStore};
