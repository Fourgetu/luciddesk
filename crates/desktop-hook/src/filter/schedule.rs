use std::time::{Duration, Instant};

/// No recovery timer exists when there is no pending work.
pub(super) fn recovery_delay(
    retry: Option<Instant>,
    updating: bool,
    queued: bool,
    now: Instant,
) -> Option<u32> {
    retry
        .map(|due| due.saturating_duration_since(now))
        .into_iter()
        .chain(updating.then_some(Duration::from_secs(1)))
        .chain(queued.then_some(Duration::from_millis(25)))
        .min()
        .map(|delay| delay.as_millis().clamp(1, u32::MAX as u128) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_has_no_timer_and_pending_work_uses_earliest_deadline() {
        let now = Instant::now();
        assert_eq!(recovery_delay(None, false, false, now), None);
        assert_eq!(recovery_delay(None, true, false, now), Some(1000));
        assert_eq!(
            recovery_delay(Some(now + Duration::from_millis(250)), true, false, now),
            Some(250)
        );
        assert_eq!(
            recovery_delay(Some(now + Duration::from_millis(250)), true, true, now),
            Some(25)
        );
        assert_eq!(recovery_delay(Some(now), false, false, now), Some(1));
    }
}
