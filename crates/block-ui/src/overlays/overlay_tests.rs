//! Overlay state rules: the component library owns presentation, while the
//! logical stack stays the single source of truth for open, close and order.
use crate::{
    Block, UiAction,
    shell::{Drawer, EditorExit, EditorKind, Surface},
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
        view
    });
    (view, cx)
}

const CRASH_RUN: &str = "00000000-0000-4000-8000-000000000001";

fn crash_message(root: &std::path::Path) -> anyhow::Result<crate::engine::Message> {
    let report = block_launcher::crash::CrashReport::new(
        block_launcher::crash::CrashContext {
            run_id: CRASH_RUN.into(),
            instance_id: "instance-a".into(),
            instance_name: "Test game".into(),
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

fn open_delete_instance(view: &Entity<Block>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::DeleteInstanceId("instance-a".into()), window, cx);
        });
    });
    cx.run_until_parked();
}

#[gpui::test]
#[expect(
    clippy::expect_used,
    reason = "Missing dialog geometry or a safe action is a test failure."
)]
fn compact_long_dialog_keeps_title_and_close_fixed_while_body_scrolls(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    cx.simulate_resize(size(px(360.), px(540.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(
                UiAction::DownloadFiles(crate::downloads::DownloadGroup::Active),
                window,
                cx,
            )
        });
    });
    cx.run_until_parked();
    let (content, insets) = cx.update(|window, _| {
        (
            crate::window_geometry::content_size(window),
            crate::window_geometry::content_insets(window),
        )
    });
    let dialog = crate::test_support::control_bounds(cx, "dialog-0").expect("dialog painted");
    let title =
        crate::test_support::control_bounds(cx, "surface-dialog-title").expect("title painted");
    let footer =
        crate::test_support::control_bounds(cx, "surface-dialog-footer").expect("footer painted");
    let scroll =
        crate::test_support::control_bounds(cx, "surface-dialog-scroll").expect("body painted");
    let body =
        crate::test_support::control_bounds(cx, "surface-dialog-body").expect("content painted");
    assert!(dialog.size.width <= content.width - px(32.));
    assert!(dialog.left() >= insets.left + px(16.));
    assert!(dialog.right() <= insets.left + content.width - px(16.));
    assert!(dialog.size.height <= content.height - px(80.));
    assert!(title.top() >= dialog.top() && title.bottom() <= scroll.top());
    assert!(scroll.bottom() <= footer.top() && footer.bottom() <= dialog.bottom());
    assert!(scroll.size.height < px(360.), "the long body must overflow");

    let position = scroll.origin + point(px(8.), px(8.));
    cx.simulate_mouse_move(position, None, Modifiers::none());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-1200.))),
        ..gpui::ScrollWheelEvent::default()
    });
    cx.run_until_parked();
    let scrolled =
        crate::test_support::control_bounds(cx, "surface-dialog-body").expect("content retained");
    assert!(scrolled.top() < body.top(), "the body must actually scroll");
    assert_eq!(
        crate::test_support::control_bounds(cx, "surface-dialog-title"),
        Some(title)
    );
    assert_eq!(
        crate::test_support::control_bounds(cx, "surface-dialog-footer"),
        Some(footer)
    );
    let close = crate::test_support::control_bounds(cx, "downloads-back").expect("Close painted");
    assert!(close.top() >= footer.top() && close.bottom() <= footer.bottom());
    click(cx, "downloads-back");
    view.read_with(cx, |view, _| assert!(!view.shell.surfaces.is_open()));
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
}

