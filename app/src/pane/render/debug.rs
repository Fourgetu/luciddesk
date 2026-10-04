//! Explicitly enabled rendering experiments and trace output.
use crate::system_info::report;
use luciddesk_diagnostics::TraceFile;
use std::{
    fmt::Arguments,
    sync::{LazyLock, Mutex},
};

const MAX_TRACE_LINES: usize = 4000;

// Opt-in, process-local switches for the portable rendering comparison pack.
// They never change the stored appearance settings.
pub fn shared_pane_tree() -> bool {
    static ENABLED: LazyLock<bool> = LazyLock::new(|| {
        std::env::var("LUCIDDESK_SHARED_PANE_TREE").is_ok_and(|value| value == "1")
    });
    *ENABLED
}

pub fn disable_backdrop() -> bool {
    static ENABLED: LazyLock<bool> = LazyLock::new(|| {
        std::env::var("LUCIDDESK_DISABLE_BACKDROP").is_ok_and(|value| value == "1")
    });
    *ENABLED
}

static RENDER_TRACE: LazyLock<Option<Mutex<TraceFile>>> = LazyLock::new(|| {
    let path = std::env::var_os("LUCIDDESK_RENDER_TRACE")?;
    let report = format!(
        "{}shared_tree={} disable_backdrop={}",
        report(),
        shared_pane_tree(),
        disable_backdrop()
    );
    TraceFile::create(
        std::path::Path::new(&path),
        env!("CARGO_PKG_VERSION"),
        env!("LUCIDDESK_BUILD_REVISION"),
        &report,
        MAX_TRACE_LINES,
    )
    .ok()
    .map(Mutex::new)
});

pub fn render_trace(event: Arguments<'_>) {
    let Some(trace) = RENDER_TRACE.as_ref() else {
        return;
    };
    let Ok(mut trace) = trace.lock() else {
        return;
    };
    let _ = trace.write("pane.render", event);
}
