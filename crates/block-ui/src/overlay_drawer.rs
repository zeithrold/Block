//! Direct manipulation for the three Home drawers. No unused snap-point model.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Drawer gestures belong to the private overlay host."
)]

use std::{cell::Cell, time::Instant};

use gpui::{
    Context, InteractiveElement as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement as _, Pixels, StatefulInteractiveElement as _, Styled as _, div, px, rgb,
};

use super::{Block, UiAction, motion::Preferences};

#[derive(Clone, Copy, Debug)]
struct Drag {
    origin: Pixels,
    offset: Pixels,
}
#[derive(Clone, Copy, Debug)]
struct Return {
    from: Pixels,
    started: Instant,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    revision: Option<usize>,
    drag: Option<Drag>,
    released: Pixels,
    returning: Cell<Option<Return>>,
    measured_height: Cell<Option<(usize, Pixels)>>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Frame {
    pub offset: Pixels,
    pub backdrop: f32,
    pub running: bool,
}

impl State {
    fn dragging(&self, revision: usize) -> bool {
        self.revision == Some(revision) && self.drag.is_some()
    }
    pub(super) fn record_height(&self, revision: usize, height: Pixels) {
        if height > px(0.) {
            self.measured_height.set(Some((revision, height)));
        }
    }

    fn height(&self, revision: usize, fallback: Pixels) -> Pixels {
        self.measured_height
            .get()
            .filter(|(measured, _)| *measured == revision)
            .map_or(fallback, |(_, height)| height)
    }
    fn begin(&mut self, revision: usize, pointer: Pixels, now: Instant, preferences: Preferences) {
        let offset = self.frame(revision, px(460.), now, preferences).offset;
        self.revision = Some(revision);
        self.returning.set(None);
        self.released = px(0.);
        self.drag = Some(Drag {
            origin: pointer - offset,
            offset,
        });
    }

    fn move_to(&mut self, revision: usize, pointer: Pixels) -> bool {
        if self.revision != Some(revision) {
            return false;
        }
        if let Some(drag) = &mut self.drag {
            drag.offset = (pointer - drag.origin).max(px(0.));
            return true;
        }
        false
    }

    fn release(
        &mut self,
        revision: usize,
        height: Pixels,
        now: Instant,
        preferences: Preferences,
    ) -> bool {
        if self.revision != Some(revision) {
            return false;
        }
        let Some(drag) = self.drag.take() else {
            return false;
        };
        let threshold = (self.height(revision, height) * 0.25).clamp(px(48.), px(120.));
        if drag.offset >= threshold {
            self.released = drag.offset;
            self.returning.set(None);
            true
        } else {
            self.released = px(0.);
            self.returning
                .set(
                    (!preferences.reduced() && drag.offset > px(0.)).then_some(Return {
                        from: drag.offset,
                        started: now,
                    }),
                );
            false
        }
    }

    fn cancel(&mut self, now: Instant, preferences: Preferences) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        self.released = px(0.);
        self.returning.set(
            (!preferences.reduced() && drag.offset > px(0.)).then_some(Return {
                from: drag.offset,
                started: now,
            }),
        );
    }

    pub(super) fn frame(
        &self,
        revision: usize,
        height: Pixels,
        now: Instant,
        preferences: Preferences,
    ) -> Frame {
        if self.revision != Some(revision) {
            return Frame {
                offset: px(0.),
                backdrop: 1.,
                running: false,
            };
        }
        let (offset, running) = match (self.drag, self.returning.get()) {
            (Some(drag), _) => (drag.offset, false),
            (None, Some(returning)) => {
                let progress = now
                    .saturating_duration_since(returning.started)
                    .as_secs_f32()
                    / 0.35;
                if preferences.reduced() || progress >= 1. {
                    self.returning.set(None);
                    (px(0.), false)
                } else {
                    (
                        returning.from
                            * (1.
                                - gpui_base::motion::Easing::CubicBezier {
                                    x1: 0.16,
                                    y1: 1.,
                                    x2: 0.3,
                                    y2: 1.,
                                }
                                .sample(progress)),
                        true,
                    )
                }
            }
            (None, None) => (self.released, false),
        };
        Frame {
            offset,
            backdrop: 1. - (offset / self.height(revision, height).max(px(1.))).clamp(0., 1.),
            running,
        }
    }
}

