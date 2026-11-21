//! Keep the checkbox's natural layout while the scroll viewport or moving page
//! clips it. A clipped control must install neither a tab stop nor a hitbox.
use std::{cell::Cell, rc::Rc};

use gpui::{
    App, Bounds, Div, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

pub(super) struct ConsentControl {
    pub(super) content: Div,
    pub(super) enabled: bool,
    pub(super) painted: Rc<Cell<bool>>,
}

impl IntoElement for ConsentControl {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for ConsentControl {
    type RequestLayoutState = <Div as Element>::RequestLayoutState;
    type PrepaintState = Option<<Div as Element>::PrepaintState>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.content.request_layout(id, inspector, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let mask = window.content_mask().bounds;
        let fits = bounds.top() >= mask.top()
            && bounds.bottom() <= mask.bottom()
            && bounds.left() >= mask.left()
            && bounds.right() <= mask.right();
        self.painted.set(false);
        fits.then(|| {
            self.content
                .prepaint(id, inspector, bounds, layout, window, cx)
        })
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(state) = state {
            self.content
                .paint(id, inspector, bounds, layout, state, window, cx);
            self.painted.set(self.enabled);
        }
    }
}