#[gpui::test]
#[expect(
    clippy::expect_used,
    reason = "Each editor exit must paint its actions and focus the safe one."
)]
fn editor_exit_actions_fit_compact_content_and_stay_restores_focus(cx: &mut TestAppContext) {
    for kind in [
        EditorKind::Instance,
        EditorKind::Instance,
        EditorKind::Provider,
    ] {
        let (view, cx) = fixture(cx);
        cx.simulate_resize(size(px(360.), px(720.)));
        cx.run_until_parked();
        let trigger = cx
            .update(|window, cx| window.focused(cx))
            .expect("trigger focused");
        let (entry, operation) = cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let entry = view.router.entry_id();
                view.shell.surfaces.open(
                    Surface::EditorExit(EditorExit {
                        entry,
                        kind,
                        exit: Box::new(UiAction::NavigationBack),
                    }),
                    window,
                    cx,
                );
                (entry, view.operation_revision)
            })
        });
        cx.run_until_parked();
        let content = cx.update(|window, _| crate::window_geometry::content_size(window));
        let dialog = crate::test_support::control_bounds(cx, "dialog-0").expect("dialog painted");
        assert!(dialog.size.width <= content.width - px(32.));
        assert!(dialog.size.height <= content.height - px(80.));
        let mut previous_bottom = dialog.top();
        for selector in [
            "editor-exit-save",
            "editor-exit-discard",
            "editor-exit-stay",
        ] {
            let action = crate::test_support::control_bounds(cx, selector).expect("action painted");
            assert!(action.left() >= dialog.left() && action.right() <= dialog.right());
            assert!(action.top() >= previous_bottom && action.bottom() <= dialog.bottom());
            previous_bottom = action.bottom();
        }
        cx.update(|window, _| {
            let stay = gpui_base::test_support::find(window, &[], &"editor-exit-stay".into())
                .expect("safe action has a native node");
            assert_eq!(stay.focused(), Some(true));
        });

        click(cx, "editor-exit-stay");
        view.read_with(cx, |view, _| {
            assert!(!view.shell.surfaces.is_open());
            assert_eq!(view.router.entry_id(), entry);
            assert_eq!(view.operation_revision, operation);
        });
        cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
        assert_eq!(cx.update(|window, cx| window.focused(cx)), Some(trigger));
    }
}

#[gpui::test]
fn late_editor_exit_dismissal_cannot_close_a_covering_dialog(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    cx.simulate_resize(size(px(360.), px(720.)));
    cx.run_until_parked();
    let trigger = cx.update(|window, cx| window.focused(cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.shell.surfaces.open(
                Surface::EditorExit(EditorExit {
                    entry: view.router.entry_id(),
                    kind: EditorKind::Instance,
                    exit: Box::new(UiAction::NavigationBack),
                }),
                window,
                cx,
            );
        });
    });
    cx.run_until_parked();
    let safe_focus = cx.update(|window, cx| window.focused(cx));
    let revision = view.read_with(cx, |view, _| {
        view.shell.surfaces.top().map(|layer| layer.revision)
    });
    open_delete_instance(&view, cx);

    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(revision) = revision {
                view.dismiss_surface_overlay(revision, window, cx);
            }
        });
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.surfaces.layers().count(), 2);
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::DeleteInstance(_))
        ));
    });
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.surfaces.layers().count(), 1);
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::EditorExit(_))
        ));
    });
    assert_eq!(cx.update(|window, cx| window.focused(cx)), safe_focus);
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| assert!(!view.shell.surfaces.is_open()));
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger);
}

#[gpui::test]
fn delete_instance_confirmation_renders_in_window_and_restores_focus(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let trigger_focus = cx.update(|window, cx| window.focused(cx));
    assert!(trigger_focus.is_some());
    open_delete_instance(&view, cx);
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
            Surface::DeleteInstance(id) if id == "instance-a"
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
    open_delete_instance(&view, cx);
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
            Some(Surface::DeleteInstance(_))
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
    open_delete_instance(&view, cx);
    click(cx, "confirm-delete");
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.surfaces.layers().count(), 0);
        assert_eq!(view.snapshot.instances.len(), 1);
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
        open_delete_instance(&view, cx);
        let revision = view.read_with(cx, |view, _| view.operation_revision);
        crate::test_support::press(cx, "enter");
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(!view.shell.surfaces.is_open());
            assert_eq!(view.snapshot.instances.len(), 1);
            assert_eq!(view.snapshot.instances[0].id, "instance-a");
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
    open_delete_instance(&view, cx);
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
                Some(Surface::DeleteInstance(_))
            ));
            assert_eq!(view.snapshot.instances.len(), 1);
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
        assert_eq!(view.snapshot.instances.len(), 1);
    });
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
}

#[gpui::test]
fn a_late_dismissal_never_closes_a_newer_overlay(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    open_delete_instance(&view, cx);
    let stale = view.read_with(cx, |view, _| {
        view.shell
            .surfaces
            .top()
            .filter(|layer| matches!(layer.kind, Surface::DeleteInstance(_)))
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
    let menu = crate::test_support::control_bounds(cx, "navigation-back");
    assert!(
        menu.is_some(),
        "the page Back control must paint behind the overlay"
    );
    let Some(menu) = menu else {
        return;
    };
    open_delete_instance(&view, cx);
    cx.simulate_click(menu.origin + point(px(6.), px(6.)), Modifiers::none());
    crate::test_support::ensure_live_window(cx);
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.router.tab(), crate::Tab::Instances);
        assert_eq!(
            view.shell.surfaces.layers().count(),
            1,
            "a background control must not receive the click"
        );
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::DeleteInstance(_))
        ));
    });
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
}

