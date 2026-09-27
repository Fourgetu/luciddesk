//! Back off idle Shell enumeration without delaying explicit change notifications.
use std::time::Duration;

const MIN_INTERVAL: Duration = Duration::from_secs(2);
const MAX_INTERVAL: Duration = Duration::from_secs(30);

pub(super) struct AuditSchedule {
    interval: Duration,
    invalidated: bool,
}

impl Default for AuditSchedule {
    fn default() -> Self {
        Self {
            interval: MIN_INTERVAL,
            invalidated: false,
        }
    }
}

impl AuditSchedule {
    pub(super) fn invalidate(&mut self) {
        self.interval = MIN_INTERVAL;
        self.invalidated = true;
    }

    pub(super) fn due(&self, elapsed: Duration) -> bool {
        self.invalidated || elapsed >= self.interval
    }

    pub(super) fn remaining(&self, elapsed: Duration) -> Duration {
        if self.invalidated { Duration::ZERO } else { self.interval.saturating_sub(elapsed) }
    }

    // Only consume the notification when a worker request is actually sent.
    pub(super) fn start(&mut self) -> bool {
        std::mem::take(&mut self.invalidated)
    }

    pub(super) fn complete(&mut self, changed: bool) {
        self.interval = if changed || self.invalidated {
            MIN_INTERVAL
        } else {
            (self.interval * 2).min(MAX_INTERVAL)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_checks_back_off_but_notifications_and_changes_reset_them() {
        let mut schedule = AuditSchedule::default();
        for seconds in [2, 4, 8, 16, 30, 30] {
            let interval = Duration::from_secs(seconds);
            assert_eq!(schedule.remaining(Duration::ZERO), interval);
            assert_eq!(schedule.remaining(interval), Duration::ZERO);
            assert!(!schedule.due(interval - Duration::from_millis(1)));
            assert!(schedule.due(interval));
            assert!(!schedule.start());
            schedule.complete(false);
        }
        schedule.invalidate();
        assert_eq!(schedule.remaining(Duration::ZERO), Duration::ZERO);
        assert!(schedule.due(Duration::ZERO));
        assert!(schedule.start());
        schedule.complete(true);
        assert!(!schedule.due(Duration::from_secs(1)));
        assert!(schedule.due(MIN_INTERVAL));
    }

    #[test]
    fn notification_during_in_flight_read_requires_another_forced_read() {
        let mut schedule = AuditSchedule::default();
        schedule.invalidate();
        assert!(schedule.start());
        // This change occurred after the worker captured its revision.
        schedule.invalidate();
        schedule.complete(false);
        assert!(schedule.due(Duration::ZERO));
        assert!(schedule.start());
        schedule.complete(false);
        assert!(!schedule.due(Duration::ZERO));
    }

    #[test]
    fn stable_minute_needs_fewer_audits_without_removing_fallback() {
        let mut schedule = AuditSchedule::default();
        let mut last = 0;
        let mut checks = 0;
        for second in 1..=60 {
            if schedule.due(Duration::from_secs(second - last)) {
                schedule.start();
                schedule.complete(false);
                last = second;
                checks += 1;
            }
        }
        assert_eq!(checks, 5);
        assert!(60 - last < 30);
    }
}
