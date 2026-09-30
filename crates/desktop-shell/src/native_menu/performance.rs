//! Opt-in menu timings; one write after dismissal, never on the opening path.
use std::time::Instant;

pub(super) struct Timings {
    start: Instant,
    stages: Vec<(&'static str, f64)>,
    enabled: bool,
}
impl Timings {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            stages: Vec::new(),
            enabled: std::env::var_os("LUCIDDESK_MENU_PERF").is_some(),
        }
    }
    pub fn mark(&mut self, name: &'static str) {
        self.at(name, Instant::now());
    }
    pub fn at(&mut self, name: &'static str, time: Instant) {
        if self.enabled {
            self.stages.push((
                name,
                time.saturating_duration_since(self.start).as_secs_f64() * 1000.0,
            ));
        }
    }
}
impl Drop for Timings {
    fn drop(&mut self) {
        if !self.enabled {
            return;
        }
        use std::io::Write;
        let Some(base) = std::env::var_os("LOCALAPPDATA") else {
            return;
        };
        let path = std::path::PathBuf::from(base)
            .join("LucidDesk")
            .join("menu-performance.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(
                file,
                "{:?} elapsed_ms={:?}",
                std::time::SystemTime::now(),
                self.stages
            );
        }
    }
}
