//! Owned visual snapshots and interruptible, independent transition lifetimes.
//!
//! Snapshots contain render data, never a GPUI `AnyElement`: elements belong to
//! the current frame's arena. The caller renders an outgoing snapshot again on
//! each frame, inside an inert subtree, and renders the current product normally.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Motion is shared by the private product views."
)]

use std::{
    cell::Cell,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, App, Context, Div, ElementId, InteractiveElement as _, ParentElement as _, Pixels,
    Point, Size, Stateful, Styled as _, Window, div, point, px, relative,
};

use gpui_base::motion::{
    Easing, Interpolate as _, MotionStatus, Presence, PresenceSample, Timing,
    Transition as BaseTransition,
};

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
            .on_prepaint_bounds(move |_, bounds, window, cx| {
                if height.get() != bounds.size.height {
                    height.set(bounds.size.height);
                    cx.notify(entity);
                    window.request_animation_frame();
                }
            })
            .child(div().inert(!state.open).child(content)),
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
    .accessibility_expanded(state.open)
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

/// A view change owns one scope. A task update within that view owns none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Page,
    Detail,
    Wizard,
    Overlay,
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
pub(super) enum Edge {
    Top,
    Right,
    Bottom,
    Left,
}

/// Popup durations and curves follow the frozen product reference. Tooltip's
/// animation uses the locked animation stylesheet's 150ms default; Dialog and
/// Menu explicitly override it to 100ms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MotionStyle {
    Navigation,
    Dialog,
    Menu { edge: Edge, align_trigger: bool },
    Tooltip { edge: Edge },
    Drawer { edge: Edge },
}

impl MotionStyle {
    pub(super) const fn duration(self) -> Duration {
        Duration::from_millis(match self {
            Self::Navigation => 460,
            Self::Dialog | Self::Menu { .. } => 100,
            Self::Tooltip { .. } => 150,
            Self::Drawer { .. } => 450,
        })
    }

    const fn easing(self) -> Easing {
        match self {
            Self::Navigation => navigation_easing(),
            Self::Drawer { .. } => Easing::CubicBezier {
                x1: 0.22,
                y1: 1.,
                x2: 0.36,
                y2: 1.,
            },
            Self::Dialog | Self::Menu { .. } | Self::Tooltip { .. } => Easing::Ease,
        }
    }

    fn timing(self) -> Timing {
        Timing::new(self.duration()).ease(self.easing())
    }
}

/// A scope-qualified generation prevents a late completion from settling a
/// newer navigation or another independently running surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Token {
    scope: Scope,
    generation: u64,
}

#[derive(Clone, Debug)]
pub(super) struct Layer<K, T> {
    pub key: K,
    pub snapshot: T,
}

/// Translation is relative to the logical viewport, so a live resize preserves
/// the transition fraction instead of leaving a fixed-width strip behind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pose {
    pub x_fraction: f32,
    pub y_fraction: f32,
    pub x_pixels: f32,
    pub y_pixels: f32,
    pub opacity: f32,
    /// Uniform paint scale, without changing the layout or shaping the text
    /// again. Presenters choose a center or the actual anchor-facing origin.
    pub scale: f32,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            x_fraction: 0.,
            y_fraction: 0.,
            x_pixels: 0.,
            y_pixels: 0.,
            opacity: 1.,
            scale: 1.,
        }
    }
}

impl Pose {
    /// Overlay presenters know their sheet's logical bounds. Scope wrappers use
    /// parent-relative layout instead, so details never translate by window width.
    pub(super) fn offset(self, viewport: Size<Pixels>) -> Point<Pixels> {
        point(
            viewport.width * self.x_fraction + px(self.x_pixels),
            viewport.height * self.y_fraction + px(self.y_pixels),
        )
    }
}

/// The parent supplies a relative, clipped viewport. Construct each child anew
/// for this frame. Only an outgoing snapshot passes `inert = true`; it keeps its
/// visual geometry while contributing no input, focus, Tab stops, or AX nodes.
pub(super) fn layer(
    id: impl Into<ElementId>,
    pose: Pose,
    inert: bool,
    content: AnyElement,
) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .left(relative(pose.x_fraction))
        .top(relative(pose.y_fraction))
        .size_full()
        .min_w_0()
        .min_h_0()
        .opacity(pose.opacity)
        .inert(inert)
        .child(
            div()
                .relative()
                .left(px(pose.x_pixels))
                .top(px(pose.y_pixels))
                .size_full()
                .min_w_0()
                .min_h_0()
                .child(content),
        )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Frame {
    pub current: Option<Pose>,
    pub outgoing: Option<Pose>,
    pub token: Option<Token>,
    /// Keep the overlay's shield mounted throughout a closing transition. Its
    /// outgoing content is inert; the shield still prevents click-through.
    pub captures_input: bool,
    pub backdrop_opacity: f32,
}

impl Frame {
    pub(super) const fn running(self) -> bool {
        self.token.is_some()
    }
}

#[derive(Clone, Copy, Debug)]
struct Run {
    token: Token,
    started: Instant,
    style: MotionStyle,
    direction: Direction,
}

