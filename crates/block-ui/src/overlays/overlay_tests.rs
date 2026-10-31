//! Overlay state rules: the component library owns presentation, while the
//! logical stack stays the single source of truth for open, close and order.
use crate::{
    Block, UiAction,
    shell::{Drawer, Surface},
};
use gpui::{
    Entity, Focusable as _, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px,
    size,
};
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

const CRASH_RUN: &str = "00000000-0000-4000-8000-000000000001";

fn crash_message(root: &std::path::Path) -> anyhow::Result<crate::engine::Message> {
    let report = block_launcher::crash::CrashReport::new(
        block_launcher::crash::CrashContext {
            run_id: CRASH_RUN.into(),
            profile_id: "profile-a".into(),
            profile_name: "Test profile".into(),
            minecraft_version: "1.20.6".into(),
            loader: None,
        },
        block_launcher::crash::ExitOutcome::start_failed(),
        "Synthetic start failure",
        Vec::new(),
    );
    let directory = block_launcher::session_host::report_directory(root, CRASH_RUN)?;
    std::fs::create_dir_all(&directory)?;
    std::fs::write(directory.join("crash.json"), serde_json::to_vec(&report)?)?;
    let guard = block_launcher::session_host::claim_help(root, CRASH_RUN)?
        .ok_or_else(|| anyhow::anyhow!("test report already has a presenter"))?;
    Ok(crate::engine::Message::Crash {
        report: Box::new(report),
        directory,
        guard,
    })
}

fn deliver_crash(
    view: &Entity<Block>,
    cx: &mut VisualTestContext,
    root: &std::path::Path,
) -> anyhow::Result<()> {
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    sender.try_send(crash_message(root)?)?;
    view.update(cx, |view, cx| {
        view.messages = Some(receiver);
        view.poll(cx);
    });
    cx.run_until_parked();
    Ok(())
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
fn enter_dismisses_a_confirmation_without_running_its_destructive_action(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _messages) = crate::Engine::with_root(root.path().to_path_buf())?;
        let (view, cx) = fixture(cx);
        view.update(cx, |view, _| view.engine = Some(engine));
        open_delete_profile(&view, cx);
        let revision = view.read_with(cx, |view, _| view.operation_revision);
        crate::test_support::press(cx, "enter");
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(!view.shell.surfaces.is_open());
            assert_eq!(view.snapshot.profiles.len(), 1);
            assert_eq!(view.snapshot.profiles[0].id, "profile-a");
            assert_eq!(view.operation_revision, revision);
            assert!(!view.busy);
        });
        cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn non_cancellable_work_blocks_both_dialog_dismissal_keys(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    open_delete_profile(&view, cx);
    view.update(cx, |view, cx| {
        view.busy = true;
        view.operation_cancellable = false;
        cx.notify();
    });
    cx.run_until_parked();
    for key in ["escape", "enter"] {
        crate::test_support::press(cx, key);
        cx.run_until_parked();
        cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
        view.read_with(cx, |view, _| {
            assert!(matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::DeleteProfile(_))
            ));
            assert_eq!(view.snapshot.profiles.len(), 1);
        });
    }
    view.update(cx, |view, cx| {
        view.busy = false;
        cx.notify();
    });
    crate::test_support::press(cx, "enter");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(!view.shell.surfaces.is_open());
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
fn escape_after_a_crash_closes_games_help_not_the_launcher(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (view, cx) = fixture(cx);
        click(cx, "games-help");
        let window_id = gpui::VisualContext::window_handle(cx).window_id();
        deliver_crash(&view, cx, root.path())?;

        crate::test_support::press(cx, "escape");
        assert!(
            cx.windows()
                .iter()
                .any(|window| window.window_id() == window_id),
            "Escape belongs to Games help and must not close the launcher"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn crash_waits_for_games_help_before_taking_focus_and_marking_seen(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _messages) = crate::Engine::with_root(root.path().to_path_buf())?;
        let (view, cx) = fixture(cx);
        view.update(cx, |view, _| {
            let mut other = view.snapshot.profiles[0].clone();
            other.id = "profile-b".into();
            view.snapshot.profiles.push(other);
            view.selected_profile = Some("profile-b".into());
            view.engine = Some(engine);
        });
        click(cx, "games-help");
        let window_id = gpui::VisualContext::window_handle(cx).window_id();
        let dialog_focus = cx.update(|window, cx| window.focused(cx));
        assert!(dialog_focus.is_some());
        deliver_crash(&view, cx, root.path())?;

        let directory = block_launcher::session_host::report_directory(root.path(), CRASH_RUN)?;
        assert!(!directory.join("seen.json").exists());
        assert!(block_launcher::session_host::claim_help(root.path(), CRASH_RUN)?.is_none());
        view.read_with(cx, |view, _| {
            assert_eq!(view.pending_crashes.len(), 1);
            assert!(view.crash_help.is_none());
            assert!(view.crash_guard.is_none());
        });
        assert_eq!(cx.update(|window, cx| window.focused(cx)), dialog_focus);
        assert!(crate::test_support::control_bounds(cx, "games-help-settings").is_some());
        assert!(crate::test_support::control_bounds(cx, "crash-return").is_none());

        crate::test_support::press(cx, "escape");
        cx.run_until_parked();
        assert!(
            cx.windows()
                .iter()
                .any(|window| window.window_id() == window_id)
        );
        cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
        view.read_with(cx, |view, _| {
            assert!(!view.shell.surfaces.is_open());
            assert!(view.pending_crashes.is_empty());
            assert!(view.crash_help.is_some());
            assert!(view.crash_guard.is_some());
        });
        assert!(directory.join("seen.json").exists());
        assert!(crate::test_support::control_bounds(cx, "crash-return").is_some());
        let crash_focus = cx.update(|window, cx| window.focused(cx));
        assert!(crash_focus.is_some());
        assert_ne!(crash_focus, dialog_focus);

        let revision = view.read_with(cx, |view, _| view.operation_revision);
        crate::test_support::press(cx, "enter");
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.crash_help.is_none());
            assert!(view.crash_guard.is_none());
            assert_eq!(view.selected_profile.as_deref(), Some("profile-a"));
            assert_eq!(view.operation_revision, revision);
            assert!(!view.busy);
            assert!(view.active_run.is_none());
            assert!(!view.pending_play);
        });
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