impl Block {
    fn drawer_drag_move(
        &mut self,
        revision: usize,
        event: &MouseMoveEvent,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .shell
            .overlays
            .top()
            .is_none_or(|layer| layer.revision != revision)
            || !self.shell.drawer_swipe.dragging(revision)
        {
            return;
        }
        if event.pressed_button == Some(MouseButton::Left) {
            self.shell.drawer_swipe.move_to(revision, event.position.y);
        } else {
            let preferences = self.motion_preferences();
            self.shell
                .drawer_swipe
                .cancel(cx.background_executor().now(), preferences);
        }
        self.motion_dirty = true;
        cx.notify();
        cx.stop_propagation();
    }

    fn drawer_drag_release(
        &mut self,
        revision: usize,
        window: &mut gpui::Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .shell
            .overlays
            .top()
            .is_none_or(|layer| layer.revision != revision)
            || !self.shell.drawer_swipe.dragging(revision)
        {
            return;
        }
        let height = (window.viewport_size().height - px(48.)).min(px(460.));
        let preferences = self.motion_preferences();
        if self.shell.drawer_swipe.release(
            revision,
            height,
            cx.background_executor().now(),
            preferences,
        ) {
            self.handle(UiAction::Drawer(None), window, cx);
        } else {
            self.motion_dirty = true;
            cx.notify();
        }
        cx.stop_propagation();
    }

