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
fn page_spec_shares_the_rendered_page_and_detail_transition_identity(cx: &mut TestAppContext) {
    use crate::navigation::Page;

    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Settings), window, cx);
    });
    frame(cx, 400);
    let index = view.read_with(cx, |view, _| view.page_spec());
    assert_eq!(index.page, Page::Settings);
    assert!(view.read_with(cx, |view, _| view.shell.settings_page.is_none()));
    assert!(crate::test_support::control_bounds(cx, "settings-page-general").is_some());
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::SettingsPage(1), window, cx);
    });
    let category = view.read_with(cx, |view, _| view.page_spec());
    assert_eq!(category.page, index.page);
    assert_eq!(category.key, index.key);
    assert_ne!(category.detail, index.detail);
}

#[gpui::test]
fn reducing_motion_during_page_swap_settles_and_releases_the_outgoing_page(
    cx: &mut TestAppContext,
) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Accounts);
        view
    });
    cx.run_until_parked();
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::About), window, cx);
    });
    frame(cx, 20);
    let outgoing = view.read_with(cx, |view, _| {
        view.motion
            .page
            .outgoing()
            .map(|layer| Rc::downgrade(&layer._snapshot))
    });
    assert!(outgoing.is_some());
    view.update(cx, |view, cx| {
        view.preferences.reduce_motion = true;
        cx.notify();
    });
    frame(cx, 0);
    assert!(view.read_with(cx, |view, _| {
        view.motion.page.outgoing().is_none()
            && !view.motion.moving
            && view.router.tab() == Tab::About
    }));
    assert!(outgoing.is_some_and(|outgoing| outgoing.upgrade().is_none()));
}

#[gpui::test]
fn page_navigation_moves_one_compact_surface_without_resizing(cx: &mut TestAppContext) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Accounts);
        view
    });
    cx.simulate_resize(size(px(360.), px(720.)));
    cx.run_until_parked();
    let native = cx.update(|window, _| window.bounds());
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::About), window, cx)
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
        assert_eq!(
            panes.detail.current.map(|pose| (pose.x, pose.opacity)),
            panes.sidebar.current.map(|pose| (pose.x, pose.opacity))
        );
        assert!(view.motion.page.outgoing().is_some());
    });
    let selector = "navigation-current-0";
    let bounds = cx.debug_bounds(selector);
    assert!(
        bounds.is_some_and(|bounds| bounds.top() == px(crate::design::CHROME_HEIGHT)),
        "only horizontal motion is allowed: {selector} {bounds:?}"
    );
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
fn list_to_detail_moves_one_body_and_keeps_chrome_and_native_bounds(cx: &mut TestAppContext) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.snapshot = crate::test_support::ready_snapshot();
        view.router.navigate(Tab::Instances);
        view
    });
    cx.simulate_resize(size(px(360.), px(720.)));
    cx.run_until_parked();
    let chrome = cx.debug_bounds("window-chrome");
    assert!(chrome.is_some());
    let native = cx.update(|window, _| window.bounds());
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Inspect("profile-a".into()), window, cx)
    });
    frame(cx, 35);
    assert_eq!(cx.debug_bounds("window-chrome"), chrome);
    let content = cx.debug_bounds("navigation-current-1");
    assert!(
        content.is_some_and(|bounds| bounds.left() > px(0.)
            && bounds.left() <= px(20.)
            && bounds.top() == px(crate::design::CHROME_HEIGHT)),
        "the compact detail body moves within the same frame: {content:?}"
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
    assert_eq!(cx.debug_bounds("window-chrome"), chrome);
    assert_eq!(cx.update(|window, _| window.bounds()), native);
}

