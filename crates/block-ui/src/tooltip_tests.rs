use super::*;
use std::time::Duration;

const SHOW_DELAY: Duration = Duration::from_millis(500);

fn frame_after(cx: &mut gpui::VisualTestContext, duration: Duration) {
    cx.executor().advance_clock(duration);
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}
fn now(cx: &mut gpui::VisualTestContext) -> std::time::Instant {
    cx.update(|_, cx| cx.background_executor().now())
}

fn session(view: &Entity<Block>, cx: &gpui::VisualTestContext) -> Rc<RefCell<Session>> {
    view.read_with(cx, |view, _| view.shell.overlays.tooltip.0.clone())
}
fn about(cx: &mut gpui::TestAppContext) -> (Entity<Block>, &mut gpui::VisualTestContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.tab = crate::Tab::About;
        view
    });
    cx.simulate_resize(gpui::size(px(960.), px(600.)));
    frame_after(cx, style().duration());
    (view, cx)
}
fn hover_copy(cx: &mut gpui::VisualTestContext) {
    let copy = cx.debug_bounds("about-copy-build");
    assert!(
        copy.is_some(),
        "the real About copy control must be visible"
    );
    if let Some(copy) = copy {
        cx.simulate_mouse_move(copy.center(), None, gpui::Modifiers::none());
    }
    frame_after(cx, Duration::ZERO);
}

#[gpui::test]
fn base_owns_the_show_delay_and_cancels_a_pending_request_after_leave(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = about(cx);
    let state = session(&view, cx);
    // Override the official defaults to prove the product consumes Base's
    // provider policy rather than maintaining a second 500ms timer.
    cx.update(|_, cx| {
        gpui_base::TooltipDefaults::new()
            .with_show_delay(Duration::from_millis(80))
            .install(cx);
    });
    hover_copy(cx);
    frame_after(cx, Duration::from_millis(79));
    assert!(state.borrow().scene.current().is_none());
    cx.simulate_mouse_move(point(px(40.), px(500.)), None, gpui::Modifiers::none());
    frame_after(cx, Duration::from_millis(500));
    assert!(state.borrow().scene.current().is_none());
    hover_copy(cx);
    frame_after(cx, Duration::from_millis(79));
    assert!(state.borrow().scene.current().is_none());
    frame_after(cx, Duration::from_millis(1));
    assert!(state.borrow().scene.current().is_some());
}

#[gpui::test]
fn navigation_cancels_base_pending_results_and_a_snapshot_has_no_second_provider(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = about(cx);
    hover_copy(cx);
    let state = session(&view, cx);
    assert!(state.borrow().provider.is_some());
    assert!(view.read_with(cx, |view, _| {
        view.shell
            .overlays
            .tooltip
            .clone()
            .0
            .borrow()
            .provider
            .is_none()
    }));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(crate::UiAction::Tab(crate::Tab::Home), window, cx);
        });
    });
    frame_after(cx, SHOW_DELAY * 2);
    assert!(state.borrow().scene.current().is_none());
    assert!(state.borrow().scene.outgoing().is_none());
    assert!(cx.debug_bounds("product-tooltip").is_none());
}

#[gpui::test]
fn base_grace_period_keeps_a_tip_then_starts_the_shared_retained_exit(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = about(cx);
    let state = session(&view, cx);
    hover_copy(cx);
    frame_after(cx, SHOW_DELAY + style().duration());
    assert!(state.borrow().scene.current().is_some());
    cx.simulate_mouse_move(point(px(40.), px(500.)), None, gpui::Modifiers::none());
    frame_after(cx, Duration::from_millis(299));
    assert!(state.borrow().scene.current().is_some());
    frame_after(cx, Duration::from_millis(1));
    assert!(state.borrow().scene.current().is_none());
    assert!(state.borrow().scene.outgoing().is_some());
    frame_after(cx, Duration::from_millis(149));
    assert!(state.borrow().scene.outgoing().is_some());
    frame_after(cx, Duration::from_millis(1));
    assert!(state.borrow().scene.outgoing().is_none());
}

