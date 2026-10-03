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
            enabled: luciddesk_diagnostics::enabled(luciddesk_diagnostics::Level::Debug),
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
        luciddesk_diagnostics::emit!(
            luciddesk_diagnostics::Level::Debug,
            "shell.menu.performance",
            "elapsed_ms={:?}",
            self.stages
        );
    }
}