#[gpui::test]
fn home_instance_sheet_opens_in_window_and_escape_restores_focus(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    cx.simulate_resize(size(px(432.), px(768.)));
    cx.run_until_parked();
    let trigger_focus = cx.update(|window, cx| window.focused(cx));
    assert!(trigger_focus.is_some());
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Drawer(Some(Drawer::Instances)), window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "the sheet stays inside Home");
    cx.update(|window, cx| assert!(crate::drawer::DrawerLayer::is_open(window, cx)));
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::Drawer(Drawer::Instances))
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
            view.handle(UiAction::Drawer(Some(Drawer::Instances)), window, cx);
            view.handle(UiAction::DeleteInstanceId("instance-a".into()), window, cx);
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::DeleteInstance(_))
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
            Some(Surface::Drawer(Drawer::Instances))
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
    cx.simulate_resize(size(px(360.), px(720.)));
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
    let viewport = crate::test_support::control_bounds(cx, "surface-dialog-scroll");
    let body = crate::test_support::control_bounds(cx, "surface-dialog-body");
    assert!(
        body.zip(viewport).is_some_and(|(body, viewport)| {
            body.top() >= viewport.top() && body.bottom() <= viewport.bottom()
        }),
        "the concise Instance help stays fully visible inside its dialog"
    );
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
fn dialog_close_uses_the_view_language_and_the_existing_cancel_action(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    view.update(cx, |view, cx| {
        view.preferences.language = Some(crate::Language::Japanese);
        cx.notify();
    });
    click(cx, "games-help");
    let label = cx.update(|window, _| {
        gpui_base::test_support::find(window, &[], &"close".into())
            .and_then(|node| node.label().map(str::to_owned))
    });
    assert_eq!(label.as_deref(), Some("閉じる"));
    click(cx, "surface-dialog-close");
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    assert!(!view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
}

#[gpui::test]
fn crash_waits_for_games_help_before_taking_focus_and_marking_seen(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _messages) = crate::Engine::with_root(root.path().to_path_buf())?;
        let (view, cx) = fixture(cx);
        view.update(cx, |view, _| {
            let mut other = view.snapshot.instances[0].clone();
            other.id = "instance-b".into();
            view.snapshot.instances.push(other);
            view.selected_instance = Some("instance-b".into());
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
        assert!(crate::test_support::control_bounds(cx, "surface-dialog-body").is_some());
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
            assert_eq!(view.selected_instance.as_deref(), Some("instance-a"));
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
        let dialog = crate::test_support::control_bounds(cx, "dialog-0")
            .ok_or_else(|| anyhow::anyhow!("missing help dialog"))?;
        cx.simulate_click(
            point(dialog.left() - px(4.), dialog.center().y),
            Modifiers::none(),
        );
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
                view.handle(UiAction::Drawer(Some(Drawer::Instances)), window, cx);
            });
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::DeleteInstanceId("instance-a".into()), window, cx);
            });
        });
        cx.run_until_parked();
        deliver_crash(&view, cx, root.path())?;

        crate::test_support::press(cx, "enter");
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::Drawer(Drawer::Instances))
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
                view.handle(UiAction::Drawer(Some(Drawer::Instances)), window, cx);
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
            crate::CrashView::new(
                *report,
                directory,
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
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
        let export = crate::test_support::control_bounds(cx, "crash-export");
        assert!(export.is_some());
        if let Some(export) = export {
            cx.simulate_click(export.center(), Modifiers::none());
        }
        cx.run_until_parked();
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
fn wizard_version_page_owns_its_input_and_escape_restores_the_draft(cx: &mut TestAppContext) {
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
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    cx.update(|window, _| {
        let snapshots = gpui_base::test_support::find_all(window, &[], &"wizard-snapshots".into());
        assert_eq!(
            snapshots.len(),
            1,
            "the version checkbox must have one owner: {snapshots:?}"
        );
        assert!(!snapshots[0].path().contains(&"dialog".into()));
        let next = gpui_base::test_support::find_all(window, &[], &"wizard-next".into());
        assert_eq!(next.len(), 1, "the page has one footer: {next:?}");
        assert_eq!(next[0].label(), Some("Apply to draft"));
        assert!(!next[0].path().contains(&"dialog".into()));
    });
    assert!(crate::test_support::control_bounds(cx, "wizard-component-version").is_none());
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.page_spec().wizard,
            Some(crate::navigation::WizardKey::Game(
                crate::game_wizard::WizardStep::Setup,
                Some(crate::game_wizard::WizardPage::Version)
            ))
        );
    });
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            assert!(!view.shell.surfaces.is_open());
            assert!(
                view.shell
                    .wizard
                    .version_search
                    .as_ref()
                    .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window)),
                "the new page focuses the retained search"
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
            if let Some(crate::Form::Instance(form)) = &view.form {
                form.version
                    .update(cx, |input, cx| input.set_value("1.21.1", cx));
            }
            cx.notify();
            window.refresh();
        });
    });
    cx.run_until_parked();
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(!view.shell.surfaces.is_open());
        assert_eq!(view.page_spec().wizard, route);
        assert!(matches!(&view.form, Some(crate::Form::Instance(form))
            if form.name.read(cx).value() == "Retained draft"
                && form.version.read(cx).value() == original_version));
    });
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger_focus);
}

