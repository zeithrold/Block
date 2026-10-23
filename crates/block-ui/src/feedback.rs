//! Shared transient feedback. Important form errors also remain on their form.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Feedback is private to the UI implementation."
)]

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use gpui::{
    Anchor, App, Context, ElementId, FocusHandle, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Render,
    SharedString, Styled as _, Window, div, px, rgb,
};
use gpui_base::{
    Root, RootPlugin, Toast as BaseToast, ToastManager, ToastMotion, ToastOptions, ToastStack,
    ToastStackState, ToastTransitionStatus,
    motion::{Presence, Transition},
};

use super::{Palette, design::Icon};

static NEXT_TOAST_ID: AtomicU64 = AtomicU64::new(1);
const MAX_TOASTS: usize = 5;
const TOAST_WIDTH: f32 = 356.0;
const TOAST_BOTTOM: f32 = 24.0;

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

fn trim_to_limit(manager: &mut ToastManager<u64, ToastValue>, now: Instant) {
    let active = manager
        .iter()
        .filter(|(_, _, status)| *status != ToastTransitionStatus::Ending)
        .map(|(id, _, _)| *id)
        .collect::<Vec<_>>();
    let overflow = active.len().saturating_sub(MAX_TOASTS);
    for id in active.into_iter().take(overflow) {
        manager.dismiss(&id, now);
    }
}

fn swipe_dismisses(dx: Pixels, dy: Pixels) -> bool {
    dx.abs() >= px(72.) || dy >= px(48.)
}

fn drag_visual(drag: Option<DragState>) -> (Pixels, Pixels, f32) {
    drag.map_or((px(0.), px(0.), 1.0), |drag| {
        let dx = drag.current.x - drag.start.x;
        let dy = drag.current.y - drag.start.y;
        let visual_x = if dx.abs() >= dy.abs() { dx } else { px(0.) };
        let visual_y = if dy >= px(0.) && dy.abs() > dx.abs() {
            dy
        } else {
            px(0.)
        };
        let progress = (visual_x.abs() / px(160.))
            .max(visual_y / px(120.))
            .clamp(0., 1.);
        (visual_x, visual_y, 1.0 - progress * 0.65)
    })
}

fn lifetime(kind: ToastKind) -> Duration {
    Duration::from_secs(if kind == ToastKind::Error { 8 } else { 4 })
}

