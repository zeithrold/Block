//! Intrinsic dimensions from one live layout, never from the scroll viewport.
use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, App, Bounds, Div, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Window, px,
};

/// Participate in the real text/layout pass, but install no hitboxes, focus
/// stops, accessibility children or paint for a disclosure awaiting space.
pub(crate) fn reserve(content: Div) -> AnyElement {
    Reserved(content).into_any()
}

struct Reserved(Div);

impl IntoElement for Reserved {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Reserved {
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
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.0.request_layout(id, inspector, window, cx).0, ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        (): &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        (): &mut (),
        (): &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
}

pub(crate) enum DrawerHeight {
    Fixed(f32),
    Content(DrawerFit),
    /// The product has dropped its draft; retain the panel through its exit.
    Retain,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct DrawerFit(Rc<Measurements>);

#[derive(Debug, Default)]
struct Measurements {
    content: Cell<Pixels>,
    footer: Cell<Pixels>,
    natural: Cell<Option<Pixels>>,
    pending_layout: Cell<bool>,
    settled: Cell<bool>,
}

impl DrawerFit {
    pub(crate) fn invalidate(&self) {
        // Keep sampling the previous target: GPUI retains keyed transitions only
        // across consecutive frames. None is reserved for the first layout.
        self.0.pending_layout.set(true);
        self.0.settled.set(false);
    }

    pub(crate) fn natural(&self) -> Option<f32> {
        self.0.natural.get().map(f32::from)
    }

    pub(crate) fn set_settled(&self, settled: bool) {
        self.0.settled.set(settled && !self.0.pending_layout.get());
    }

    pub(crate) fn settled(&self) -> bool {
        self.0.settled.get()
    }

    /// Only the active route registers this hook. The measured child is inside
    /// the focus-paint padding, which is offset by the strip's negative margins.
    pub(crate) fn content(&self, inner: Div) -> Div {
        let metrics = self.0.clone();
        inner.on_children_prepainted(move |children, _, _| {
            if let [content] = children.as_slice() {
                metrics.content.set(content.size.height);
            }
        })
    }

    pub(crate) fn page(&self, page: Div) -> Div {
        let metrics = self.0.clone();
        page.on_children_prepainted(move |children, _, _| {
            if let [_, footer] = children.as_slice() {
                metrics.footer.set(footer.size.height);
            }
        })
    }

    /// GPUI runs the parent's callback *after* its children. Publish one
    /// complete target, not three partial ones that could restart the transition.
    pub(crate) fn panel(&self, panel: Div, vertical_insets: Pixels) -> Div {
        let metrics = self.0.clone();
        panel.on_children_prepainted(move |children, window, cx| {
            if let [handle, header, ..] = children.as_slice() {
                let natural = (vertical_insets
                    + handle.size.height
                    + header.size.height
                    + metrics.content.get()
                    + metrics.footer.get())
                .ceil()
                .max(px(0.));
                let invalidated = metrics.pending_layout.replace(false);
                if metrics.natural.replace(Some(natural)) != Some(natural) || invalidated {
                    window.defer(cx, |window, _| window.refresh());
                }
            }
        })
    }
}