#[gpui::test]
fn scrolling_before_navigation_freezes_the_last_visible_detail(cx: &mut TestAppContext) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.snapshot = crate::test_support::ready_snapshot();
        view.router.open_record(
            Tab::Instances,
            crate::navigation::RecordRef::Profile("profile-a".into()),
        );
        view
    });
    cx.run_until_parked();
    let body = view.read_with(cx, |view, _| view.shell.games.detail_scroll.bounds());
    cx.simulate_mouse_move(body.center(), None, gpui::Modifiers::none());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: body.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-120.))),
        ..gpui::ScrollWheelEvent::default()
    });
    cx.run_until_parked();
    frame(cx, 30);
    let offset = view.read_with(cx, |view, _| view.shell.games.detail_scroll.offset());
    assert!(
        offset.y < px(0.),
        "the real detail must have scrolled: {offset:?}"
    );
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::EditInstance, window, cx);
        assert_eq!(
            view.motion.detail.current().map(|layer| layer
                ._snapshot
                .shell
                .games
                .detail_scroll
                .offset()),
            Some(offset),
            "the dispatch boundary must capture the live scroll before any new layout"
        );
    });
    cx.run_until_parked();
    frame(cx, 20);
    assert_eq!(
        view.read_with(cx, |view, _| view
            .motion
            .detail
            .outgoing()
            .map(|layer| layer._snapshot.shell.games.detail_scroll.offset())),
        Some(offset),
        "the outgoing surface must preserve the last visible scroll, not its entry snapshot"
    );
    frame(cx, 240);
    assert!(view.read_with(cx, |view, _| view.motion.detail.outgoing().is_none()));
    crate::test_support::press(cx, "alt-left alt-left");
    assert_eq!(
        view.read_with(cx, |view, _| view.router.screen().clone()),
        crate::navigation::Screen::List,
        "both Back keys must route while the returning detail is still mounting"
    );
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
        view.handle(UiAction::Tab(Tab::Accounts), window, cx)
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
        assert!(outgoing.is_some_and(|layer| layer._snapshot.router.tab() == Tab::Accounts));
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
        view.router.navigate(Tab::Accounts);
        view
    });
    cx.simulate_resize(size(px(1152.), px(720.)));
    cx.run_until_parked();
    let old_add = cx.debug_bounds("sign-in");
    assert!(old_add.is_some());
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::About), window, cx)
    });
    cx.run_until_parked();
    if let Some(bounds) = old_add {
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    }
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.router.tab() == Tab::About
        && view.shell.auth.is_none()));
}

#[gpui::test]
fn early_tab_events_settle_navigation_and_forward_once_after_the_clean_focus_tree(
    cx: &mut TestAppContext,
) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.run_until_parked();
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Accounts), window, cx)
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
fn home_about_navigation_is_reversible_at_fixed_bounds(cx: &mut TestAppContext) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view
    });
    cx.simulate_resize(size(px(360.), px(720.)));
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.router.tab() == Tab::Home));
    let native = cx.update(|window, _| window.bounds());

    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::About), window, cx);
    });
    cx.run_until_parked();
    frame(cx, 20);
    assert!(view.read_with(cx, |view, _| {
        view.motion.moving
            && view
                .motion
                .page
                .outgoing()
                .is_some_and(|layer| layer._snapshot.router.tab() == Tab::Home)
    }));
    let incoming = cx.debug_bounds("navigation-current-0");
    assert!(
        incoming.is_some_and(|bounds| bounds.left() > px(0.) && bounds.left() <= px(12.)),
        "About enters from the right within the compact frame: {incoming:?}"
    );
    assert_eq!(cx.update(|window, _| window.bounds()), native);

    frame(cx, 260);
    assert!(view.read_with(cx, |view, _| !view.motion.moving
        && view.motion.page.outgoing().is_none()));

    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::NavigationBack, window, cx)
    });
    cx.run_until_parked();
    frame(cx, 20);
    let incoming = cx.debug_bounds("navigation-current-0");
    assert!(
        incoming.is_some_and(|bounds| bounds.left() < px(0.) && bounds.left() >= px(-12.)),
        "Home must slide back in from the left: {incoming:?}"
    );
    assert!(view.read_with(cx, |view, _| {
        view.motion
            .page
            .outgoing()
            .is_some_and(|layer| layer._snapshot.router.tab() == Tab::About)
    }));
    assert_eq!(cx.update(|window, _| window.bounds()), native);
    frame(cx, 260);
    assert!(view.read_with(cx, |view, _| {
        view.router.tab() == Tab::Home
            && !view.motion.moving
            && view.motion.page.outgoing().is_none()
    }));
}
