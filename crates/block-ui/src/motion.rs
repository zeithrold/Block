//! Independent product motion using upstream timing, opacity and offset.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Motion is shared by the private product views."
)]

use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, App, Context, Div, ElementId, InteractiveElement as _, ParentElement as _, Pixels,
    Stateful, Styled as _, Window, div, px,
};

use gpui_base::motion::{Easing, Interpolate as _, Timing};

#[derive(Clone, Copy, Debug)]
struct DisclosureRun {
    from: f32,
    started: Instant,
}

/// The Home avatar's background changes independently of navigation and the
/// avatar button's hitbox. Reversals start at the currently painted fraction.
#[derive(Clone, Debug, Default)]
pub(super) struct HoverDecoration {
    hovered: bool,
    run: Cell<Option<DisclosureRun>>,
}

impl HoverDecoration {
    pub(super) fn set_hovered(&mut self, hovered: bool, now: Instant, preferences: Preferences) {
        if self.hovered == hovered {
            return;
        }
        let (from, _) = self.sample(now, preferences);
        self.hovered = hovered;
        self.run
            .set((!preferences.reduced()).then_some(DisclosureRun { from, started: now }));
    }

    pub(super) fn sample(&self, now: Instant, preferences: Preferences) -> (f32, bool) {
        let target = if self.hovered { 1. } else { 0. };
        let Some(run) = self.run.get() else {
            return (target, false);
        };
        let sample = Timing::new(Duration::from_millis(500))
            .ease(navigation_easing())
            .sample(now.saturating_duration_since(run.started));
        if preferences.reduced() || sample.finished {
            self.run.set(None);
            return (target, false);
        }
        (
            run.from.interpolate(&target, sample.directed_progress),
            true,
        )
    }
}

/// A disclosure retains its content through close, but stops its interaction
/// immediately. The measured height changes with the real content and viewport.
#[derive(Debug)]
pub(super) struct Disclosure {
    open: bool,
    run: Cell<Option<DisclosureRun>>,
    height: Rc<Cell<Pixels>>,
}

impl Default for Disclosure {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Clone for Disclosure {
    fn clone(&self) -> Self {
        Self {
            open: self.open,
            run: self.run.clone(),
            height: Rc::new(Cell::new(self.height.get())),
        }
    }
}

impl Disclosure {
    pub(super) fn new(open: bool) -> Self {
        Self {
            open,
            run: Cell::new(None),
            height: Rc::new(Cell::new(px(0.))),
        }
    }

    pub(super) const fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn set_open(&mut self, open: bool, now: Instant, preferences: Preferences) {
        if open == self.open {
            return;
        }
        let (from, _) = self.sample(now, preferences);
        self.open = open;
        self.run
            .set((!preferences.reduced()).then_some(DisclosureRun { from, started: now }));
    }

