//! Fixed-frame bindings for the shared page and pane motion lifecycle.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Private product frame bindings."
)]

use std::{cell::Cell, rc::Rc};

use gpui::{AnyElement, Context, ParentElement as _, Styled, Window};
#[cfg(debug_assertions)]
use gpui::{IntoElement as _, div};

use super::Block;

#[cfg(debug_assertions)]
#[path = "frame_trace.rs"]
pub(super) mod trace;

/// A shared observation of page motion, without a separate clock or layers.
#[derive(Debug, Default)]
pub(super) struct FrameMotion {
    running: Rc<Cell<bool>>,
}
impl FrameMotion {
    pub(super) fn frozen(&self) -> Self {
        Self {
            running: self.running.clone(),
        }
    }
}

impl Block {
    pub(super) fn frame_sidebar<E: Styled + gpui::ParentElement>(&self, view: E) -> E {
        let view = if !self.visual_only {
            view.relative().children(self.motion.page.paint_probe())
        } else {
            view
        };
        #[cfg(debug_assertions)]
        let view = if trace::enabled() {
            view.relative().child(trace::part_probe(
                "sidebar",
                self.motion.outgoing_page.get(),
            ))
        } else {
            view
        };
        view
    }

    pub(super) fn frame_detail<E: Styled + gpui::ParentElement>(&self, view: E) -> E {
        let view = if !self.visual_only {
            view.relative().children(self.motion.page.paint_probe())
        } else {
            view
        };
        #[cfg(debug_assertions)]
        let view = if trace::enabled() {
            view.relative()
                .child(trace::part_probe("detail", self.motion.outgoing_page.get()))
        } else {
            view
        };
        view
    }

