//! Shared transient feedback. Important form errors also remain on their form.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Feedback is private to the UI implementation."
)]

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use gpui::{
    Anchor, App, Axis, Context, ElementId, FocusHandle, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Render,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _, px, rgb,
};
use gpui_base::{
    Root, RootPlugin, TestSupportExt as _, Toast as BaseToast, ToastManager, ToastMotion,
    ToastOptions, ToastStack, ToastStackState, ToastTransitionStatus,
    motion::{Easing, Presence, Transition, transition},
};

use super::{Palette, design::Icon};

static NEXT_TOAST_ID: AtomicU64 = AtomicU64::new(1);
const MAX_TOASTS: usize = 5;
const TOAST_WIDTH: f32 = 356.0;
const TOAST_BOTTOM: f32 = 24.0;
const NORMAL_EXIT_DURATION: Duration = Duration::from_millis(400);
const COLLAPSED_EXIT_DURATION: Duration = Duration::from_millis(500);
const SWIPE_EXIT_DURATION: Duration = Duration::from_millis(200);
// A collapsed card fades faster than it leaves: the slide is what reads as motion.
const COLLAPSED_FADE_DURATION: Duration = Duration::from_millis(200);

#[path = "toast_motion.rs"]
mod toast_motion;
use toast_motion::{SwipeExit, ToastPosition};
#[cfg(debug_assertions)]
#[path = "toast_trace.rs"]
pub(super) mod toast_trace;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum ToastKind {
    Success,
    Error,
    Info,
}

#[derive(Clone, Debug)]
pub(super) struct Toast {
    pub id: u64,
    pub message: String,
    pub kind: ToastKind,
    pub expires: Instant,
}

impl Toast {
    pub(super) fn new(message: String, kind: ToastKind) -> Self {
        Self {
            id: NEXT_TOAST_ID.fetch_add(1, Ordering::Relaxed),
            message,
            kind,
            expires: Instant::now()
                + Duration::from_secs(if matches!(kind, ToastKind::Error) {
                    8
                } else {
                    4
                }),
        }
    }
}

type ToastValue = Rc<RefCell<Toast>>;

fn trim_to_limit(manager: &ToastManager<u64, ToastValue>, ending: &mut HashSet<u64>) {
    let active = manager
        .iter()
        .filter(|(id, _, status)| *status != ToastTransitionStatus::Ending && !ending.contains(id))
        .map(|(id, _, _)| *id)
        .collect::<Vec<_>>();
    let overflow = active.len().saturating_sub(MAX_TOASTS);
    ending.extend(active.into_iter().take(overflow));
}

fn swipe_dismisses(dx: Pixels, dy: Pixels) -> bool {
    dx.abs() >= px(72.) || dy >= px(48.)
}

fn drag_visual(drag: Option<DragState>) -> gpui::Point<Pixels> {
    drag.map_or(gpui::Point::default(), |drag| {
        let dx = drag.current.x - drag.start.x;
        let dy = drag.current.y - drag.start.y;
        drag.origin
            + match drag.axis {
                Some(Axis::Horizontal) => gpui::point(dx, px(0.)),
                Some(Axis::Vertical) => gpui::point(
                    px(0.),
                    if dy >= px(0.) {
                        dy
                    } else {
                        dy / (1.5 + dy.abs() / px(20.))
                    },
                ),
                None => gpui::Point::default(),
            }
    })
}

fn lifetime(kind: ToastKind) -> Duration {
    Duration::from_secs(if kind == ToastKind::Error { 8 } else { 4 })
}

fn coalesce_existing(
    manager: &ToastManager<u64, ToastValue>,
    ending: &HashSet<u64>,
    toast: &Toast,
    now: Instant,
) -> Option<u64> {
    let id = manager.iter().find_map(|(id, value, status)| {
        (status != ToastTransitionStatus::Ending
            && !ending.contains(id)
            && value.borrow().kind == toast.kind
            && value.borrow().message == toast.message)
            .then_some(*id)
    })?;
    manager.get(&id)?.borrow_mut().expires = now + lifetime(toast.kind);
    Some(id)
}

fn pause_expiries(manager: &ToastManager<u64, ToastValue>, elapsed: Duration) {
    for (_, value, status) in manager.iter() {
        if status != ToastTransitionStatus::Ending {
            value.borrow_mut().expires += elapsed;
        }
    }
}

