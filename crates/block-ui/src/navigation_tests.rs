//! Pane boundaries and input behavior for the user's navigation-motion rules.
use crate::{Tab, UiAction};
use gpui::{TestAppContext, VisualTestContext, px, size};
use std::{rc::Rc, time::Duration};

fn frame(cx: &mut VisualTestContext, ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[gpui::test]
fn management_navigation_staggers_small_horizontal_panes_without_resizing(cx: &mut TestAppContext) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.tab = Tab::Accounts;
        view
    });
    cx.simulate_resize(size(px(1152.), px(720.)));
    cx.run_until_parked();
    let native = cx.update(|window, _| window.bounds());
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Settings), window, cx)
    });
    cx.run_until_parked();
    frame(cx, 20);
    view.read_with(cx, |view, _| {
        let panes = view.motion.page_frames.get();
        assert!(
            panes
                .sidebar
                .current
                .is_some_and(|pose| pose.opacity > 0. && pose.x > 0. && pose.x <= 12.)
        );
        assert!(
            panes
                .detail
                .current
                .is_some_and(|pose| pose.opacity == 0. && pose.x == 20.)
        );
        assert!(view.motion.page.outgoing().is_some());
    });
    for selector in ["management-sidebar", "management-content"] {
        let bounds = cx.debug_bounds(selector);
        assert!(
            bounds.is_some_and(|bounds| bounds.top() == px(crate::design::CHROME_HEIGHT)),
            "only horizontal motion is allowed: {selector} {bounds:?}"
        );
    }
    frame(cx, 60);
    view.read_with(cx, |view, _| {
        let panes = view.motion.page_frames.get();
        assert!(panes.sidebar.current.is_some_and(|pose| pose.opacity > 0.));
        assert!(
            panes
                .detail
                .current
                .is_some_and(|pose| pose.opacity > 0. && pose.x <= 20.)
        );
    });
    frame(cx, 160);
    assert_eq!(cx.update(|window, _| window.bounds()), native);
    assert!(view.read_with(cx, |view, _| !view.motion.moving
        && view.motion.page.outgoing().is_none()));
}

#[gpui::test]
fn selecting_a_management_subpage_moves_only_its_right_content(cx: &mut TestAppContext) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.tab = Tab::Settings;
        view.open_settings(cx);
        view
    });
    cx.simulate_resize(size(px(1152.), px(720.)));
    cx.run_until_parked();
    let sidebar = cx.debug_bounds("management-sidebar");
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::SettingsPage(1), window, cx)
    });
    frame(cx, 35);
    assert_eq!(cx.debug_bounds("management-sidebar"), sidebar);
    let content = cx.debug_bounds("navigation-current-1");
    assert!(
        content.is_some_and(|bounds| bounds.left() > px(360.)
            && bounds.left() <= px(380.)
            && bounds.top() == px(crate::design::CHROME_HEIGHT)),
        "only the right content moves horizontally: {content:?}"
    );
    view.read_with(cx, |view, cx| {
        let pane = view.page_pane_pose(true);
        assert_eq!(pane.x, 0.);
        assert_eq!(pane.opacity, 1.);
        assert!(view.motion.page.outgoing().is_none());
        let detail = view
            .motion
            .detail
            .sample(cx.background_executor().now(), view.motion_preferences());
        assert!(
            detail
                .current
                .is_some_and(|pose| pose.x > 0. && pose.x <= 20. && pose.opacity > 0.)
        );
        assert!(
            detail
                .outgoing
                .is_some_and(|pose| pose.x >= -20. && pose.opacity > 0.)
        );
    });
    frame(cx, 145);
    assert!(view.read_with(cx, |view, _| view.motion.detail.outgoing().is_none()));
    assert_eq!(cx.debug_bounds("management-sidebar"), sidebar);
}

#[gpui::test]
fn rapid_navigation_retains_one_outgoing_page_and_reduced_motion_releases_it(
    cx: &mut TestAppContext,
) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(1152.), px(720.)));
    cx.run_until_parked();
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Settings), window, cx)
    });
    frame(cx, 70);
    let before = view.read_with(cx, |view, _| view.motion.page_frames.get().sidebar.current);
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::About), window, cx)
    });
    cx.run_until_parked();
    let retained = view.read_with(cx, |view, _| {
        assert_eq!(
            view.motion
                .page_frames
                .get()
                .sidebar
                .outgoing
                .map(|pose| (pose.x, pose.opacity)),
            before.map(|pose| (pose.x, pose.opacity))
        );
        let outgoing = view.motion.page.outgoing();
        assert!(outgoing.is_some_and(|layer| layer._snapshot.tab == Tab::Settings));
        outgoing.map(|layer| Rc::downgrade(&layer._snapshot))
    });
    view.update(cx, |view, cx| {
        view.preferences.reduce_motion = true;
        cx.notify();
    });
    frame(cx, 0);
    assert!(view.read_with(cx, |view, _| !view.motion.moving
        && view.motion.page.outgoing().is_none()));
    assert!(retained.is_some_and(|retained| retained.upgrade().is_none()));
}

