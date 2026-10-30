//! Geometry and pointer gesture decisions for a bottom drawer.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const DISMISS_DISTANCE_FRACTION: f32 = 0.3;
const MIN_FLING_DISTANCE: f32 = 16.0;
const MIN_FLING_VELOCITY: f32 = 800.0;
const MAX_VELOCITY_SAMPLE_AGE: Duration = Duration::from_millis(100);
const MAX_RELEASE_AGE: Duration = Duration::from_millis(120);

/// Tracks a downward pointer drag that began on the drawer's handle.
///
/// Body scrolling should be handled by the scroll view and must not call
/// `update`; this type deliberately knows nothing about scroll gestures.
pub(super) struct DrawerDrag {
    start_y: f32,
    last_y: f32,
    last_at: Instant,
    last_motion_at: Option<Instant>,
    offset: f32,
    downward_samples: VecDeque<(Instant, f32)>,
}

impl DrawerDrag {
    pub(super) fn new(start_y: f32, now: Instant) -> Self {
        let start_y = finite_or_zero(start_y);
        Self {
            start_y,
            last_y: start_y,
            last_at: now,
            last_motion_at: None,
            offset: 0.0,
            downward_samples: VecDeque::new(),
        }
    }

    /// Records a pointer position and returns the current downward translation.
    /// Upward movement reduces the translation but never makes it negative.
    pub(super) fn update(&mut self, y: f32, now: Instant) -> f32 {
        let y = finite_or_zero(y);
        let delta = y - self.last_y;

        if delta > 0.0 {
            self.last_motion_at = Some(now);
            self.prune_velocity_samples(now);
            if self.downward_samples.is_empty() {
                self.downward_samples.push_back((self.last_at, self.last_y));
            }
            self.downward_samples.push_back((now, y));
        } else if delta < 0.0 {
            // Reversing direction cancels the pending fling; the next
            // downward movement begins a fresh sample window.
            self.downward_samples.clear();
            self.last_motion_at = None;
        }

        self.last_y = y;
        self.last_at = now;
        self.offset = (y - self.start_y).max(0.0);
        self.offset
    }

    pub(super) fn offset(&self) -> f32 {
        self.offset
    }

    /// Dismisses when the drag has crossed 30% of the panel height.
    ///
    /// Velocity is intentionally evaluated by `release`, which receives the
    /// release time and can reject stale movement after the pointer pauses.
    pub(super) fn should_dismiss(&self, panel_height: f32) -> bool {
        panel_height.is_finite()
            && panel_height > 0.0
            && self.offset + f32::EPSILON * panel_height >= panel_height * DISMISS_DISTANCE_FRACTION
    }

    /// Applies distance and recent-fling dismissal rules at pointer release.
    /// A fling needs at least 16 px of accumulated downward movement across a
    /// recent sample window of at most 100 ms, at 800 px/s or faster. The total
    /// drag must also have reached 16 px. Upward movement resets that window.
    pub(super) fn release(&self, now: Instant, panel_height: f32) -> bool {
        if self.should_dismiss(panel_height) {
            return true;
        }

        let fresh_motion = self
            .last_motion_at
            .is_some_and(|at| now.saturating_duration_since(at) <= MAX_RELEASE_AGE);
        let meaningful_drag = self.offset >= MIN_FLING_DISTANCE;
        let sample = self
            .downward_samples
            .front()
            .zip(self.downward_samples.back())
            .map(|((start_at, start_y), (end_at, end_y))| {
                let distance = end_y - start_y;
                let elapsed = end_at.saturating_duration_since(*start_at);
                let velocity = if elapsed.is_zero() {
                    0.0
                } else {
                    distance / elapsed.as_secs_f32()
                };
                (distance, elapsed, *end_at, velocity)
            });
        let fresh_sample = sample.is_some_and(|(_, elapsed, end_at, _)| {
            !elapsed.is_zero()
                && elapsed <= MAX_VELOCITY_SAMPLE_AGE
                && now.saturating_duration_since(end_at) <= MAX_VELOCITY_SAMPLE_AGE
        });
        let meaningful_sample = sample.is_some_and(|(distance, _, _, velocity)| {
            distance >= MIN_FLING_DISTANCE && velocity >= MIN_FLING_VELOCITY
        });

        fresh_motion && fresh_sample && meaningful_drag && meaningful_sample
    }

    fn prune_velocity_samples(&mut self, now: Instant) {
        while self
            .downward_samples
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > MAX_VELOCITY_SAMPLE_AGE)
        {
            self.downward_samples.pop_front();
        }
    }
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn later(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    #[test]
    fn offset_tracks_downward_translation_and_never_goes_negative() {
        let now = Instant::now();
        let mut drag = DrawerDrag::new(100.0, now);
        assert_eq!(drag.update(135.0, later(now, 20)), 35.0);
        assert_eq!(drag.offset(), 35.0);
        assert_eq!(drag.update(90.0, later(now, 40)), 0.0);
        assert_eq!(drag.offset(), 0.0);
    }

    #[test]
    fn distance_dismissal_uses_panel_fraction() {
        let now = Instant::now();
        let mut drag = DrawerDrag::new(0.0, now);
        drag.update(59.0, later(now, 100));
        assert!(!drag.should_dismiss(200.0));
        drag.update(60.0, later(now, 110));
        assert!(drag.should_dismiss(200.0));
    }

    #[test]
    fn meaningful_recent_downward_fling_dismisses() {
        let now = Instant::now();
        let mut drag = DrawerDrag::new(0.0, now);
        drag.update(24.0, later(now, 20));
        assert!(drag.release(later(now, 20), 300.0));
    }

    #[test]
    fn accumulated_small_pointer_events_can_form_a_fast_fling() {
        let now = Instant::now();
        let mut drag = DrawerDrag::new(0.0, now);
        for step in 1..=4 {
            drag.update(step as f32 * 5.0, later(now, step * 5));
        }
        assert_eq!(drag.offset(), 20.0);
        assert!(drag.release(later(now, 20), 300.0));
    }

    #[test]
    fn tiny_fast_jitter_and_stale_velocity_do_not_dismiss() {
        let now = Instant::now();
        let mut jitter = DrawerDrag::new(0.0, now);
        jitter.update(5.0, later(now, 1));
        assert!(!jitter.release(later(now, 1), 300.0));

        let mut stale = DrawerDrag::new(0.0, now);
        for step in 1..=4 {
            stale.update(step as f32 * 5.0, later(now, step * 5));
        }
        assert!(!stale.release(later(now, 200), 300.0));
    }

    #[test]
    fn upward_drag_does_not_dismiss() {
        let now = Instant::now();
        let mut drag = DrawerDrag::new(100.0, now);
        for step in 1..=4 {
            drag.update(100.0 + step as f32 * 5.0, later(now, step * 5));
        }
        drag.update(119.0, later(now, 25));
        assert_eq!(drag.offset(), 19.0);
        assert!(!drag.release(later(now, 25), 300.0));
    }
}