fn crash_follows_pointer_dismissal(cx: &mut TestAppContext, backdrop: bool) -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let (view, cx) = fixture(cx);
    click(cx, "games-help");
    let window_id = gpui::VisualContext::window_handle(cx).window_id();
    deliver_crash(&view, cx, root.path())?;
    if backdrop {
        cx.simulate_click(point(px(20.), px(250.)), Modifiers::none());
        cx.run_until_parked();
    } else {
        click(cx, "close-drawer");
    }
    assert!(
        cx.windows()
            .iter()
            .any(|window| window.window_id() == window_id)
    );
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    view.read_with(cx, |view, _| {
        assert!(!view.shell.surfaces.is_open());
        assert!(view.pending_crashes.is_empty());
        assert!(view.crash_help.is_some());
    });
    assert!(crate::test_support::control_bounds(cx, "crash-return").is_some());
    crate::test_support::press(cx, "enter");
    assert!(view.read_with(cx, |view, _| view.crash_help.is_none()));
    Ok(())
}

#[gpui::test]
fn crash_follows_the_games_help_close_button(cx: &mut TestAppContext) {
    let result = crash_follows_pointer_dismissal(cx, false);
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn crash_follows_the_games_help_backdrop(cx: &mut TestAppContext) {
    let result = crash_follows_pointer_dismissal(cx, true);
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn crash_waits_for_nested_dialog_and_the_drawer_exit(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let _system_motion = crate::platform::test_system_motion(false);
        let (view, cx) = fixture(cx);
        view.update(cx, |view, _| {
            view.preferences.reduce_motion = false;
            view.saved_preferences.reduce_motion = false;
        });
        cx.simulate_resize(size(px(432.), px(768.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Drawer(Some(Drawer::Profiles)), window, cx);
            });
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::DeleteProfileId("profile-a".into()), window, cx);
            });
        });
        cx.run_until_parked();
        deliver_crash(&view, cx, root.path())?;

        crate::test_support::press(cx, "enter");
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::Drawer(Drawer::Profiles))
            ));
            assert_eq!(view.pending_crashes.len(), 1);
            assert!(view.crash_help.is_none());
        });
        cx.update(|window, cx| {
            assert!(!window.has_active_dialog(cx));
            assert!(crate::drawer::DrawerLayer::is_open(window, cx));
        });

        let drawer_focus = cx.update(|window, cx| window.focused(cx));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(300));
        cx.run_until_parked();
        assert_eq!(cx.update(|window, cx| window.focused(cx)), drawer_focus);
        crate::test_support::press(cx, "escape");
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(!view.shell.surfaces.is_open());
            assert_eq!(view.pending_crashes.len(), 1);
            assert!(view.crash_help.is_none());
        });
        cx.update(|window, cx| assert!(crate::drawer::DrawerLayer::is_open(window, cx)));
        assert_eq!(cx.update(|window, cx| window.focused(cx)), drawer_focus);

        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.update(|window, app| {
            assert!(
                window.simulate_next_frame(app) > 0,
                "drawer exit requests a frame"
            );
        });
        cx.run_until_parked();
        cx.update(|window, cx| assert!(!crate::drawer::DrawerLayer::is_open(window, cx)));
        view.read_with(cx, |view, _| {
            assert!(view.pending_crashes.is_empty());
            assert!(view.crash_help.is_some());
            assert!(view.crash_guard.is_some());
        });
        assert!(crate::test_support::control_bounds(cx, "crash-return").is_some());
        crate::test_support::press(cx, "enter");
        assert!(view.read_with(cx, |view, _| view.crash_help.is_none()));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn crash_waits_for_a_drawer_reopened_during_exit(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _messages) = crate::Engine::with_root(root.path().to_path_buf())?;
        let _system_motion = crate::platform::test_system_motion(false);
        let (view, cx) = fixture(cx);
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = false;
            view.saved_preferences.reduce_motion = false;
            view.engine = Some(engine);
            cx.notify();
        });
        cx.simulate_resize(size(px(432.), px(768.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Drawer(Some(Drawer::Profiles)), window, cx);
            });
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        deliver_crash(&view, cx, root.path())?;
        let directory = block_launcher::session_host::report_directory(root.path(), CRASH_RUN)?;

        crate::test_support::press(cx, "escape");
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        cx.update(|window, app| {
            assert!(window.simulate_next_frame(app) > 0);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Drawer(Some(Drawer::Accounts)), window, cx);
            });
        });
        cx.run_until_parked();
        let reopened_focus = cx.update(|window, cx| window.focused(cx));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.update(|window, app| {
            assert!(window.simulate_next_frame(app) > 0);
        });
        cx.run_until_parked();
        assert_eq!(cx.update(|window, cx| window.focused(cx)), reopened_focus);
        assert!(!directory.join("seen.json").exists());
        assert!(block_launcher::session_host::claim_help(root.path(), CRASH_RUN)?.is_none());
        view.read_with(cx, |view, _| {
            assert!(matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::Drawer(Drawer::Accounts))
            ));
            assert_eq!(view.pending_crashes.len(), 1);
            assert!(view.crash_help.is_none());
        });

        crate::test_support::press(cx, "escape");
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.update(|window, app| {
            assert!(window.simulate_next_frame(app) > 0);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(!view.shell.surfaces.is_open());
            assert!(view.pending_crashes.is_empty());
            assert!(view.crash_help.is_some());
        });
        assert!(directory.join("seen.json").exists());
        assert!(crate::test_support::control_bounds(cx, "crash-return").is_some());
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn crash_keeps_focus_after_games_help_is_confirmed(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (view, cx) = fixture(cx);
        click(cx, "games-help");
        deliver_crash(&view, cx, root.path())?;

        crate::test_support::press(cx, "enter");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.crash_help.is_some()));
        let crash_focus = cx.update(|window, cx| window.focused(cx));
        assert!(crash_focus.is_some());
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(300));
        cx.run_until_parked();
        assert_eq!(
            cx.update(|window, cx| window.focused(cx)),
            crash_focus,
            "a completed dialog must not restore its old focus over crash help"
        );
        crate::test_support::press(cx, "enter");
        assert!(view.read_with(cx, |view, _| view.crash_help.is_none()));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn escape_closes_crash_help_without_an_overlay(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (view, cx) = fixture(cx);
        let window_id = gpui::VisualContext::window_handle(cx).window_id();
        deliver_crash(&view, cx, root.path())?;
        assert!(view.read_with(cx, |view, _| view.crash_help.is_some()));
        crate::test_support::press(cx, "escape");
        assert!(
            cx.windows()
                .iter()
                .all(|window| window.window_id() != window_id)
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn escape_closes_standalone_crash_help(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let crate::engine::Message::Crash {
            report,
            directory,
            guard: _guard,
        } = crash_message(root.path())?
        else {
            anyhow::bail!("the test message must contain a crash");
        };
        let (_, cx) = crate::test_support::window(cx, |window, cx| {
            crate::CrashView::new(*report, directory, |_, _| {}, window, cx)
        });
        let window_id = gpui::VisualContext::window_handle(cx).window_id();
        crate::test_support::press(cx, "escape");
        assert!(
            cx.windows()
                .iter()
                .all(|window| window.window_id() != window_id)
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn crash_export_picker_keeps_its_view_when_the_launcher_updates(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (view, cx) = fixture(cx);
        deliver_crash(&view, cx, root.path())?;
        let help = view.read_with(cx, |view, _| {
            view.crash_help.as_ref().map(Entity::entity_id)
        });
        assert!(help.is_some());
        crate::test_support::press(cx, "tab tab enter");
        assert!(cx.did_prompt_for_new_path());
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view
                .crash_help
                .as_ref()
                .map(Entity::entity_id)),
            help
        );
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view
                .crash_help
                .as_ref()
                .map(Entity::entity_id)),
            help
        );
        assert!(crate::test_support::control_bounds(cx, "crash-return").is_some());
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

fn crash_waits_for_component_overlay(cx: &mut TestAppContext, sheet: bool) -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let (view, cx) = fixture(cx);
    cx.update(|window, cx| {
        if sheet {
            window.open_sheet(cx, |sheet, _, _| sheet.title("Component sheet"));
        } else {
            window.open_dialog(cx, |dialog, _, _| dialog.title("Component dialog"));
        }
    });
    cx.run_until_parked();
    let overlay_focus = cx.update(|window, cx| window.focused(cx));
    deliver_crash(&view, cx, root.path())?;
    assert_eq!(cx.update(|window, cx| window.focused(cx)), overlay_focus);
    view.read_with(cx, |view, _| {
        assert_eq!(view.pending_crashes.len(), 1);
        assert!(view.crash_help.is_none());
    });
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert!(!window.has_active_sheet(cx));
    });
    view.read_with(cx, |view, _| {
        assert!(view.pending_crashes.is_empty());
        assert!(view.crash_help.is_some());
    });
    Ok(())
}

