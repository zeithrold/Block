//! Pointer, layout and focus regressions against the production drawer host.
use std::time::Duration;

use crate::{Block, UiAction, shell::Drawer};
use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px, size};

fn fixture(
    cx: &mut TestAppContext,
    reduced: bool,
) -> (
    Entity<Block>,
    &mut VisualTestContext,
    crate::platform::SystemMotionOverride,
) {
    let system_motion = crate::platform::test_system_motion(false);
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
    (view, cx, system_motion)
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
    let (view, cx, _system_motion) = fixture(cx, true);
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
    let (view, cx, _system_motion) = fixture(cx, true);
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
fn fixed_panel_keeps_management_action_visible(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    cx.simulate_resize(size(px(360.), px(720.)));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    let panel = bounds(cx, "block-drawer");
    assert_eq!(panel.size.height, px(540.));
    assert!(panel.origin.y >= bounds(cx, "window-title").bottom() + px(super::TITLE_BAR_GAP));
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

/// A panel wanting more room than the window holds stops a gap below the title
/// bar instead of covering it, so its rounded top corners stay on screen and
/// the surface still reads as a drawer.
#[gpui::test]
fn full_height_panel_stops_a_gap_below_the_title_bar(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    cx.simulate_resize(size(px(360.), px(540.)));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    let title = bounds(cx, "window-title");
    let backdrop = bounds(cx, "block-drawer-backdrop");
    let panel = bounds(cx, "block-drawer");
    assert!(
        panel.size.height < px(540.),
        "the window cannot hold the request"
    );
    assert_eq!(panel.origin.y, title.bottom() + px(super::TITLE_BAR_GAP));
    assert_eq!(
        panel.size.height,
        backdrop.size.height - title.size.height - px(super::TITLE_BAR_GAP),
        "the panel fills the sheet down to its bottom edge"
    );
    assert!(
        bounds(cx, "drawer-handle").origin.y >= title.bottom() + px(super::TITLE_BAR_GAP),
        "the drag handle is reachable below the title bar"
    );
    for selector in ["selector-results", "manage-games"] {
        let control = bounds(cx, selector);
        assert!(
            control.origin.y >= panel.origin.y && control.bottom() <= panel.bottom(),
            "{selector} overflows the shortened panel: {control:?}, {panel:?}"
        );
    }
    let quad = panel_quad(cx);
    let scale = cx.update(|window, _| window.scale_factor());
    let corners = quad.corner_radii;
    assert!((corners.top_left.0 - super::PANEL_RADIUS * scale).abs() < 0.01);
    assert!((corners.top_right.0 - super::PANEL_RADIUS * scale).abs() < 0.01);
    assert_eq!(corners.bottom_left.0, 0.);
    assert_eq!(corners.bottom_right.0, 0.);
    assert!(
        quad.content_mask.bounds.origin.y.0 <= quad.bounds.origin.y.0,
        "the panel's top corners fall outside the sheet's clip"
    );
}

/// The dismissal threshold follows the shortened panel: a 152 px pull is 32% of
/// the 472 px a 540 px window holds below its title bar and gap, but only 28% of
/// the requested 540 px, so a gesture still measured against the request would
/// rebound here.
#[gpui::test]
fn full_height_gesture_measures_the_shortened_panel(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    cx.simulate_resize(size(px(360.), px(540.)));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    let handle = bounds(cx, "drawer-handle").center();
    let pulled = handle + point(px(0.), px(152.));
    cx.simulate_mouse_down(handle, MouseButton::Left, Modifiers::none());
    frame(cx, 100);
    cx.simulate_mouse_move(pulled, MouseButton::Left, Modifiers::none());
    frame(cx, 150); // A pause removes fling velocity, so distance decides.
    cx.simulate_mouse_up(pulled, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert!(!view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
}

/// The painted background of the settled panel, found by its painted size.
#[expect(
    clippy::expect_used,
    reason = "A missing panel background is a test failure."
)]
fn panel_quad(cx: &mut VisualTestContext) -> gpui::Quad {
    let panel = bounds(cx, "block-drawer");
    cx.update(|window, _| {
        let scale = window.scale_factor();
        window
            .painted_quads()
            .into_iter()
            .find(|quad| {
                quad.background.as_solid().is_some()
                    && (quad.bounds.size.width.0 - f32::from(panel.size.width) * scale).abs() < 1.
                    && (quad.bounds.size.height.0 - f32::from(panel.size.height) * scale).abs() < 1.
            })
            .expect("drawer panel background painted")
    })
}

#[gpui::test]
fn exit_keeps_body_and_modal_input_until_animation_finishes(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, false);
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
    let (view, cx, _system_motion) = fixture(cx, false);
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

    let (view, cx, _system_motion) = fixture(cx, false);
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

#[gpui::test]
fn system_reduced_motion_closes_immediately_with_the_app_preference_off(cx: &mut TestAppContext) {
    let (view, cx, _normal_policy) = fixture(cx, false);
    let invoking_focus = cx.update(|window, cx| window.focused(cx));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    frame(cx, 500);
    let _reduced_policy = crate::platform::test_system_motion(true);
    act(&view, UiAction::Drawer(None), cx);
    assert!(!view.read_with(cx, |view, _| view.preferences.reduce_motion));
    assert!(crate::test_support::control_bounds(cx, "block-drawer").is_none());
    assert_eq!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
}

/// Deliver the animation request itself, so this path cannot hide a missing
/// invalidation behind a forced window.refresh().
fn animation_frame(cx: &mut VisualTestContext, millis: u64) {
    cx.executor().advance_clock(Duration::from_millis(millis));
    cx.update(|window, app| {
        assert!(
            window.simulate_next_frame(app) > 0,
            "motion requested another frame"
        );
    });
    cx.run_until_parked();
}

fn scrim_alpha(cx: &mut VisualTestContext) -> f32 {
    let bounds = bounds(cx, "block-drawer-backdrop");
    cx.update(|window, _| {
        let scale = window.scale_factor();
        window
            .painted_quads()
            .into_iter()
            .filter(|quad| {
                (quad.bounds.size.width.0 - f32::from(bounds.size.width) * scale).abs() < 1.
                    && (quad.bounds.size.height.0 - f32::from(bounds.size.height) * scale).abs()
                        < 1.
            })
            .filter_map(|quad| quad.background.as_solid())
            .filter(|color| color.l == 0. && color.a <= 0.1001)
            .map(|color| color.a)
            .fold(0., f32::max)
    })
}

#[gpui::test]
fn entrance_matches_browser_panel_and_backdrop_tracks_without_forced_refresh(
    cx: &mut TestAppContext,
) {
    let (view, cx, _system_motion) = fixture(cx, false);
    act(&view, UiAction::Drawer(Some(Drawer::Accounts)), cx);
    let start = bounds(cx, "block-drawer");
    let mut samples = Vec::new();
    let mut previous = 0;
    // Independent browser cubic-bezier reference samples for each CSS track.
    for (time, panel, scrim) in [
        (45, 0.4010969, 0.2698207),
        (90, 0.6739781, 0.6556943),
        (135, 0.8321921, 0.8471536),
        (225, 0.9613825, 0.9548094),
        (315, 0.9941938, 0.9882384),
        (450, 1., 1.),
    ] {
        animation_frame(cx, time - previous);
        previous = time;
        samples.push((bounds(cx, "block-drawer"), scrim_alpha(cx), panel, scrim));
    }
    let settled = bounds(cx, "block-drawer");
    let travel = settled.size.height + px(2.);
    assert!((start.origin.y - settled.origin.y - travel).abs() < px(0.6));
    for (painted, alpha, panel, scrim) in samples {
        let expected = settled.origin.y + travel * (1. - panel);
        assert!(
            (painted.origin.y - expected).abs() < px(0.6),
            "{painted:?} vs {expected:?}"
        );
        assert!(
            (alpha - 0.1 * scrim).abs() < 0.0001,
            "{alpha} vs {}",
            0.1 * scrim
        );
    }
}

#[gpui::test]
fn reduced_motion_enters_with_settled_panel_and_backdrop(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    act(&view, UiAction::Drawer(Some(Drawer::Accounts)), cx);
    let panel = bounds(cx, "block-drawer");
    assert_eq!(
        panel.origin.y,
        bounds(cx, "block-drawer-backdrop").bottom() - panel.size.height
    );
    assert!((scrim_alpha(cx) - 0.1).abs() < 0.0001);
}