    fn sample(&self, now: Instant, preferences: Preferences) -> (f32, bool) {
        let target = if self.open { 1. } else { 0. };
        let Some(run) = self.run.get() else {
            return (target, false);
        };
        let sample = Timing::new(Duration::from_millis(350))
            .ease(navigation_easing())
            .sample(now.saturating_duration_since(run.started));
        if preferences.reduced() || sample.finished {
            self.run.set(None);
            return (target, false);
        }
        (
            run.from.interpolate(&target, sample.directed_progress),
            true,
        )
    }
}

pub(super) fn disclosure<T: 'static>(
    id: impl Into<ElementId>,
    state: &Disclosure,
    preferences: Preferences,
    content: AnyElement,
    window: &Window,
    cx: &Context<'_, T>,
) -> Stateful<Div> {
    let (fraction, running) = state.sample(cx.background_executor().now(), preferences);
    if running {
        window.request_animation_frame();
    }
    let height = state.height.clone();
    let entity = cx.entity().entity_id();
    let view = div().id(id).w_full().min_h_0().overflow_hidden();
    // At rest, natural layout responds immediately to new text and reduced
    // motion. A measured zero from the previous frame must not clip an open
    // control until a second animation frame happens to arrive.
    let view = if state.open && !running {
        view.h_auto()
    } else {
        view.h(state.height.get() * fraction)
    };
    view.child(
        div()
            .id("disclosure-content")
            .w_full()
            .h_auto()
            .flex_none()
            .opacity(fraction)
            .relative()
            .child(
                gpui::canvas(
                    move |bounds, window, cx| {
                        if height.get() != bounds.size.height {
                            height.set(bounds.size.height);
                            cx.notify(entity);
                            window.request_animation_frame();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(div().child(content)),
    )
}

pub(super) fn disclosure_trigger<T: 'static>(
    id: impl Into<gpui::SharedString>,
    label: impl Into<gpui::SharedString>,
    state: &Disclosure,
    palette: super::Palette,
    preferences: Preferences,
    cx: &Context<'_, T>,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> super::components::Control {
    let (fraction, _) = state.sample(cx.background_executor().now(), preferences);
    super::components::button(
        id,
        label,
        palette,
        super::components::ButtonVariant::Ghost,
        super::components::ControlState::default(),
        on_activate,
    )
    .w_full()
    .justify_between()
    .child(
        super::design::Icon::ChevronDown
            .render(14., palette.muted)
            .with_transformation(gpui::Transformation::rotate(gpui::radians(
                -std::f32::consts::PI * fraction,
            ))),
    )
}

/// Page, detail and wizard transitions have independent clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Page,
    Detail,
    Wizard,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Direction {
    #[default]
    Forward,
    Back,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Preferences {
    pub app_reduced: bool,
    pub system_reduced: bool,
}
impl Preferences {
    pub(super) const fn reduced(self) -> bool {
        self.app_reduced || self.system_reduced
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MotionStyle {
    Navigation,
    Wizard,
}

#[derive(Clone, Debug)]
pub(super) struct Layer<K, T> {
    pub key: K,
    pub _snapshot: T,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(super) struct Pose {
    pub x: f32,
    pub opacity: f32,
}
impl Default for Pose {
    fn default() -> Self {
        Self { x: 0., opacity: 1. }
    }
}
impl Pose {
    pub(super) fn apply<E: gpui::Styled>(self, element: E) -> E {
        element.relative().left(px(self.x)).opacity(self.opacity)
    }
    fn mix(self, target: Self, progress: f32) -> Self {
        Self {
            x: self.x.interpolate(&target.x, progress),
            opacity: self.opacity.interpolate(&target.opacity, progress),
        }
    }
}

pub(super) fn layer(id: impl Into<ElementId>, pose: Pose, content: AnyElement) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .left(px(pose.x))
        .top_0()
        .size_full()
        .min_w_0()
        .min_h_0()
        .opacity(pose.opacity)
        .child(content)
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub(super) struct Frame {
    pub current: Option<Pose>,
    pub outgoing: Option<Pose>,
    running: bool,
}
impl Frame {
    pub(super) const fn running(self) -> bool {
        self.running
    }
}

#[derive(Clone, Debug)]
pub(super) struct Transition<K, T> {
    style: MotionStyle,
    current: Option<Layer<K, T>>,
    outgoing: Option<Layer<K, T>>,
    outgoing_from: [Pose; 2],
    scope: Scope,
    direction: Direction,
    started: Option<Instant>,
    painted: Rc<Cell<Option<Instant>>>,
}
impl<K: PartialEq, T> Transition<K, T> {
    pub(super) fn new(scope: Scope) -> Self {
        Self {
            style: MotionStyle::Navigation,
            current: None,
            outgoing: None,
            outgoing_from: [Pose::default(); 2],
            scope,
            direction: Direction::Forward,
            started: None,
            painted: Rc::default(),
        }
    }
    pub(super) fn reconcile(
        &mut self,
        next: Option<Layer<K, T>>,
        direction: Direction,
        style: MotionStyle,
        now: Instant,
        preferences: Preferences,
    ) {
        let changed =
            self.current.as_ref().map(|layer| &layer.key) != next.as_ref().map(|layer| &layer.key);
        if changed {
            let previous = [
                self.sample(now, preferences),
                self.sample_delayed(now, preferences, 40),
            ];
            let retain_outgoing = previous[0].outgoing.is_some_and(|outgoing| {
                previous[0]
                    .current
                    .is_none_or(|current| outgoing.opacity > current.opacity)
            });
            self.outgoing_from = previous.map(|frame| {
                if retain_outgoing {
                    frame.outgoing.unwrap_or_default()
                } else {
                    frame.current.unwrap_or_default()
                }
            });
            self.outgoing = if next.is_some() && !preferences.reduced() {
                if retain_outgoing {
                    self.outgoing.take()
                } else {
                    self.current.take()
                }
            } else {
                None
            };
            self.started = self.outgoing.as_ref().map(|_| now);
            // A stale paint callback from an interrupted run cannot start its
            // replacement. Each run owns a fresh acknowledgement cell.
            self.painted = Rc::default();
            self.direction = direction;
            self.style = style;
        }
        self.current = next;
        if preferences.reduced() {
            self.settle();
        }
    }
    pub(super) fn sample(&self, now: Instant, preferences: Preferences) -> Frame {
        self.sample_delayed(now, preferences, 0)
    }
    pub(super) fn sample_delayed(
        &self,
        now: Instant,
        preferences: Preferences,
        delay_ms: u64,
    ) -> Frame {
        let duration = Duration::from_millis(if self.style == MotionStyle::Wizard {
            300
        } else if self.scope == Scope::Page {
            200
        } else {
            180
        });
        let delay = Duration::from_millis(delay_ms);
        let elapsed = self.started.filter(|_| !preferences.reduced()).map(|_| {
            self.painted.get().map_or(Duration::ZERO, |painted| {
                now.saturating_duration_since(painted)
            })
        });
        let progress = self
            .started
            .filter(|_| !preferences.reduced())
            .map_or(1., |_| {
                let elapsed = elapsed.unwrap_or_default();
                if elapsed < delay {
                    return 0.;
                }
                Timing::new(duration)
                    .ease(navigation_easing())
                    .sample(elapsed - delay)
                    .directed_progress
            });
        let sign = if self.direction == Direction::Forward {
            1.
        } else {
            -1.
        };
        // The immediate page track belongs to the sidebar. Delayed page
        // content and in-page content share the slightly longer travel.
        let distance = if self.style == MotionStyle::Wizard {
            0.
        } else if self.scope == Scope::Page && delay_ms == 0 {
            12.
        } else {
            20.
        };
        Frame {
            current: self.current.as_ref().map(|_| Pose {
                x: (1. - progress) * sign * distance,
                opacity: if self.style == MotionStyle::Wizard {
                    1.
                } else {
                    progress
                },
            }),
            outgoing: self.outgoing.as_ref().map(|_| {
                self.outgoing_from[usize::from(delay_ms != 0)].mix(
                    Pose {
                        x: -sign * distance,
                        opacity: if self.style == MotionStyle::Wizard {
                            1.
                        } else {
                            0.
                        },
                    },
                    progress,
                )
            }),
            running: elapsed.is_some_and(|elapsed| elapsed < duration + delay),
        }
    }
    pub(super) fn frame_with_base(
        &mut self,
        _: impl Into<ElementId>,
        preferences: Preferences,
        _: &mut Window,
        cx: &mut App,
    ) -> Frame {
        let frame = self.sample_delayed(
            cx.background_executor().now(),
            preferences,
            if self.scope == Scope::Page { 40 } else { 0 },
        );
        if !frame.running() {
            self.settle();
        }
        frame
    }
    pub(super) const fn current(&self) -> Option<&Layer<K, T>> {
        self.current.as_ref()
    }
    pub(super) const fn outgoing(&self) -> Option<&Layer<K, T>> {
        self.outgoing.as_ref()
    }
    pub(super) fn paint_probe(&self) -> Option<impl gpui::IntoElement + use<K, T>> {
        if self.started.is_none() || self.painted.get().is_some() {
            return None;
        }
        let painted = self.painted.clone();
        Some(
            gpui::canvas(
                |_, _, _| {},
                move |_, (), window, cx| {
                    if painted.get().is_none() {
                        painted.set(Some(cx.background_executor().now()));
                        window.request_animation_frame();
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
    }
    pub(super) fn settle(&mut self) {
        self.started = None;
        self.outgoing = None;
    }
}

const fn navigation_easing() -> Easing {
    Easing::CubicBezier {
        x1: 0.16,
        y1: 1.,
        x2: 0.3,
        y2: 1.,
    }
}

#[cfg(test)]
mod navigation_clock_tests {
    use super::{Direction, Layer, MotionStyle, Preferences, Scope, Transition};
    use std::time::{Duration, Instant};

    fn page() -> (Transition<&'static str, ()>, Instant) {
        let now = Instant::now();
        let mut page = Transition::new(Scope::Page);
        for key in ["a", "b"] {
            page.reconcile(
                Some(Layer { key, _snapshot: () }),
                Direction::Forward,
                MotionStyle::Navigation,
                now,
                Preferences::default(),
            );
        }
        (page, now)
    }

    #[test]
    fn a_slow_first_paint_does_not_consume_the_pane_start_delay() {
        let (page, now) = page();
        let first_paint = now + Duration::from_millis(500);
        assert!(
            page.sample(first_paint, Preferences::default())
                .current
                .is_some_and(|pose| pose.x == 12. && pose.opacity == 0.)
        );
        page.painted.set(Some(first_paint));
        let sample = first_paint + Duration::from_millis(20);
        assert!(
            page.sample(sample, Preferences::default())
                .current
                .is_some_and(|pose| pose.x > 0. && pose.x < 12. && pose.opacity > 0.)
        );
        assert!(
            page.sample_delayed(sample, Preferences::default(), 40)
                .current
                .is_some_and(|pose| pose.opacity == 0.)
        );
        assert!(
            !page
                .sample_delayed(
                    first_paint + Duration::from_millis(240),
                    Preferences::default(),
                    40
                )
                .running()
        );
    }

    #[test]
    fn a_stale_paint_acknowledgement_cannot_start_a_retargeted_transition() {
        let (mut page, now) = page();
        let stale = page.painted.clone();
        page.reconcile(
            Some(Layer {
                key: "c",
                _snapshot: (),
            }),
            Direction::Forward,
            MotionStyle::Navigation,
            now,
            Preferences::default(),
        );
        stale.set(Some(now));
        let frame = page.sample(now + Duration::from_secs(1), Preferences::default());
        assert!(
            frame
                .current
                .is_some_and(|pose| pose.x == 12. && pose.opacity == 0.)
        );
        assert!(page.outgoing().is_some_and(|layer| layer.key == "a"));
    }
}