    pub(super) fn render_frame_body(&self, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let now = cx.background_executor().now();
        let preferences = self.motion_preferences();
        let running = self
            .motion
            .page
            .sample_delayed(now, preferences, 40)
            .running()
            || self.motion.detail.sample(now, preferences).running()
            || self.motion.wizard.sample(now, preferences).running();
        if !self.visual_only {
            self.motion.frame.running.set(running);
        }
        let body = self.render_navigation_body(window, cx);
        #[cfg(debug_assertions)]
        if trace::enabled() {
            return div()
                .size_full()
                .relative()
                .child(body)
                .child(trace::probe(
                    self.page_spec().page == super::navigation::Page::Home,
                    running,
                    self.page_pane_pose(true),
                    self.page_pane_pose(false),
                    self.navigation_trace_sample(cx),
                ))
                .into_any_element();
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use crate::{Tab, UiAction};
    use gpui::{TestAppContext, VisualTestContext, px, size};
    use std::{rc::Rc, time::Duration};

    fn frame(cx: &mut VisualTestContext, millis: u64) {
        cx.executor().advance_clock(Duration::from_millis(millis));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn navigation_preserves_compact_native_bounds_for_every_destination(cx: &mut TestAppContext) {
        for reduced in [false, true] {
            let _system_motion = crate::platform::test_system_motion(false);
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.router.navigate(Tab::Home);
                view.preferences.reduce_motion = reduced;
                view.saved_preferences.reduce_motion = reduced;
                view
            });
            cx.simulate_resize(size(px(360.), px(720.)));
            cx.run_until_parked();
            let original = cx.update(|window, _| window.bounds());
            for tab in [
                Tab::Accounts,
                Tab::About,
                Tab::Instances,
                Tab::Settings,
                Tab::Home,
            ] {
                view.update_in(cx, |view, window, cx| {
                    view.handle(UiAction::Tab(tab), window, cx);
                });
                for delta in [0, 20, 100, 300] {
                    frame(cx, delta);
                    assert_eq!(cx.update(|window, _| window.bounds()), original);
                }
                assert_eq!(view.read_with(cx, |view, _| view.router.tab()), tab);
                assert!(view.read_with(cx, |view, _| !view.motion.frame.running.get()));
            }
        }
    }

    #[gpui::test]
    fn interrupted_page_motion_keeps_bounds_then_reduced_motion_releases_layers(
        cx: &mut TestAppContext,
    ) {
        let _system_motion = crate::platform::test_system_motion(false);
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.router.navigate(Tab::Accounts);
            view
        });
        cx.simulate_resize(size(px(360.), px(720.)));
        cx.run_until_parked();
        let native = cx.update(|window, _| window.bounds());
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::About), window, cx);
        });
        frame(cx, 60);
        assert!(view.read_with(cx, |view, _| view.motion.frame.running.get()));
        let outgoing = view.read_with(cx, |view, _| {
            view.motion
                .page
                .outgoing()
                .map(|layer| Rc::downgrade(&layer._snapshot))
        });
        assert!(outgoing.is_some());
        let painted = view.read_with(cx, |view, _| view.page_pane_pose(true));
        assert!(
            painted.opacity > 0.5,
            "the incoming page owns the next exit"
        );
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Accounts), window, cx);
        });
        frame(cx, 0);
        assert_eq!(
            view.read_with(cx, |view, _| view
                .motion
                .page_frames
                .get()
                .sidebar
                .outgoing
                .map(|pose| (pose.x, pose.opacity))),
            Some((painted.x, painted.opacity))
        );
        frame(cx, 20);
        assert_eq!(cx.update(|window, _| window.bounds()), native);
        assert_eq!(
            view.read_with(cx, |view, _| view.router.tab()),
            Tab::Accounts
        );
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            cx.notify();
        });
        frame(cx, 0);
        assert_eq!(cx.update(|window, _| window.bounds()), native);
        assert!(
            view.read_with(cx, |view, _| !view.motion.frame.running.get()
                && view.motion.page.outgoing().is_none()
                && view.motion.detail.outgoing().is_none()
                && view.motion.wizard.outgoing().is_none())
        );
        assert!(outgoing.is_some_and(|outgoing| outgoing.upgrade().is_none()));
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = false;
            cx.notify();
        });
        frame(cx, 0);
        assert!(view.read_with(cx, |view, _| !view.motion.frame.running.get()));
        assert_eq!(cx.update(|window, _| window.bounds()), native);
    }

    #[gpui::test]
    fn incoming_account_action_waits_for_page_motion_to_finish(cx: &mut TestAppContext) {
        let _system_motion = crate::platform::test_system_motion(false);
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.router.navigate(Tab::About);
            view
        });
        cx.simulate_resize(size(px(360.), px(720.)));
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Accounts), window, cx);
        });
        frame(cx, 0);
        assert!(view.read_with(cx, |view, _| view.motion.frame.running.get()));
        let add = cx.debug_bounds("sign-in");
        assert!(
            add.is_some(),
            "the incoming account action must be laid out"
        );
        if let Some(add) = add {
            cx.simulate_click(add.center(), gpui::Modifiers::none());
        }
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.shell.auth.is_none()));
        frame(cx, 400);
        let add = cx.debug_bounds("sign-in");
        assert!(
            add.is_some(),
            "the settled account action must remain available"
        );
        if let Some(add) = add {
            cx.simulate_click(add.center(), gpui::Modifiers::none());
        }
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.shell.auth.is_some()));
    }

    #[gpui::test]
    fn system_reduced_motion_settles_pages_without_changing_native_bounds(cx: &mut TestAppContext) {
        let _system_motion = crate::platform::test_system_motion(true);
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.router.navigate(Tab::Home);
            view.preferences.reduce_motion = false;
            view
        });
        cx.simulate_resize(size(px(360.), px(720.)));
        cx.run_until_parked();
        let native = cx.update(|window, _| window.bounds());
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Accounts), window, cx);
        });
        frame(cx, 0);
        assert_eq!(cx.update(|window, _| window.bounds()), native);
        assert_eq!(
            view.read_with(cx, |view, _| view.router.tab()),
            Tab::Accounts
        );
        assert!(
            view.read_with(cx, |view, _| !view.motion.frame.running.get()
                && view.motion.page.outgoing().is_none())
        );
    }
}
