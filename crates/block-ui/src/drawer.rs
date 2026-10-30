//! Bottom drawer presentation over the library's modal sheet and focus trap.
//! Product state owns open/close; this layer retains the last body through exit.
#![expect(clippy::redundant_pub_crate, reason = "Private product presentation.")]

use std::time::Duration;

use gpui::{
    App, Context, CursorStyle, ElementId, FocusHandle, InteractiveElement as _, IntoElement,
    MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement as _, Render, Styled as _,
    Subscription, WeakEntity, Window, div, px, rgb,
};
use gpui_base::{
    Root, RootPlugin, Sheet,
    motion::{Easing, Presence, PresencePhase, Spring, Transition, spring},
};

use super::{Block, drawer_gestures::DrawerDrag, shell::Surface};
use gpui_component::WindowExt as _;

const DURATION: Duration = Duration::from_millis(450);
const PANEL_EASING: Easing = Easing::CubicBezier {
    x1: 0.22,
    y1: 1.,
    x2: 0.36,
    y2: 1.,
};
const BACKDROP_EASING: Easing = Easing::CubicBezier {
    x1: 0.32,
    y1: 0.72,
    x2: 0.,
    y2: 1.,
};

#[cfg(debug_assertions)]
#[path = "drawer_trace.rs"]
mod drawer_trace;
#[cfg(debug_assertions)]
pub(super) fn motion_trace_enabled() -> bool {
    drawer_trace::enabled()
}
#[cfg(debug_assertions)]
pub(super) fn start_motion_trace(
    owner: WeakEntity<Block>,
    window: &mut Window,
    cx: &mut Context<'_, Block>,
) {
    drawer_trace::start(owner, window, cx);
}

struct ActiveDrawer {
    owner: WeakEntity<Block>,
    kind: Surface,
    revision: usize,
    focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    closing: bool,
    drag: Option<DrawerDrag>,
    offset: f32,
    height: f32,
    _observer: Subscription,
}

pub(super) struct DrawerLayer {
    active: Option<ActiveDrawer>,
    pending_return: Option<FocusHandle>,
}

impl DrawerLayer {
    pub(super) fn new(_: &mut Window, _: &mut Context<'_, Self>) -> Self {
        Self {
            active: None,
            pending_return: None,
        }
    }

    pub(super) fn open(
        owner: WeakEntity<Block>,
        kind: Surface,
        revision: usize,
        focus: FocusHandle,
        return_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(layer) = Root::read(window, cx).plugin::<Self>() else {
            return;
        };
        layer.update(cx, |layer, cx| {
            let Some(view) = owner.upgrade() else {
                return;
            };
            // Reopening during exit must return to the original trigger rather
            // than an input belonging to the body that is about to disappear.
            let return_focus = layer
                .active
                .take()
                .filter(|active| active.closing)
                .and_then(|active| active.return_focus)
                .or_else(|| layer.pending_return.take())
                .or(return_focus);
            let observer = cx.observe(&view, |_, _, cx| cx.notify());
            layer.active = Some(ActiveDrawer {
                owner,
                kind,
                revision,
                focus,
                return_focus,
                closing: false,
                drag: None,
                offset: 0.,
                height: 0.,
                _observer: observer,
            });
            cx.notify();
        });
    }

    pub(super) fn close(
        revision: usize,
        return_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(layer) = window
            .root::<Root>()
            .flatten()
            .and_then(|root| root.read(cx).plugin::<Self>())
        else {
            return false;
        };
        layer.update(cx, |layer, cx| {
            if let Some(active) = layer
                .active
                .as_mut()
                .filter(|active| active.revision == revision)
            {
                active.closing = true;
                if let Some(drag) = active.drag.take() {
                    active.offset = drag.offset();
                }
                if return_focus.is_some() {
                    active.return_focus = return_focus;
                }
                cx.notify();
                return true;
            }
            false
        })
    }

    pub(super) fn closing_return_focus(window: &Window, cx: &App) -> Option<FocusHandle> {
        let layer = window.root::<Root>().flatten()?.read(cx).plugin::<Self>()?;
        let layer = layer.read(cx);
        layer
            .active
            .as_ref()
            .filter(|active| active.closing)
            .and_then(|active| active.return_focus.clone())
            .or_else(|| layer.pending_return.clone())
    }

