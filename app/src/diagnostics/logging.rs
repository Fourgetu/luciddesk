//! Host adapter: application metadata and desktop connection notices.
pub use luciddesk_diagnostics::{Level, level, log, set_level};
use std::{
    io,
    path::{Path, PathBuf},
};
pub fn init_logging(database: &Path) {
    luciddesk_diagnostics::initialize(
        database,
        env!("CARGO_PKG_VERSION"),
        env!("LUCIDDESK_BUILD_REVISION"),
        super::report,
    );
}
pub fn desktop_connection_log(database: &Path, error: Option<&str>) -> io::Result<PathBuf> {
    init_logging(database);
    luciddesk_diagnostics::try_log(
        if error.is_some() {
            Level::Error
        } else {
            Level::Info
        },
        "desktop.connection",
        error.unwrap_or("connection recovered"),
    )?;
    luciddesk_diagnostics::path().ok_or_else(|| io::Error::other("diagnostics not initialized"))
}
