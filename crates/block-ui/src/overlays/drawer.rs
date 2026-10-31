//! Bottom drawer presentation over the library's modal sheet and focus trap.
//! Product state owns open/close; this layer retains the last body through exit.
#![expect(clippy::redundant_pub_crate, reason = "Private product presentation.")]

use std::time::Duration;

use gpui::{
    AnyElement, App, Context, CursorStyle, Edges, ElementId, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, MouseButton, MouseMoveEvent, MouseUpEvent,
    ParentElement as _, Pixels, Render, StatefulInteractiveElement as _, Styled as _, Subscription,
    WeakEntity, Window, div, px, rgb,
};
use gpui_base::{
    Root, RootPlugin, Sheet,
    motion::{Easing, Presence, PresencePhase, Spring, Transition, spring},
};

use super::{
    Block,
    appearance::Palette,
    components::{self, ButtonSize, ButtonVariant, Control, ControlState},
    design::{CONTROL_HEIGHT, Icon},
    drawer_gestures::DrawerDrag,
    shell::Surface,
};
use gpui_component::WindowExt as _;

const DURATION: Duration = Duration::from_millis(450);
/// A full-height panel stops this far below the product title bar, so its top
/// corners have page behind them instead of the window edge.
const TITLE_BAR_GAP: f32 = 12.;
/// The panel's top corners are the part that still reads as a drawer when it
/// grows tall.
const PANEL_RADIUS: f32 = 20.;
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

    /// The drawer owns focus until its deferred return has settled.
    pub(super) fn is_open(window: &Window, cx: &App) -> bool {
        Root::read(window, cx)
            .plugin::<Self>()
            .is_some_and(|layer| {
                let layer = layer.read(cx);
                layer.active.is_some() || layer.pending_return.is_some()
            })
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

/// One frame of the active drawer: the sampled motion channels and the geometry
/// the panel and its sheet share.
struct DrawerFrame {
    revision: usize,
    kind: Surface,
    owner: WeakEntity<Block>,
    palette: Palette,
    language: crate::Language,
    title: String,
    description: String,
    focus: FocusHandle,
    insets: Edges<Pixels>,
    width: Pixels,
    height: Pixels,
    panel_height: f32,
    translation: f32,
    scrim: f32,
    dragging: bool,
    closing: bool,
    #[cfg(debug_assertions)]
    trace: Option<drawer_trace::FrameSample>,
}

impl DrawerFrame {
    /// Resolve this frame's geometry, motion and product chrome. The two
    /// presence channels stay with the layer that owns the exit.
    fn sample(
        active: &mut ActiveDrawer,
        owner: &Entity<Block>,
        progress: f32,
        backdrop: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let revision = active.revision;
        let kind = active.kind.clone();
        let insets = super::window_geometry::content_insets(window);
        let viewport = window.viewport_size();
        let width = (viewport.width - insets.left - insets.right).max(px(0.));
        let height = (viewport.height - insets.top - insets.bottom).max(px(0.));
        // The sheet spans the whole window so its scrim keeps the chrome modal;
        // only the panel stops below the title bar when it runs out of room.
        let available_height =
            (f32::from(height) - super::design::CHROME_HEIGHT - TITLE_BAR_GAP).max(0.);
        let (palette, language, (title, description, desired_height), reduced) = {
            let view = owner.read(cx);
            (
                view.palette,
                view.language(),
                view.surface_sheet_chrome(&kind),
                view.motion_preferences().reduced(),
            )
        };
        let panel_height = spring(
            ElementId::NamedInteger("block-drawer-height".into(), revision as u64),
            desired_height.min(available_height),
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
        let translation = (panel_height + 2.) * (1. - progress) + offset * progress;
        let scrim = backdrop * (1. - offset / panel_height.max(1.)).clamp(0., 1.);
        #[cfg(debug_assertions)]
        let trace = drawer_trace::sample(drawer_trace::FrameSample {
            revision,
            render_ms: 0.,
            progress,
            translation,
            scrim,
            height: panel_height,
            offset,
            closing: active.closing,
            reduced: cx.reduce_motion(),
        });
        Self {
            revision,
            kind,
            owner: active.owner.clone(),
            palette,
            language,
            title,
            description,
            focus: active.focus.clone(),
            insets,
            width,
            height,
            panel_height,
            translation,
            scrim,
            dragging: active.drag.is_some(),
            closing: active.closing,
            #[cfg(debug_assertions)]
            trace,
        }
    }
}

/// Rendering helpers that turn one resolved frame into elements.
impl DrawerLayer {
    /// Finish an exit: drop the presentation and return focus once no dialog
    /// owns it.
    fn retire(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let active = self.active.take();
        let owner = active.as_ref().map(|active| active.owner.clone());
        self.pending_return = active.and_then(|active| active.return_focus);
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
                    // A queued crash may take focus only after this return.
                    if let Some(owner) = owner {
                        let _ = owner.update(cx, |view, cx| {
                            if !view.pending_crashes.is_empty() {
                                cx.notify();
                            }
                        });
                    }
                }
            });
        });
    }

    /// The panel: drag handle, product header and the live body.
    fn panel(
        &self,
        frame: &DrawerFrame,
        owner: &Entity<Block>,
        body: AnyElement,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let palette = frame.palette;
        let revision = frame.revision;
        let handle = div()
            .id("drawer-handle")
            .debug_selector(|| "drawer-handle".into())
            .h(px(28.))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .cursor(if frame.dragging {
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
        let dismiss = frame.owner.clone();
        let close = components::button(
            "drawer-close",
            tr!(frame.language, "common.close"),
            palette,
            ButtonVariant::Ghost,
            ControlState::default(),
            move |_, window, cx| {
                let _ = dismiss.update(cx, |view, cx| {
                    view.dismiss_surface_overlay(revision, window, cx)
                });
            },
        );
        let import_back = self.import_header_back(frame, owner, cx);
        let closing = frame.closing;
        let panel = div()
            .id("block-drawer")
            .debug_selector(|| "block-drawer".into())
            .absolute()
            .left_0()
            .bottom(px(-frame.translation))
            .w_full()
            .h(px(frame.panel_height))
            .overflow_hidden()
            .occlude()
            .flex()
            .flex_col()
            .bg(rgb(palette.popover))
            .border_t_1()
            .border_color(rgb(palette.border))
            .rounded_t(px(PANEL_RADIUS))
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
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .min_h(px(CONTROL_HEIGHT))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .children(import_back)
                            .child(
                                div()
                                    .debug_selector(|| "drawer-title".into())
                                    .flex_1()
                                    .min_w_0()
                                    .text_lg()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(frame.title.clone()),
                            )
                            .child(close),
                    )
                    .child(
                        div()
                            .debug_selector(|| "drawer-description".into())
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child(frame.description.clone()),
                    ),
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
        let panel = panel.children(
            frame
                .trace
                .map(|sample| drawer_trace::probe("panel", sample)),
        );
        panel.into_any_element()
    }

    /// Import games keeps one layer across its pages; only the list offers the
    /// way back to its sources.
    fn import_header_back(
        &self,
        frame: &DrawerFrame,
        owner: &Entity<Block>,
        cx: &App,
    ) -> Option<Control> {
        (matches!(
            frame.kind,
            Surface::Drawer(super::shell::Drawer::ImportGames)
        ) && owner.read(cx).shell.import_games.page == super::import_games::Page::List)
            .then(|| {
                let target = owner.downgrade();
                components::button_sized(
                    "import-header-back",
                    "",
                    frame.palette,
                    ButtonVariant::Ghost,
                    ButtonSize::Icon,
                    ControlState::default(),
                    move |_, window, cx| {
                        let _ = target.update(cx, |view, cx| view.import_header_back(window, cx));
                    },
                )
                .aria_label(tr!(frame.language, "chrome.back_to_sources"))
                .child(Icon::ChevronLeft.render(16., frame.palette.foreground))
            })
    }

    /// The library sheet: full-window scrim, focus trap and dismissal around the
    /// panel, plus the capture-phase drag channel.
    fn sheet(
        &self,
        frame: &DrawerFrame,
        panel: AnyElement,
        popups: Vec<AnyElement>,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Div {
        let revision = frame.revision;
        let dismiss = frame.owner.clone();
        let cancel = frame.owner.clone();
        let overlay = div()
            .debug_selector(|| "block-drawer-backdrop".into())
            .absolute()
            .inset_0()
            .occlude()
            .bg(gpui::hsla(0., 0., 0., 0.1 * frame.scrim));
        #[cfg(debug_assertions)]
        let overlay = overlay.children(
            frame
                .trace
                .map(|sample| drawer_trace::probe("backdrop", sample)),
        );
        div()
            .absolute()
            .inset_0()
            .font_family(super::design::FONT_FAMILY)
            .text_color(rgb(frame.palette.foreground))
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
                    .focus_handle(frame.focus.clone())
                    .overlay_closable(!frame.closing)
                    .request_close(move |window, cx| {
                        let _ = dismiss.update(cx, |view, cx| {
                            if view.shell.surfaces.select_above(revision) {
                                view.handle(super::UiAction::Drawer(None), window, cx);
                            } else {
                                view.dismiss_surface_overlay(revision, window, cx);
                            }
                        });
                    })
                    .top(frame.insets.top)
                    .left(frame.insets.left)
                    .w(frame.width)
                    .h(frame.height)
                    .overflow_hidden()
                    .overlay(overlay)
                    // Keep anchored popups above the scrim and outside the
                    // clipped panel, but inside the sheet's focus trap.
                    .surface(
                        div()
                            .absolute()
                            .inset_0()
                            .on_action(move |_: &gpui_base::actions::Cancel, window, cx| {
                                // A nested selector owns Cancel even after Tab
                                // moves focus to another control in this sheet.
                                let handled = cancel
                                    .update(cx, |view, cx| {
                                        if !view.shell.surfaces.select_above(revision) {
                                            return false;
                                        }
                                        view.handle(super::UiAction::Drawer(None), window, cx);
                                        true
                                    })
                                    .unwrap_or(false);
                                if handled {
                                    cx.stop_propagation();
                                } else {
                                    cx.propagate();
                                }
                            })
                            .child(panel)
                            .children(popups),
                    ),
            )
    }
}

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
            self.retire(window, cx);
            return div().into_any_element();
        }
        let Some(owner) = active.owner.upgrade() else {
            self.active = None;
            return div().into_any_element();
        };
        let frame = DrawerFrame::sample(
            active,
            &owner,
            presence.progress,
            backdrop.progress,
            window,
            cx,
        );
        let (body, popups) = owner.update(cx, |view, cx| {
            let body = view.render_surface_sheet_body(&frame.kind, window, cx);
            let popups = if view.shell.surfaces.contains(frame.revision) {
                view.render_popups(true, cx)
            } else {
                Vec::new()
            };
            (body, popups)
        });
        let panel = self.panel(&frame, &owner, body, cx);
        self.sheet(&frame, panel, popups, cx).into_any_element()
    }
}

#[cfg(test)]
#[path = "drawer_tests.rs"]
mod tests;
