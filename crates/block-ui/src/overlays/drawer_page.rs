//! Page content inside an existing Drawer. The Sheet still owns chrome, focus
//! trapping, dismissal, and its independent entrance/exit animation.
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, ScrollHandle,
    Styled as _, Window, div, px, relative, rems,
};
use gpui_base::motion::{Easing, Interpolate as _, Timing};

use crate::{
    Palette, UiAction,
    design::{FOCUS_RING_SPREAD, Icon},
    scroll_area::ScrollAreaExt as _,
};

#[derive(Default)]
pub(super) struct HeaderSlots {
    pub leading: Option<HeaderAction>,
    pub trailing: Option<HeaderAction>,
}

pub(super) struct HeaderAction {
    pub id: &'static str,
    pub label: String,
    pub icon: Icon,
    pub action: UiAction,
    pub disabled: bool,
}

pub(super) struct DrawerPage {
    id: &'static str,
    body: AnyElement,
    footer: AnyElement,
}

impl DrawerPage {
    pub(super) const fn new(id: &'static str, body: AnyElement, footer: AnyElement) -> Self {
        Self { id, body, footer }
    }

    pub(super) fn render(self) -> AnyElement {
        div()
            .id(self.id)
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.body)
            .child(div().flex_shrink_0().pt_3().child(self.footer))
            .into_any_element()
    }
}

/// Single-page body with the scrollbar in the outer gutter; its handle
/// belongs to the caller, never to a transient render.
pub(super) fn scroll_body(
    id: &'static str,
    scroll: &ScrollHandle,
    palette: Palette,
    content: AnyElement,
) -> AnyElement {
    viewport()
        .child(
            ScrollPage {
                id,
                scroll,
                active: true,
            }
            .body(palette, content),
        )
        .into_any_element()
}

pub(super) struct ScrollPage<'a> {
    pub id: &'static str,
    pub scroll: &'a ScrollHandle,
    pub active: bool,
}

impl ScrollPage<'_> {
    pub(super) fn body(self, palette: Palette, content: AnyElement) -> AnyElement {
        let id = self.id;
        let outset = px(FOCUS_RING_SPREAD);
        div()
            .id(id)
            .debug_selector(move || id.into())
            .size_full()
            .min_w_0()
            .pr_4()
            .scroll_area(self.scroll, id, palette, false)
            .tab_stop(self.active)
            .child(
                div()
                    .w_full()
                    .pl(outset)
                    .py(outset)
                    .flex()
                    .flex_col()
                    .child(content),
            )
            .into_any_element()
    }
}

fn viewport() -> gpui::Div {
    // Borrow focus paint space from the panel inset; the scrollbar occupies
    // the original right gutter rather than reducing the content width.
    let outset = px(FOCUS_RING_SPREAD);
    div()
        .relative()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .ml(-outset)
        .mt(-outset)
        .mb(-outset)
        .mr(rems(-1.))
        .overflow_hidden()
}

#[derive(Clone, Copy, Debug)]
struct Slide {
    from: f32,
    started: Instant,
}

#[derive(Clone, Copy)]
pub(super) struct MotionTime {
    pub now: Instant,
    pub reduced: bool,
}

/// The path contains live route identities only, not form data or snapshots.
/// A popped route can still paint until its exit finishes; it cannot be Back's target.
#[derive(Clone, Debug)]
pub(super) struct DrawerPages<P> {
    ancestors: Vec<P>,
    active: P,
    outgoing: Vec<P>,
    slide: Cell<Option<Slide>>,
}

impl<P: PartialEq> DrawerPages<P> {
    pub(super) const fn new(root: P) -> Self {
        Self {
            ancestors: Vec::new(),
            active: root,
            outgoing: Vec::new(),
            slide: Cell::new(None),
        }
    }

    pub(super) const fn active(&self) -> &P {
        &self.active
    }

    pub(super) fn can_back(&self) -> bool {
        !self.ancestors.is_empty()
    }

    pub(super) fn push(&mut self, page: P, time: MotionTime) {
        if page != self.active {
            let from = self.position(time);
            self.outgoing.clear();
            self.ancestors
                .push(std::mem::replace(&mut self.active, page));
            self.start(from, time);
        }
    }

    pub(super) fn back(&mut self, time: MotionTime) {
        let from = self.position(time);
        if let Some(parent) = self.ancestors.pop() {
            if self.slide.get().is_none() {
                self.outgoing.clear();
            }
            self.outgoing
                .push(std::mem::replace(&mut self.active, parent));
            self.start(from, time);
        }
    }

    fn start(&mut self, from: f32, time: MotionTime) {
        let moving = !time.reduced && (from - self.depth()).abs() > f32::EPSILON;
        self.slide.set(moving.then_some(Slide {
            from,
            started: time.now,
        }));
    }

    fn depth(&self) -> f32 {
        self.ancestors.len() as f32
    }

    fn position(&self, time: MotionTime) -> f32 {
        let to = self.depth();
        if let Some(slide) = self.slide.get() {
            let progress = Timing::new(Duration::from_millis(280))
                .ease(Easing::Ease)
                .sample(time.now.saturating_duration_since(slide.started));
            if !time.reduced && !progress.finished {
                return slide.from.interpolate(&to, progress.directed_progress);
            }
            self.slide.set(None);
        }
        to
    }

    /// The active page accepts input; only the moving strip retains departing
    /// content. Padding in each scroll body compensates for the focus-paint inset.
    pub(super) fn render(
        &self,
        time: MotionTime,
        window: &Window,
        mut body: impl FnMut(&P, bool) -> AnyElement,
    ) -> AnyElement {
        let position = self.position(time);
        let moving = self.slide.get().is_some();
        if moving {
            window.request_animation_frame();
        }
        let mut strip = viewport();
        for (depth, route) in self
            .ancestors
            .iter()
            .chain(std::iter::once(&self.active))
            .chain(self.outgoing.iter().rev())
            .enumerate()
        {
            let active = depth == self.ancestors.len();
            if moving || active {
                strip = strip.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(depth as f32 - position))
                        .w_full()
                        .min_h_0()
                        .overflow_hidden()
                        .child(body(route, active)),
                );
            }
        }
        strip.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_and_reentry_reverse_from_the_current_offset() {
        let start = Instant::now();
        let time = |ms, reduced| MotionTime {
            now: start + Duration::from_millis(ms),
            reduced,
        };
        let mut pages = DrawerPages::new("index");
        pages.push("details", time(0, false));
        let outward = pages.position(time(100, false));
        assert!(outward > 0. && outward < 1.);
        pages.back(time(100, false));
        assert_eq!(pages.active(), &"index");
        assert!(!pages.can_back());
        assert_eq!(pages.position(time(100, false)), outward);
        let returning = pages.position(time(140, false));
        assert!(returning < outward);
        pages.push("details", time(140, false));
        assert_eq!(pages.position(time(140, false)), returning);
        assert_eq!(pages.position(time(150, true)), 1.);
        assert_eq!(pages.position(time(151, false)), 1.);
    }
}