    pub(super) fn drawer_swipe_handle(
        &self,
        revision: usize,
        cx: &Context<'_, Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let weak_move = cx.entity().downgrade();
        let weak_up = weak_move.clone();
        div()
            .id(("drawer-swipe-handle", revision))
            .debug_selector(|| "drawer-swipe-handle".into())
            .w_full()
            .h(px(16.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor(gpui::CursorStyle::OpenHand)
            .active(|style| style.cursor(gpui::CursorStyle::ClosedHand))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                    if view
                        .shell
                        .overlays
                        .top()
                        .is_some_and(|layer| layer.revision == revision)
                    {
                        let preferences = view.motion_preferences();
                        view.shell.drawer_swipe.begin(
                            revision,
                            event.position.y,
                            cx.background_executor().now(),
                            preferences,
                        );
                        view.motion_dirty = true;
                        cx.notify();
                    }
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .w(px(96.))
                    .h(px(4.))
                    .rounded_full()
                    .bg(rgb(self.palette.secondary)),
            )
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |_, (), window, _| {
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if !phase.bubble() {
                                return;
                            }
                            let _ = weak_move.update(cx, |view, cx| {
                                view.drawer_drag_move(revision, event, cx);
                            });
                        });
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                            if !phase.bubble() || event.button != MouseButton::Left {
                                return;
                            }
                            let _ = weak_up.update(cx, |view, cx| {
                                view.drawer_drag_release(revision, window, cx);
                            });
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn dragging_follows_real_distance_fades_backdrop_and_keeps_revisions_separate() {
        let now = Instant::now();
        let prefs = Preferences::default();
        let mut state = State::default();
        state.begin(1, px(100.), now, prefs);
        assert!(state.move_to(1, px(160.)));
        let frame = state.frame(1, px(300.), now, prefs);
        assert_eq!(frame.offset, px(60.));
        assert!((frame.backdrop - 0.8).abs() < f32::EPSILON);
        assert!(!frame.running);
        assert_eq!(state.frame(2, px(300.), now, prefs).offset, px(0.));
        assert!(!state.move_to(2, px(999.)));
        assert!(state.move_to(1, px(80.)));
        assert_eq!(state.frame(1, px(300.), now, prefs).offset, px(0.));
        state.record_height(1, px(200.));
        state.begin(1, px(0.), now, prefs);
        state.move_to(1, px(60.));
        assert!((state.frame(1, px(460.), now, prefs).backdrop - 0.7).abs() < f32::EPSILON);
        assert!(
            state.release(1, px(460.), now, prefs),
            "dismissal uses the real measured sheet height"
        );
    }

    #[test]
    fn a_short_drag_returns_from_current_distance_and_large_drag_dismisses() {
        let now = Instant::now();
        let prefs = Preferences::default();
        let mut state = State::default();
        state.begin(1, px(0.), now, prefs);
        state.move_to(1, px(40.));
        assert!(!state.release(1, px(400.), now, prefs));
        assert_eq!(state.frame(1, px(400.), now, prefs).offset, px(40.));
        let halfway = state.frame(1, px(400.), now + Duration::from_millis(100), prefs);
        assert!(halfway.running && halfway.offset > px(0.) && halfway.offset < px(40.));
        assert_eq!(
            state
                .frame(1, px(400.), now + Duration::from_millis(350), prefs)
                .offset,
            px(0.)
        );
        state.begin(2, px(0.), now, prefs);
        state.move_to(2, px(120.));
        assert!(state.release(2, px(400.), now, prefs));
        let frozen = state.clone();
        state.begin(3, px(0.), now, prefs);
        assert_eq!(frozen.frame(2, px(400.), now, prefs).offset, px(120.));
        assert_eq!(state.frame(3, px(400.), now, prefs).offset, px(0.));
    }

    #[test]
    fn pointer_cancellation_and_reduced_motion_never_dismiss() {
        let now = Instant::now();
        let normal = Preferences::default();
        let mut state = State::default();
        state.begin(1, px(0.), now, normal);
        state.move_to(1, px(150.));
        let reduced = Preferences {
            app_reduced: true,
            ..normal
        };
        state.cancel(now, reduced);
        assert_eq!(state.frame(1, px(400.), now, normal).offset, px(0.));
        assert!(!state.release(1, px(400.), now, normal));
        state.begin(1, px(0.), now, normal);
        state.move_to(1, px(20.));
        state.release(1, px(400.), now, normal);
        assert!(state.frame(1, px(400.), now, normal).running);
        assert_eq!(state.frame(1, px(400.), now, reduced).offset, px(0.));
        assert!(!state.frame(1, px(400.), now, normal).running);
    }

    fn settle(cx: &mut gpui::VisualTestContext, duration: Duration) {
        cx.executor().advance_clock(duration);
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn the_real_home_drawer_tracks_pointer_outside_the_grabber_then_returns_or_dismisses(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.handle(
                UiAction::Drawer(Some(crate::shell::Drawer::Downloads)),
                window,
                cx,
            );
            view
        });
        cx.simulate_resize(gpui::size(px(432.), px(768.)));
        settle(cx, Duration::from_millis(500));
        let sheet = cx.debug_bounds("drawer-sheet");
        let handle = cx.debug_bounds("drawer-swipe-handle");
        assert!(sheet.is_some() && handle.is_some());
        if let (Some(sheet), Some(handle)) = (sheet, handle) {
            let origin = handle.center();
            let short = origin + gpui::point(px(0.), px(40.));
            cx.simulate_mouse_down(origin, MouseButton::Left, gpui::Modifiers::none());
            cx.simulate_mouse_move(short, Some(MouseButton::Left), gpui::Modifiers::none());
            cx.run_until_parked();
            assert!(
                cx.debug_bounds("drawer-sheet")
                    .is_some_and(|moved| (moved.top() - sheet.top() - px(40.)).abs() <= px(1.))
            );
            cx.simulate_mouse_up(short, MouseButton::Left, gpui::Modifiers::none());
            settle(cx, Duration::from_millis(100));
            assert!(view.read_with(cx, |view, _| view.shell.overlays.is_open()));
            assert!(cx.debug_bounds("drawer-sheet").is_some_and(
                |moved| moved.top() > sheet.top() && moved.top() < sheet.top() + px(40.)
            ));
            settle(cx, Duration::from_millis(250));
            assert!(
                cx.debug_bounds("drawer-sheet")
                    .is_some_and(|moved| (moved.top() - sheet.top()).abs() <= px(1.))
            );
            let far = origin + gpui::point(px(0.), px(160.));
            cx.simulate_mouse_down(origin, MouseButton::Left, gpui::Modifiers::none());
            cx.simulate_mouse_move(far, Some(MouseButton::Left), gpui::Modifiers::none());
            cx.simulate_mouse_up(far, MouseButton::Left, gpui::Modifiers::none());
            assert!(!view.read_with(cx, |view, _| view.shell.overlays.is_open()));
            settle(cx, Duration::from_millis(500));
            cx.update(|window, cx| {
                assert!(view.read(cx).focus.is_focused(window));
            });
        }
    }
}