#[gpui::test]
fn wizard_component_page_keeps_its_draft_beneath_a_nested_selector(cx: &mut TestAppContext) {
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
        assert!(!controls[0].path().contains(&"dialog".into()));
    });
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.page_spec().wizard,
            Some(crate::navigation::WizardKey::Game(
                crate::game_wizard::WizardStep::Setup,
                Some(WizardPage::Loader)
            ))
        );
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
    click(cx, "cancel-instance");
    view.read_with(cx, |view, _| {
        assert!(matches!(&view.form, Some(crate::Form::Instance(form)) if !form.is_neoforge()));
        assert_eq!(view.page_spec().wizard, route);
    });

    let body = view.read_with(cx, |view, _| view.shell.wizard.body_scroll.bounds());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: body.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-1200.))),
        ..gpui::ScrollWheelEvent::default()
    });
    cx.run_until_parked();
    assert!(
        crate::test_support::control_bounds(cx, "wizard-component-location")
            .is_some_and(|card| card.top() >= body.top() && card.bottom() <= body.bottom()),
        "the larger location card must remain reachable by scrolling"
    );
    click(cx, "wizard-component-location");
    cx.update(|window, _| {
        let controls =
            gpui_base::test_support::find_all(window, &[], &"wizard-java-runtime-select".into());
        assert_eq!(
            controls.len(),
            1,
            "the runtime selector has one owner: {controls:?}"
        );
        assert!(!controls[0].path().contains(&"dialog".into()));
    });
    click(cx, "wizard-java-runtime");
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::Select(_))
        ));
        assert_eq!(
            view.page_spec().wizard,
            Some(crate::navigation::WizardKey::Game(
                crate::game_wizard::WizardStep::Setup,
                Some(WizardPage::Location)
            ))
        );
    });
    cx.update(|window, _| {
        let controls =
            gpui_base::test_support::find_all(window, &[], &"wizard-java-runtime-select".into());
        assert_eq!(
            controls.len(),
            1,
            "a nested selector must not remount its owner: {controls:?}"
        );
        assert!(!controls[0].path().contains(&"dialog".into()));
    });
    assert!(crate::test_support::control_bounds(cx, "wizard-component-version").is_none());
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(!view.shell.surfaces.is_open());
        assert_eq!(view.shell.wizard.page, Some(WizardPage::Location));
    });
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(!view.shell.surfaces.is_open());
        assert_eq!(view.page_spec().wizard, route);
    });
}