#[gpui::test]
fn escape_preserves_focus_suppresses_reopening_and_closes_only_the_tip(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = about(cx);
    let state = session(&view, cx);
    let initial_focus = cx.update(|window, cx| window.focused(cx));
    hover_copy(cx);
    frame_after(cx, SHOW_DELAY + style().duration());
    assert!(state.borrow().scene.current().is_some());
    assert_eq!(cx.update(|window, cx| window.focused(cx)), initial_focus);
    cx.simulate_keystrokes("escape");
    frame_after(cx, Duration::ZERO);
    assert!(state.borrow().scene.current().is_none());
    assert!(state.borrow().scene.outgoing().is_some());
    assert!(
        cx.update(|window, _| window
            .accessibility_tree()
            .nodes
            .iter()
            .all(|(_, node)| node.role() != gpui::Role::Tooltip)),
        "the actual native tree must omit the closing inert tooltip"
    );
    frame_after(cx, SHOW_DELAY * 2);
    assert!(state.borrow().scene.current().is_none());
    assert_eq!(cx.update(|window, cx| window.focused(cx)), initial_focus);
    assert!(view.read_with(cx, |view, _| view.tab == crate::Tab::About));
    cx.simulate_mouse_move(point(px(40.), px(500.)), None, gpui::Modifiers::none());
    frame_after(cx, Duration::ZERO);
    hover_copy(cx);
    frame_after(cx, SHOW_DELAY);
    assert!(state.borrow().scene.current().is_some());
}

#[gpui::test]
fn keyboard_focus_uses_the_base_controls_existing_handle_and_reduced_exit_is_immediate(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = about(cx);
    // Focus through the official native action path, independently of pointer
    // hover. There is no synthetic tooltip-specific focus handle.
    cx.update(|window, cx| {
        let node = window
            .accessibility_tree()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Copy build information"))
            .map(|(id, node)| {
                assert_eq!(node.description(), Some("Copy build information"));
                *id
            });
        assert!(node.is_some());
        if let Some(node) = node {
            window.simulate_accessibility_action(
                gpui::accesskit::ActionRequest {
                    action: gpui::accesskit::Action::Focus,
                    target_node: node,
                    target_tree: gpui::accesskit::TreeId::ROOT,
                    data: None,
                },
                cx,
            );
        }
    });
    frame_after(cx, SHOW_DELAY);
    let state = session(&view, cx);
    let focus = state
        .borrow()
        .trigger
        .as_ref()
        .and_then(|trigger| trigger.focus.clone());
    assert!(focus.is_some());
    assert!(state.borrow().scene.current().is_some());
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            assert!(view.dismiss_tooltip(window, cx));
        });
        assert!(focus.as_ref().is_some_and(|focus| focus.is_focused(window)));
    });
    assert!(state.borrow().scene.current().is_none());
    assert!(state.borrow().scene.outgoing().is_none());
}

#[test]
fn arrow_and_scale_origin_follow_the_actual_positioner_projection_near_corners() {
    let size = gpui::size(px(180.), px(36.));
    for (left, anchor_x) in [(8_u16, 0_u16), (74, 160), (172, 352)] {
        let bounds = Bounds::new(point(px(f32::from(left)), px(56.)), size);
        let anchor = Bounds::new(
            point(px(f32::from(anchor_x)), px(100.)),
            gpui::size(px(8.), px(20.)),
        );
        let edge = resolved_edge(bounds, anchor);
        assert_eq!(edge, Edge::Top);
        let arrow = arrow_center(bounds, anchor, edge);
        let scale = tooltip_scale_origin(anchor, bounds.origin, bounds.size, edge);
        assert!(arrow.x >= bounds.left() + px(10.) && arrow.x <= bounds.right() - px(10.));
        assert_eq!(arrow.y, bounds.bottom() - px(2.));
        assert_eq!(scale, point(arrow.x, anchor.top()));
        if anchor_x == 160 {
            assert_eq!(arrow.x, anchor.center().x);
        }
    }
}

