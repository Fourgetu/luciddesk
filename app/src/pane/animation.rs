use std::time::{Duration, Instant};

/// One timeline supplies both the content and material opacity. Window resizing
/// keeps its existing cubic curve and Win32 geometry updates.
pub struct Fade {
    manager: windows_animation::Manager,
    value: windows_animation::Variable,
    duration: Duration,
}

impl Fade {
    pub fn new(duration: Duration) -> canvas_core::Result<Self> {
        let manager = windows_animation::Manager::new()?;
        let value = manager.create_variable(0.0)?;
        let library = windows_animation::TransitionLibrary::new()?;
        let transition = if duration.is_zero() {
            library.instantaneous(1.0)?
        } else {
            library.linear(duration.as_secs_f64(), 1.0)?
        };
        manager.schedule_transition(&value, &transition, 0.0)?;
        Ok(Self {
            manager,
            value,
            duration,
        })
    }

    pub fn sample(&self, elapsed: Duration) -> canvas_core::Result<f32> {
        self.manager.update(elapsed.as_secs_f64())?;
        // Commit an exact final value even when a modal loop skips frame ticks.
        if elapsed >= self.duration {
            return Ok(1.0);
        }
        Ok(self.value.value()?.clamp(0.0, 1.0) as f32)
    }
}

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
    fn animation_manager_fade_handles_midpoint_delays_and_disabled_animation() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let fade = Fade::new(Duration::from_millis(120)).unwrap();
        assert_eq!(fade.sample(Duration::ZERO).unwrap(), 0.0);
        assert!((fade.sample(Duration::from_millis(60)).unwrap() - 0.5).abs() < 0.001);
        assert_eq!(fade.sample(Duration::from_secs(1)).unwrap(), 1.0);
        let instant = Fade::new(Duration::ZERO).unwrap();
        assert_eq!(instant.sample(Duration::ZERO).unwrap(), 1.0);
    }
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
