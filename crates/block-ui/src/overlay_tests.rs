//! Overlay state rules: the component library owns presentation, while the
//! logical stack stays the single source of truth for open, close and order.
use crate::{
    Block, UiAction,
    shell::{Drawer, Surface},
};
use gpui::{Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext, point, px, size};
use gpui_component::WindowExt as _;

fn fixture(cx: &mut TestAppContext) -> (Entity<Block>, &mut VisualTestContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.saved_preferences.reduce_motion = true;
        view.snapshot = crate::test_support::ready_snapshot();
        view.selected_instance = Some("instance-a".into());
        view.selected_profile = Some("profile-a".into());
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    (view, cx)
}

#[expect(
    clippy::expect_used,
    reason = "A missing painted control is an explicit test failure."
)]
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = crate::test_support::control_bounds(cx, selector)
        .expect("the required overlay control did not paint");
    cx.simulate_click(bounds.center(), Modifiers::none());
    crate::test_support::ensure_live_window(cx);
    cx.run_until_parked();
    crate::test_support::ensure_live_window(cx);
    cx.run_until_parked();
}

fn open_delete_profile(view: &Entity<Block>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::DeleteProfileId("profile-a".into()), window, cx);
        });
    });
    cx.run_until_parked();
}

#[gpui::test]
fn delete_profile_confirmation_renders_in_window_and_restores_focus(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let trigger_focus = cx.update(|window, cx| window.focused(cx));
    assert!(trigger_focus.is_some());
    open_delete_profile(&view, cx);
    assert_eq!(
        cx.windows().len(),
        1,
        "a confirmation must not open a separate OS window"
    );
    cx.update(|window, cx| {
        assert!(window.has_active_dialog(cx));
    });
    view.read_with(cx, |view, _| {
        assert!(view.shell.surfaces.top().is_some_and(|layer| matches!(
            &layer.kind,
            Surface::DeleteProfile(id) if id == "profile-a"
        )));
    });
    assert!(crate::test_support::control_bounds(cx, "dialog-layer").is_some());
    assert!(crate::test_support::control_bounds(cx, "confirm-delete").is_some());
    assert!(crate::test_support::control_bounds(cx, "close-drawer").is_some());
    let dialog_focus = cx.update(|window, cx| window.focused(cx));
    assert!(dialog_focus.is_some());
    assert_ne!(dialog_focus, trigger_focus, "focus moves into the layer");

    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    view.read_with(cx, |view, _| assert!(!view.shell.surfaces.is_open()));
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
    });
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        trigger_focus,
        "focus returns to the invoking control"
    );
}

#[gpui::test]
fn escape_closes_only_the_topmost_overlay(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    open_delete_profile(&view, cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(
                UiAction::RequestSignOut("account-fixture".into()),
                window,
                cx,
            );
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::RemoveAccount(_))
        ));
        assert_eq!(view.shell.surfaces.layers().count(), 2);
    });

    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::DeleteProfile(_))
        ));
        assert_eq!(view.shell.surfaces.layers().count(), 1);
    });
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));

    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.surfaces.layers().count(), 0);
    });
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
}

#[gpui::test]
fn confirming_closes_the_overlay_once_and_keeps_the_catalog(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    open_delete_profile(&view, cx);
    click(cx, "confirm-delete");
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.surfaces.layers().count(), 0);
        assert_eq!(view.snapshot.profiles.len(), 1);
    });
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
}

#[gpui::test]
fn a_late_dismissal_never_closes_a_newer_overlay(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    open_delete_profile(&view, cx);
    let stale = view.read_with(cx, |view, _| {
        view.shell
            .surfaces
            .top()
            .filter(|layer| matches!(layer.kind, Surface::DeleteProfile(_)))
            .map_or(0, |layer| layer.revision)
    });
    assert_ne!(stale, 0);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let focus = view.focus.clone();
            view.shell.surfaces.close(window, &focus, cx);
        });
    });
    assert_eq!(
        cx.windows().len(),
        1,
        "closing a dialog must not remove the product window"
    );

    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(
                UiAction::RequestSignOut("account-fixture".into()),
                window,
                cx,
            );
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.dismiss_surface_overlay(stale, window, cx)
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(
            matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::RemoveAccount(_))
            ),
            "a stale dismissal must not remove the current overlay"
        );
    });
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
}

#[gpui::test]
fn the_scrim_blocks_pointer_input_to_the_background(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    cx.run_until_parked();
    let menu = crate::test_support::control_bounds(cx, "app-menu");
    assert!(
        menu.is_some(),
        "the shell menu must paint behind the overlay"
    );
    let Some(menu) = menu else {
        return;
    };
    open_delete_profile(&view, cx);
    cx.simulate_click(menu.origin + point(px(6.), px(6.)), Modifiers::none());
    crate::test_support::ensure_live_window(cx);
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.shell.surfaces.layers().count(),
            1,
            "a background control must not receive the click"
        );
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::DeleteProfile(_))
        ));
    });
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
}

