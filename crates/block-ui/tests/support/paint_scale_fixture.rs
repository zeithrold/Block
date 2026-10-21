#![expect(
    clippy::redundant_pub_crate,
    reason = "The same private behavior fixture is shared with the integration harness and explicit native example."
)]
#![expect(
    clippy::suboptimal_flops,
    reason = "Expected corner calculations deliberately match the existing GPU shader arithmetic, without fused rounding."
)]
#![expect(
    clippy::panic,
    reason = "An invalid embedded fixture must fail the regression instead of silently skipping image rendering."
)]

use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyView, Bounds, BoxShadow, Context, Entity, FocusHandle, Image, ImageFormat,
    InteractiveElement as _, IntoElement, PaintedPrimitive, ParentElement as _, Point, Render,
    ScaledPixels, StatefulInteractiveElement as _, Styled as _, UnderlineStyle, Window,
    accesskit::{Action, Node, Role},
    canvas, deferred, div, img, point, px, rgb, size,
};

pub(super) struct PaintedControl {
    pub(super) renders: Rc<Cell<usize>>,
    pub(super) presses: Rc<Cell<usize>>,
    pub(super) focus: FocusHandle,
    pub(super) deferred: bool,
}

impl Render for PaintedControl {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let mut node = Node::new(Role::Button);
        node.set_label("Scaled popup action");
        node.add_action(Action::Click);
        let presses = self.presses.clone();
        let control = div()
            .id("painted-control")
            .debug_selector(|| "painted-control".into())
            .size_full()
            .track_focus(&self.focus)
            .tab_index(0)
            .on_click({
                let presses = presses.clone();
                move |_, _, _| presses.set(presses.get() + 1)
            })
            .accessibility(node, false, move |_, _, _| presses.set(presses.get() + 1))
            .child(
                canvas(
                    |_, _, _| (),
                    |bounds, (), window, _| {
                        let shape = Bounds::new(
                            bounds.origin + point(px(12.), px(16.)),
                            size(px(120.), px(64.)),
                        );
                        window.paint_drop_shadows(
                            shape,
                            gpui::Corners::all(px(8.)),
                            &[BoxShadow {
                                inset: false,
                                color: rgb(0x10_20_30).into(),
                                offset: point(px(2.), px(4.)),
                                blur_radius: px(10.),
                                spread_radius: px(1.),
                            }],
                        );
                        window.paint_quad(gpui::quad(
                            shape,
                            px(8.),
                            rgb(0x33_66_99),
                            px(2.),
                            rgb(0x12_34_56),
                            gpui::BorderStyle::Solid,
                        ));
                        let mut path = gpui::Path::new(bounds.origin + point(px(20.), px(90.)));
                        path.line_to(bounds.origin + point(px(80.), px(90.)));
                        path.line_to(bounds.origin + point(px(50.), px(105.)));
                        window.paint_path(path, rgb(0x00_aa_88));
                        window.paint_underline(
                            bounds.origin + point(px(15.), px(110.)),
                            px(90.),
                            &UnderlineStyle {
                                thickness: px(2.),
                                color: Some(rgb(0x44_77_aa).into()),
                                wavy: true,
                            },
                        );
                    },
                )
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left(px(16.))
                    .top(px(24.))
                    .text_size(px(16.))
                    .child("中文 GPUI 🎮"),
            )
            .child(
                img(Image::from_bytes(
                    ImageFormat::Png,
                    include_bytes!("../../assets/block.png").to_vec(),
                )
                .to_image_data(cx.svg_renderer())
                .unwrap_or_else(|error| panic!("embedded native fixture image: {error}")))
                .absolute()
                .left(px(150.))
                .top(px(20.))
                .w(px(32.))
                .h(px(32.)),
            );
        if self.deferred {
            deferred(control).into_any_element()
        } else {
            control.into_any_element()
        }
    }
}