fn expired_present(manager: &ToastManager<u64, ToastValue>, now: Instant) -> Vec<u64> {
    manager
        .iter()
        .filter_map(|(id, value, status)| {
            (status == ToastTransitionStatus::Present && now >= value.borrow().expires)
                .then_some(*id)
        })
        .collect()
}

fn animation_duration(reduced_motion: bool) -> Duration {
    if reduced_motion {
        Duration::ZERO
    } else {
        Duration::from_millis(400)
    }
}

fn css_transition(reduced_motion: bool) -> Transition {
    Transition::new(animation_duration(reduced_motion)).easing(Easing::Ease)
}

fn manager_motion(reduced_motion: bool) -> ToastMotion {
    let mut motion = ToastMotion::sonner();
    if reduced_motion {
        motion.duration = Duration::ZERO;
    }
    // Presence owns the visible exit. Only dismiss from the manager after its
    // Absent sample, so a delayed render cannot lose an unfinished transition.
    motion.exit_duration = Duration::ZERO;
    motion
}

#[derive(Clone, Copy)]
struct DragState {
    start: gpui::Point<Pixels>,
    current: gpui::Point<Pixels>,
    origin: gpui::Point<Pixels>,
    axis: Option<Axis>,
}

impl DragState {
    fn new(start: gpui::Point<Pixels>, origin: gpui::Point<Pixels>) -> Self {
        Self {
            start,
            current: start,
            origin,
            axis: None,
        }
    }

    fn update(&mut self, position: gpui::Point<Pixels>) {
        self.current = position;
        let delta = position - self.start;
        if self.axis.is_none() && delta.x.abs().max(delta.y.abs()) > px(1.) {
            self.axis = Some(if delta.x.abs() > delta.y.abs() {
                Axis::Horizontal
            } else {
                Axis::Vertical
            });
        }
    }

    fn dismisses(self) -> bool {
        let offset = drag_visual(Some(self));
        match self.axis {
            Some(Axis::Horizontal) => swipe_dismisses(offset.x, px(0.)),
            Some(Axis::Vertical) => swipe_dismisses(px(0.), offset.y),
            None => false,
        }
    }
}

struct ToastAppearance {
    palette: Palette,
    width: Pixels,
    rank: usize,
}

/// Animated entry and exit channels of one toast, sampled once per render.
struct ToastMotionState {
    presence: f32,
    opacity: f32,
    swipe_progress: f32,
    drag_offset: gpui::Point<Pixels>,
    swipe: Option<SwipeExit>,
    collapsed_exit: bool,
    mounted: bool,
}

/// The card's animated width and the content/visibility reveals that keep a
/// collapsed stack readable.
struct ToastLayout {
    width: Pixels,
    contents: f32,
    visibility: f32,
}

pub(super) struct ToastLayer {
    manager: ToastManager<u64, ToastValue>,
    stack_state: ToastStackState,
    stack_focus: FocusHandle,
    close_focus: HashMap<u64, FocusHandle>,
    gestures: Rc<RefCell<HashMap<u64, DragState>>>,
    swipe_exits: HashMap<u64, SwipeExit>,
    ending: HashSet<u64>,
    settled_exits: HashSet<u64>,
    timer_running: bool,
    last_tick: Option<Instant>,
    reduced_motion: bool,
    bottom: Pixels,
}

impl ToastLayer {
    pub(super) fn new(_: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let motion = manager_motion(cx.reduce_motion());
        Self {
            manager: ToastManager::new(motion),
            stack_state: ToastStackState::default(),
            stack_focus: cx.focus_handle(),
            close_focus: HashMap::new(),
            gestures: Rc::default(),
            swipe_exits: HashMap::new(),
            ending: HashSet::new(),
            settled_exits: HashSet::new(),
            timer_running: false,
            last_tick: None,
            reduced_motion: cx.reduce_motion(),
            bottom: px(TOAST_BOTTOM),
        }
    }

    /// Adds messages through the ToastLayer installed on this window's Base Root.
    pub(super) fn enqueue(toasts: Vec<Toast>, window: &mut Window, cx: &mut App) {
        if toasts.is_empty() {
            return;
        }
        let Some(root) = window.root::<Root>().flatten() else {
            return;
        };
        let Some(layer) = root.read(cx).plugin::<Self>() else {
            return;
        };
        layer.update(cx, |layer, cx| {
            layer.enqueue_inner(toasts, cx);
        });
    }

