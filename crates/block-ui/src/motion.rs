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
    Stateful, Styled as _, Window, div, px, relative,
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
}

#[derive(Clone, Debug)]
pub(super) struct Layer<K, T> {
    pub key: K,
    pub _snapshot: T,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Pose {
    pub x_fraction: f32,
    pub opacity: f32,
}
impl Default for Pose {
    fn default() -> Self {
        Self {
            x_fraction: 0.,
            opacity: 1.,
        }
    }
}

pub(super) fn layer(id: impl Into<ElementId>, pose: Pose, content: AnyElement) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .left(relative(pose.x_fraction))
        .top_0()
        .size_full()
        .min_w_0()
        .min_h_0()
        .opacity(pose.opacity)
        .child(content)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Frame {
    pub current: Option<Pose>,
    running: bool,
}
impl Frame {
    pub(super) const fn running(self) -> bool {
        self.running
    }
}

#[derive(Clone, Debug)]
pub(super) struct Transition<K, T> {
    current: Option<Layer<K, T>>,
    direction: Direction,
    started: Option<Instant>,
}
impl<K: PartialEq, T> Transition<K, T> {
    pub(super) const fn new(_: Scope) -> Self {
        Self {
            current: None,
            direction: Direction::Forward,
            started: None,
        }
    }
    pub(super) fn reconcile(
        &mut self,
        next: Option<Layer<K, T>>,
        direction: Direction,
        _: MotionStyle,
        now: Instant,
        preferences: Preferences,
    ) {
        let changed =
            self.current.as_ref().map(|layer| &layer.key) != next.as_ref().map(|layer| &layer.key);
        if changed {
            self.started =
                (self.current.is_some() && next.is_some() && !preferences.reduced()).then_some(now);
            self.direction = direction;
        }
        self.current = next;
        if preferences.reduced() {
            self.settle();
        }
    }
    pub(super) fn sample(&self, now: Instant, preferences: Preferences) -> Frame {
        let progress = self
            .started
            .filter(|_| !preferences.reduced())
            .map_or(1., |started| {
                Timing::new(Duration::from_millis(460))
                    .ease(navigation_easing())
                    .sample(now.saturating_duration_since(started))
                    .directed_progress
            });
        let sign = if self.direction == Direction::Forward {
            1.
        } else {
            -1.
        };
        Frame {
            current: self.current.as_ref().map(|_| Pose {
                x_fraction: (1. - progress) * sign,
                opacity: progress,
            }),
            running: progress < 1.,
        }
    }
    pub(super) fn frame_with_base(
        &mut self,
        _: impl Into<ElementId>,
        preferences: Preferences,
        _: &mut Window,
        cx: &mut App,
    ) -> Frame {
        let frame = self.sample(cx.background_executor().now(), preferences);
        if !frame.running() {
            self.settle();
        }
        frame
    }
    pub(super) const fn current(&self) -> Option<&Layer<K, T>> {
        self.current.as_ref()
    }
    pub(super) fn settle(&mut self) {
        self.started = None;
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
