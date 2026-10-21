//! Base owns tooltip delay, grace-period switching and popup positioning. Block
//! adds trigger registration, Escape suppression and retained visual exits.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Tooltip presentation is private to the product view."
)]

use super::{
    Block,
    motion::{Direction, Edge, Layer, MotionStyle, Pose, Preferences, Scope, Transition},
};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Element, ElementId, Entity, FocusHandle,
    GlobalElementId, InspectorElementId, InteractiveElement as _, IntoElement as _, LayoutId,
    MouseButton, ParentElement as _, Pixels, Point, Render, SharedString, Size, Styled as _,
    Window, canvas, deferred, div, point, px,
};
use std::{
    cell::{Cell, RefCell},
    hash::{Hash as _, Hasher as _},
    rc::{Rc, Weak},
};

#[derive(Clone, Debug)]
struct Trigger {
    key: u64,
    text: SharedString,
    bounds: Bounds<Pixels>,
    visible: Bounds<Pixels>,
    focus: Option<FocusHandle>,
    hovered: bool,
    focused: bool,
}

#[derive(Clone, Debug)]
struct Tip {
    key: u64,
    text: SharedString,
    anchor: Bounds<Pixels>,
}

/// Observations reset and collected only within one actual draw.
#[derive(Default)]
struct FrameObservation {
    trigger: bool,
    popup: bool,
}

struct Session {
    provider: Option<Entity<gpui_base::TooltipOverlay>>,
    generation: u64,
    invalidated: bool,
    frame: FrameObservation,
    requested: bool,
    suppressed: Option<u64>,
    trigger: Option<Trigger>,
    scene: Transition<u64, Tip>,
    palette: super::Palette,
    preferences: Preferences,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            provider: None,
            generation: 0,
            invalidated: false,
            frame: FrameObservation::default(),
            requested: false,
            suppressed: None,
            trigger: None,
            scene: Transition::new(Scope::Overlay),
            palette: super::Palette::LIGHT,
            preferences: Preferences::default(),
        }
    }
}

#[derive(Default)]
pub(super) struct TooltipHost(Rc<RefCell<Session>>);

// Read-only page snapshots never own a second provider or its asynchronous
// tasks. The live window alone retains this transient, nonmodal presentation.
impl Clone for TooltipHost {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl std::fmt::Debug for TooltipHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.0.borrow();
        f.debug_struct("TooltipHost")
            .field("generation", &state.generation)
            .field("requested", &state.requested)
            .field("suppressed", &state.suppressed)
            .finish_non_exhaustive()
    }
}
impl TooltipHost {
    pub(super) fn invalidate(&self) {
        let mut state = self.0.borrow_mut();
        state.generation = state.generation.wrapping_add(1);
        state.invalidated = true;
        state.requested = false;
        state.suppressed = None;
        state.trigger = None;
        state.scene = Transition::new(Scope::Overlay);
    }
}

const fn style() -> MotionStyle {
    MotionStyle::Tooltip { edge: Edge::Bottom }
}
fn preferences(view: &Block) -> Preferences {
    Preferences {
        app_reduced: view.preferences.reduce_motion,
        system_reduced: super::platform::system_reduce_motion(),
    }
}