#[gpui::test]
fn crash_waits_for_a_component_dialog(cx: &mut TestAppContext) {
    let result = crash_waits_for_component_overlay(cx, false);
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn crash_waits_for_a_component_sheet(cx: &mut TestAppContext) {
    let result = crash_waits_for_component_overlay(cx, true);
    assert!(result.is_ok(), "{result:?}");
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
#[expect(
    clippy::expect_used,
    reason = "The real card is required to exercise the overlay entry."
)]
fn wizard_section_opens_as_dialog_and_escape_cancels_the_draft(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.begin_game_wizard(cx);
            if let Some(crate::Form::Instance(form)) = &view.form {
                form.name
                    .update(cx, |input, cx| input.set_value("Retained draft", cx));
            }
            if let Some(search) = &view.shell.wizard.version_search {
                search.update(cx, |input, cx| input.set_value("1.2", cx));
            }
            cx.notify();
            window.refresh();
        });
    });
    cx.run_until_parked();
    let (route, search_id, original_version) = view.read_with(cx, |view, cx| {
        let version = match &view.form {
            Some(crate::Form::Instance(form)) => form.version.read(cx).value(),
            _ => String::new(),
        };
        (
            view.page_spec().wizard,
            view.shell
                .wizard
                .version_search
                .as_ref()
                .map(Entity::entity_id),
            version,
        )
    });
    assert_eq!(
        route,
        Some(crate::navigation::WizardKey::Game(
            crate::game_wizard::WizardStep::Setup,
            None
        ))
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.shell
                .wizard
                .body_scroll
                .set_offset(point(px(0.), px(-24.)));
            cx.notify();
            window.refresh();
        });
    });
    cx.run_until_parked();
    let outer_offset = view.read_with(cx, |view, _| view.shell.wizard.body_scroll.offset());
    assert!(
        outer_offset.y < px(0.),
        "the setup viewport must be scrolled"
    );
    let card = crate::test_support::control_bounds(cx, "wizard-component-version")
        .expect("the Minecraft version card must be painted");
    cx.simulate_mouse_down(card.center(), MouseButton::Left, Modifiers::none());
    let trigger_focus = cx.update(|window, cx| window.focused(cx));
    cx.simulate_mouse_up(card.center(), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "the section stays inside the wizard");
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    cx.update(|window, _| {
        let snapshots = gpui_base::test_support::find_all(window, &[], &"wizard-snapshots".into());
        assert_eq!(
            snapshots.len(),
            1,
            "the version checkbox must have one owner: {snapshots:?}"
        );
        assert!(
            snapshots[0].path().contains(&"dialog".into()),
            "the sole version checkbox must be inside the dialog: {snapshots:?}"
        );
        let next = gpui_base::test_support::find_all(window, &[], &"wizard-next".into());
        assert_eq!(
            next.len(),
            1,
            "the outer footer must stay mounted: {next:?}"
        );
        assert_eq!(next[0].label(), Some("Continue"));
        assert!(!next[0].path().contains(&"dialog".into()));
    });
    assert!(crate::test_support::control_bounds(cx, "wizard-component-version").is_some());
    let outer_bounds = crate::test_support::control_bounds(cx, "wizard-body");
    let section_bounds = crate::test_support::control_bounds(cx, "wizard-section-body");
    view.read_with(cx, |view, _| {
        assert_eq!(view.page_spec().wizard, route);
        assert_eq!(view.shell.wizard.body_scroll.offset(), outer_offset);
        assert_eq!(Some(view.shell.wizard.body_scroll.bounds()), outer_bounds);
        assert_eq!(
            Some(view.shell.wizard.section_scroll.bounds()),
            section_bounds
        );
    });
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            assert!(matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::WizardSection(_))
            ));
            assert!(
                view.shell
                    .wizard
                    .version_search
                    .as_ref()
                    .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window)),
                "the section search is focused inside the dialog"
            );
            assert_eq!(
                view.shell
                    .wizard
                    .version_search
                    .as_ref()
                    .map(Entity::entity_id),
                search_id
            );
            assert!(
                view.shell
                    .wizard
                    .version_search
                    .as_ref()
                    .is_some_and(|input| input.read(cx).value() == "1.2")
            );
        });
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.shell
                .wizard
                .section_scroll
                .set_offset(point(px(0.), px(-30.)));
            if let Some(crate::Form::Instance(form)) = &view.form {
                form.version
                    .update(cx, |input, cx| input.set_value("1.21.1", cx));
            }
            cx.notify();
            window.refresh();
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(view.shell.wizard.section_scroll.offset().y < px(0.));
        assert_eq!(view.shell.wizard.body_scroll.offset(), outer_offset);
    });
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(!view.shell.surfaces.is_open());
        assert_eq!(view.page_spec().wizard, route);
        assert_eq!(view.shell.wizard.body_scroll.offset(), outer_offset);
        assert!(matches!(&view.form, Some(crate::Form::Instance(form))
            if form.name.read(cx).value() == "Retained draft"
                && form.version.read(cx).value() == original_version));
    });
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger_focus);
}

