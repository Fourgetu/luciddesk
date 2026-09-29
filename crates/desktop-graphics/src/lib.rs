//! Private graphics ABI boundary, generated against windows-core 0.100.
#[allow(
    dead_code,
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types,
    clippy::upper_case_acronyms
)]
mod dcomp {
    include!("bindings/dcomp.rs");
}
#[allow(
    dead_code,
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types
)]
pub mod dwm {
    include!("bindings/dwm.rs");
}

mod layer;
pub use layer::{Layer, clear_thread_cache};
