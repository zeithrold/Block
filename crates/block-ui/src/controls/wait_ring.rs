//! Compact activity indicator for inline waiting states.

use std::{f32::consts::TAU, time::Duration};

use gpui::{
    Animation, AnimationExt as _, AnyElement, IntoElement, ParentElement as _, PathBuilder,
    RenderOnce, Styled as _, canvas, div, point, px, rgb,
};

use crate::Palette;

#[derive(IntoElement)]
pub(super) struct WaitRing {
    palette: Palette,
    phase: f32,
}

impl WaitRing {
    /// GPUI also honors the effective application-wide reduced-motion policy.
    pub(super) fn element(palette: Palette, id: &'static str, animate: bool) -> AnyElement {
        let ring = Self { palette, phase: 0. };
        if animate {
            ring.with_animation(
                id,
                Animation::new(Duration::from_millis(1600)).repeat(),
                |ring, phase| Self { phase, ..ring },
            )
            .into_any_element()
        } else {
            ring.into_any_element()
        }
    }
}

impl RenderOnce for WaitRing {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        div()
            .relative()
            .size(px(14.))
            .flex_shrink_0()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_full()
                    .border(px(1.6))
                    .border_color(rgb(self.palette.border)),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, _| {
                        let (sin, cos) = (self.phase * TAU - TAU * 0.375).sin_cos();
                        let center = bounds.center();
                        let radius = px(6.2);
                        let offset = point(radius * cos, radius * sin);
                        let mut arc = PathBuilder::stroke(px(1.6));
                        arc.move_to(center + offset);
                        arc.arc_to(point(radius, radius), px(0.), false, true, center - offset);
                        if let Ok(path) = arc.build() {
                            window.paint_path(path, rgb(self.palette.foreground));
                        }
                    },
                )
                .size_full(),
            )
    }
}