#[gpui::test]
fn actual_base_positioned_tooltip_paints_its_scaled_arrow_without_clipping(
    cx: &mut gpui::TestAppContext,
) {
    // The standard cross-platform TestPlatform uses NoopTextSystem. Its real
    // geometry/arrow path remains covered; glyph rasterization is checked with
    // CoreText below instead of claiming pixels from placeholder font metrics.
    check_actual_tooltip_paint(cx, false);
}

#[cfg(target_os = "macos")]
#[test]
fn actual_core_text_tooltip_paints_visible_scaled_glyphs_and_arrow() {
    let mut cx = crate::test_support::core_text_context(
        "actual_core_text_tooltip_paints_visible_scaled_glyphs_and_arrow",
    );
    check_actual_tooltip_paint(&mut cx, true);
    cx.run_until_parked();
    cx.update(|cx| {
        cx.background_executor().forbid_parking();
        cx.quit();
    });
    cx.run_until_parked();
}

fn check_actual_tooltip_paint(cx: &mut gpui::TestAppContext, check_glyphs: bool) {
    let (view, cx) = about(cx);
    cx.simulate_resize(gpui::size(px(360.), px(540.)));
    frame_after(cx, Duration::ZERO);
    hover_copy(cx);
    frame_after(cx, SHOW_DELAY);
    frame_after(cx, Duration::from_millis(30));
    let state = session(&view, cx);
    let (pose, anchor) = {
        let state = state.borrow();
        (
            state.scene.sample(now(cx), Preferences::default()).current,
            state.scene.current().map(|layer| layer.snapshot.anchor),
        )
    };
    let geometry = cx
        .debug_bounds("product-tooltip")
        .zip(anchor)
        .map(|(bounds, anchor)| (bounds, resolved_edge(bounds, anchor)));
    assert!(pose.is_some_and(|pose| pose.scale > 0.95 && pose.scale < 1.));
    if let (Some(pose), Some((layout, edge))) = (pose, geometry) {
        cx.update(|window, _| {
            let expected = f32::from(layout.size.width) * pose.scale * window.scale_factor();
            let primitives = window.painted_primitives();
            let bubble = primitives.iter().find(|primitive| {
                primitive.kind == "quad" && (primitive.bounds.size.width.0 - expected).abs() < 0.1
            });
            assert!(
                bubble.is_some(),
                "the actual Base tooltip background must use the paint scale"
            );
            if let Some(bubble) = bubble {
                assert!(
                    primitives.iter().any(|arrow| arrow.kind == "path"
                        && !arrow.vertices.is_empty()
                        && match edge {
                            Edge::Top => arrow.bounds.bottom() > bubble.bounds.bottom(),
                            Edge::Bottom => arrow.bounds.top() < bubble.bounds.top(),
                            Edge::Left => arrow.bounds.right() > bubble.bounds.right(),
                            Edge::Right => arrow.bounds.left() < bubble.bounds.left(),
                        }
                        && arrow
                            .vertices
                            .iter()
                            .all(|vertex| arrow.content_mask.bounds.contains(vertex))),
                    "the real arrow must extend beyond the bubble within the renderer clip"
                );
                if check_glyphs {
                    assert!(
                        primitives.iter().any(|glyph| matches!(glyph.kind, "monochrome" | "subpixel")
                            && bubble.bounds.intersects(&glyph.bounds)
                            && glyph.content_mask.bounds.intersects(&glyph.bounds)),
                        "the scaled tooltip must retain visible, unclipped text glyphs; bubble={bubble:?}; glyphs={:?}",
                        primitives.iter().filter(|primitive| matches!(primitive.kind, "monochrome" | "subpixel")).collect::<Vec<_>>()
                    );
                }
            }
        });
    }
}