#[gpui::test]
fn wizard_section_keeps_its_own_page_beneath_a_nested_selector(cx: &mut TestAppContext) {
    use crate::game_wizard::WizardPage;

    let (view, cx) = fixture(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.begin_game_wizard(cx);
            if let Some(search) = &view.shell.wizard.loader_search {
                search.update(cx, |input, cx| input.set_value("retained search", cx));
            }
            cx.notify();
            window.refresh();
        });
    });
    cx.run_until_parked();
    let route = view.read_with(cx, |view, _| view.page_spec().wizard);

    click(cx, "wizard-component-loader");
    cx.update(|window, _| {
        let controls =
            gpui_base::test_support::find_all(window, &[], &"wizard-loader-vanilla".into());
        assert_eq!(controls.len(), 1, "the loader has one owner: {controls:?}");
        assert!(controls[0].path().contains(&"dialog".into()));
    });
    view.read_with(cx, |view, cx| {
        assert_eq!(view.page_spec().wizard, route);
        assert!(
            view.shell
                .wizard
                .loader_search
                .as_ref()
                .is_some_and(|search| search.read(cx).value() == "retained search")
        );
    });
    click(cx, "wizard-loader-neoforge");
    view.read_with(cx, |view, cx| {
        assert!(
            view.shell
                .wizard
                .loader_search
                .as_ref()
                .is_some_and(|search| search.read(cx).value().is_empty())
        );
        assert!(matches!(&view.form, Some(crate::Form::Instance(form)) if form.is_neoforge()));
    });
    click(cx, "section-cancel");
    view.read_with(cx, |view, _| {
        assert!(matches!(&view.form, Some(crate::Form::Instance(form)) if !form.is_neoforge()));
        assert_eq!(view.page_spec().wizard, route);
    });

    click(cx, "wizard-component-location");
    cx.update(|window, _| {
        let controls =
            gpui_base::test_support::find_all(window, &[], &"wizard-java-runtime-select".into());
        assert_eq!(
            controls.len(),
            1,
            "the runtime selector has one owner: {controls:?}"
        );
        assert!(controls[0].path().contains(&"dialog".into()));
    });
    click(cx, "wizard-java-runtime");
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::Select(_))
        ));
        assert!(
            view.shell.surfaces.layers().any(|layer| {
                matches!(layer.kind, Surface::WizardSection(WizardPage::Location))
            })
        );
        assert_eq!(view.page_spec().wizard, route);
    });
    cx.update(|window, _| {
        let controls =
            gpui_base::test_support::find_all(window, &[], &"wizard-java-runtime-select".into());
        assert_eq!(
            controls.len(),
            1,
            "a nested selector must not remount its owner: {controls:?}"
        );
        assert!(controls[0].path().contains(&"dialog".into()));
    });
    assert!(crate::test_support::control_bounds(cx, "wizard-component-version").is_some());

    // A section replacement updates the layer without changing the draft page.
    // The dialog must follow that layer even while another surface is on top.
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::WizardPage(Some(WizardPage::Version)), window, cx);
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.wizard.page, Some(WizardPage::Location));
        assert_eq!(view.page_spec().wizard, route);
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::Select(_))
        ));
    });
    cx.update(|window, _| {
        let controls = gpui_base::test_support::find_all(window, &[], &"wizard-snapshots".into());
        assert_eq!(
            controls.len(),
            1,
            "the layer's new page must be mounted: {controls:?}"
        );
        assert!(controls[0].path().contains(&"dialog".into()));
    });
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::WizardSection(WizardPage::Version))
        ));
    });
    click(cx, "section-cancel");
    view.read_with(cx, |view, _| {
        assert!(!view.shell.surfaces.is_open());
        assert_eq!(view.page_spec().wizard, route);
    });
}

