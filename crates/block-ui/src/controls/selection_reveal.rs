//! Text-sized selection details. The card owns its pixel spring, not the page.

use std::time::Duration;

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, Div, Element, ElementId, Entity,
    FontWeight, GlobalElementId, InspectorElementId, InteractiveElement as _, IntoElement,
    LayoutId, ParentElement as _, Pixels, SharedString, Size, Style, Styled as _, TextStyle,
    Window, div, point, px, relative, rgb, size,
};
use gpui_base::motion::{Spring, spring};

#[derive(Clone, Default)]
struct MeasuredText {
    height: Option<Pixels>,
    text: Option<SharedString>,
}

struct SelectionReveal {
    id: ElementId,
    text: Option<SharedString>,
    color: u32,
    reduced: bool,
}

struct LayoutState {
    retained: Entity<MeasuredText>,
    text: Option<SharedString>,
    style: TextStyle,
    requested: Option<f32>,
}

pub(super) fn reveal(
    id: impl Into<SharedString>,
    text: Option<String>,
    color: u32,
    reduced: bool,
) -> Div {
    let id = id.into();
    let selector = id.to_string();
    div()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .debug_selector(move || selector.clone())
        .child(SelectionReveal {
            id: id.into(),
            text: text.map(Into::into),
            color,
            reduced,
        })
}

fn measure(
    text: Option<&SharedString>,
    style: &TextStyle,
    width: Option<Pixels>,
    window: &mut Window,
) -> Size<Pixels> {
    let Some(text) = text else {
        return Size::default();
    };
    let line_height = window.pixel_snap(px(18.));
    match window.text_system().shape_text(
        text.clone(),
        px(12.),
        &[style.to_run(text.len())],
        width,
        None,
    ) {
        Ok(lines) => {
            let mut measured = size(px(0.), px(8.));
            for line in lines {
                let line = line.size(line_height);
                measured.width = measured.width.max(line.width).ceil();
                measured.height += line.height;
            }
            measured
        }
        Err(error) => {
            tracing::error!(event = "ui.selection_details.measure_failed", %error);
            Size::default()
        }
    }
}

impl SelectionReveal {
    fn policy(&self, window: &Window) -> Spring {
        Spring::new(Duration::from_millis(260))
            .with_travel(!self.reduced)
            .with_epsilon(0.49 / window.scale_factor())
    }
}

impl IntoElement for SelectionReveal {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectionReveal {
    type RequestLayoutState = LayoutState;
    type PrepaintState = Option<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
    ) -> (LayoutId, LayoutState) {
        let retained = window.use_keyed_state("measured-text", cx, |_, _| MeasuredText::default());
        let previous = retained.read(cx).clone();
        let text = self.text.clone().or(previous.text);
        let requested = previous.height.map(|height| {
            if self.text.is_some() {
                f32::from(height)
            } else {
                0.
            }
        });
        let sampled =
            requested.map(|target| spring("height", target, self.policy(window), window, cx));
        let mut text_style = window.text_style();
        text_style.font_weight = FontWeight::NORMAL;
        text_style.color = rgb(self.color).into();
        let measured_style = text_style.clone();
        let measured_text = text.clone();
        let visible = self.text.is_some();
        let reduced = self.reduced || cx.reduce_motion();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.flex_shrink = 0.;
        let layout = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
            });
            let natural = measure(measured_text.as_ref(), &measured_style, width, window);
            let target = if visible { natural.height } else { px(0.) };
            size(
                known.width.unwrap_or(natural.width),
                if reduced {
                    target
                } else {
                    sampled.map_or(target, |height| px(height.max(0.)))
                },
            )
        });
        (
            layout,
            LayoutState {
                retained,
                text,
                style: text_style,
                requested,
            },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut LayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let natural = measure(
            state.text.as_ref(),
            &state.style,
            Some(bounds.size.width),
            window,
        );
        let target = if self.text.is_some() {
            f32::from(natural.height)
        } else {
            0.
        };
        if state.requested != Some(target) {
            spring("height", target, self.policy(window), window, cx);
        }
        state.retained.update(cx, |retained, _| {
            retained.height = Some(natural.height);
            retained.text = state.text.clone();
        });
        if bounds.size.height <= px(0.) || natural.height <= px(0.) {
            return None;
        }
        let text = state.text.clone()?;
        let progress = (f32::from(bounds.size.height) / f32::from(natural.height)).clamp(0., 1.);
        let mut child = div()
            .pt(px(8.))
            .text_size(px(12.))
            .line_height(px(18.))
            .font_weight(FontWeight::NORMAL)
            .text_color(rgb(self.color))
            .whitespace_normal()
            .opacity(progress)
            .child(text)
            .into_any_element();
        child.layout_as_root(
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::MinContent,
            ),
            window,
            cx,
        );
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            child.prepaint_at(
                bounds.origin + point(px(0.), px(-2. * (1. - progress))),
                window,
                cx,
            );
        });
        Some(child)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut LayoutState,
        child: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(child) = child {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                child.paint(window, cx)
            });
        }
    }
}

#[cfg(test)]
#[path = "selection_reveal_tests.rs"]
mod tests;