#[derive(Clone, Debug)]
pub(super) struct Transition<K, T> {
    scope: Scope,
    current: Option<Layer<K, T>>,
    outgoing: Option<Layer<K, T>>,
    run: Option<Run>,
    generation: u64,
    outgoing_generation: Option<u64>,
    base_frame: Option<Frame>,
}

impl<K: PartialEq, T> Transition<K, T> {
    pub(super) const fn new(scope: Scope) -> Self {
        Self {
            scope,
            current: None,
            outgoing: None,
            run: None,
            generation: 0,
            outgoing_generation: None,
            base_frame: None,
        }
    }

    /// Provide the newest owned visual data. Updating the same identity refreshes
    /// the live snapshot without changing its start time or the frozen exit.
    /// A new identity drops an interrupted exit, matching a skipped transition.
    pub(super) fn reconcile(
        &mut self,
        next: Option<Layer<K, T>>,
        direction: Direction,
        style: MotionStyle,
        now: Instant,
        preferences: Preferences,
    ) {
        self.base_frame = None;
        let same_key = match (&self.current, &next) {
            (Some(current), Some(next)) => current.key == next.key,
            (None, None) => true,
            _ => false,
        };
        if same_key {
            self.current = next;
            if preferences.reduced() {
                self.settle();
            }
            return;
        }

        self.outgoing_generation = self.current.as_ref().map(|_| self.generation);
        self.outgoing = self.current.take();
        self.current = next;
        self.generation = self.generation.wrapping_add(1);
        let first_navigation = self.outgoing.is_none() && style == MotionStyle::Navigation;
        let unanimated_select = matches!(
            style,
            MotionStyle::Menu {
                align_trigger: true,
                ..
            }
        );
        if preferences.reduced() || first_navigation || unanimated_select {
            self.settle();
        } else {
            self.run = Some(Run {
                token: Token {
                    scope: self.scope,
                    generation: self.generation,
                },
                started: now,
                style,
                direction,
            });
        }
    }

    /// Call with the GPUI executor's clock. A deterministic test can advance that
    /// clock without sleeps; system/app reduced motion settles a running exit.
    pub(super) fn frame(&mut self, now: Instant, preferences: Preferences) -> Frame {
        self.base_frame = None;
        if preferences.reduced() {
            self.settle();
        } else if let Some(run) = self.run
            && run
                .style
                .timing()
                .sample(now.saturating_duration_since(run.started))
                .finished
        {
            self.finish(run.token);
        }
        self.sample(now, preferences)
    }

    /// Read geometry from immutable page renderers. The owner calls `frame` to
    /// release completed snapshots; sampling after completion already returns
    /// the settled geometry and never paints an expired interaction layer.
    pub(super) fn sample(&self, now: Instant, preferences: Preferences) -> Frame {
        if !preferences.reduced()
            && let Some(frame) = self.base_frame
        {
            return frame;
        }
        let Some(run) = self.run.filter(|run| {
            !preferences.reduced()
                && !run
                    .style
                    .timing()
                    .sample(now.saturating_duration_since(run.started))
                    .finished
        }) else {
            return Frame {
                current: self.current.as_ref().map(|_| Pose::default()),
                outgoing: None,
                token: None,
                captures_input: self.scope == Scope::Overlay && self.current.is_some(),
                backdrop_opacity: if self.current.is_some() { 1. } else { 0. },
            };
        };
        let elapsed = now.saturating_duration_since(run.started);
        let progress = run.style.timing().sample(elapsed).directed_progress;
        let opening = self.outgoing.is_none();
        let closing = self.current.is_none();
        let backdrop_progress = if matches!(run.style, MotionStyle::Drawer { .. }) {
            Timing::new(run.style.duration())
                .ease(backdrop_easing())
                .sample(elapsed)
                .directed_progress
        } else {
            progress
        };
        Frame {
            current: self.current.as_ref().map(|_| pose(run, progress, false)),
            outgoing: self.outgoing.as_ref().map(|_| pose(run, progress, true)),
            token: Some(run.token),
            captures_input: self.scope == Scope::Overlay,
            backdrop_opacity: if opening {
                backdrop_progress
            } else if closing {
                1. - backdrop_progress
            } else {
                1.
            },
        }
    }