fn coalesce_existing(
    manager: &ToastManager<u64, ToastValue>,
    toast: &Toast,
    now: Instant,
) -> Option<u64> {
    let id = manager.iter().find_map(|(id, value, status)| {
        (status != ToastTransitionStatus::Ending
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
        Duration::from_millis(180)
    }
}

#[derive(Clone, Copy)]
struct DragState {
    start: gpui::Point<Pixels>,
    current: gpui::Point<Pixels>,
}

pub(super) struct ToastLayer {
    manager: ToastManager<u64, ToastValue>,
    stack_state: ToastStackState,
    stack_focus: FocusHandle,
    close_focus: HashMap<u64, FocusHandle>,
    gestures: Rc<RefCell<HashMap<u64, DragState>>>,
    timer_running: bool,
    last_tick: Option<Instant>,
    reduced_motion: bool,
    bottom: Pixels,
}

impl ToastLayer {
    pub(super) fn new(_: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let motion = if cx.reduce_motion() {
            ToastMotion {
                duration: Duration::ZERO,
                exit_duration: Duration::ZERO,
                ..ToastMotion::sonner()
            }
        } else {
            ToastMotion::sonner()
        };
        Self {
            manager: ToastManager::new(motion),
            stack_state: ToastStackState::default(),
            stack_focus: cx.focus_handle(),
            close_focus: HashMap::new(),
            gestures: Rc::default(),
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
            if let Some(id) = coalesce_existing(&self.manager, &toast, now) {
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
        trim_to_limit(&mut self.manager, now);

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
        let expired = expired_present(&self.manager, now);
        let mut changed = advance.changed;
        for (id, _) in advance.removed {
            self.close_focus.remove(&id);
            self.gestures.borrow_mut().remove(&id);
        }
        for id in expired {
            changed |= self.manager.dismiss(&id, now);
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
        let now = cx.background_executor().now();
        if self.manager.dismiss(&id, now) {
            cx.notify();
        }
    }

    fn release(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<'_, Self>) {
        let gestures = std::mem::take(&mut *self.gestures.borrow_mut());
        if gestures.is_empty() {
            return;
        }
        for (id, drag) in gestures {
            if swipe_dismisses(
                event.position.x - drag.start.x,
                event.position.y - drag.start.y,
            ) {
                self.dismiss(id, cx);
            }
        }
        cx.notify();
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
            .filter(|(_, _, phase)| *phase != ToastTransitionStatus::Ending)
            .map(|(id, value, _)| (*id, value.clone()))
            .collect::<Vec<_>>();
        let motion = if reduced {
            ToastMotion {
                duration: Duration::ZERO,
                exit_duration: Duration::ZERO,
                ..ToastMotion::sonner()
            }
        } else {
            ToastMotion::sonner()
        };
        self.manager = ToastManager::new(motion);
        for (id, value) in entries {
            self.manager
                .push(id, value, ToastOptions::default(), now - motion.duration);
        }
        self.manager.advance(now, true);
        self.close_focus
            .retain(|id, _| self.manager.get(id).is_some());
        self.gestures
            .borrow_mut()
            .retain(|id, _| self.manager.get(id).is_some());
    }

    fn toast_element(
        &mut self,
        id: u64,
        toast: Toast,
        status: ToastTransitionStatus,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let (icon, tone, tint) = match toast.kind {
            ToastKind::Success => (Icon::Check, palette.success, palette.success_background),
            ToastKind::Error => (
                Icon::Error,
                palette.destructive,
                palette.destructive_background,
            ),
            ToastKind::Info => (Icon::Info, palette.primary, palette.secondary),
        };
        let close = cx.listener(move |this, _: &gpui::ClickEvent, _, cx| this.dismiss(id, cx));
        let close_focus = self
            .close_focus
            .get(&id)
            .cloned()
            .unwrap_or_else(|| cx.focus_handle());
        let gestures = self.gestures.clone();
        let press = move |event: &MouseDownEvent, _: &mut Window, _: &mut App| {
            gestures.borrow_mut().insert(
                id,
                DragState {
                    start: event.position,
                    current: event.position,
                },
            );
        };
        let drag = self.gestures.borrow().get(&id).copied();
        let (drag_x, drag_y, drag_opacity) = drag_visual(drag);

        let presence = Presence::new(
            ElementId::NamedInteger("block-toast-presence".into(), id),
            status != ToastTransitionStatus::Ending,
        )
        .transition(Transition::new(animation_duration(cx.reduce_motion())))
        .sample(window, cx);
        let card = div()
            .debug_selector(|| "toast".into())
            .occlude()
            .on_mouse_down(MouseButton::Left, press)
            .opacity(presence.progress * drag_opacity)
            .relative()
            .left(drag_x)
            .top(drag_y + px(12. * (1. - presence.progress)))
            .flex()
            .items_center()
            .gap(px(12.))
            .w_full()
            .min_w_0()
            .px(px(16.))
            .py(px(14.))
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.card))
            .text_color(rgb(palette.foreground))
            .shadow_lg()
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
            );
        BaseToast::new(ElementId::NamedInteger("block-toast".into(), id))
            .transition_status(status)
            .child(card)
    }
}

impl RootPlugin for ToastLayer {}

impl Render for ToastLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.sync_motion(cx);
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
            .motion(if cx.reduce_motion() {
                ToastMotion {
                    duration: Duration::ZERO,
                    exit_duration: Duration::ZERO,
                    ..ToastMotion::sonner()
                }
            } else {
                ToastMotion::sonner()
            })
            .w(width)
            .max_w(px(TOAST_WIDTH));

        let entries = self
            .manager
            .visible(MAX_TOASTS)
            .map(|(id, value, status)| (*id, value.borrow().clone(), status))
            .collect::<Vec<_>>();
        for (id, toast, status) in entries {
            stack = stack.item(
                ElementId::NamedInteger("block-toast-stack-item".into(), id),
                self.toast_element(id, toast, status, palette, window, cx),
            );
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
                                            drag.current = event.position;
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