#[gpui::test]
fn outgoing_management_controls_cannot_activate_under_the_incoming_page(cx: &mut TestAppContext) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.tab = Tab::Accounts;
        view
    });
    cx.simulate_resize(size(px(1152.), px(720.)));
    cx.run_until_parked();
    let old_add = cx.debug_bounds("sign-in");
    assert!(old_add.is_some());
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Settings), window, cx)
    });
    cx.run_until_parked();
    if let Some(bounds) = old_add {
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    }
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.tab == Tab::Settings
        && view.shell.auth.is_none()
        && view.shell.settings_page == 0));
}

#[gpui::test]
fn early_tab_events_settle_navigation_and_forward_once_after_the_clean_focus_tree(
    cx: &mut TestAppContext,
) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.run_until_parked();
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Settings), window, cx)
    });
    cx.run_until_parked();
    view.update_in(cx, |view, window, cx| {
        view.move_focus(false, window, cx);
        view.move_focus(true, window, cx);
        assert_eq!(view.motion.pending_focus, [false, true]);
        assert!(view.motion.page.outgoing().is_none());
    });
    cx.run_until_parked();
    for _ in 0..2 {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.refresh();
        });
        cx.run_until_parked();
    }
    assert!(
        view.read_with(cx, |view, _| view.motion.pending_focus.is_empty()
            && !view.motion.moving)
    );
}

#[gpui::test]
fn welcome_overview_swap_slides_out_then_resizes_then_reveals(cx: &mut TestAppContext) {
    use crate::first_run::{Action as FirstRunAction, Stage};

    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.begin_first_run(cx);
        view
    });
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.shell.first_run.stage == Stage::Welcome));

    // Welcome slides left; no resize is requested yet.
    view.update_in(cx, |view, window, cx| view.first_run_overview(window, cx));
    cx.run_until_parked();
    frame(cx, 20);
    assert!(view.read_with(cx, |view, _| {
        view.motion.first_run_switch.is_some() && !view.motion.frame.active()
    }));
    let outgoing = cx.debug_bounds("first-run-switch-outgoing");
    assert!(
        outgoing.is_some_and(|bounds| bounds.left() < px(0.)),
        "Welcome must slide away before the frame resizes: {outgoing:?}"
    );

    // The frame widens with the content hidden, then Overview slides in.
    frame(cx, 200);
    assert!(
        view.read_with(cx, |view, _| view.motion.frame.active()),
        "the frame resize must follow the slide"
    );
    frame(cx, 500);
    frame(cx, 30);
    let incoming = cx.debug_bounds("first-run-switch-incoming");
    assert!(
        incoming.is_some_and(|bounds| bounds.left() > px(0.)),
        "Overview must slide in only after the resize: {incoming:?}"
    );
    frame(cx, 260);
    assert!(view.read_with(cx, |view, _| view.motion.first_run_switch.is_none()));

    // The same sequence runs backwards when returning to Welcome.
    view.update_in(cx, |view, window, cx| {
        view.handle(
            UiAction::FirstRun(FirstRunAction::BackToWelcome),
            window,
            cx,
        )
    });
    cx.run_until_parked();
    frame(cx, 20);
    let outgoing = cx.debug_bounds("first-run-switch-outgoing");
    assert!(
        outgoing.is_some_and(|bounds| bounds.left() > px(0.)),
        "Overview must slide away before the frame shrinks: {outgoing:?}"
    );
    frame(cx, 200);
    assert!(view.read_with(cx, |view, _| view.motion.frame.active()));
    frame(cx, 500);
    frame(cx, 30);
    let incoming = cx.debug_bounds("first-run-switch-incoming");
    assert!(
        incoming.is_some_and(|bounds| bounds.left() < px(0.)),
        "Welcome must slide back in from the left: {incoming:?}"
    );
    frame(cx, 260);
    assert!(view.read_with(cx, |view, _| {
        view.shell.first_run.stage == Stage::Welcome && view.motion.first_run_switch.is_none()
    }));
}