    /// Advance real product layers through Base Presence. Only the immutable
    /// sampled frame is exposed to renderers; frozen data ownership stays here.
    pub(super) fn frame_with_base(
        &mut self,
        id: impl Into<ElementId>,
        preferences: Preferences,
        window: &mut Window,
        cx: &mut App,
    ) -> Frame
    where
        K: Hash,
    {
        if preferences.reduced() || cx.reduce_motion() {
            self.settle();
        }
        let id = id.into();
        let run = self.run;
        let style = run.map_or(MotionStyle::Navigation, |run| run.style);
        let duration = if run.is_some() {
            style.duration()
        } else {
            Duration::ZERO
        };
        let current = self.current.as_ref().map(|layer| {
            sample_presence(
                &id,
                &layer.key,
                self.generation,
                true,
                duration,
                style.easing(),
                window,
                cx,
            )
        });
        let outgoing = self.outgoing.as_ref().map(|layer| {
            sample_presence(
                &id,
                &layer.key,
                self.outgoing_generation.unwrap_or(self.generation),
                false,
                duration,
                style.easing(),
                window,
                cx,
            )
        });
        let running = current
            .into_iter()
            .chain(outgoing)
            .any(|sample| matches!(sample.status, MotionStatus::Delayed | MotionStatus::Running));
        let backdrop_opacity =
            if self.scope != Scope::Overlay || (current.is_some() && outgoing.is_some()) {
                if self.current.is_some() { 1. } else { 0. }
            } else if let Some(layer) = self.current.as_ref().or(self.outgoing.as_ref()) {
                let generation = if self.current.is_some() {
                    self.generation
                } else {
                    self.outgoing_generation.unwrap_or(self.generation)
                };
                let backdrop_id = ElementId::NamedChild(id.into(), "backdrop".into());
                let easing = if matches!(style, MotionStyle::Drawer { .. }) {
                    backdrop_easing()
                } else {
                    style.easing()
                };
                sample_presence(
                    &backdrop_id,
                    &layer.key,
                    generation,
                    self.current.is_some(),
                    duration,
                    easing,
                    window,
                    cx,
                )
                .progress
            } else {
                0.
            };
        let frame = if let Some(run) = run.filter(|_| running) {
            Frame {
                current: current.map(|sample| pose(run, sample.progress, false)),
                outgoing: outgoing
                    .filter(|sample| sample.should_render())
                    .map(|sample| pose(run, 1. - sample.progress, true)),
                token: Some(run.token),
                captures_input: self.scope == Scope::Overlay,
                backdrop_opacity,
            }
        } else {
            self.settle();
            Frame {
                current: self.current.as_ref().map(|_| Pose::default()),
                outgoing: None,
                token: None,
                captures_input: self.scope == Scope::Overlay && self.current.is_some(),
                backdrop_opacity: if self.current.is_some() { 1. } else { 0. },
            }
        };
        self.base_frame = Some(frame);
        frame
    }

    pub(super) const fn current(&self) -> Option<&Layer<K, T>> {
        self.current.as_ref()
    }

    pub(super) const fn outgoing(&self) -> Option<&Layer<K, T>> {
        self.outgoing.as_ref()
    }

    /// Only a matching completion owns the retained exit. Prefer `frame` for
    /// ordinary rendering; this also supports a scheduled completion callback.
    pub(super) fn finish(&mut self, token: Token) -> bool {
        if self.run.is_some_and(|run| run.token == token) {
            self.settle();
            true
        } else {
            false
        }
    }

    pub(super) fn settle(&mut self) {
        self.outgoing = None;
        self.outgoing_generation = None;
        self.run = None;
        self.base_frame = None;
    }
}

fn pose(run: Run, progress: f32, outgoing: bool) -> Pose {
    let distance = if outgoing { progress } else { 1. - progress };
    let mut pose = Pose::default();
    match run.style {
        MotionStyle::Navigation => {
            let sign = if run.direction == Direction::Forward {
                1.
            } else {
                -1.
            };
            pose.x_fraction = distance * if outgoing { -sign } else { sign };
        }
        MotionStyle::Dialog => {
            pose.opacity = if outgoing { 1. - progress } else { progress };
            pose.scale = 0.05_f32.mul_add(pose.opacity, 0.95);
        }
        MotionStyle::Menu { edge, .. } | MotionStyle::Tooltip { edge } => {
            pose.opacity = if outgoing { 1. - progress } else { progress };
            pose.scale = 0.05_f32.mul_add(pose.opacity, 0.95);
            match edge {
                Edge::Top => pose.y_pixels = distance * 8.,
                Edge::Right => pose.x_pixels = distance * -8.,
                Edge::Bottom => pose.y_pixels = distance * -8.,
                Edge::Left => pose.x_pixels = distance * 8.,
            }
        }
        MotionStyle::Drawer { edge } => match edge {
            Edge::Top => {
                pose.y_fraction = -distance;
                pose.y_pixels = distance * -2.;
            }
            Edge::Right => {
                pose.x_fraction = distance;
                pose.x_pixels = distance * 2.;
            }
            Edge::Bottom => {
                pose.y_fraction = distance;
                pose.y_pixels = distance * 2.;
            }
            Edge::Left => {
                pose.x_fraction = -distance;
                pose.x_pixels = distance * -2.;
            }
        },
    }
    pose
}

const fn navigation_easing() -> Easing {
    Easing::CubicBezier {
        x1: 0.16,
        y1: 1.,
        x2: 0.3,
        y2: 1.,
    }
}

