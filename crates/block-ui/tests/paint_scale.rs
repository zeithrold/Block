//! Uniform popup scale regressions. `CoreText` glyph verification runs separately
//! in the explicit local macOS `native_paint_scale` example.

#[path = "support/clipped_deferred.rs"]
mod clipped_deferred;
#[path = "support/paint_scale_fixture.rs"]
mod fixture;

use clipped_deferred::clipped_deferred;
use fixture::{Harness, PaintedControl, assert_scaled, near, value};
use gpui::{
    AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, Modifiers,
    PaintedPrimitive, ParentElement as _, Point, Render, StatefulInteractiveElement as _,
    Styled as _, Window,
    accesskit::{Action, ActionRequest, TreeId},
    canvas, div, point, px, rgb, size,
};
use std::{cell::Cell, rc::Rc};

fn snapshot(cx: &mut gpui::VisualTestContext) -> Vec<PaintedPrimitive> {
    cx.update(|window, _| window.painted_primitives())
}

#[expect(
    clippy::too_many_lines,
    reason = "The sequence must retain one entity across actual scale, accessibility, replay, and focus restoration."
)]
fn exercise_scale(cx: &mut gpui::TestAppContext, deferred: bool, accessibility: bool) {
    if !accessibility {
        cx.disable_accessibility();
    }
    let renders = Rc::new(Cell::new(0));
    let presses = Rc::new(Cell::new(0));
    let backdrop = Rc::new(Cell::new(0));
    let (view, cx) = cx.add_window_view(|_, cx| Harness {
        factor: 1.,
        nested: false,
        explicit_inert: false,
        backdrop: backdrop.clone(),
        child: cx.new(|cx| PaintedControl {
            renders: renders.clone(),
            presses: presses.clone(),
            focus: cx.focus_handle().tab_stop(true),
            deferred,
        }),
    });
    cx.run_until_parked();
    let original = snapshot(cx);
    for kind in ["quad", "shadow", "path", "underline", "polychrome"] {
        assert!(
            original.iter().any(|primitive| primitive.kind == kind),
            "missing actual {kind} paint"
        );
    }
    let layout = cx.debug_bounds("painted-control").unwrap_or_default();
    assert_eq!(layout.size, size(px(200.), px(140.)));
    let node = accessibility
        .then(|| {
            cx.update(|window, _| {
                window
                    .accessibility_tree()
                    .nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some("Scaled popup action"))
                    .map(|(id, _)| *id)
            })
        })
        .flatten();
    assert_eq!(node.is_some(), accessibility);
    let initial_renders = renders.get();
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    if accessibility {
        assert!(
            renders.get() > initial_renders,
            "AX-active views recompute the official live semantic tree"
        );
        let refreshed_node = cx.update(|window, _| {
            window
                .accessibility_tree()
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Scaled popup action"))
                .map(|(id, _)| *id)
        });
        assert_eq!(refreshed_node, node, "recomputation preserves identity");
    } else {
        assert_eq!(
            renders.get(),
            initial_renders,
            "AX-disabled views must actually replay their cache"
        );
    }
    let focus = view.read_with(cx, |view, cx| view.child.read(cx).focus.clone());
    cx.update(|window, cx| focus.focus(window, cx));
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        view.factor = 0.8;
        cx.notify();
    });
    cx.run_until_parked();
    let scaled = snapshot(cx);
    let scale_factor = cx.update(|window, _| window.scale_factor());
    assert_scaled(
        &original,
        &scaled,
        0.8,
        point(px(150.), px(130.)).scale(scale_factor),
    );
    assert_eq!(
        cx.debug_bounds("painted-control").unwrap_or_default(),
        layout,
        "text/layout geometry must not reflow"
    );
    cx.update(|window, cx| {
        assert!(
            !accessibility
                || !window
                    .accessibility_tree()
                    .nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some("Scaled popup action"))
        );
        if let Some(id) = node {
            window.simulate_accessibility_action(
                ActionRequest {
                    action: Action::Click,
                    target_tree: TreeId::ROOT,
                    target_node: id,
                    data: None,
                },
                cx,
            );
        }
    });
    cx.simulate_click(layout.center(), Modifiers::none());
    assert_eq!(presses.get(), 0);
    assert_eq!(
        backdrop.get(),
        0,
        "scaled popup must retain passive occlusion"
    );
    let cached_renders = renders.get();
    for _ in 0..3 {
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_scaled(&scaled, &snapshot(cx), 1., Point::default());
    }
    if !accessibility {
        assert_eq!(
            renders.get(),
            cached_renders,
            "exercise actual cached paint replay"
        );
    }
    view.update(cx, |view, cx| {
        view.factor = 1.;
        cx.notify();
    });
    cx.run_until_parked();
    assert_scaled(&original, &snapshot(cx), 1., Point::default());
    cx.update(|window, _| {
        assert!(
            focus.is_focused(window),
            "original focus handle survives scale",
        );
        if accessibility {
            let restored_node = window
                .accessibility_tree()
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Scaled popup action"))
                .map(|(id, _)| *id);
            assert_eq!(restored_node, node);
        }
    });
    cx.simulate_click(layout.center(), Modifiers::none());
    assert_eq!(presses.get(), 1, "settled popup is interactive again");
    view.update(cx, |view, cx| {
        view.factor = 0.8;
        view.explicit_inert = true;
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_click(layout.center(), Modifiers::none());
    assert!(
        backdrop.get() > 0,
        "explicit inert removes even the passive shield"
    );
}

#[gpui::test]
fn cached_paint_scale_transforms_text_images_paths_metrics_and_restores_focus(
    cx: &mut gpui::TestAppContext,
) {
    exercise_scale(cx, false, false);
}

#[gpui::test]
fn deferred_paint_scale_survives_repeated_cache_replay_without_double_scale(
    cx: &mut gpui::TestAppContext,
) {
    exercise_scale(cx, true, false);
}

#[gpui::test]
fn active_accessibility_scale_recomputes_live_nodes_and_restores_the_same_focus(
    cx: &mut gpui::TestAppContext,
) {
    exercise_scale(cx, false, true);
}

#[gpui::test]
fn active_accessibility_deferred_scale_hides_nodes_and_preserves_geometry(
    cx: &mut gpui::TestAppContext,
) {
    exercise_scale(cx, true, true);
}

struct Clipped {
    scale: f32,
}

impl Render for Clipped {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div().w(px(110.)).h(px(100.)).overflow_hidden().child(
            div()
                .absolute()
                .left(px(80.))
                .top(px(20.))
                .w(px(100.))
                .h(px(60.))
                .paint_scale_at(self.scale, point(px(80.), px(20.)))
                .overflow_hidden()
                .child(
                    canvas(
                        |_, _, _| (),
                        |bounds, (), window, _| {
                            window.paint_quad(gpui::fill(
                                Bounds::new(bounds.origin, size(px(200.), px(60.))),
                                rgb(0xff_33_55),
                            ));
                        },
                    )
                    .size_full(),
                ),
        )
    }
}