#[derive(Default)]
struct FocusObservation {
    live: RefCell<Option<FocusHandle>>,
    disabled: RefCell<Option<FocusHandle>>,
    inert_called: Cell<bool>,
}
struct FocusHarness(Rc<FocusObservation>);
impl Render for FocusHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl gpui::IntoElement {
        let live = self.0.clone();
        let disabled = self.0.clone();
        let inert = self.0.clone();
        let button = |id, state| {
            super::super::components::button(
                id,
                "Test",
                super::super::Palette::LIGHT,
                super::super::components::ButtonVariant::Ghost,
                state,
                |(), _, _| {},
            )
        };
        div()
            .child(
                button("live", super::super::components::ControlState::default())
                    .on_prepaint_bounds(move |_, _, window, cx| {
                        *live.live.borrow_mut() = window.current_element_focus_handle(cx);
                    }),
            )
            .child(
                button(
                    "disabled",
                    super::super::components::ControlState {
                        disabled: true,
                        ..Default::default()
                    },
                )
                .on_prepaint_bounds(move |_, _, window, cx| {
                    *disabled.disabled.borrow_mut() = window.current_element_focus_handle(cx);
                }),
            )
            .child(
                div().inert(true).child(
                    button("inert", super::super::components::ControlState::default())
                        .on_prepaint_bounds(move |_, _, window, cx| {
                            inert.inert_called.set(true);
                            assert!(window.current_element_focus_handle(cx).is_none());
                        }),
                ),
            )
    }
}
#[gpui::test]
fn prepaint_focus_getter_has_no_disabled_or_inert_focus_leak(cx: &mut gpui::TestAppContext) {
    let observation = Rc::new(FocusObservation::default());
    let (_, cx) = crate::test_support::window(cx, |_, _| FocusHarness(observation.clone()));
    cx.run_until_parked();
    assert!(observation.live.borrow().is_some());
    assert!(observation.disabled.borrow().is_none());
    assert!(
        !observation.inert_called.get(),
        "inert descendants do not register triggers"
    );
}

#[gpui::test]
fn actual_base_tooltip_tracks_resize_stays_inside_a_short_viewport_and_never_replays_refresh(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(gpui::size(px(432.), px(180.)));
    frame_after(cx, Duration::from_millis(500));
    if let Some(close) = cx.debug_bounds("window-close") {
        cx.simulate_mouse_move(close.center(), None, gpui::Modifiers::none());
    }
    assert!(
        cx.debug_bounds("window-close").is_some(),
        "the actual window close control must exist"
    );
    frame_after(cx, SHOW_DELAY + style().duration());
    let state = session(&view, cx);
    assert!(state.borrow().scene.current().is_some());
    let token = state
        .borrow()
        .scene
        .sample(now(cx), Preferences::default())
        .token;
    for _ in 0..3 {
        frame_after(cx, Duration::ZERO);
    }
    assert_eq!(
        state
            .borrow()
            .scene
            .sample(now(cx), Preferences::default())
            .token,
        token
    );
    cx.simulate_resize(gpui::size(px(360.), px(120.)));
    frame_after(cx, Duration::ZERO);
    let popup = cx.debug_bounds("product-tooltip");
    assert!(
        popup.is_some_and(|bounds| bounds.left() >= px(8.)
            && bounds.right() <= px(352.)
            && bounds.top() >= px(8.)
            && bounds.bottom() <= px(112.)),
        "resized popup {popup:?}"
    );
    let close = cx.debug_bounds("window-close");
    assert!(
        state.borrow().scene.current().is_none()
            || close.is_some_and(|close| {
                state
                    .borrow()
                    .scene
                    .current()
                    .is_some_and(|layer| layer.snapshot.anchor == close)
            })
    );
}