    /// Reserve the product's action rail so transient feedback never covers a
    /// retry, Save or Play button, including in the short portrait window.
    pub(super) fn reserve_actions(bottom: Pixels, window: &Window, cx: &mut App) {
        let Some(layer) = window
            .root::<Root>()
            .flatten()
            .and_then(|root| root.read(cx).plugin::<Self>())
        else {
            return;
        };
        layer.update(cx, |layer, cx| {
            if layer.bottom != bottom {
                layer.bottom = bottom;
                if !layer.manager.is_empty() {
                    cx.notify();
                }
            }
        });
    }

    fn enqueue_inner(&mut self, toasts: Vec<Toast>, cx: &mut Context<'_, Self>) {
        let now = cx.background_executor().now();
        for mut toast in toasts {
            // The shell creates Toasts with the wall clock; adapt them to the
            // executor clock so virtual-time tests and expiry share one clock.
            toast.expires = now + lifetime(toast.kind);
            if let Some(id) = coalesce_existing(&self.manager, &self.ending, &toast, now) {
                if let Some(value) = self.manager.get(&id) {
                    value.borrow_mut().id = id;
                }
                continue;
            }

            let id = toast.id;
            self.manager.push(
                id,
                Rc::new(RefCell::new(toast)),
                ToastOptions::default(),
                now,
            );
            self.close_focus.insert(id, cx.focus_handle());
        }

        // Bound active items while allowing their exit transitions to finish.
        trim_to_limit(&self.manager, &mut self.ending);

        if !self.timer_running {
            self.timer_running = true;
            self.last_tick = Some(now);
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(50))
                        .await;
                    let keep_running = this.update(cx, |layer, cx| {
                        let now = cx.background_executor().now();
                        layer.advance(now, cx)
                    });
                    if !matches!(keep_running, Ok(true)) {
                        break;
                    }
                }
            })
            .detach();
        }
        cx.notify();
    }

    fn advance(&mut self, now: Instant, cx: &mut Context<'_, Self>) -> bool {
        let paused = self.stack_state.is_expanded() || !self.gestures.borrow().is_empty();
        let delta = self
            .last_tick
            .replace(now)
            .map(|last| now.saturating_duration_since(last))
            .unwrap_or_default();
        if paused {
            // Shift deadlines by actual executor elapsed time, so delayed timer
            // callbacks cannot consume the paused duration.
            pause_expiries(&self.manager, delta);
        }

        let advance = self.manager.advance(now, paused);
        let mut changed = advance.changed;
        for id in expired_present(&self.manager, now) {
            changed |= self.ending.insert(id);
        }
        for (id, _) in advance.removed {
            self.close_focus.remove(&id);
            self.gestures.borrow_mut().remove(&id);
            self.swipe_exits.remove(&id);
            self.ending.remove(&id);
            self.settled_exits.remove(&id);
        }
        if changed {
            cx.notify();
        }
        if self.manager.is_empty() {
            self.timer_running = false;
            self.last_tick = None;
            return false;
        }
        true
    }

    fn dismiss(&mut self, id: u64, cx: &mut Context<'_, Self>) {
        if self.manager.get(&id).is_some() && self.ending.insert(id) {
            cx.notify();
        }
    }

    fn release(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<'_, Self>) {
        let gestures = std::mem::take(&mut *self.gestures.borrow_mut());
        if gestures.is_empty() {
            return;
        }
        for (id, drag) in gestures {
            let _ = self.finish_drag(id, drag, event.position, cx);
        }
        cx.notify();
    }

    fn finish_drag(
        &mut self,
        id: u64,
        mut drag: DragState,
        position: gpui::Point<Pixels>,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        drag.update(position);
        if let Some(axis) = drag.axis.filter(|_| drag.dismisses()) {
            self.swipe_exits
                .insert(id, SwipeExit::new(drag_visual(Some(drag)), axis));
            self.dismiss(id, cx);
            true
        } else {
            false
        }
    }

    #[cfg(debug_assertions)]
    fn trace_swipe(
        &mut self,
        id: u64,
        bounds: gpui::Bounds<Pixels>,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let start = gpui::point(
            bounds.origin.x + bounds.size.width / 2.,
            bounds.origin.y + bounds.size.height / 2.,
        );
        let drag = DragState::new(start, gpui::Point::default());
        let dismissed = self.finish_drag(id, drag, start + gpui::point(px(96.), px(0.)), cx);
        cx.notify();
        dismissed
    }

    fn sync_motion(&mut self, cx: &mut Context<'_, Self>) {
        let reduced = cx.reduce_motion();
        if self.reduced_motion == reduced {
            return;
        }
        self.reduced_motion = reduced;
        let now = cx.background_executor().now();
        // ToastManager's published API fixes its durations at construction.
        // Preserve values/deadlines while settling existing entries; enabling
        // reduced motion removes ending entries immediately, without replay.
        let entries = self
            .manager
            .iter()
            .filter(|(id, _, phase)| {
                *phase != ToastTransitionStatus::Ending && !self.ending.contains(id)
            })
            .map(|(id, value, _)| (*id, value.clone()))
            .collect::<Vec<_>>();
        let motion = manager_motion(reduced);
        self.manager = ToastManager::new(motion);
        for (id, value) in entries {
            self.manager
                .push(id, value, ToastOptions::default(), now - motion.duration);
        }
        self.manager.advance(now, true);
        self.ending.clear();
        self.close_focus
            .retain(|id, _| self.manager.get(id).is_some());
        self.gestures
            .borrow_mut()
            .retain(|id, _| self.manager.get(id).is_some());
        self.swipe_exits
            .retain(|id, _| self.manager.get(id).is_some());
        self.settled_exits
            .retain(|id| self.manager.get(id).is_some());
    }

    fn toast_element(
        &mut self,
        id: u64,
        toast: Toast,
        status: ToastTransitionStatus,
        appearance: ToastAppearance,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<gpui::AnyElement> {
        let ToastAppearance {
            palette,
            width,
            rank,
        } = appearance;
        let motion = self.toast_motion(id, rank, status, window, cx);
        if !motion.mounted {
            #[cfg(debug_assertions)]
            if toast_trace::enabled() {
                toast_trace::unmounted(
                    id,
                    status == ToastTransitionStatus::Ending,
                    cx.reduce_motion(),
                    window,
                );
            }
            self.retire(id, cx);
            return None;
        }
        let layout = self.toast_layout(id, width, rank, window, cx);
        let card = self.toast_card(id, toast, palette, &motion, &layout, cx);
        #[cfg(debug_assertions)]
        let card = traced_card(card, id, status, &motion, cx.reduce_motion());
        let card = card
            .id(ElementId::NamedInteger("block-toast-card".into(), id))
            .test_support();
        let toast = BaseToast::new(ElementId::NamedInteger("block-toast".into(), id))
            .transition_status(status)
            .child(div().debug_selector(|| "toast".into()).child(card));
        Some(
            ToastPosition::new(
                id,
                toast.into_any_element(),
                motion.presence,
                motion.drag_offset,
                motion.swipe.map(|swipe| (swipe, motion.swipe_progress)),
                motion.collapsed_exit,
            )
            .into_any_element(),
        )
    }

    /// Samples the drag, swipe, presence and opacity channels of one toast.
    /// Each transition keeps its element id so a render eases from the previous
    /// frame's value instead of restarting.
    fn toast_motion(
        &mut self,
        id: u64,
        rank: usize,
        status: ToastTransitionStatus,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> ToastMotionState {
        let reduced = cx.reduce_motion();
        let expanded = self.stack_state.is_expanded();
        let policy = css_transition(reduced);
        let drag = self.gestures.borrow().get(&id).copied();
        let drag_offset = transition(
            ElementId::NamedInteger("block-toast-drag".into(), id),
            drag_visual(drag),
            if drag.is_some() {
                Transition::new(Duration::ZERO)
            } else {
                policy.clone()
            },
            window,
            cx,
        );
        let swipe = self.swipe_exits.get(&id).copied();
        let swipe_progress = transition(
            ElementId::NamedInteger("block-toast-swipe".into(), id),
            if swipe.is_none() { 1. } else { 0. },
            Transition::new(if reduced {
                Duration::ZERO
            } else {
                SWIPE_EXIT_DURATION
            })
            .easing(Easing::EaseOut),
            window,
            cx,
        );
        let collapsed_exit = status == ToastTransitionStatus::Ending && !expanded && rank > 0;
        let presence_duration = if reduced {
            Duration::ZERO
        } else if swipe.is_some() {
            SWIPE_EXIT_DURATION
        } else if collapsed_exit {
            COLLAPSED_EXIT_DURATION
        } else {
            NORMAL_EXIT_DURATION
        };
        let presence = Presence::new(
            ElementId::NamedInteger("block-toast-presence".into(), id),
            status != ToastTransitionStatus::Ending,
        )
        .transition(Transition::new(presence_duration).easing(Easing::Ease))
        .sample(window, cx);
        let opacity = Presence::new(
            ElementId::NamedInteger("block-toast-opacity".into(), id),
            status != ToastTransitionStatus::Ending || swipe.is_some(),
        )
        .transition(if collapsed_exit {
            Transition::new(if reduced {
                Duration::ZERO
            } else {
                COLLAPSED_FADE_DURATION
            })
            .easing(Easing::Ease)
        } else {
            policy
        })
        .sample(window, cx);
        ToastMotionState {
            presence: presence.progress,
            opacity: opacity.progress,
            swipe_progress,
            drag_offset,
            swipe,
            collapsed_exit,
            mounted: presence.should_render(),
        }
    }

    /// Samples the card width and the reveals that keep a collapsed stack
    /// readable. A toast that is already leaving skips them: nothing paints.
    fn toast_layout(
        &self,
        id: u64,
        width: Pixels,
        rank: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> ToastLayout {
        let expanded = self.stack_state.is_expanded();
        let policy = css_transition(cx.reduce_motion());
        ToastLayout {
            width: transition(
                ElementId::NamedInteger("block-toast-width".into(), id),
                width
                    * if expanded {
                        1.
                    } else {
                        1. - 0.05 * rank.min(2) as f32
                    },
                policy.clone(),
                window,
                cx,
            ),
            contents: transition(
                ElementId::NamedInteger("block-toast-content".into(), id),
                if expanded || rank == 0 { 1. } else { 0. },
                policy.clone(),
                window,
                cx,
            ),
            visibility: transition(
                ElementId::NamedInteger("block-toast-visibility".into(), id),
                if expanded || rank < 3 { 1. } else { 0. },
                policy,
                window,
                cx,
            ),
        }
    }

    /// Drops a toast whose exit transition has finished. Presence reports Absent
    /// one frame after the visible exit, so only this point settles the entry.
    fn retire(&mut self, id: u64, cx: &mut Context<'_, Self>) {
        self.close_focus.remove(&id);
        self.gestures.borrow_mut().remove(&id);
        self.swipe_exits.remove(&id);
        self.settled_exits.insert(id);
        self.manager.dismiss(&id, cx.background_executor().now());
    }

    /// One toast's card: the animated container plus the dismiss-by-swipe press
    /// target. Geometry is sampled by the caller, so this only paints.
    fn toast_card(
        &self,
        id: u64,
        toast: Toast,
        palette: Palette,
        motion: &ToastMotionState,
        layout: &ToastLayout,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Div {
        let gestures = self.gestures.clone();
        let entity_id = cx.entity().entity_id();
        let drag_offset = motion.drag_offset;
        let press = move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
            gestures
                .borrow_mut()
                .insert(id, DragState::new(event.position, drag_offset));
            cx.notify(entity_id);
        };
        div()
            .occlude()
            .on_mouse_down(MouseButton::Left, press)
            .opacity(motion.opacity * motion.swipe_progress * layout.visibility)
            .when(layout.visibility == 0., |card| card.invisible())
            .w(layout.width)
            .mx_auto()
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.card))
            .text_color(rgb(palette.foreground))
            .shadow_lg()
            .child(self.toast_row(id, toast, palette, layout.contents, cx))
    }

    /// The visible row of one toast: state icon, message and dismiss control.
    fn toast_row(
        &self,
        id: u64,
        toast: Toast,
        palette: Palette,
        contents: f32,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let (icon, tone, tint) = toast_tone(toast.kind, palette);
        let close = cx.listener(move |this, _: &gpui::ClickEvent, _, cx| this.dismiss(id, cx));
        let close_focus = self
            .close_focus
            .get(&id)
            .cloned()
            .unwrap_or_else(|| cx.focus_handle());
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .w_full()
            .min_w_0()
            .px(px(16.))
            .py(px(14.))
            .opacity(contents)
            .child(icon.render(18., tone))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(SharedString::from(toast.message)),
            )
            .child(
                gpui_base::Button::new(ElementId::NamedInteger("block-toast-close".into(), id))
                    .accessibility_label("Dismiss notification")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .rounded(px(8.))
                    .bg(rgb(tint))
                    .cursor_pointer()
                    .on_click(close)
                    .track_focus(&close_focus)
                    .tab_index(0)
                    .tab_stop(true)
                    .child(Icon::Close.render(14., palette.muted)),
            )
    }
}

