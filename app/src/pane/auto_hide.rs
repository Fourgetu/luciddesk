//! Hover transitions schedule one deadline; settled panes have no timer.
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct AutoHide {
    hover: Option<(bool, Instant)>,
}

impl AutoHide {
    pub(super) fn reset(&mut self) {
        self.hover = None;
    }

    pub(super) fn update(
        &mut self,
        enabled: bool,
        suspended: bool,
        collapsed: bool,
        hovered: bool,
        now: Instant,
    ) -> (Option<Duration>, Option<bool>) {
        if !enabled || suspended {
            self.reset();
            return (None, None);
        }
        if self.hover.is_none_or(|(previous, _)| previous != hovered) {
            self.hover = Some((hovered, now));
        }
        if collapsed != hovered {
            return (None, None);
        }
        let delay = Duration::from_millis(if hovered { 120 } else { 600 });
        let remaining = delay.saturating_sub(now.saturating_duration_since(self.hover.unwrap().1));
        if remaining.is_zero() {
            (None, Some(!hovered))
        } else {
            (Some(remaining), None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movement_does_not_extend_deadline_and_settled_panes_stop_timing() {
        let mut state = AutoHide::default();
        let now = Instant::now();
        assert_eq!(
            state.update(true, false, true, true, now),
            (Some(Duration::from_millis(120)), None)
        );
        assert_eq!(
            state.update(true, false, true, true, now + Duration::from_millis(100)),
            (Some(Duration::from_millis(20)), None)
        );
        assert_eq!(
            state.update(true, false, true, true, now + Duration::from_millis(120)),
            (None, Some(false))
        );
        assert_eq!(
            state.update(true, false, false, true, now + Duration::from_secs(1)),
            (None, None)
        );
    }

    #[test]
    fn leaving_or_suspending_restarts_the_dwell_period() {
        let mut state = AutoHide::default();
        let now = Instant::now();
        state.update(true, false, false, false, now);
        assert_eq!(
            state.update(true, true, false, false, now + Duration::from_secs(1)),
            (None, None)
        );
        assert_eq!(
            state.update(true, false, false, false, now + Duration::from_secs(2)),
            (Some(Duration::from_millis(600)), None)
        );
        assert_eq!(
            state.update(true, false, false, true, now + Duration::from_secs(3)),
            (None, None)
        );
        assert_eq!(
            state.update(false, false, false, false, now + Duration::from_secs(4)),
            (None, None)
        );
    }
}
