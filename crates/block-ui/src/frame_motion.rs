//! One clock coordinates Home, the sidebar, the native width and the detail pane.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Private product frame bindings."
)]

use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _, Pixels,
    Size, Styled, Window, div, px, rgb, size,
};
use gpui_base::motion::{Easing, Timing};

use super::{Block, Tab, window_geometry};

#[cfg(debug_assertions)]
#[path = "frame_trace.rs"]
pub(super) mod trace;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Pose {
    x: f32,
    y: f32,
    opacity: f32,
}
impl Pose {
    const VISIBLE: Self = Self {
        x: 0.,
        y: 0.,
        opacity: 1.,
    };
    const HOME_HIDDEN: Self = Self {
        x: 0.,
        y: -12.,
        opacity: 0.,
    };
    const SIDEBAR_HIDDEN: Self = Self {
        x: 0.,
        y: 14.,
        opacity: 0.,
    };
    const DETAIL_HIDDEN: Self = Self {
        x: 24.,
        y: 0.,
        opacity: 0.,
    };
    fn mix(self, target: Self, t: f32) -> Self {
        Self {
            x: mix(self.x, target.x, t),
            y: mix(self.y, target.y, t),
            opacity: mix(self.opacity, target.opacity, t),
        }
    }
    fn apply<E: Styled>(self, element: E) -> E {
        element
            .relative()
            .left(px(self.x))
            .top(px(self.y))
            .opacity(self.opacity)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Presentation {
    active: bool,
    home: Pose,
    sidebar: Pose,
    detail: Pose,
    outer_width: f32,
    home_width: f32,
    manage_width: f32,
    content_height: f32,
}
impl Presentation {
    fn settled(
        home: bool,
        home_size: Size<Pixels>,
        manage_size: Size<Pixels>,
        window: &Window,
    ) -> Self {
        let insets = window_geometry::content_insets(window);
        let horizontal = f32::from(insets.left + insets.right);
        Self {
            active: false,
            home: if home {
                Pose::VISIBLE
            } else {
                Pose::HOME_HIDDEN
            },
            sidebar: if home {
                Pose::SIDEBAR_HIDDEN
            } else {
                Pose::VISIBLE
            },
            detail: if home {
                Pose::DETAIL_HIDDEN
            } else {
                Pose::VISIBLE
            },
            outer_width: f32::from(if home {
                home_size.width
            } else {
                manage_size.width
            }),
            home_width: f32::from(home_size.width) - horizontal,
            manage_width: f32::from(manage_size.width) - horizontal,
            content_height: f32::from(window_geometry::content_size(window).height),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Run {
    started: Instant,
    from: Presentation,
    to: Presentation,
    to_home: bool,
    // A reversal during the first stage has no width to retract. It can return
    // directly instead of waiting through an empty window-resize stage.
    short_return: bool,
}
impl Run {
    fn sample(self, now: Instant) -> Presentation {
        let elapsed = now.saturating_duration_since(self.started);
        let (home, sidebar, width, detail, end) = if self.to_home && self.short_return {
            ((20, 120), (0, 90), (0, 0), (0, 80), 140)
        } else if self.to_home {
            ((250, 130), (225, 105), (95, 195), (0, 110), 380)
        } else {
            ((0, 85), (30, 100), (130, 190), (200, 180), 380)
        };
        Presentation {
            active: elapsed < Duration::from_millis(end),
            home: self.from.home.mix(self.to.home, progress(elapsed, home)),
            sidebar: self
                .from
                .sidebar
                .mix(self.to.sidebar, progress(elapsed, sidebar)),
            detail: self
                .from
                .detail
                .mix(self.to.detail, progress(elapsed, detail)),
            outer_width: mix(
                self.from.outer_width,
                self.to.outer_width,
                progress(elapsed, width),
            ),
            ..self.to
        }
    }
}
fn mix(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}
fn progress(elapsed: Duration, (delay, duration): (u64, u64)) -> f32 {
    if elapsed < Duration::from_millis(delay) {
        return 0.;
    }
    if duration == 0 {
        return 1.;
    }
    Timing::new(Duration::from_millis(duration))
        .ease(Easing::CubicBezier {
            x1: 0.22,
            y1: 1.,
            x2: 0.36,
            y2: 1.,
        })
        .sample(elapsed - Duration::from_millis(delay))
        .directed_progress
}

#[derive(Debug, Default)]
pub(super) struct FrameMotion {
    observed_home: Option<bool>,
    run: Option<Run>,
    outgoing: Option<Rc<Block>>,
    presentation: Rc<Cell<Presentation>>,
}
impl FrameMotion {
    pub(super) fn frozen(&self) -> Self {
        Self {
            presentation: self.presentation.clone(),
            ..Self::default()
        }
    }
    pub(super) fn active(&self) -> bool {
        self.presentation.get().active
    }
}

impl Block {
    pub(super) fn is_home_frame(&self) -> bool {
        (self.tab == Tab::Home
            || self.tab == Tab::Onboarding
                && self.shell.first_run.enabled
                && self.shell.first_run.stage == super::first_run::Stage::Welcome)
            && self.form.is_none()
            && self.shell.auth.is_none()
    }

    /// Navigation requests are explicit: scenario setup and manual resizing do
    /// not accidentally replay an entrance or override the user's window size.
    pub(super) fn request_frame_resize(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let home = self.is_home_frame();
        let frame = &self.motion.frame;
        let previous = frame.observed_home.unwrap_or_else(|| {
            self.motion
                .page
                .current()
                .is_some_and(|layer| layer._snapshot.is_home_frame())
        });
        if previous == home {
            return;
        }
        let now = cx.background_executor().now();
        let (home_size, manage_size) = window_geometry::navigation_sizes(window, cx);
        let target = Presentation::settled(home, home_size, manage_size, window);
        let from = frame.run.map_or_else(
            || Presentation::settled(previous, home_size, manage_size, window),
            |run| run.sample(now),
        );
        let from = Presentation {
            outer_width: f32::from(window.viewport_size().width),
            ..from
        };
        let outgoing = self
            .motion
            .page
            .current()
            .map(|layer| layer._snapshot.clone());
        let reduced = self.motion_preferences().reduced();
        let frame = &mut self.motion.frame;
        frame.observed_home = Some(home);
        if reduced {
            frame.run = None;
            frame.outgoing = None;
            frame.presentation.set(target);
            window.resize(size(px(target.outer_width), window.viewport_size().height));
        } else {
            frame.outgoing = outgoing
                .filter(|snapshot| snapshot.is_home_frame() != home)
                .or_else(|| frame.outgoing.take());
            let run = Run {
                started: now,
                from,
                to: target,
                to_home: home,
                short_return: home && (from.outer_width - target.outer_width).abs() < 1.,
            };
            frame.presentation.set(run.sample(now));
            frame.run = Some(run);
            cx.notify();
        }
    }

    pub(super) fn prepare_window_frame(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.visual_only {
            return;
        }
        let home = self.is_home_frame();
        let now = cx.background_executor().now();
        let preferences = self.motion_preferences();
        let frame = &mut self.motion.frame;
        if let Some(run) = frame.run {
            let value = if preferences.reduced() {
                run.to
            } else {
                run.sample(now)
            };
            frame.presentation.set(value);
            let width = px(value.outer_width.round());
            if (window.viewport_size().width - width).abs() >= px(1.) {
                window.resize(size(width, window.viewport_size().height));
            }
            if value.active {
                window.request_animation_frame();
            } else {
                frame.run = None;
                frame.outgoing = None;
            }
        } else {
            // A direct model/fixture change settles without native side effects.
            frame.observed_home = Some(home);
            let (home_size, manage_size) = window_geometry::navigation_sizes(window, cx);
            frame
                .presentation
                .set(Presentation::settled(home, home_size, manage_size, window));
        }
    }

    pub(super) fn page_viewport(&self, window: &Window) -> Size<Pixels> {
        let value = self.motion.frame.presentation.get();
        if value.active {
            size(
                px(if self.is_home_frame() {
                    value.home_width
                } else {
                    value.manage_width
                }),
                px(value.content_height),
            )
        } else {
            window_geometry::content_size(window)
        }
    }
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
        if self.motion.frame.active() {
            self.motion.frame.presentation.get().sidebar.apply(view)
        } else {
            self.page_pane_pose(true).apply(view)
        }
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
        if self.motion.frame.active() {
            self.motion.frame.presentation.get().detail.apply(view)
        } else {
            self.page_pane_pose(false).apply(view)
        }
    }

    pub(super) fn render_frame_body(&self, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let frame = &self.motion.frame;
        let value = frame.presentation.get();
        if !value.active {
            let body = self.render_navigation_body(window, cx);
            #[cfg(debug_assertions)]
            if trace::enabled() {
                return div()
                    .size_full()
                    .relative()
                    .child(body)
                    .child(trace::probe(
                        value,
                        self.is_home_frame(),
                        self.navigation_trace_sample(cx),
                    ))
                    .into_any_element();
            }
            return body;
        }
        let home = self.is_home_frame();
        let outgoing = frame.outgoing.as_deref();
        let home_view = if home {
            Some(self)
        } else {
            outgoing.filter(|view| view.is_home_frame())
        };
        let manage_view = if home {
            outgoing.filter(|view| !view.is_home_frame())
        } else {
            Some(self)
        };
        let mut root = div()
            .id("frame-morph")
            .debug_selector(|| "frame-morph".into())
            .size_full()
            .relative()
            .overflow_hidden()
            // Frozen and incoming bodies are visual until the sequence completes.
            // Chrome remains live, so Back/menu can reverse or retarget it.
            .capture_any_mouse_down(|_, _, cx| cx.stop_propagation())
            .capture_key_down(|event, _, cx| {
                if event.keystroke.key != "escape" {
                    cx.stop_propagation();
                }
            })
            .capture_key_up(|_, _, cx| cx.stop_propagation());
        if let Some(view) = manage_view {
            root = root.child(
                div()
                    .id("frame-manage")
                    .absolute()
                    .top_0()
                    .left_0()
                    .w(px(value.manage_width))
                    .h_full()
                    .child(view.render_page_body(window, cx)),
            );
        }
        if value.home.opacity > 0.001
            && let Some(view) = home_view
        {
            root = root.child(
                div()
                    .id("frame-home")
                    .absolute()
                    .top(px(value.home.y))
                    .left(px(value.home.x))
                    .opacity(value.home.opacity)
                    .w(px(value.home_width))
                    .h_full()
                    .bg(rgb(self.palette.background))
                    .child(view.render_page_body(window, cx)),
            );
        }
        #[cfg(debug_assertions)]
        if trace::enabled() {
            root = root.child(trace::probe(value, home, self.navigation_trace_sample(cx)));
        }
        root.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Pose, Presentation, Run};
    use crate::{Tab, UiAction};
    use gpui::{TestAppContext, VisualTestContext, px, size};
    use std::time::{Duration, Instant};
    fn run(to_home: bool) -> Run {
        let home = Presentation {
            home: Pose::VISIBLE,
            sidebar: Pose::SIDEBAR_HIDDEN,
            detail: Pose::DETAIL_HIDDEN,
            outer_width: 360.,
            ..Presentation::default()
        };
        let manage = Presentation {
            home: Pose::HOME_HIDDEN,
            sidebar: Pose::VISIBLE,
            detail: Pose::VISIBLE,
            outer_width: 1152.,
            ..Presentation::default()
        };
        Run {
            started: Instant::now(),
            from: if to_home { manage } else { home },
            to: if to_home { home } else { manage },
            to_home,
            short_return: false,
        }
    }
    #[test]
    fn forward_sequence_staggers_sidebar_resize_and_detail() {
        let run = run(false);
        let sample = |ms| run.sample(run.started + Duration::from_millis(ms));
        assert!(sample(20).home.opacity < 1.);
        assert_eq!(sample(20).sidebar.opacity, 0.);
        assert!(sample(85).sidebar.opacity > 0.);
        assert_eq!(sample(129).outer_width, 360.);
        assert_eq!(sample(199).detail.opacity, 0.);
        assert!(sample(260).outer_width > 360.);
        assert!(sample(260).detail.opacity > 0.);
        assert_eq!(sample(380).outer_width, 1152.);
        assert!(!sample(380).active);
    }
    #[test]
    fn reverse_hides_content_before_retracting_then_restores_home() {
        let run = run(true);
        let sample = |ms| run.sample(run.started + Duration::from_millis(ms));
        assert!(sample(60).detail.opacity < 1.);
        assert_eq!(sample(94).outer_width, 1152.);
        assert_eq!(sample(249).home.opacity, 0.);
        assert!(sample(300).home.opacity > 0.);
        assert_eq!(sample(380).outer_width, 360.);
        assert_eq!(sample(380).home, Pose::VISIBLE);
    }
    #[test]
    fn retarget_starts_from_the_current_presentation() {
        let forward = run(false);
        let now = forward.started + Duration::from_millis(240);
        let painted = forward.sample(now);
        let reverse = Run {
            from: painted,
            started: now,
            ..run(true)
        };
        let first = reverse.sample(now);
        assert_eq!(first.outer_width, painted.outer_width);
        assert_eq!(first.home, painted.home);
        assert_eq!(first.sidebar, painted.sidebar);
        assert_eq!(first.detail, painted.detail);
    }

    fn frame(cx: &mut VisualTestContext, millis: u64) {
        cx.executor().advance_clock(Duration::from_millis(millis));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        // TestWindow.resize records OS bounds but emits no platform resize
        // callback. Deliver the viewport update that a real backend emits.
        let actual = cx.update(|window, _| window.bounds().size);
        cx.simulate_resize(actual);
        cx.run_until_parked();
    }

    #[gpui::test]
    fn native_width_and_real_sidebar_preserve_home_height_and_origin(cx: &mut TestAppContext) {
        let _system_motion = crate::platform::test_system_motion(false);
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.tab = Tab::Home;
            view
        });
        cx.simulate_resize(size(px(360.), px(720.)));
        cx.run_until_parked();
        let origin = cx.update(|window, _| window.bounds().origin);
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Accounts), window, cx)
        });
        cx.run_until_parked();
        for delta in [60, 90, 100, 130] {
            frame(cx, delta);
            cx.update(|window, _| {
                assert_eq!(window.bounds().origin, origin);
                assert_eq!(window.bounds().size.height, px(720.));
            });
            let sidebar = cx.debug_bounds("management-sidebar");
            assert!(
                sidebar.is_some_and(|bounds| bounds.size.width == px(360.)),
                "fixed outer sidebar bounds: {sidebar:?}"
            );
        }
        assert!(!view.read_with(cx, |view, _| view.motion.frame.active()));
        assert_eq!(cx.update(|window, _| window.bounds().size.width), px(1152.));
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Home), window, cx)
        });
        frame(cx, 380);
        assert_eq!(
            cx.update(|window, _| window.bounds()),
            gpui::Bounds::new(origin, size(px(360.), px(720.)))
        );
        assert!(view.read_with(cx, |view, _| view.motion.frame.outgoing.is_none()));
    }

    #[gpui::test]
    fn reversal_and_runtime_reduce_motion_release_frozen_layers(cx: &mut TestAppContext) {
        let _system_motion = crate::platform::test_system_motion(false);
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.tab = Tab::Home;
            view
        });
        cx.simulate_resize(size(px(360.), px(600.)));
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Accounts), window, cx)
        });
        frame(cx, 100);
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Home), window, cx)
        });
        frame(cx, 140);
        assert!(!view.read_with(cx, |view, _| view.motion.frame.active()));
        assert_eq!(
            cx.update(|window, _| window.bounds().size),
            size(px(360.), px(600.))
        );
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Accounts), window, cx)
        });
        frame(cx, 240);
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            cx.notify();
        });
        frame(cx, 0);
        assert_eq!(
            cx.update(|window, _| window.bounds().size),
            size(px(1152.), px(600.))
        );
        assert!(view.read_with(cx, |view, _| !view.motion.frame.active()
            && view.motion.frame.outgoing.is_none()));
    }

    #[gpui::test]
    fn invisible_incoming_body_cannot_activate_an_account_action(cx: &mut TestAppContext) {
        let _system_motion = crate::platform::test_system_motion(false);
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.tab = Tab::Home;
            view
        });
        cx.simulate_resize(size(px(360.), px(720.)));
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Accounts), window, cx)
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.motion.frame.active()));
        let add = cx.debug_bounds("sign-in");
        assert!(
            add.is_some(),
            "the incoming account list must be laid out before expansion"
        );
        if let Some(add) = add {
            cx.simulate_click(add.center(), gpui::Modifiers::none());
        }
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.shell.auth.is_none()));
        frame(cx, 380);
        if let Some(add) = cx.debug_bounds("sign-in") {
            cx.simulate_click(add.center(), gpui::Modifiers::none());
        }
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.shell.auth.is_some()));
    }

    #[gpui::test]
    fn system_reduced_motion_settles_the_frame_with_application_motion_enabled(
        cx: &mut TestAppContext,
    ) {
        let _system_motion = crate::platform::test_system_motion(true);
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.tab = Tab::Home;
            view.preferences.reduce_motion = false;
            view
        });
        cx.simulate_resize(size(px(360.), px(720.)));
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Accounts), window, cx)
        });
        cx.run_until_parked();
        assert_eq!(
            cx.update(|window, _| window.bounds().size),
            size(px(1152.), px(720.))
        );
        assert!(view.read_with(cx, |view, _| !view.motion.frame.active()
            && view.motion.frame.outgoing.is_none()));
    }
}
