//! Work flourish — a short-lived pulse spawned when a cron transitions
//! from idle into `Running`. The operator sees a ring expand around the
//! job's sprite for ~800 ms, which makes the "something's running" state
//! noticeable without requiring the operator to be actively watching.
//!
//! Storage mirrors `ThoughtBubble`: the `App` owns a `Vec<Flourish>`,
//! expires entries on each tick, and the scene reads it to render.

use std::time::{Duration, Instant};

use crate::domain::JobId;

/// Total visible lifetime. Long enough to register visually, short
/// enough that rapid successive triggers don't pile up into a ring
/// stack.
pub const FLOURISH_LIFETIME: Duration = Duration::from_millis(800);

/// One pulse. `spawn()` records the target job and the start instant;
/// renderers derive their pulse radius / alpha from the elapsed time.
#[derive(Debug, Clone)]
pub struct Flourish {
    pub job_id: JobId,
    pub spawned: Instant,
}

impl Flourish {
    pub fn spawn(job_id: JobId) -> Self {
        Self {
            job_id,
            spawned: Instant::now(),
        }
    }

    /// Normalized progress through the lifetime, 0.0 .. 1.0. Callers
    /// map this to a widening radius and a fade-out alpha.
    pub fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.spawned).as_secs_f32();
        let total = FLOURISH_LIFETIME.as_secs_f32();
        (elapsed / total).clamp(0.0, 1.0)
    }

    pub fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.spawned) >= FLOURISH_LIFETIME
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_crosses_zero_to_one() {
        let f = Flourish::spawn(JobId::new("zpool-health-check"));
        // Just after spawn, progress ≈ 0.
        assert!(f.progress(Instant::now()) < 0.05);
        // Long past lifetime, clamped to 1.0.
        let far_future = f.spawned + FLOURISH_LIFETIME * 2;
        assert_eq!(f.progress(far_future), 1.0);
        assert!(f.expired(far_future));
    }
}