#[gpui::test]
fn onboarding_configuration_owns_one_live_advanced_input(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.begin_game_wizard(cx);
            view.router.navigate(crate::Tab::Onboarding);
            view.router.enter_game_wizard();
            if let Some(instance) = &mut view.shell.wizard.draft {
                instance.options = true;
                instance
                    .memory
                    .update(cx, |input, cx| input.set_value("invalid", cx));
                instance
                    .jvm
                    .update(cx, |input, cx| input.set_value("-Xms1G", cx));
            }
            cx.notify();
            window.refresh();
        });
    });
    cx.run_until_parked();
    click(cx, "wizard-component-location");
    assert!(crate::test_support::control_bounds(cx, "wizard-component-location").is_none());
    assert!(crate::test_support::control_bounds(cx, "wizard-field-Memory").is_some());
    assert!(crate::test_support::control_bounds(cx, "wizard-field-Jvm").is_some());
    click(cx, "wizard-next");

    let memory = crate::test_support::control_bounds(cx, "wizard-field-Memory");
    let section = crate::test_support::control_bounds(cx, "wizard-body");
    assert!(
        memory.zip(section).is_some_and(|(memory, section)| {
            memory.top() >= section.top() && memory.bottom() <= section.bottom()
        }),
        "the invalid shared field must be revealed in the page: {memory:?}, {section:?}"
    );
    view.read_with(cx, |view, _| {
        assert!(view.shell.wizard.body_scroll.offset().y < px(0.));
    });
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            assert!(view.shell.wizard.draft.as_ref().is_some_and(|instance| {
                instance.options
                    && instance.memory.read(cx).value() == "invalid"
                    && instance.jvm.read(cx).value() == "-Xms1G"
                    && instance.memory.read(cx).focus_handle(cx).is_focused(window)
            }));
        });
    });
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert!(view.shell.wizard.draft.as_ref().is_some_and(|instance| {
            instance.options
                && instance.memory.read(cx).value() == "invalid"
                && instance.jvm.read(cx).value() == "-Xms1G"
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

#[gpui::test]
fn compact_wizard_sections_and_provider_dialog_actions_stay_reachable(cx: &mut TestAppContext) {
    for language in crate::Language::ALL {
        let (view, cx) = fixture(cx);
        cx.simulate_resize(size(px(360.), px(540.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.language = Some(language);
                view.preferences.reduce_motion = true;
                view.begin_game_wizard(cx);
                view.handle(
                    UiAction::WizardPage(Some(crate::WizardPage::OptiFine)),
                    window,
                    cx,
                );
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
        for selector in ["cancel-instance", "wizard-back", "wizard-next"] {
            let action = crate::test_support::control_bounds(cx, selector);
            assert!(
                action.is_some_and(|action| action.left() >= px(0.)
                    && action.right() <= px(360.)
                    && action.bottom() <= px(540.)),
                "{language:?}/{selector}: {action:?}"
            );
        }
        crate::test_support::press(cx, "escape");
        cx.run_until_parked();
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::CancelForm, window, cx);
                view.handle(UiAction::SignIn, window, cx);
                view.choose_auth_provider(block_auth::AccountProvider::Authlib, cx);
                view.handle(UiAction::AuthProviderAdd, window, cx);
            });
        });
        cx.run_until_parked();
        let viewport = crate::test_support::control_bounds(cx, "auth-provider-scroll");
        assert!(
            viewport.is_some(),
            "{language:?}: provider form must scroll"
        );
        let Some(viewport) = viewport else {
            return;
        };
        assert!(viewport.left() >= px(0.) && viewport.right() <= px(360.));
        assert!(viewport.bottom() <= px(540.));
        let position = viewport.center();
        cx.simulate_mouse_move(position, None, Modifiers::none());
        cx.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-1200.))),
            ..gpui::ScrollWheelEvent::default()
        });
        cx.run_until_parked();
        let save = crate::test_support::control_bounds(cx, "auth-provider-save");
        assert!(
            save.is_some_and(
                |save| save.top() >= viewport.top() && save.bottom() <= viewport.bottom()
            ),
            "{language:?}: provider Save must be reachable: save={save:?}, viewport={viewport:?}"
        );
    }
}

/// A sheet header, a confirmation title and its notes are product text: each
/// paints in the view's own language while the logical surface stays the same.
#[gpui::test]
fn surface_names_resolve_in_the_view_language(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.snapshot = crate::test_support::ready_snapshot();
        view
    });
    for (language, sheet_title, sheet_description, instance_title, provider_title) in [
        (
            crate::Language::English,
            "Switch Account",
            "Choose the account you want to use.",
            "Delete Test game?",
            "Remove this provider?",
        ),
        (
            crate::Language::SimplifiedChinese,
            "切换账户",
            "选择要使用的账户。",
            "删除 Test game？",
            "移除此提供者？",
        ),
        (
            crate::Language::Japanese,
            "アカウントを切り替え",
            "使用するアカウントを選択してください。",
            "Test game を削除しますか？",
            "このプロバイダーを削除しますか？",
        ),
    ] {
        view.update(cx, |view, _| view.preferences.language = Some(language));
        let (chrome, named, unnamed, provider) = view.read_with(cx, |view, _| {
            (
                view.surface_sheet_chrome(&Surface::Drawer(Drawer::Accounts)),
                view.surface_dialog_title(&Surface::DeleteInstance("instance-a".into())),
                view.surface_dialog_title(&Surface::DeleteInstance("missing".into())),
                view.surface_dialog_title(&Surface::RemoveProvider("missing".into())),
            )
        });
        assert_eq!(chrome.0, sheet_title, "{}", language.code());
        assert_eq!(chrome.1, sheet_description, "{}", language.code());
        assert_eq!(named, instance_title, "{}", language.code());
        assert_eq!(provider, provider_title, "{}", language.code());
        assert!(
            !unnamed.contains("Test game"),
            "{} must not name an absent instance: {unnamed}",
            language.code()
        );
    }
}