/// The state icon and its tone/tint pair for one toast kind.
fn toast_tone(kind: ToastKind, palette: Palette) -> (Icon, u32, u32) {
    match kind {
        ToastKind::Success => (Icon::Check, palette.success, palette.success_background),
        ToastKind::Error => (
            Icon::Error,
            palette.destructive,
            palette.destructive_background,
        ),
        ToastKind::Info => (Icon::Info, palette.primary, palette.secondary),
    }
}

/// In a traced build, records the card's real painted geometry.
#[cfg(debug_assertions)]
fn traced_card(
    card: gpui::Div,
    id: u64,
    status: ToastTransitionStatus,
    motion: &ToastMotionState,
    reduced: bool,
) -> gpui::Div {
    let ending = status == ToastTransitionStatus::Ending;
    card.when(toast_trace::enabled(), |card| {
        card.child(toast_trace::probe(
            id,
            (!ending).then_some(motion.presence),
            ending.then_some(if motion.swipe.is_some() {
                motion.swipe_progress
            } else {
                motion.presence
            }),
            ending,
            reduced,
        ))
    })
}

impl RootPlugin for ToastLayer {}

impl Render for ToastLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.sync_motion(cx);
        #[cfg(debug_assertions)]
        if toast_trace::enabled() {
            let managed_ids = self
                .manager
                .iter()
                .map(|(id, _, _)| *id)
                .collect::<Vec<_>>();
            toast_trace::observe_managed_ids(&managed_ids, cx.reduce_motion(), window);
        }
        if self.manager.is_empty() {
            return div();
        }
        let palette = cx
            .try_global::<Palette>()
            .copied()
            .unwrap_or(Palette::LIGHT);
        let viewport = window.viewport_size();
        let inset = super::window_geometry::content_insets(window);
        let content_width = (viewport.width - inset.left - inset.right - px(32.)).max(px(0.));
        let width = content_width.min(px(TOAST_WIDTH));
        let mut stack = ToastStack::new("block-toast-stack", self.stack_state.clone())
            .placement(Anchor::BottomCenter)
            .focus_handle(self.stack_focus.clone())
            // Keep Base's measurement, focus and hover behavior. Its public
            // stack API only exposes springs, so animate the final measured
            // item positions with CSS ease in ToastPosition instead.
            .motion(ToastMotion {
                duration: Duration::ZERO,
                exit_duration: Duration::ZERO,
                collapsed_scale_step: 0.,
                collapsed_visible: usize::MAX,
                ..ToastMotion::sonner()
            })
            .w(width)
            .max_w(px(TOAST_WIDTH));

        let entries = self
            .manager
            .iter()
            .filter(|(id, _, _)| !self.settled_exits.contains(id))
            .map(|(id, value, status)| {
                let status = if self.ending.contains(id) {
                    ToastTransitionStatus::Ending
                } else {
                    status
                };
                (*id, value.borrow().clone(), status)
            })
            .collect::<Vec<_>>();
        let count = entries.len();
        for (index, (id, toast, status)) in entries.into_iter().enumerate() {
            if let Some(element) = self.toast_element(
                id,
                toast,
                status,
                ToastAppearance {
                    palette,
                    width,
                    rank: count - index - 1,
                },
                window,
                cx,
            ) {
                stack = stack.item(
                    ElementId::NamedInteger("block-toast-stack-item".into(), id),
                    element,
                );
            }
        }
        div()
            .absolute()
            .inset_0()
            .child(
                gpui::canvas(|_, _, _| {}, {
                    let layer = cx.weak_entity();
                    move |_, _, window, _| {
                        let moving = layer.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase != gpui::DispatchPhase::Capture {
                                return;
                            }
                            let _ = moving.update(cx, |layer, cx| {
                                let mut gestures = layer.gestures.borrow_mut();
                                if !gestures.is_empty() {
                                    if event.pressed_button == Some(MouseButton::Left) {
                                        for drag in gestures.values_mut() {
                                            drag.update(event.position);
                                        }
                                    } else {
                                        gestures.clear();
                                    }
                                    cx.notify();
                                }
                            });
                        });
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                            if phase == gpui::DispatchPhase::Capture
                                && event.button == MouseButton::Left
                            {
                                let _ =
                                    layer.update(cx, |layer, cx| layer.release(event, window, cx));
                            }
                        });
                    }
                })
                .absolute()
                .inset_0(),
            )
            .flex()
            .justify_center()
            .items_end()
            // Root plugins already live inside WindowBorder's content box.
            // Only the width calculation needs the outer viewport's insets.
            .pb(self.bottom)
            .child(stack)
    }
}

#[cfg(test)]
#[path = "toast_tests.rs"]
mod tests;