    #[cfg(test)]
    pub(super) fn is_open(window: &Window, cx: &App) -> bool {
        Root::read(window, cx)
            .plugin::<Self>()
            .is_some_and(|layer| layer.read(cx).active.is_some())
    }

    fn release(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let Some(mut drag) = active.drag.take() else {
            return;
        };
        let now = cx.background_executor().now();
        drag.update(f32::from(event.position.y), now);
        if drag.release(now, active.height) {
            active.offset = drag.offset();
            let owner = active.owner.clone();
            let revision = active.revision;
            // Closing updates both product and presentation; settle this borrow
            // before the owner asks this layer to start its exit.
            window.defer(cx, move |window, cx| {
                let _ = owner.update(cx, |view, cx| {
                    view.dismiss_surface_overlay(revision, window, cx)
                });
            });
        } else {
            active.offset = 0.;
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl RootPlugin for DrawerLayer {}

impl Render for DrawerLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let Some(active) = self.active.as_mut() else {
            return div().into_any_element();
        };
        let revision = active.revision;
        let presence = Presence::new(
            ElementId::NamedInteger("block-drawer-presence".into(), revision as u64),
            !active.closing,
        )
        .transition(Transition::new(DURATION).easing(PANEL_EASING))
        .sample(window, cx);
        let backdrop = Presence::new(
            ElementId::NamedInteger("block-drawer-backdrop".into(), revision as u64),
            !active.closing,
        )
        .transition(Transition::new(DURATION).easing(BACKDROP_EASING))
        .sample(window, cx);
        if presence.phase == PresencePhase::Absent {
            self.pending_return = self.active.take().and_then(|active| active.return_focus);
            let layer = cx.weak_entity();
            window.defer(cx, move |window, cx| {
                let _ = layer.update(cx, |layer, cx| {
                    if layer.active.is_none() {
                        let focus = layer.pending_return.take();
                        if !window.has_active_dialog(cx)
                            && let Some(focus) = focus
                        {
                            focus.focus(window, cx);
                        }
                    }
                });
            });
            return div().into_any_element();
        }

        let Some(owner) = active.owner.upgrade() else {
            self.active = None;
            return div().into_any_element();
        };
        let palette = owner.read(cx).palette;
        let (title, description, desired_height) =
            owner.read(cx).surface_sheet_chrome(&active.kind);
        let insets = super::window_geometry::content_insets(window);
        let viewport = window.viewport_size();
        let width = (viewport.width - insets.left - insets.right).max(px(0.));
        let height = (viewport.height - insets.top - insets.bottom).max(px(0.));
        let reduced = owner.read(cx).motion_preferences().reduced();
        let panel_height = spring(
            ElementId::NamedInteger("block-drawer-height".into(), revision as u64),
            desired_height,
            Spring::new(if reduced {
                Duration::ZERO
            } else {
                Duration::from_millis(260)
            }),
            window,
            cx,
        )
        .max(0.);
        active.height = panel_height;
        let offset = spring(
            ElementId::NamedInteger("block-drawer-offset".into(), revision as u64),
            active.offset,
            Spring::new(if active.drag.is_some() {
                Duration::ZERO
            } else {
                Duration::from_millis(280)
            }),
            window,
            cx,
        );
        // Match the popup's closed transform, including its 2px border bleed.
        // Backdrop opacity follows its own CSS timing channel.
        let translation =
            (panel_height + 2.) * (1. - presence.progress) + offset * presence.progress;
        let scrim_opacity = backdrop.progress * (1. - offset / panel_height.max(1.)).clamp(0., 1.);
        let focus = active.focus.clone();
        let dismiss = active.owner.clone();
        let dismiss_button = active.owner.clone();
        let dragging = active.drag.is_some();
        let closing = active.closing;
        let kind = active.kind.clone();
        let body = owner.update(cx, |view, cx| {
            view.render_surface_sheet_body(&kind, window, cx)
        });
        let handle = div()
            .id("drawer-handle")
            .debug_selector(|| "drawer-handle".into())
            .h(px(28.))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .cursor(if dragging {
                CursorStyle::ClosedHand
            } else {
                CursorStyle::OpenHand
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|layer, event: &gpui::MouseDownEvent, _, cx| {
                    if let Some(active) = layer.active.as_mut().filter(|active| !active.closing) {
                        active.drag = Some(DrawerDrag::new(
                            f32::from(event.position.y),
                            cx.background_executor().now(),
                        ));
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .w(px(80.))
                    .h(px(4.))
                    .rounded_full()
                    .bg(rgb(palette.border)),
            );
        let close = gpui_component::button::Button::new("drawer-close")
            .label("Close")
            .on_click(move |_, window, cx| {
                let _ = dismiss_button.update(cx, |view, cx| {
                    view.dismiss_surface_overlay(revision, window, cx)
                });
            });
        let import_back =
            matches!(kind, Surface::Drawer(super::shell::Drawer::ImportGames)).then(|| {
                let visible =
                    owner.read(cx).shell.import_games.page == super::import_games::Page::List;
                let target = active.owner.clone();
                div().w(px(28.)).flex_shrink_0().children(visible.then(|| {
                    gpui_component::button::Button::new("import-header-back")
                        .label("←")
                        .w(px(28.))
                        .h(px(28.))
                        .p_0()
                        .on_click(move |_, window, cx| {
                            let _ =
                                target.update(cx, |view, cx| view.import_header_back(window, cx));
                        })
                }))
            });
        #[cfg(debug_assertions)]
        let trace_sample = drawer_trace::sample(drawer_trace::FrameSample {
            revision,
            render_ms: 0.,
            progress: presence.progress,
            translation,
            scrim: scrim_opacity,
            height: panel_height,
            offset,
            closing,
            reduced: cx.reduce_motion(),
        });
        let surface = div()
            .id("block-drawer")
            .debug_selector(|| "block-drawer".into())
            .absolute()
            .left_0()
            .bottom(px(-translation))
            .w_full()
            .h(px(panel_height))
            .overflow_hidden()
            .occlude()
            .flex()
            .flex_col()
            .bg(rgb(palette.popover))
            .border_t_1()
            .border_color(rgb(palette.border))
            .rounded_t(px(20.))
            .shadow_lg()
            .capture_any_mouse_down(move |_, _, cx| {
                if closing {
                    cx.stop_propagation();
                }
            })
            .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
            .child(handle)
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(20.))
                    .pb(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .children(import_back)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(palette.muted))
                                    .child(description),
                            ),
                    )
                    .child(close),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(px(20.))
                    .pb(px(20.))
                    .flex()
                    .flex_col()
                    .child(body),
            );
        #[cfg(debug_assertions)]
        let surface =
            surface.children(trace_sample.map(|sample| drawer_trace::probe("panel", sample)));
        let overlay = div()
            .debug_selector(|| "block-drawer-backdrop".into())
            .absolute()
            .inset_0()
            .occlude()
            .bg(gpui::hsla(0., 0., 0., 0.1 * scrim_opacity));
        #[cfg(debug_assertions)]
        let overlay =
            overlay.children(trace_sample.map(|sample| drawer_trace::probe("backdrop", sample)));
        div()
            .absolute()
            .inset_0()
            .font_family(super::design::FONT_FAMILY)
            .text_color(rgb(palette.foreground))
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
                                if let Some(active) = layer.active.as_mut() {
                                    if event.pressed_button == Some(MouseButton::Left) {
                                        if let Some(drag) = &mut active.drag {
                                            active.offset = drag.update(
                                                f32::from(event.position.y),
                                                cx.background_executor().now(),
                                            );
                                            cx.stop_propagation();
                                            cx.notify();
                                        }
                                    } else if active.drag.take().is_some() {
                                        active.offset = 0.;
                                        cx.notify();
                                    }
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
            .child(
                Sheet::new(cx)
                    .focus_handle(focus)
                    .overlay_closable(!closing)
                    .request_close(move |window, cx| {
                        let _ = dismiss.update(cx, |view, cx| {
                            view.dismiss_surface_overlay(revision, window, cx)
                        });
                    })
                    .top(insets.top)
                    .left(insets.left)
                    .w(width)
                    .h(height)
                    .overflow_hidden()
                    .overlay(overlay)
                    .surface(surface),
            )
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "drawer_tests.rs"]
mod tests;