pub(super) struct Harness {
    pub(super) factor: f32,
    pub(super) nested: bool,
    pub(super) explicit_inert: bool,
    pub(super) child: Entity<PaintedControl>,
    pub(super) backdrop: Rc<Cell<usize>>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        let backdrop = self.backdrop.clone();
        let control = AnyView::from(self.child.clone())
            .cached(gpui::StyleRefinement::default().w(px(200.)).h(px(140.)));
        let content = if self.nested {
            div()
                .size_full()
                .paint_scale_at(0.75, point(px(30.), px(40.)))
                .child(control)
        } else {
            div().size_full().child(control)
        };
        div()
            .id("backdrop")
            .size_full()
            .on_click(move |_, _, _| backdrop.set(backdrop.get() + 1))
            .child(
                div()
                    .id("popup")
                    .absolute()
                    .left(px(50.))
                    .top(px(60.))
                    .w(px(200.))
                    .h(px(140.))
                    .occlude()
                    .inert(self.explicit_inert)
                    .paint_scale(self.factor, point(0.5, 0.5))
                    .child(content),
            )
    }
}

pub(super) fn value(value: ScaledPixels) -> f32 {
    value / ScaledPixels::from(1.)
}

pub(super) fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.02, "{actual} != {expected}");
}

pub(super) fn assert_scaled(
    before: &[PaintedPrimitive],
    after: &[PaintedPrimitive],
    factor: f32,
    origin: Point<ScaledPixels>,
) {
    assert_eq!(
        before.len(),
        after.len(),
        "all original primitives must still paint"
    );
    for (before, after) in before.iter().zip(after) {
        assert_eq!(before.kind, after.kind);
        near(
            value(after.bounds.origin.x),
            value(origin.x) + (value(before.bounds.origin.x) - value(origin.x)) * factor,
        );
        near(
            value(after.bounds.origin.y),
            value(origin.y) + (value(before.bounds.origin.y) - value(origin.y)) * factor,
        );
        near(
            value(after.bounds.size.width),
            value(before.bounds.size.width) * factor,
        );
        near(
            value(after.bounds.size.height),
            value(before.bounds.size.height) * factor,
        );
        if let (Some(before_matrix), Some(after_matrix)) =
            (before.transformation, after.transformation)
        {
            // Match the actual shader's transformed vertex, not just its raw
            // bounds. This catches accidental double scaling of real glyphs.
            for (before, after) in [
                (before.bounds.origin, after.bounds.origin),
                (before.bounds.bottom_right(), after.bounds.bottom_right()),
            ] {
                let apply = |matrix: gpui::TransformationMatrix, point: Point<ScaledPixels>| {
                    [
                        matrix.rotation_scale[0][0] * value(point.x)
                            + matrix.rotation_scale[0][1] * value(point.y)
                            + matrix.translation[0],
                        matrix.rotation_scale[1][0] * value(point.x)
                            + matrix.rotation_scale[1][1] * value(point.y)
                            + matrix.translation[1],
                    ]
                };
                let before = apply(before_matrix, before);
                let after = apply(after_matrix, after);
                near(
                    after[0],
                    value(origin.x) + (before[0] - value(origin.x)) * factor,
                );
                near(
                    after[1],
                    value(origin.y) + (before[1] - value(origin.y)) * factor,
                );
            }
        }
        if let (Some(before), Some(after)) = (before.blur_radius, after.blur_radius) {
            near(value(after), value(before) * factor);
        }
        if let (Some(before), Some(after)) = (before.thickness, after.thickness) {
            near(value(after), value(before) * factor);
        }
        if let (Some(before), Some(after)) = (before.corner_radii, after.corner_radii) {
            near(value(after.top_left), value(before.top_left) * factor);
        }
        if let (Some(before), Some(after)) = (before.border_widths, after.border_widths) {
            near(value(after.left), value(before.left) * factor);
        }
        for (before, after) in before.vertices.iter().zip(&after.vertices) {
            near(
                value(after.x),
                value(origin.x) + (value(before.x) - value(origin.x)) * factor,
            );
            near(
                value(after.y),
                value(origin.y) + (value(before.y) - value(origin.y)) * factor,
            );
        }
    }
}
