//! Compact drill-in actions retain native activation and text measurement.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, Hsla, InspectorElementId,
    InteractiveElement as _, IntoElement, LayoutId, ParentElement as _, Pixels, ScaledPixels,
    SharedString, StatefulInteractiveElement as _, Styled as _, TextLayout, TextStyleRefinement,
    UnderlineStyle, Window, point, px, rgb, underline_y_offset,
};

use super::{
    appearance::Palette,
    base_controls::Control,
    components::{ControlState, interactive_control},
    design::{Icon, SMALL_FONT_SIZE, focus_ring},
};

pub(super) fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    palette: Palette,
    state: ControlState,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Control {
    let label = label.into();
    let foreground = palette.foreground;
    let mut control = interactive_control(
        id,
        label.clone(),
        state,
        gpui::Role::Button,
        foreground,
        on_activate,
    )
    .flex()
    .items_center()
    .justify_center()
    .flex_shrink_0()
    .gap(px(2.))
    .h(px(24.))
    .px_0()
    .py_0()
    .border_0()
    .rounded(px(3.))
    .text_size(px(SMALL_FONT_SIZE))
    .line_height(px(18.))
    .font_weight(gpui::FontWeight::NORMAL)
    .whitespace_nowrap()
    .text_color(rgb(foreground).alpha(0.55))
    .focus_visible(move |style| style.shadow(focus_ring(palette.ring)));
    if state.interactive() {
        control = control
            .hover(move |style| style.text_color(rgb(foreground)))
            .active(|style| style.opacity(0.78));
    }
    control
        .child(DashedLabel {
            label,
            foreground: rgb(foreground).into(),
        })
        .child(Icon::ChevronRight.render(12., foreground))
}

struct DashedLabel {
    label: SharedString,
    foreground: Hsla,
}

impl IntoElement for DashedLabel {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for DashedLabel {
    type RequestLayoutState = TextLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, TextLayout) {
        window.with_text_style(
            Some(TextStyleRefinement {
                color: Some(self.foreground),
                ..Default::default()
            }),
            |window| self.label.request_layout(id, inspector_id, window, cx),
        )
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut TextLayout,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.label
            .prepaint(id, inspector_id, bounds, layout, window, cx);
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut TextLayout,
        prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let decoration_color = window.text_style().color;
        window.with_text_style(
            Some(TextStyleRefinement {
                color: Some(self.foreground),
                ..Default::default()
            }),
            |window| {
                self.label
                    .paint(id, inspector_id, bounds, layout, prepaint, window, cx);
            },
        );
        let lines = layout.line_layouts();
        let Some(line) = lines.first() else {
            return;
        };
        let line = &line.unwrapped_layout;
        let origin = layout.bounds().origin
            + point(
                px(0.),
                underline_y_offset(layout.line_height(), line.ascent, line.descent),
            );
        let style = UnderlineStyle {
            thickness: px(1.),
            color: Some(decoration_color),
            wavy: false,
        };
        let underline = window.underline_bounds(origin, line.width, &style);
        // Anchor the pattern before clipping, and keep both dashes and gaps on
        // device pixels. The native painter owns stroke snapping and opacity.
        let dash = ScaledPixels((2. * window.scale_factor()).round().max(1.));
        let mut gaps = Vec::new();
        let mut start = underline.left() + dash;
        while start < underline.right() {
            let end = (start + dash).min(underline.right());
            gaps.push(start..end);
            start = end + dash;
        }
        window.paint_underline_with_exclusions(origin, line.width, &style, &gaps);
    }
}
