use std::time::{Duration, Instant};

pub struct Fold {
    pub from: f32,
    pub to: f32,
    pub from_reveal: f32,
    pub to_reveal: f32,
    pub started: Instant,
    pub duration: Duration,
}

impl Fold {
    pub fn sample(&self, now: Instant) -> (f32, f32, bool) {
        let fraction = if self.duration.is_zero() {
            1.0
        } else {
            (now.saturating_duration_since(self.started).as_secs_f32()
                / self.duration.as_secs_f32())
            .min(1.0)
        };
        let eased = 1.0 - (1.0 - fraction).powi(3);
        (
            self.from + (self.to - self.from) * eased,
            self.from_reveal + (self.to_reveal - self.from_reveal) * eased,
            fraction >= 1.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_fold_reverses_without_a_position_jump() {
        let started = Instant::now();
        let duration = Duration::from_millis(200);
        let fold = Fold {
            from: 400.0,
            to: 38.0,
            from_reveal: 1.0,
            to_reveal: 0.0,
            started,
            duration,
        };
        let now = started + Duration::from_millis(80);
        let (height, reveal, done) = fold.sample(now);
        assert!(!done);
        assert!(height > 38.0 && height < 400.0);
        let reverse = Fold {
            from: height,
            to: 400.0,
            from_reveal: reveal,
            to_reveal: 1.0,
            started: now,
            duration,
        };
        assert_eq!(reverse.sample(now), (height, reveal, false));
        assert_eq!(reverse.sample(now + duration), (400.0, 1.0, true));
        assert_eq!(fold.sample(started + duration), (38.0, 0.0, true));
    }
}
