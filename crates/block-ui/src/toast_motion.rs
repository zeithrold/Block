//! Animate measured toast positions after Base has resolved stack layout.
//!
//! Interpolating the final bottom edge avoids composing independent height and
//! offset springs. Prepaint offsets move native hitboxes and accessibility with
//! the card, including pointer tracking outside its previous bounds.

use gpui::{
    AnyElement, App, Axis, Bounds, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Point, Window, point, px,
};
use gpui_base::motion::transition;

#[derive(Clone, Copy)]
pub(super) struct SwipeExit {
    offset: Point<Pixels>,
    axis: Axis,
}

impl SwipeExit {
    pub(super) const fn new(offset: Point<Pixels>, axis: Axis) -> Self {
        Self { offset, axis }
    }

    fn offset(self, bounds: Bounds<Pixels>, progress: f32) -> Point<Pixels> {
        let mut offset = self.offset;
        if self.axis == Axis::Horizontal {
            offset.x += bounds.size.width * offset.x.signum() * (1. - progress);
        } else {
            offset.y += bounds.size.height * (1. - progress);
        }
        offset
    }
}

pub(super) struct ToastPosition {
    id: u64,
    child: AnyElement,
    presence: f32,
    drag: Point<Pixels>,
    swipe: Option<(SwipeExit, f32)>,
    collapsed_exit: bool,
}

impl ToastPosition {
    pub(super) fn new(
        id: u64,
        child: AnyElement,
        presence: f32,
        drag: Point<Pixels>,
        swipe: Option<(SwipeExit, f32)>,
        collapsed_exit: bool,
    ) -> Self {
        Self {
            id,
            child,
            presence,
            drag,
            swipe,
            collapsed_exit,
        }
    }
}

impl IntoElement for ToastPosition {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ToastPosition {
    type RequestLayoutState = ();
    type PrepaintState = ();

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
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        // Base bootstraps its measured height while the first frame is fully
        // transparent. Initialize position only once that measurement exists.
        let bottom = if self.presence == 0. {
            bounds.bottom()
        } else {
            transition(
                ElementId::NamedInteger("block-toast-bottom".into(), self.id),
                bounds.bottom(),
                super::css_transition(cx.reduce_motion()),
                window,
                cx,
            )
        };
        let offset = if let Some((swipe, progress)) = self.swipe {
            swipe.offset(bounds, progress)
        } else {
            self.drag
                + point(
                    px(0.),
                    bounds.size.height
                        * (1. - self.presence)
                        * if self.collapsed_exit { 0.4 } else { 1. },
                )
        } + point(px(0.), bottom - bounds.bottom());
        window.with_element_offset(offset, |window| {
            self.child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}
