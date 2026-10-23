//! Pointer, layout and focus regressions against the production drawer host.
use std::time::Duration;

use crate::{Block, UiAction, shell::Drawer};
use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px, size};

fn fixture(cx: &mut TestAppContext, reduced: bool) -> (Entity<Block>, &mut VisualTestContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.snapshot = crate::test_support::ready_snapshot();
        view.selected_profile = Some("profile-a".into());
        view.selected_instance = Some("instance-a".into());
        view.preferences.reduce_motion = reduced;
        view.saved_preferences.reduce_motion = reduced;
        view
    });
    cx.simulate_resize(size(px(432.), px(768.)));
    cx.run_until_parked();
    (view, cx)
}

fn act(view: &Entity<Block>, action: UiAction, cx: &mut VisualTestContext) {
    cx.update(|window, cx| view.update(cx, |view, cx| view.handle(action, window, cx)));
    cx.run_until_parked();
}

fn frame(cx: &mut VisualTestContext, millis: u64) {
    cx.executor().advance_clock(Duration::from_millis(millis));
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[expect(
    clippy::expect_used,
    reason = "Missing painted geometry is a test failure."
)]
fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> gpui::Bounds<gpui::Pixels> {
    crate::test_support::control_bounds(cx, selector).expect("drawer control painted")
}

#[gpui::test]
fn handle_drag_tracks_pointer_rebounds_then_dismisses_and_restores_focus(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx, true);
    let invoking_focus = cx.update(|window, cx| window.focused(cx));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    let start_panel = bounds(cx, "block-drawer");
    let handle = bounds(cx, "drawer-handle").center();
    cx.simulate_mouse_down(handle, MouseButton::Left, Modifiers::none());
    cx.update(|window, app| {
        let layer = gpui_base::Root::read(window, app).plugin::<super::DrawerLayer>();
        assert!(
            layer.is_some_and(|layer| layer
                .read(app)
                .active
                .as_ref()
                .is_some_and(|active| active.drag.is_some())),
            "handle press starts the drag"
        );
    });
    frame(cx, 100);
    let moved = handle + point(px(0.), px(40.));
    cx.simulate_mouse_move(moved, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        bounds(cx, "block-drawer").origin.y,
        start_panel.origin.y + px(40.)
    );
    frame(cx, 150); // A pause removes fling velocity, so a short pull rebounds.
    cx.simulate_mouse_up(moved, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(bounds(cx, "block-drawer").origin.y, start_panel.origin.y);
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.is_open()));

    let handle = bounds(cx, "drawer-handle").center();
    let end = handle + point(px(0.), start_panel.size.height * 0.35);
    cx.simulate_mouse_down(handle, MouseButton::Left, Modifiers::none());
    frame(cx, 100);
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert!(!view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
    assert!(crate::test_support::control_bounds(cx, "block-drawer").is_none());
    assert_eq!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
}

#[gpui::test]
fn body_pointer_motion_does_not_start_a_drawer_gesture(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx, true);
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    let panel = bounds(cx, "block-drawer");
    let list = bounds(cx, "selector-results");
    let start = list.origin + point(px(8.), px(8.));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        start + point(px(0.), px(100.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_up(
        start + point(px(0.), px(100.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.run_until_parked();
    assert_eq!(bounds(cx, "block-drawer").origin.y, panel.origin.y);
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
}

#[gpui::test]
fn short_window_caps_panel_and_keeps_management_action_visible(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx, true);
    cx.simulate_resize(size(px(360.), px(540.)));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    let panel = bounds(cx, "block-drawer");
    assert!(panel.size.height <= px(540. * 0.8));
    for selector in ["drawer-handle", "selector-results", "manage-games"] {
        let control = bounds(cx, selector);
        assert!(
            control.size.height > px(0.),
            "{selector} collapsed: {control:?}"
        );
        assert!(
            control.origin.y >= panel.origin.y && control.bottom() <= panel.bottom(),
            "{selector} overflows the panel: {control:?}, {panel:?}"
        );
    }
}

#[gpui::test]
fn exit_keeps_body_and_modal_input_until_animation_finishes(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx, false);
    let invoking_focus = cx.update(|window, cx| window.focused(cx));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    frame(cx, 500);
    let panel = bounds(cx, "block-drawer");
    act(&view, UiAction::Drawer(None), cx);
    assert!(!view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
    assert!(
        crate::test_support::control_bounds(cx, "manage-games").is_some(),
        "exit retains the real body"
    );
    assert_ne!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
    frame(cx, 100);
    assert!(bounds(cx, "block-drawer").origin.y > panel.origin.y);
    frame(cx, 400);
    assert!(crate::test_support::control_bounds(cx, "block-drawer").is_none());
    assert_eq!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
}

#[gpui::test]
fn rapid_reopen_cannot_be_removed_by_the_previous_exit(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx, false);
    let invoking_focus = cx.update(|window, cx| window.focused(cx));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    frame(cx, 500);
    let stale = view.read_with(cx, |view, _| {
        view.shell.surfaces.top().map(|layer| layer.revision)
    });
    act(&view, UiAction::Drawer(None), cx);
    frame(cx, 100);
    act(&view, UiAction::Drawer(Some(Drawer::Accounts)), cx);
    frame(cx, 500);
    assert!(crate::test_support::control_bounds(cx, "manage-accounts").is_some());
    if let Some(stale) = stale {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.dismiss_surface_overlay(stale, window, cx)
            })
        });
    }
    cx.run_until_parked();
    assert!(crate::test_support::control_bounds(cx, "manage-accounts").is_some());
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
    act(&view, UiAction::Drawer(None), cx);
    frame(cx, 500);
    assert_eq!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
}

#[gpui::test]
fn dialog_opened_during_drawer_exit_keeps_focus_and_returns_to_the_trigger(
    cx: &mut TestAppContext,
) {
    use gpui_component::WindowExt as _;

    let (view, cx) = fixture(cx, false);
    let invoking_focus = cx.update(|window, cx| window.focused(cx));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    frame(cx, 500);
    act(&view, UiAction::Drawer(None), cx);
    act(&view, UiAction::DeleteProfileId("profile-a".into()), cx);
    let dialog_focus = cx.update(|window, cx| window.focused(cx));
    frame(cx, 500);
    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    assert_eq!(cx.update(|window, cx| window.focused(cx)), dialog_focus);
    crate::test_support::press(cx, "escape");
    frame(cx, 250);
    assert_eq!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
}
