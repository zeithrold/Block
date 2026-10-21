//! Explicitly clipped deferred drawing for the ancestor-mask regression.
//!
//! GPUI's ordinary deferred element deliberately detaches its child from the
//! ancestor mask. This fixture opts into the public explicit-mask API instead.

#![expect(
    clippy::redundant_pub_crate,
    reason = "The private fixture exposes its element only to the enclosing integration test."
)]

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

pub(super) fn clipped_deferred(child: impl IntoElement) -> ClippedDeferred {
    ClippedDeferred {
        child: Some(child.into_any_element()),
    }
}

pub(super) struct ClippedDeferred {
    child: Option<AnyElement>,
}

#[expect(
    clippy::expect_used,
    reason = "GPUI's Element lifecycle requests layout once before consuming the child in prepaint."
)]
impl Element for ClippedDeferred {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let child = self.child.as_mut().expect("child exists until prepaint");
        (child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        (): &mut Self::RequestLayoutState,
        window: &mut Window,
        _: &mut App,
    ) {
        let child = self.child.take().expect("child is deferred exactly once");
        let offset = window.element_offset();
        let mask = window.content_mask();
        window.defer_draw(child, offset, 0, Some(mask));
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        (): &mut Self::RequestLayoutState,
        (): &mut Self::PrepaintState,
        _: &mut Window,
        _: &mut App,
    ) {
    }
}

impl IntoElement for ClippedDeferred {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
