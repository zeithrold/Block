//! Reusable vertical entrance for authentication and game wizard bodies.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Wizard entrance motion is private product presentation."
)]

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, Context, IntoElement as _, ParentElement as _, Styled as _, Window, div, px,
};
use gpui_base::motion::{Easing, Interpolate as _, Timing};

use crate::motion::Preferences;

const BODY_DURATION: Duration = Duration::from_millis(300);
const CHILD_DURATION: Duration = Duration::from_millis(180);
const CHILD_ENTRY_DELAY: Duration = Duration::from_millis(55);
const CHILD_TRAVEL: f32 = 12.;

#[derive(Clone, Copy, Debug)]
struct Run {
    from: f32,
    to: f32,
    started: Instant,
    delay: Duration,
    duration: Duration,
}

#[derive(Debug, Default)]
struct State {
    active: bool,
    body: f32,
    child: f32,
    body_run: Option<Run>,
    child_run: Option<Run>,
    max_child_delay: Duration,
}

/// Motion state shared by the live wizard and its outgoing visual snapshot.
#[derive(Clone, Debug, Default)]
pub(super) struct WizardEntrance(Rc<RefCell<State>>);

impl WizardEntrance {
    /// Reconcile the visible wizard lifecycle. Calling this with the same
    /// `active` value never restarts an in-flight entrance.
    pub(super) fn reconcile(&self, active: bool, now: Instant, preferences: Preferences) {
        let mut state = self.0.borrow_mut();
        let settled_body = state.body;
        let settled_child = state.child;
        let max_child_delay = state.max_child_delay;
        let body = sample_track(&mut state.body_run, settled_body, now, Duration::ZERO);
        let child = sample_track(&mut state.child_run, settled_child, now, max_child_delay);
        let changed = state.active != active;
        if preferences.reduced() {
            state.active = active;
            state.body = if active { 1. } else { 0. };
            state.child = if active { 1. } else { 0. };
            state.body_run = None;
            state.child_run = None;
        } else if changed {
            let target = if active { 1. } else { 0. };
            state.active = active;
            state.body = body;
            state.child = child;
            state.body_run = Some(Run {
                from: body,
                to: target,
                started: now,
                delay: Duration::ZERO,
                duration: BODY_DURATION,
            });
            state.child_run = Some(Run {
                from: child,
                to: target,
                started: now,
                delay: if active {
                    CHILD_ENTRY_DELAY
                } else {
                    Duration::ZERO
                },
                duration: CHILD_DURATION,
            });
        } else {
            state.body = body;
            state.child = child;
        }
    }

    /// Move the whole body from one content viewport below its final position.
    /// `content_height` excludes the fixed 56 px title area.
    pub(super) fn apply<T: 'static>(
        &self,
        content: AnyElement,
        content_height: gpui::Pixels,
        window: &Window,
        cx: &Context<'_, T>,
    ) -> AnyElement {
        let now = cx.background_executor().now();
        let (fraction, running) = {
            let mut state = self.0.borrow_mut();
            let settled = state.body;
            let fraction = sample_track(&mut state.body_run, settled, now, Duration::ZERO);
            state.body = fraction;
            (fraction, state.body_run.is_some())
        };
        if running {
            window.request_animation_frame();
        }
        div()
            .relative()
            .size_full()
            .min_h_0()
            .top(content_height * (1. - fraction))
            .child(content)
            .into_any_element()
    }

    /// Add the small staggered child entrance: 12 px of travel and opacity.
    /// Supply an extra delay to order siblings; reversal starts from the current
    /// child fraction and runs without an additional delay.
    pub(super) fn stagger<T: 'static>(
        &self,
        content: AnyElement,
        delay: Duration,
        window: &Window,
        cx: &Context<'_, T>,
    ) -> AnyElement {
        let now = cx.background_executor().now();
        let (fraction, running) = {
            let mut state = self.0.borrow_mut();
            state.max_child_delay = state.max_child_delay.max(delay);
            let mut per_child_run = state.child_run;
            let fraction = sample_track(&mut per_child_run, state.child, now, delay);
            let running = per_child_run.is_some();
            (fraction, running)
        };
        if running {
            window.request_animation_frame();
        }
        div()
            .relative()
            .top(px(CHILD_TRAVEL * (1. - fraction)))
            .opacity(fraction)
            .h_full()
            .min_h_0()
            .child(content)
            .into_any_element()
    }
}

fn sample_track(run: &mut Option<Run>, settled: f32, now: Instant, extra_delay: Duration) -> f32 {
    let Some(track) = *run else {
        return settled;
    };
    let delay = track.delay.saturating_add(if track.to > track.from {
        extra_delay
    } else {
        Duration::ZERO
    });
    let elapsed = now.saturating_duration_since(track.started);
    if elapsed < delay {
        return track.from;
    }
    let progress = Timing::new(track.duration)
        .ease(Easing::CubicBezier {
            x1: 0.16,
            y1: 1.,
            x2: 0.3,
            y2: 1.,
        })
        .sample(elapsed - delay);
    if progress.finished {
        *run = None;
        track.to
    } else {
        track
            .from
            .interpolate(&track.to, progress.directed_progress)
    }
}

#[cfg(test)]
mod tests {
    use super::{BODY_DURATION, Run, WizardEntrance, sample_track};
    use crate::motion::Preferences;
    use std::time::{Duration, Instant};

    #[test]
    fn body_track_finishes_after_300_ms_and_reversal_is_continuous() {
        let start = Instant::now();
        let mut run = Some(Run {
            from: 0.,
            to: 1.,
            started: start,
            delay: Duration::ZERO,
            duration: BODY_DURATION,
        });
        let current = sample_track(
            &mut run,
            0.,
            start + Duration::from_millis(120),
            Duration::ZERO,
        );
        assert!(current > 0.5 && current < 1.);
        run = Some(Run {
            from: current,
            to: 0.,
            started: start + Duration::from_millis(120),
            delay: Duration::ZERO,
            duration: BODY_DURATION,
        });
        assert_eq!(
            sample_track(
                &mut run,
                current,
                start + Duration::from_millis(120),
                Duration::ZERO
            ),
            current
        );
        assert_eq!(
            sample_track(
                &mut run,
                current,
                start + Duration::from_millis(420),
                Duration::ZERO
            ),
            0.
        );
        assert!(run.is_none());
    }

    #[test]
    fn child_delay_only_holds_child_progress() {
        let start = Instant::now();
        let mut run = Some(Run {
            from: 0.,
            to: 1.,
            started: start,
            delay: Duration::from_millis(55),
            duration: Duration::from_millis(180),
        });
        assert_eq!(
            sample_track(
                &mut run,
                0.,
                start + Duration::from_millis(40),
                Duration::ZERO
            ),
            0.
        );
        assert_eq!(
            sample_track(
                &mut run,
                0.,
                start + Duration::from_millis(40),
                Duration::from_millis(20)
            ),
            0.
        );
        assert!(
            sample_track(
                &mut run,
                0.,
                start + Duration::from_millis(80),
                Duration::ZERO
            ) > 0.
        );
        assert!(run.is_some());
    }

    #[test]
    fn reduced_motion_settles_active_state_without_a_run() {
        let entrance = WizardEntrance::default();
        let now = Instant::now();
        entrance.reconcile(
            true,
            now,
            Preferences {
                app_reduced: false,
                system_reduced: true,
            },
        );
        let state = entrance.0.borrow();
        assert_eq!(state.body, 1.);
        assert_eq!(state.child, 1.);
        assert!(state.body_run.is_none() && state.child_run.is_none());
        assert!(state.active);
    }
}