#[gpui::test]
fn scaling_preserves_ancestor_clip_but_transforms_inner_clip(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, _| Clipped { scale: 1. });
    cx.run_until_parked();
    let scale = cx.update(|window, _| window.scale_factor());
    view.update(cx, |view, cx| {
        view.scale = 0.5;
        cx.notify();
    });
    cx.run_until_parked();
    let primitives = snapshot(cx);
    assert_eq!(primitives.len(), 1);
    let quad = &primitives[0];
    near(value(quad.bounds.origin.x), 80. * scale);
    near(value(quad.bounds.size.width), 100. * scale);
    near(value(quad.content_mask.bounds.origin.x), 80. * scale);
    near(value(quad.content_mask.bounds.size.width), 30. * scale);
    near(value(quad.content_mask.bounds.size.height), 30. * scale);
}

#[gpui::test]
fn nested_absolute_and_relative_scales_compose_once_in_cached_deferred_draws(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = cx.add_window_view(|_, cx| Harness {
        factor: 1.,
        nested: true,
        explicit_inert: false,
        backdrop: Rc::new(Cell::new(0)),
        child: cx.new(|cx| PaintedControl {
            renders: Rc::new(Cell::new(0)),
            presses: Rc::new(Cell::new(0)),
            focus: cx.focus_handle().tab_stop(true),
            deferred: true,
        }),
    });
    cx.run_until_parked();
    let before = snapshot(cx);
    let scale = cx.update(|window, _| window.scale_factor());
    view.update(cx, |view, cx| {
        view.factor = 0.8;
        cx.notify();
    });
    cx.run_until_parked();
    assert_scaled(
        &before,
        &snapshot(cx),
        0.8,
        point(px(150.), px(130.)).scale(scale),
    );
    for _ in 0..3 {
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_scaled(
            &before,
            &snapshot(cx),
            0.8,
            point(px(150.), px(130.)).scale(scale),
        );
    }
}

struct DeferredClip {
    child: gpui::Entity<DeferredClipControl>,
}

struct DeferredClipControl {
    presses: Rc<Cell<usize>>,
    renders: Rc<Cell<usize>>,
}

impl Render for DeferredClipControl {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let presses = self.presses.clone();
        clipped_deferred(
            div()
                .id("clipped-deferred-control")
                .w(px(100.))
                .h(px(60.))
                .bg(rgb(0xff_33_55))
                .on_click(move |_, _, _| presses.set(presses.get() + 1)),
        )
    }
}

impl Render for DeferredClip {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div().w(px(110.)).h(px(100.)).overflow_hidden().child(
            div()
                .absolute()
                .left(px(80.))
                .top(px(20.))
                .w(px(100.))
                .h(px(60.))
                .child(
                    gpui::AnyView::from(self.child.clone())
                        .cached(gpui::StyleRefinement::default().w(px(100.)).h(px(60.))),
                ),
        )
    }
}

#[gpui::test]
fn deferred_visual_clip_also_limits_its_real_hitbox(cx: &mut gpui::TestAppContext) {
    // This regression exercises actual cached replay; accessibility behavior is
    // covered separately because an active semantic tree deliberately bypasses
    // the live view cache.
    cx.disable_accessibility();
    let presses = Rc::new(Cell::new(0));
    let renders = Rc::new(Cell::new(0));
    let (view, cx) = cx.add_window_view(|_, cx| DeferredClip {
        child: cx.new(|_| DeferredClipControl {
            presses: presses.clone(),
            renders: renders.clone(),
        }),
    });
    cx.run_until_parked();
    let primitive = snapshot(cx);
    assert_eq!(primitive.len(), 1);
    let scale = cx.update(|window, _| window.scale_factor());
    near(
        value(primitive[0].content_mask.bounds.size.width),
        110. * scale,
    );
    let initial = renders.get();
    for _ in 0..3 {
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
    assert_eq!(
        renders.get(),
        initial,
        "the deferred mask must also survive actual cached replay"
    );
    for count in 1..=3 {
        cx.simulate_click(point(px(90.), px(40.)), Modifiers::none());
        assert_eq!(presses.get(), count);
        cx.simulate_click(point(px(150.), px(40.)), Modifiers::none());
        assert_eq!(
            presses.get(),
            count,
            "visually clipped deferred control must not receive an invisible click"
        );
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
}
