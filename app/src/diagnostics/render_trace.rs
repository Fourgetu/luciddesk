//! Explicitly enabled rendering experiments and trace output.
use super::report;
use std::sync::LazyLock;

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

struct RenderTrace {
    file: std::fs::File,
    started: std::time::Instant,
    lines: usize,
}
static RENDER_TRACE: LazyLock<Option<std::sync::Mutex<RenderTrace>>> = LazyLock::new(|| {
    use std::io::Write;
    let path = std::env::var_os("LUCIDDESK_RENDER_TRACE")?;
    let mut file = std::fs::File::create(path).ok()?;
    let _ = writeln!(
        file,
        "{}pid={} shared_tree={} disable_backdrop={}",
        report(),
        std::process::id(),
        shared_pane_tree(),
        disable_backdrop()
    );
    Some(std::sync::Mutex::new(RenderTrace {
        file,
        started: std::time::Instant::now(),
        lines: 0,
    }))
});

pub fn render_trace(event: std::fmt::Arguments<'_>) {
    use std::io::Write;
    let Some(trace) = RENDER_TRACE.as_ref() else {
        return;
    };
    let Ok(mut trace) = trace.lock() else {
        return;
    };
    // Keep support logs bounded even if a driver continuously rejects frames.
    if trace.lines >= 4000 {
        return;
    }
    trace.lines += 1;
    let elapsed = trace.started.elapsed().as_millis();
    let _ = writeln!(trace.file, "{elapsed}ms {event}");
}
