//! Application diagnostics. No storage/database dependency and no background writes.
//!
//! Initialize once after resolving the data directory. File logging defaults to ERROR;
//! Settings apply the saved diagnostics level through set_level.
//! Rendering trace output remains a separate explicitly enabled support tool.
mod clipboard;
mod logging;
mod render_trace;
mod system;

pub use clipboard::copy;
pub use logging::{Level, desktop_connection_log, init_logging, log, level, set_level};
pub use render_trace::{disable_backdrop, render_trace, shared_pane_tree};
pub use system::{report, system};