/// Decorate the existing control, preserving its native Base activation,
/// focus, disabled state, semantics and stable identity.
pub(super) fn trigger<E: gpui::StatefulInteractiveElement>(
    control: E,
    text: impl Into<SharedString>,
    cx: &Context<'_, Block>,
) -> E {
    let text = text.into();
    let description = text.to_string();
    let registration = Rc::new(RefCell::new(None::<Trigger>));
    let prepaint = registration.clone();
    let view = cx.weak_entity();
    control
        .aria_description(description)
        .on_prepaint_bounds(move |id, bounds, window, cx| {
            let Some(id) = id else {
                return;
            };
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            id.hash(&mut hash);
            let focus = window.current_element_focus_handle(cx);
            let info = Trigger {
                key: hash.finish(),
                text: text.clone(),
                bounds,
                visible: bounds.intersect(&window.content_mask().bounds),
                focused: focus.as_ref().is_some_and(|focus| focus.is_focused(window)),
                focus,
                hovered: false,
            };
            *prepaint.borrow_mut() = Some(info.clone());
            let _ = view.update(cx, |view, cx| view.observe_tooltip(info, None, window, cx));
        })
        .on_hover(cx.listener(move |view, hovered: &bool, window, cx| {
            if let Some(info) = registration.borrow().clone() {
                view.observe_tooltip(info, Some(*hovered), window, cx);
            }
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|view, _, window, cx| {
                view.dismiss_tooltip(window, cx);
            }),
        )
}

fn provider(session: &Rc<RefCell<Session>>, cx: &mut App) -> Entity<gpui_base::TooltipOverlay> {
    if let Some(provider) = session.borrow().provider.clone() {
        return provider;
    }
    let weak = Rc::downgrade(session);
    let provider = cx.new(|_| {
        gpui_base::TooltipOverlay::new().render_with(move |content, _, _, _| {
            if let Some(session) = weak.upgrade() {
                session.borrow_mut().frame.popup = true;
            }
            content.into_any_element()
        })
    });
    session.borrow_mut().provider = Some(provider.clone());
    provider
}

impl Block {
    fn observe_tooltip(
        &self,
        mut info: Trigger,
        hovered: Option<bool>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.visual_only {
            return;
        }
        let visible = info
            .visible
            .intersect(&Bounds::new(Point::default(), window.viewport_size()));
        info.focused = !visible.is_empty()
            && info
                .focus
                .as_ref()
                .is_some_and(|focus| focus.is_focused(window));
        let session = self.shell.overlays.tooltip.0.clone();
        // Navigation may replace a trigger before the next draw. Cancel the
        // previous Base request before accepting this new generation.
        let reset = {
            let mut state = session.borrow_mut();
            let reset = state.invalidated.then(|| state.provider.clone()).flatten();
            state.invalidated = false;
            reset
        };
        if let Some(provider) = reset {
            provider.update(cx, gpui_base::TooltipOverlay::hide);
        }
        let (show, hide, refresh, generation) = {
            let mut state = session.borrow_mut();
            let same = state
                .trigger
                .as_ref()
                .is_some_and(|current| current.key == info.key);
            info.hovered = hovered.unwrap_or_else(|| {
                same && state
                    .trigger
                    .as_ref()
                    .is_some_and(|current| current.hovered)
                    && visible.contains(&window.mouse_position())
            });
            if !same && !info.hovered && !info.focused {
                return;
            }
            state.frame.trigger = true;
            let engaged = info.hovered || info.focused;
            let previous_engaged = same
                && state
                    .trigger
                    .as_ref()
                    .is_some_and(|old| old.hovered || old.focused);
            let changed = !same
                || state
                    .trigger
                    .as_ref()
                    .is_some_and(|old| old.bounds != info.bounds || old.text != info.text);
            if !same || !engaged {
                state.suppressed = None;
            }
            state.palette = self.palette;
            state.preferences = preferences(self);
            state.trigger = Some(info.clone());
            let show =
                engaged && state.suppressed != Some(info.key) && (changed || !previous_engaged);
            let hide = !engaged && previous_engaged;
            // Base's grace period may keep the previous tooltip visible after
            // hover leaves during a resize. Refresh its real anchor without
            // restarting our same-key visual transition, then retain Base's
            // hide request rather than leaving a detached tooltip behind.
            let refresh = !engaged
                && changed
                && same
                && state.scene.current().is_some()
                && state.suppressed != Some(info.key);
            if show {
                state.requested = true;
            }
            if hide {
                state.requested = false;
            }
            (show, hide, refresh, state.generation)
        };
        if !show && !hide && !refresh {
            return;
        }
        let provider = provider(&session, cx);
        if show || refresh {
            let weak = Rc::downgrade(&session);
            let tip = Tip {
                key: info.key,
                text: info.text,
                anchor: info.bounds,
            };
            let anchor = positioning_anchor(tip.anchor);
            // Retain one content entity for this request so animation/status
            // redraws also retain the native accessibility node identity.
            let content = cx.new(|_| TipContent {
                tip,
                session: weak,
                generation,
            });
            provider.update(cx, |provider, cx| {
                provider.request_show(
                    gpui_base::TooltipRequest::new(anchor, move |_, _| content.clone().into()),
                    window,
                    cx,
                );
            });
        }
        if hide || refresh {
            provider.update(cx, |provider, cx| provider.request_hide(window, cx));
        }
    }

    /// Escape cancels Base's pending request and suppresses reopening until the
    /// trigger disengages. Focus remains on the existing control.
    pub(super) fn dismiss_tooltip(&self, _: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let session = self.shell.overlays.tooltip.0.clone();
        let provider = {
            let mut state = session.borrow_mut();
            if !state.requested && state.scene.current().is_none() {
                return false;
            }
            state.requested = false;
            state.suppressed = state.trigger.as_ref().map(|trigger| trigger.key);
            state.scene.reconcile(
                None,
                Direction::Forward,
                style(),
                cx.background_executor().now(),
                preferences(self),
            );
            state.provider.clone()
        };
        if let Some(provider) = provider {
            provider.update(cx, gpui_base::TooltipOverlay::hide);
        }
        cx.notify();
        true
    }

    pub(super) fn render_tooltip(&self, _: &Window, _: &Context<'_, Self>) -> AnyElement {
        if self.visual_only {
            return div().into_any_element();
        }
        let session = self.shell.overlays.tooltip.0.clone();
        {
            let mut state = session.borrow_mut();
            state.frame.trigger = false;
            state.palette = self.palette;
            state.preferences = preferences(self);
        }
        canvas(
            move |_, window, cx| {
                let (base, reset) = {
                    let mut state = session.borrow_mut();
                    state.frame.popup = false;
                    let reset =
                        state.invalidated || (!state.frame.trigger && state.trigger.is_some());
                    state.invalidated = false;
                    if reset {
                        state.requested = false;
                        state.trigger = None;
                        state.scene = Transition::new(Scope::Overlay);
                    }
                    (state.provider.clone(), reset)
                };
                let mut base = base.map(|provider| {
                    if reset {
                        provider.update(cx, gpui_base::TooltipOverlay::hide);
                    }
                    let mut element = provider.into_any_element();
                    element.prepaint_as_root(
                        Point::default(),
                        window.viewport_size().into(),
                        window,
                        cx,
                    );
                    element
                });
                let outgoing = {
                    let mut state = session.borrow_mut();
                    let now = cx.background_executor().now();
                    let preferences = state.preferences;
                    if !state.frame.popup {
                        state
                            .scene
                            .reconcile(None, Direction::Forward, style(), now, preferences);
                    }
                    let frame = state.scene.frame(now, preferences);
                    if frame.running() {
                        window.request_animation_frame();
                    }
                    frame
                        .outgoing
                        .zip(state.scene.outgoing())
                        .map(|(pose, layer)| (layer.snapshot.clone(), pose, state.palette))
                };
                let mut outgoing = outgoing.map(|(tip, pose, palette)| {
                    let mut element = deferred(
                        gpui_base::TooltipPositioner::new(positioning_anchor(tip.anchor)).child(
                            div()
                                .p(px(4.))
                                .child(TipPaint::new(tip, pose, true, palette, window)),
                        ),
                    )
                    .with_priority(200)
                    .into_any_element();
                    element.prepaint_as_root(
                        Point::default(),
                        window.viewport_size().into(),
                        window,
                        cx,
                    );
                    element
                });
                // The elements live for this draw only; retained data is the
                // owned Tip snapshot, never a cached interactive element.
                (base.take(), outgoing.take())
            },
            |_, (base, outgoing), window, cx| {
                for mut element in base.into_iter().chain(outgoing) {
                    element.paint(window, cx);
                }
            },
        )
        .absolute()
        .size_full()
        .into_any_element()
    }
}

struct TipContent {
    tip: Tip,
    session: Weak<RefCell<Session>>,
    generation: u64,
}
impl Render for TipContent {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> impl gpui::IntoElement {
        let Some(session) = self.session.upgrade() else {
            return div().into_any_element();
        };
        let (pose, palette) = {
            let mut state = session.borrow_mut();
            if state.generation != self.generation || state.invalidated {
                return div().into_any_element();
            }
            let preferences = state.preferences;
            let now = cx.background_executor().now();
            state.scene.reconcile(
                Some(Layer {
                    key: self.tip.key,
                    snapshot: self.tip.clone(),
                }),
                Direction::Forward,
                style(),
                now,
                preferences,
            );
            let frame = state.scene.frame(now, preferences);
            if frame.running() {
                window.request_animation_frame();
            }
            (frame.current.unwrap_or_default(), state.palette)
        };
        div()
            .p(px(4.))
            .child(TipPaint::new(
                self.tip.clone(),
                pose,
                false,
                palette,
                window,
            ))
            .into_any_element()
    }
}

/// Keep Base's measured layout untouched. Only the visual subtree is offset
/// and scaled around the final native positioner's anchor projection.
struct TipPaint {
    content: AnyElement,
    tip: Tip,
    pose: Pose,
    inert: bool,
    edge: Rc<Cell<Edge>>,
}
impl TipPaint {
    fn new(tip: Tip, pose: Pose, inert: bool, palette: super::Palette, window: &Window) -> Self {
        let edge = Rc::new(Cell::new(Edge::Bottom));
        let max_width = (window.viewport_size().width - px(24.))
            .max(px(1.))
            .min(px(320.));
        let content = div()
            .id("product-tooltip")
            .debug_selector(|| "product-tooltip".into())
            .relative()
            .opacity(pose.opacity)
            .child(
                gpui_base::Tooltip::new("tooltip-semantics")
                    .max_w(max_width)
                    .px_3()
                    .py(px(6.))
                    .rounded_md()
                    .bg(gpui::rgb(palette.foreground))
                    .text_color(gpui::rgb(palette.background))
                    .text_xs()
                    .child(tip.text.clone()),
            )
            .child(tooltip_arrow(tip.anchor, edge.clone(), palette))
            .into_any_element();
        Self {
            content,
            tip,
            pose,
            inert,
            edge,
        }
    }
}
impl gpui::IntoElement for TipPaint {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for TipPaint {
    type RequestLayoutState = ();
    type PrepaintState = Point<Pixels>;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (
            window.with_inert_subtree(self.inert, |window| self.content.request_layout(window, cx)),
            (),
        )
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        (): &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Point<Pixels> {
        let edge = resolved_edge(bounds, self.tip.anchor);
        self.edge.set(edge);
        let distance = px(self.pose.x_pixels.abs().max(self.pose.y_pixels.abs()));
        let offset = match edge {
            Edge::Top => point(px(0.), distance),
            Edge::Bottom => point(px(0.), -distance),
            Edge::Left => point(distance, px(0.)),
            Edge::Right => point(-distance, px(0.)),
        };
        let origin =
            tooltip_scale_origin(self.tip.anchor, bounds.origin, bounds.size, edge) + offset;
        window.with_element_offset(offset, |window| {
            window.with_inert_subtree(self.inert, |window| {
                window.with_paint_scale(self.pose.scale, origin, |window| {
                    self.content.prepaint(window, cx);
                });
            });
        });
        origin
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        (): &mut (),
        origin: &mut Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_inert_subtree(self.inert, |window| {
            window.with_paint_scale(self.pose.scale, *origin, |window| {
                self.content.paint(window, cx);
            });
        });
    }
}

// Base's provider fixes a 4px window margin and exposes no gap setting. A
// 4px content inset plus a 4px expanded anchor supplies Block's 8px gap and
// window margin while all measurement/flipping/clamping remains in Base.
fn positioning_anchor(anchor: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds::new(
        anchor.origin - point(px(4.), px(4.)),
        anchor.size + gpui::size(px(8.), px(8.)),
    )
}

fn resolved_edge(bounds: Bounds<Pixels>, anchor: Bounds<Pixels>) -> Edge {
    if bounds.bottom() <= anchor.top() {
        Edge::Top
    } else if bounds.top() >= anchor.bottom() {
        Edge::Bottom
    } else if bounds.right() <= anchor.left() {
        Edge::Left
    } else if bounds.left() >= anchor.right() {
        Edge::Right
    } else if bounds.center().y < anchor.center().y {
        Edge::Top
    } else {
        Edge::Bottom
    }
}

/// The arrow and scale origin share the actual anchor projection after the
/// bubble has been constrained to the window, including near-corner triggers.
fn tooltip_scale_origin(
    anchor: Bounds<Pixels>,
    origin: Point<Pixels>,
    size: Size<Pixels>,
    edge: Edge,
) -> Point<Pixels> {
    let center = arrow_center(Bounds::new(origin, size), anchor, edge);
    match edge {
        Edge::Top => point(center.x, anchor.top()),
        Edge::Bottom => point(center.x, anchor.bottom()),
        Edge::Left => point(anchor.left(), center.y),
        Edge::Right => point(anchor.right(), center.y),
    }
}

fn arrow_center(bounds: Bounds<Pixels>, anchor: Bounds<Pixels>, edge: Edge) -> Point<Pixels> {
    let inset_x = px(10.).min(bounds.size.width / 2.);
    let inset_y = px(10.).min(bounds.size.height / 2.);
    let cross_x = anchor
        .center()
        .x
        .clamp(bounds.left() + inset_x, bounds.right() - inset_x);
    let cross_y = anchor
        .center()
        .y
        .clamp(bounds.top() + inset_y, bounds.bottom() - inset_y);
    match edge {
        Edge::Top => point(cross_x, bounds.bottom() - px(2.)),
        Edge::Bottom => point(cross_x, bounds.top() + px(2.)),
        Edge::Left => point(bounds.right() - px(2.), cross_y),
        Edge::Right => point(bounds.left() + px(2.), cross_y),
    }
}

fn tooltip_arrow(
    anchor: Bounds<Pixels>,
    edge: Rc<Cell<Edge>>,
    palette: super::Palette,
) -> impl gpui::IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let center = arrow_center(bounds, anchor, edge.get());
            // The reference rotates a 10px square with rounded 2px corners by
            // 45 degrees. Draw that vector shape without a bitmap or reflow.
            let radius = px(5. * std::f32::consts::SQRT_2);
            let corner = px(std::f32::consts::SQRT_2);
            let mut path = gpui::PathBuilder::fill();
            path.move_to(center + point(-corner, -radius + corner));
            path.curve_to(
                center + point(corner, -radius + corner),
                center + point(px(0.), -radius),
            );
            path.line_to(center + point(radius - corner, -corner));
            path.curve_to(
                center + point(radius - corner, corner),
                center + point(radius, px(0.)),
            );
            path.line_to(center + point(corner, radius - corner));
            path.curve_to(
                center + point(-corner, radius - corner),
                center + point(px(0.), radius),
            );
            path.line_to(center + point(-radius + corner, corner));
            path.curve_to(
                center + point(-radius + corner, -corner),
                center + point(-radius, px(0.)),
            );
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, gpui::rgb(palette.foreground));
            }
        },
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
#[path = "tooltip_tests.rs"]
mod tests;