const fn backdrop_easing() -> Easing {
    Easing::CubicBezier {
        x1: 0.32,
        y1: 0.72,
        x2: 0.,
        y2: 1.,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The product supplies one independent Base Presence identity, target and timing policy."
)]
fn sample_presence<K: Hash>(
    id: &ElementId,
    key: &K,
    generation: u64,
    present: bool,
    duration: Duration,
    easing: Easing,
    window: &mut Window,
    cx: &mut App,
) -> PresenceSample {
    let mut hash = DefaultHasher::new();
    key.hash(&mut hash);
    generation.hash(&mut hash);
    let channel = ElementId::NamedChild(
        id.clone().into(),
        format!("layer-{:016x}", hash.finish()).into(),
    );
    Presence::new(channel, present)
        .transition(BaseTransition::new(duration).easing(easing))
        .sample(window, cx)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{
        Direction, Edge, Layer, MotionStyle, Pose, Preferences, Scope, Transition,
        navigation_easing,
    };
    use gpui::{
        Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
        Window, div, point, px, size,
    };

    struct BasePresenceFixture {
        key: u8,
        preferences: Preferences,
        scene: Transition<u8, ()>,
        sampled: Option<super::Frame>,
    }

    impl Render for BasePresenceFixture {
        fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            cx.set_reduce_motion(self.preferences.reduced());
            self.scene.reconcile(
                Some(Layer {
                    key: self.key,
                    snapshot: (),
                }),
                Direction::Forward,
                MotionStyle::Navigation,
                cx.background_executor().now(),
                self.preferences,
            );
            let frame =
                self.scene
                    .frame_with_base("base-product-scene", self.preferences, window, cx);
            self.sampled = Some(frame);
            let viewport = div().w(px(320.)).h(px(100.)).relative().overflow_hidden();
            let viewport = if let Some(pose) = frame.outgoing {
                viewport.child(super::layer(
                    "base-outgoing",
                    pose,
                    true,
                    div()
                        .id("base-outgoing-body")
                        .debug_selector(|| "base-outgoing-body".into())
                        .size_full()
                        .into_any_element(),
                ))
            } else {
                viewport
            };
            viewport.child(super::layer(
                "base-current",
                frame.current.unwrap_or_default(),
                false,
                div()
                    .id("base-current-body")
                    .debug_selector(|| "base-current-body".into())
                    .size_full()
                    .into_any_element(),
            ))
        }
    }

    #[gpui::test]
    fn base_presence_drives_real_product_layers_and_releases_the_exit(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(|_, _| BasePresenceFixture {
            key: 1,
            preferences: Preferences::default(),
            scene: Transition::new(Scope::Detail),
            sampled: None,
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.sampled.is_some_and(|frame| !frame.running())
        }));
        view.update(cx, |view, cx| {
            view.key = 2;
            cx.notify();
        });
        cx.run_until_parked();
        let token = view.read_with(cx, |view, _| view.sampled.and_then(|frame| frame.token));
        assert!(token.is_some());
        assert_eq!(
            cx.debug_bounds("base-current-body")
                .map(|bounds| bounds.left()),
            Some(px(320.))
        );
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let current = cx.debug_bounds("base-current-body");
        let outgoing = cx.debug_bounds("base-outgoing-body");
        assert!(current.is_some_and(|bounds| bounds.left() > px(0.) && bounds.left() < px(320.)));
        assert!(outgoing.is_some_and(|bounds| bounds.left() < px(0.)));
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.sampled.and_then(|frame| frame.token)),
            token
        );
        cx.executor().advance_clock(Duration::from_millis(360));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.scene.outgoing().is_none()));
        assert!(cx.debug_bounds("base-outgoing-body").is_none());
        assert_eq!(
            cx.debug_bounds("base-current-body")
                .map(|bounds| bounds.left()),
            Some(px(0.))
        );
    }

    #[gpui::test]
    fn base_presence_uses_the_effective_reduced_motion_policy_without_replaying(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(|_, _| BasePresenceFixture {
            key: 1,
            preferences: Preferences::default(),
            scene: Transition::new(Scope::Detail),
            sampled: None,
        });
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            view.key = 2;
            cx.notify();
        });
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            view.preferences.system_reduced = true;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.update(|_, cx| cx.reduce_motion()));
        assert!(view.read_with(cx, |view, _| view.scene.outgoing().is_none()
            && view.sampled.is_some_and(|frame| !frame.running())));
        view.update(cx, |view, cx| {
            view.preferences.system_reduced = false;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.sampled.is_some_and(|frame| !frame.running())
        }));
        assert_eq!(
            cx.debug_bounds("base-current-body")
                .map(|bounds| bounds.left()),
            Some(px(0.))
        );
    }

    #[test]
    fn disclosure_reverses_from_visible_height_and_reduced_motion_cancels_the_run() {
        let now = Instant::now();
        let normal = Preferences::default();
        let mut state = super::Disclosure::new(false);
        state.set_open(true, now, normal);
        let halfway = now + Duration::from_millis(100);
        let (height, running) = state.sample(halfway, normal);
        assert!(running && height > 0. && height < 1.);
        state.set_open(false, halfway, normal);
        assert_eq!(state.sample(halfway, normal).0, height);
        assert!(state.sample(halfway + Duration::from_millis(50), normal).0 < height);
        let reduced = Preferences {
            system_reduced: true,
            ..normal
        };
        assert_eq!(state.sample(halfway, reduced), (0., false));
        assert_eq!(state.sample(halfway, normal), (0., false));
        let frozen = state.clone();
        state.height.set(px(240.));
        assert_eq!(frozen.height.get(), px(0.));
    }

    struct DisclosureFixture {
        state: super::Disclosure,
        activations: usize,
    }

    impl Render for DisclosureFixture {
        fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .w(px(320.))
                .child(super::disclosure(
                    "disclosure",
                    &self.state,
                    Preferences::default(),
                    div()
                        .h(px(120.))
                        .child(crate::components::button(
                            "disclosure-action",
                            "Action",
                            crate::Palette::LIGHT,
                            crate::components::ButtonVariant::Outline,
                            crate::components::ControlState::default(),
                            cx.listener(|view, (): &(), _, _| view.activations += 1),
                        ))
                        .into_any_element(),
                    window,
                    cx,
                ))
                .child(
                    div()
                        .id("disclosure-footer")
                        .debug_selector(|| "disclosure-footer".into())
                        .h(px(32.)),
                )
        }
    }

    #[gpui::test]
    fn disclosure_measures_real_content_and_closing_content_stops_interaction_immediately(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(|_, _| DisclosureFixture {
            state: super::Disclosure::new(true),
            activations: 0,
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("disclosure-footer")
                .map(|bounds| bounds.top()),
            Some(px(120.))
        );
        let action = cx.debug_bounds("disclosure-action");
        assert!(action.is_some());
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.state.set_open(
                    false,
                    cx.background_executor().now(),
                    Preferences::default(),
                );
                cx.notify();
            });
        });
        cx.run_until_parked();
        if let Some(action) = action {
            cx.simulate_click(action.center(), gpui::Modifiers::none());
        }
        assert_eq!(view.read_with(cx, |view, _| view.activations), 0);
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("disclosure-footer")
                .is_some_and(|bounds| bounds.top() > px(0.) && bounds.top() < px(120.))
        );
        cx.executor().advance_clock(Duration::from_millis(250));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("disclosure-footer")
                .map(|bounds| bounds.top()),
            Some(px(0.))
        );
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "The fixture is a mounted optional layer for reconcile."
    )]
    fn layer(key: u8, value: &'static str) -> Option<Layer<u8, &'static str>> {
        Some(Layer {
            key,
            snapshot: value,
        })
    }

    #[test]
    fn task_refresh_keeps_exit_frozen_and_original_start_time() {
        let now = Instant::now();
        let mut transition = Transition::new(Scope::Page);
        transition.reconcile(
            layer(1, "Home"),
            Direction::Forward,
            MotionStyle::Navigation,
            now,
            Preferences::default(),
        );
        assert!(!transition.frame(now, Preferences::default()).running());
        transition.reconcile(
            layer(2, "Games"),
            Direction::Forward,
            MotionStyle::Navigation,
            now,
            Preferences::default(),
        );
        let token = transition.frame(now, Preferences::default()).token;
        let halfway = now + Duration::from_millis(230);
        transition.reconcile(
            layer(2, "Games with real task result"),
            Direction::Forward,
            MotionStyle::Navigation,
            halfway,
            Preferences::default(),
        );
        assert_eq!(
            transition.frame(halfway, Preferences::default()).token,
            token
        );
        assert_eq!(
            transition.outgoing().map(|layer| layer.snapshot),
            Some("Home")
        );
        assert_eq!(
            transition.current().map(|layer| layer.snapshot),
            Some("Games with real task result")
        );
        assert!(
            !transition
                .frame(now + Duration::from_millis(460), Preferences::default())
                .running()
        );
        assert!(transition.outgoing().is_none());
    }

    #[test]
    fn forward_and_back_move_full_logical_viewport_without_fading() {
        for (direction, sign) in [(Direction::Forward, 1.), (Direction::Back, -1.)] {
            let now = Instant::now();
            let mut transition = Transition::new(Scope::Detail);
            transition.reconcile(
                layer(1, "A"),
                direction,
                MotionStyle::Navigation,
                now,
                Preferences::default(),
            );
            transition.reconcile(
                layer(2, "B"),
                direction,
                MotionStyle::Navigation,
                now,
                Preferences::default(),
            );
            let frame = transition.frame(now, Preferences::default());
            assert_eq!(frame.current.map(|pose| pose.x_fraction), Some(sign));
            assert_eq!(frame.outgoing.map(|pose| pose.x_fraction), Some(0.));
            assert_eq!(frame.current.map(|pose| pose.opacity), Some(1.));
            let frame = transition.frame(now + Duration::from_millis(230), Preferences::default());
            assert!(
                frame
                    .current
                    .is_some_and(|pose| pose.x_fraction * sign > 0.)
            );
            assert!(
                frame
                    .outgoing
                    .is_some_and(|pose| pose.x_fraction * sign < 0.)
            );
            let incoming = frame.current.unwrap_or_default();
            let outgoing = frame.outgoing.unwrap_or_default();
            assert!((incoming.x_fraction - outgoing.x_fraction - sign).abs() < 0.0001);
        }
    }

    #[test]
    fn interrupted_navigation_drops_old_exit_and_rejects_late_completion() {
        let now = Instant::now();
        let mut transition = Transition::new(Scope::Wizard);
        transition.reconcile(
            layer(1, "Setup"),
            Direction::Forward,
            MotionStyle::Navigation,
            now,
            Preferences::default(),
        );
        transition.reconcile(
            layer(2, "Profile"),
            Direction::Forward,
            MotionStyle::Navigation,
            now,
            Preferences::default(),
        );
        let first = transition.frame(now, Preferences::default()).token;
        transition.reconcile(
            layer(3, "Review"),
            Direction::Forward,
            MotionStyle::Navigation,
            now + Duration::from_millis(70),
            Preferences::default(),
        );
        assert_eq!(
            transition.outgoing().map(|layer| layer.snapshot),
            Some("Profile")
        );
        assert!(first.is_some_and(|token| !transition.finish(token)));
        assert!(
            transition
                .frame(now + Duration::from_millis(100), Preferences::default())
                .running()
        );
    }

    #[test]
    fn immutable_sample_omits_expired_exit_and_frame_releases_owned_snapshot() {
        let now = Instant::now();
        let mut transition = Transition::new(Scope::Page);
        transition.reconcile(
            layer(1, "A"),
            Direction::Forward,
            MotionStyle::Navigation,
            now,
            Preferences::default(),
        );
        transition.reconcile(
            layer(2, "B"),
            Direction::Forward,
            MotionStyle::Navigation,
            now,
            Preferences::default(),
        );
        let finished = now + Duration::from_millis(460);
        let sample = transition.sample(finished, Preferences::default());
        assert!(!sample.running());
        assert!(sample.outgoing.is_none());
        assert_eq!(sample.current, Some(Pose::default()));
        assert!(
            transition.outgoing().is_some(),
            "read-only sampling cannot mutate ownership"
        );
        transition.frame(finished, Preferences::default());
        assert!(transition.outgoing().is_none());
    }

    #[test]
    fn closing_overlay_retains_shield_until_exit_completes() {
        let now = Instant::now();
        let mut transition = Transition::new(Scope::Overlay);
        transition.reconcile(
            layer(1, "Dialog"),
            Direction::Forward,
            MotionStyle::Dialog,
            now,
            Preferences::default(),
        );
        transition.frame(now + Duration::from_millis(100), Preferences::default());
        transition.reconcile(
            None,
            Direction::Back,
            MotionStyle::Dialog,
            now + Duration::from_millis(200),
            Preferences::default(),
        );
        let frame = transition.frame(now + Duration::from_millis(250), Preferences::default());
        assert!(frame.captures_input);
        assert!(frame.current.is_none());
        assert!(frame.outgoing.is_some());
        assert!(frame.backdrop_opacity > 0. && frame.backdrop_opacity < 1.);
        let frame = transition.frame(now + Duration::from_millis(300), Preferences::default());
        assert!(!frame.captures_input);
        assert!(transition.outgoing().is_none());
    }

    #[test]
    fn popup_scale_keeps_the_reference_duration_and_status_updates_do_not_restart_it() {
        let now = Instant::now();
        for style in [
            MotionStyle::Dialog,
            MotionStyle::Menu {
                edge: Edge::Bottom,
                align_trigger: false,
            },
            MotionStyle::Tooltip { edge: Edge::Top },
        ] {
            let mut scene = Transition::new(Scope::Overlay);
            scene.reconcile(
                layer(1, "opening"),
                Direction::Forward,
                style,
                now,
                Preferences::default(),
            );
            assert_eq!(
                scene
                    .sample(now, Preferences::default())
                    .current
                    .map(|pose| pose.scale),
                Some(0.95)
            );
            let halfway = now + style.duration() / 2;
            let before_refresh = scene.sample(halfway, Preferences::default());
            scene.reconcile(
                layer(1, "new real task status"),
                Direction::Forward,
                style,
                halfway,
                Preferences::default(),
            );
            assert_eq!(
                scene.sample(halfway, Preferences::default()),
                before_refresh
            );
            assert!(
                before_refresh
                    .current
                    .is_some_and(|pose| pose.scale > 0.95 && pose.scale < 1.)
            );
            assert_eq!(
                scene
                    .frame(now + style.duration(), Preferences::default())
                    .current,
                Some(Pose::default())
            );
            let closing = now + style.duration() * 2;
            scene.reconcile(
                None,
                Direction::Back,
                style,
                closing,
                Preferences::default(),
            );
            assert_eq!(
                scene
                    .sample(closing, Preferences::default())
                    .outgoing
                    .map(|pose| pose.scale),
                Some(1.)
            );
            let exiting = scene.sample(closing + style.duration() / 2, Preferences::default());
            assert!(
                exiting.captures_input
                    && exiting
                        .outgoing
                        .is_some_and(|pose| pose.scale > 0.95 && pose.scale < 1.)
            );
            assert!(
                !scene
                    .frame(closing + style.duration(), Preferences::default())
                    .captures_input
            );
        }
    }

    #[test]
    fn hover_decoration_reverses_without_moving_the_button_and_reduced_motion_settles_it() {
        let now = Instant::now();
        let mut hover = super::HoverDecoration::default();
        hover.set_hovered(true, now, Preferences::default());
        assert_eq!(hover.sample(now, Preferences::default()), (0., true));
        let halfway = now + Duration::from_millis(250);
        let fraction = hover.sample(halfway, Preferences::default()).0;
        assert!(fraction > 0. && fraction < 1.);
        hover.set_hovered(false, halfway, Preferences::default());
        assert_eq!(hover.sample(halfway, Preferences::default()).0, fraction);
        assert!(
            hover
                .sample(halfway + Duration::from_millis(100), Preferences::default())
                .0
                < fraction
        );
        for reduced in [
            Preferences {
                app_reduced: true,
                system_reduced: false,
            },
            Preferences {
                app_reduced: false,
                system_reduced: true,
            },
        ] {
            hover.set_hovered(true, halfway, Preferences::default());
            assert_eq!(hover.sample(halfway, reduced), (1., false));
            assert_eq!(hover.sample(halfway, Preferences::default()), (1., false));
            hover.set_hovered(false, halfway, reduced);
            assert_eq!(hover.sample(halfway, Preferences::default()), (0., false));
        }
    }

    fn advance_product_frame(cx: &mut gpui::VisualTestContext, millis: u64) {
        cx.executor().advance_clock(Duration::from_millis(millis));
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    fn assert_painted_width(cx: &mut gpui::VisualTestContext, logical_width: gpui::Pixels) {
        cx.update(|window, _| {
            let expected = f32::from(logical_width) * window.scale_factor();
            assert!(
                window.painted_primitives().iter().any(|primitive| {
                    primitive.kind == "quad"
                        && (primitive.bounds.size.width / gpui::ScaledPixels::from(1.) - expected)
                            .abs()
                            < 0.1
                }),
                "no renderer quad has the expected scaled width {expected}"
            );
        });
    }

    #[gpui::test]
    fn real_dialog_scales_paint_without_reflow_or_backdrop_click_through(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(crate::tests::test_view);
        cx.simulate_resize(size(px(960.), px(600.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(crate::UiAction::GamesHelp, window, cx);
            });
        });
        advance_product_frame(cx, 20);
        let layout = cx.debug_bounds("drawer-sheet");
        let close = cx.debug_bounds("close-drawer");
        assert!(layout.is_some() && close.is_some());
        let scale = view.read_with(cx, |view, cx| {
            view.motion.overlays[0]
                .sample(cx.background_executor().now(), view.motion_preferences())
                .current
                .map_or(1., |pose| pose.scale)
        });
        assert!(scale > 0.95 && scale < 1.);
        if let Some(layout) = layout {
            assert_painted_width(cx, layout.size.width * scale);
        }
        if let Some(close) = close {
            cx.simulate_click(close.center(), gpui::Modifiers::none());
        }
        assert!(
            view.read_with(cx, |view, _| view.shell.overlays.is_open()),
            "an inert animated popup must shield its own interior from the dismiss backdrop"
        );
        advance_product_frame(cx, 80);
        assert_eq!(
            cx.debug_bounds("drawer-sheet"),
            layout,
            "scaling must never reflow the dialog"
        );
        if let Some(close) = close {
            cx.simulate_click(close.center(), gpui::Modifiers::none());
        }
        assert!(!view.read_with(cx, |view, _| view.shell.overlays.is_open()));
        let menu = cx.debug_bounds("app-menu");
        if let Some(menu) = menu {
            cx.simulate_click(menu.center(), gpui::Modifiers::none());
        }
        assert!(
            !view.read_with(cx, |view, _| view.shell.overlays.is_open()),
            "the retained exit shield must block the underlying menu"
        );
        advance_product_frame(cx, 100);
        if let Some(menu) = menu {
            cx.simulate_click(menu.center(), gpui::Modifiers::none());
        }
        assert!(view.read_with(cx, |view, _| view.shell.overlays.is_open()));
        advance_product_frame(cx, 20);
        let menu_width = cx
            .debug_bounds("drawer-sheet")
            .map(|bounds| bounds.size.width);
        let scale = view.read_with(cx, |view, cx| {
            view.motion.overlays[0]
                .sample(cx.background_executor().now(), view.motion_preferences())
                .current
                .map_or(1., |pose| pose.scale)
        });
        assert!(scale > 0.95 && scale < 1.);
        if let Some(width) = menu_width {
            assert_painted_width(cx, width * scale);
        }
    }

    #[gpui::test]
    fn real_avatar_hover_scales_only_the_decoration_and_keeps_the_button_interactive(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            let account = crate::test_support::test_account();
            assert!(account.is_ok(), "{account:?}");
            if let Ok(account) = account {
                view.selected_account = Some(crate::account_id(&account));
                view.accounts = vec![account];
            }
            view.tab = crate::Tab::Home;
            view
        });
        cx.simulate_resize(size(px(432.), px(768.)));
        advance_product_frame(cx, 500);
        let button = cx.debug_bounds("account-avatar");
        assert!(button.is_some());
        if let Some(button) = button {
            cx.simulate_mouse_move(button.center(), None, gpui::Modifiers::none());
        }
        advance_product_frame(cx, 100);
        let fraction = view.read_with(cx, |view, cx| {
            view.shell
                .home
                .borrow()
                .avatar_hover
                .sample(cx.background_executor().now(), view.motion_preferences())
                .0
        });
        assert!(fraction > 0. && fraction < 1.);
        assert_eq!(cx.debug_bounds("account-avatar"), button);
        if let Some(button) = button {
            assert_painted_width(
                cx,
                (button.size.width + px(32.)) * 0.25_f32.mul_add(fraction, 0.75),
            );
            cx.simulate_click(button.center(), gpui::Modifiers::none());
        }
        assert!(
            view.read_with(cx, |view, _| view.shell.overlays.top().is_some_and(
                |layer| matches!(
                    layer.kind,
                    crate::shell::Surface::Drawer(crate::Drawer::Accounts)
                )
            ))
        );
    }

    #[test]
    fn either_reduced_motion_preference_settles_running_layers_immediately() {
        for preferences in [
            Preferences {
                app_reduced: true,
                system_reduced: false,
            },
            Preferences {
                app_reduced: false,
                system_reduced: true,
            },
        ] {
            let now = Instant::now();
            let mut transition = Transition::new(Scope::Page);
            transition.reconcile(
                layer(1, "A"),
                Direction::Forward,
                MotionStyle::Navigation,
                now,
                Preferences::default(),
            );
            transition.reconcile(
                layer(2, "B"),
                Direction::Forward,
                MotionStyle::Navigation,
                now,
                Preferences::default(),
            );
            let frame = transition.frame(now + Duration::from_millis(20), preferences);
            assert!(!frame.running());
            assert!(frame.outgoing.is_none());
            assert_eq!(frame.current, Some(Pose::default()));
        }
    }

    #[test]
    fn four_scopes_do_not_share_completion_or_transition_lifetime() {
        let now = Instant::now();
        let mut transitions =
            [Scope::Page, Scope::Detail, Scope::Wizard, Scope::Overlay].map(Transition::new);
        for transition in &mut transitions {
            transition.reconcile(
                layer(1, "A"),
                Direction::Forward,
                MotionStyle::Navigation,
                now,
                Preferences::default(),
            );
            transition.reconcile(
                layer(2, "B"),
                Direction::Forward,
                MotionStyle::Navigation,
                now,
                Preferences::default(),
            );
        }
        let token = transitions[0].frame(now, Preferences::default()).token;
        assert!(token.is_some_and(|token| !transitions[1].finish(token)));
        assert!(token.is_some_and(|token| transitions[0].finish(token)));
        for transition in &mut transitions[1..] {
            assert!(transition.frame(now, Preferences::default()).running());
        }
    }

    #[test]
    fn drawer_uses_its_own_duration_and_align_trigger_select_is_static() {
        let now = Instant::now();
        let mut transition = Transition::new(Scope::Overlay);
        transition.reconcile(
            layer(1, "Drawer"),
            Direction::Forward,
            MotionStyle::Drawer { edge: Edge::Right },
            now,
            Preferences::default(),
        );
        let frame = transition.frame(now, Preferences::default());
        assert_eq!(frame.current.map(|pose| pose.x_fraction), Some(1.));
        assert!(
            transition
                .frame(now + Duration::from_millis(449), Preferences::default())
                .running()
        );
        assert!(
            !transition
                .frame(now + Duration::from_millis(450), Preferences::default())
                .running()
        );
        transition.reconcile(
            layer(2, "Aligned select"),
            Direction::Forward,
            MotionStyle::Menu {
                edge: Edge::Bottom,
                align_trigger: true,
            },
            now,
            Preferences::default(),
        );
        assert!(!transition.frame(now, Preferences::default()).running());
    }

    #[test]
    fn easing_has_exact_endpoints_and_solves_css_time_axis() {
        assert_eq!(navigation_easing().sample(0.), 0.);
        assert_eq!(navigation_easing().sample(1.), 1.);
        let halfway = navigation_easing().sample(0.5);
        assert!((halfway - 0.9718).abs() < 0.001);
        let mut previous = 0.;
        for step in 0_u8..=100 {
            let progress = navigation_easing().sample(f32::from(step) / 100.);
            assert!(progress >= previous && (0. ..=1.).contains(&progress));
            previous = progress;
        }
    }

    struct ScopeFixture {
        width: f32,
        height: f32,
    }

    impl Render for ScopeFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            let pose = Pose {
                x_fraction: 0.25,
                y_fraction: 0.5,
                x_pixels: 2.,
                y_pixels: -3.,
                opacity: 1.,
                scale: 1.,
            };
            div()
                .w(px(self.width))
                .h(px(self.height))
                .relative()
                .child(super::layer(
                    "motion-geometry",
                    pose,
                    true,
                    div()
                        .id("motion-content")
                        .debug_selector(|| "motion-content".into())
                        .size_full()
                        .into_any_element(),
                ))
        }
    }

    #[gpui::test]
    fn layer_translation_tracks_its_own_scope_during_parent_resize(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| ScopeFixture {
            width: 320.,
            height: 200.,
        });
        cx.simulate_resize(size(px(960.), px(600.)));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("motion-content");
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            assert_eq!(bounds.origin, point(px(82.), px(97.)));
            assert_eq!(bounds.size, size(px(320.), px(200.)));
        }
        view.update(cx, |view, cx| {
            view.width = 160.;
            view.height = 100.;
            cx.notify();
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("motion-content");
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            assert_eq!(bounds.origin, point(px(42.), px(47.)));
            assert_eq!(bounds.size, size(px(160.), px(100.)));
        }
    }
}