#[gpui::test]
fn home_profile_sheet_opens_in_window_and_escape_restores_focus(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    cx.simulate_resize(size(px(432.), px(768.)));
    cx.run_until_parked();
    let trigger_focus = cx.update(|window, cx| window.focused(cx));
    assert!(trigger_focus.is_some());
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Drawer(Some(Drawer::Profiles)), window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "the sheet stays inside Home");
    cx.update(|window, cx| assert!(crate::drawer::DrawerLayer::is_open(window, cx)));
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::Drawer(Drawer::Profiles))
        ));
    });
    assert!(crate::test_support::control_bounds(cx, "manage-games").is_some());
    let sheet_focus = cx.update(|window, cx| window.focused(cx));
    assert!(sheet_focus.is_some());
    assert_ne!(sheet_focus, trigger_focus, "focus moves into the sheet");

    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    view.read_with(cx, |view, _| assert!(!view.shell.surfaces.is_open()));
    cx.update(|window, cx| assert!(!crate::drawer::DrawerLayer::is_open(window, cx)));
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        trigger_focus,
        "focus returns to the invoking control"
    );
}

#[gpui::test]
fn escape_closes_a_confirmation_above_the_sheet_before_the_sheet(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    cx.simulate_resize(size(px(432.), px(768.)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Drawer(Some(Drawer::Profiles)), window, cx);
            view.handle(UiAction::DeleteProfileId("profile-a".into()), window, cx);
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::DeleteProfile(_))
        ));
        assert_eq!(view.shell.surfaces.layers().count(), 2);
    });
    cx.update(|window, cx| {
        assert!(crate::drawer::DrawerLayer::is_open(window, cx));
        assert!(window.has_active_dialog(cx));
    });

    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::Drawer(Drawer::Profiles))
        ));
        assert_eq!(view.shell.surfaces.layers().count(), 1);
    });
    cx.update(|window, cx| {
        assert!(crate::drawer::DrawerLayer::is_open(window, cx));
        assert!(!window.has_active_dialog(cx));
    });

    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| assert!(!view.shell.surfaces.is_open()));
    cx.update(|window, cx| assert!(!crate::drawer::DrawerLayer::is_open(window, cx)));
}

#[gpui::test]
fn games_help_opens_as_a_dialog_inside_the_product_window(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let trigger_focus = cx.update(|window, cx| window.focused(cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::GamesHelp, window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::GamesHelp)
        ));
    });
    assert!(crate::test_support::control_bounds(cx, "games-help-settings").is_some());
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| assert!(!view.shell.surfaces.is_open()));
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger_focus);
}

#[gpui::test]
fn download_files_dialog_stacks_above_the_downloads_sheet(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    cx.simulate_resize(size(px(432.), px(768.)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Drawer(Some(Drawer::Downloads)), window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(
                UiAction::DownloadFiles(crate::downloads::DownloadGroup::Active),
                window,
                cx,
            );
        });
    });
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "details no longer leave the window");
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::DownloadFiles(_))
        ));
        assert_eq!(view.shell.surfaces.layers().count(), 2);
    });
    cx.update(|window, cx| {
        assert!(crate::drawer::DrawerLayer::is_open(window, cx));
        assert!(window.has_active_dialog(cx));
    });
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::Drawer(Drawer::Downloads))
        ));
    });
    cx.update(|window, cx| {
        assert!(crate::drawer::DrawerLayer::is_open(window, cx));
        assert!(!window.has_active_dialog(cx));
    });
}

#[gpui::test]
fn onboarding_section_opens_as_dialog_and_escape_cancels_the_draft(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let trigger_focus = cx.update(|window, cx| window.focused(cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.begin_onboarding(cx);
            view.open_onboarding_section(crate::game_wizard::WizardPage::Version, window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "the section stays inside Welcome");
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            assert!(matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::OnboardingSection(_))
            ));
            assert!(
                view.shell
                    .wizard
                    .version_search
                    .as_ref()
                    .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window)),
                "the section search is focused inside the dialog"
            );
        });
    });
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| assert!(!view.shell.surfaces.is_open()));
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger_focus);
}

#[gpui::test]
fn auth_provider_add_opens_as_dialog_and_escape_dismisses_it(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let trigger_focus = cx.update(|window, cx| window.focused(cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.shell
                .surfaces
                .open(Surface::AuthProviderAdd, window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(
        cx.windows().len(),
        1,
        "the provider form stays inside the window"
    );
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::AuthProviderAdd)
        ));
    });
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| assert!(!view.shell.surfaces.is_open()));
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger_focus);
}