#[gpui::test]
fn onboarding_configuration_owns_advanced_inputs_while_the_dialog_is_open(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.begin_game_wizard(cx);
            view.router.navigate(crate::Tab::Onboarding);
            view.router.enter_game_wizard();
            if let Some(profile) = &mut view.shell.wizard.profile {
                profile.options = true;
                profile
                    .memory
                    .update(cx, |input, cx| input.set_value("invalid", cx));
                profile
                    .jvm
                    .update(cx, |input, cx| input.set_value("-Xms1G", cx));
            }
            cx.notify();
            window.refresh();
        });
    });
    cx.run_until_parked();
    let memory_height = crate::test_support::control_bounds(cx, "wizard-field-Memory")
        .map(|bounds| bounds.size.height);
    let jvm_height = crate::test_support::control_bounds(cx, "wizard-field-Jvm")
        .map(|bounds| bounds.size.height);
    let outer_before = view.read_with(cx, |view, _| view.shell.wizard.body_scroll.max_offset().y);
    click(cx, "wizard-component-location");
    let outer_during = view.read_with(cx, |view, _| view.shell.wizard.body_scroll.max_offset().y);
    assert!(
        memory_height
            .zip(jvm_height)
            .is_some_and(|(memory, jvm)| { outer_before - outer_during >= memory + jvm }),
        "the open background disclosure must release both shared fields: before={outer_before:?}, during={outer_during:?}, memory={memory_height:?}, jvm={jvm_height:?}"
    );
    click(cx, "section-done");

    let memory = crate::test_support::control_bounds(cx, "wizard-field-Memory");
    let section = crate::test_support::control_bounds(cx, "wizard-section-body");
    assert!(
        memory.zip(section).is_some_and(|(memory, section)| {
            memory.top() >= section.top() && memory.bottom() <= section.bottom()
        }),
        "the invalid shared field must be revealed in the section: {memory:?}, {section:?}"
    );
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.wizard.body_scroll.offset().y, px(0.));
        assert!(view.shell.wizard.section_scroll.offset().y < px(0.));
    });
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            assert!(view.shell.wizard.profile.as_ref().is_some_and(|profile| {
                profile.options
                    && profile.memory.read(cx).value() == "invalid"
                    && profile.jvm.read(cx).value() == "-Xms1G"
                    && profile.memory.read(cx).focus_handle(cx).is_focused(window)
            }));
        });
    });
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(view.shell.wizard.profile.as_ref().is_some_and(|profile| {
            profile.options
                && profile.memory.read(cx).value() == "invalid"
                && profile.jvm.read(cx).value() == "-Xms1G"
        }));
    });
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
