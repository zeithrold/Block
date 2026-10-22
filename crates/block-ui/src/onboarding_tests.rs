//! Setup keeps component and account choices local until the real save boundary.

use super::{Progress, Step};
use crate::{
    Form, Tab, UiAction, game_wizard::WizardPage, shell::Surface, test_support::test_account,
};
use gpui::{EntityInputHandler as _, Focusable as _, TestAppContext, VisualTestContext, px, size};

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = crate::test_support::control_bounds(cx, selector);
    assert!(bounds.is_some(), "missing {selector}");
    if let Some(bounds) = bounds {
        let window_count = cx.windows().len();
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        if cx.windows().len() > window_count
            && let Some(window) = cx.windows().last()
        {
            *cx = VisualTestContext::from_window(*window, &cx.cx);
        }
        crate::test_support::ensure_live_window(cx);
        cx.run_until_parked();
        crate::test_support::ensure_live_window(cx);
    }
}

#[track_caller]
fn required_bounds(
    cx: &mut VisualTestContext,
    selector: &'static str,
) -> gpui::Bounds<gpui::Pixels> {
    let bounds = crate::test_support::control_bounds(cx, selector);
    assert!(bounds.is_some(), "missing rendered control: {selector}");
    bounds.unwrap_or_default()
}

#[gpui::test]
fn resizing_setup_keeps_component_rows_square_and_the_note_next_to_them(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.begin_onboarding(cx);
        });
    });
    for (width, height) in [
        (960., 600.),
        (360., 648.),
        (963., 688.),
        (360., 540.),
        (960., 600.),
    ] {
        cx.simulate_resize(size(px(width), px(height)));
        cx.run_until_parked();
        let grid = required_bounds(cx, "onboarding-components");
        let note = required_bounds(cx, "onboarding-draft-note");
        let tile = required_bounds(cx, "wizard-component-version");
        let scroll = view.read_with(cx, |view, _| view.shell.onboarding.scroll.clone());
        assert!(
            scroll.offset().y >= -scroll.max_offset().y,
            "resizing must clamp the previous scroll offset before new input"
        );
        assert!(
            (tile.size.height - tile.size.width).abs() <= px(1.),
            "component tile must stay square"
        );
        let rows = if width < 640. { 2. } else { 1. };
        let expected_height = tile.size.height * rows + px(12. * (rows - 1.));
        assert!(
            (grid.size.height - expected_height).abs() <= px(1.),
            "grid must reserve only its actual component rows: grid={grid:?}, tile={tile:?}"
        );
        assert!(
            (note.top() - grid.bottom() - px(24.)).abs() <= px(1.),
            "draft note must follow grid without blank stretched rows: grid={grid:?}, note={note:?}"
        );
        scroll.set_offset(gpui::point(px(0.), px(-100_000.)));
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        let viewport = required_bounds(cx, "onboarding-body");
        let note = required_bounds(cx, "onboarding-draft-note");
        let footer = required_bounds(cx, "onboarding-footer");
        assert!(
            note.bottom() <= viewport.bottom(),
            "the real last content must be reachable within the viewport"
        );
        assert!(
            note.top() >= viewport.top(),
            "the complete last note must be visible at the end of scrolling"
        );
        assert!(
            viewport.bottom() <= footer.top(),
            "the fixed footer must remain outside the scroll viewport"
        );
        assert!(
            scroll.offset().y >= -scroll.max_offset().y,
            "resizing must clamp the old scroll offset"
        );
    }
}

#[gpui::test]
fn tab_during_configuration_entrance_reaches_the_live_modal_controls(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.begin_onboarding(cx);
            view.open_onboarding_section(WizardPage::Version, window, cx);
        });
    });
    crate::test_support::press(cx, "tab");
    cx.update(|window, cx| {
        let view = view.read(cx);
        assert!(
            view.shell
                .surfaces
                .top()
                .is_some_and(|layer| layer.focus.contains_focused(window, cx))
        );
        assert!(
            view.shell
                .wizard
                .version_search
                .as_ref()
                .is_some_and(|input| !input.read(cx).focus_handle(cx).is_focused(window))
        );
    });
}

#[gpui::test]
fn component_dialog_cancel_rolls_back_all_values_and_done_keeps_only_the_draft(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.begin_onboarding(cx);
            view.open_onboarding_section(WizardPage::Location, window, cx);
            if let Some(profile) = &mut view.shell.wizard.profile {
                profile
                    .width
                    .update(cx, |field, cx| field.set_value("1920", cx));
                profile
                    .memory
                    .update(cx, |field, cx| field.set_value("4096", cx));
                profile.fullscreen = true;
            }
            view.finish_onboarding_section(true, window, cx);
            assert!(view.shell.surfaces.top().is_none());
            assert!(view.shell.wizard.profile.as_ref().is_some_and(|profile| {
                profile.width.read(cx).value() == "1280"
                    && profile.memory.read(cx).value() == "2048"
                    && !profile.fullscreen
            }));
            view.open_onboarding_section(WizardPage::Location, window, cx);
            if let Some(profile) = &view.shell.wizard.profile {
                profile
                    .width
                    .update(cx, |field, cx| field.set_value("1600", cx));
            }
            view.finish_onboarding_section(false, window, cx);
            assert!(
                view.shell
                    .wizard
                    .profile
                    .as_ref()
                    .is_some_and(|profile| profile.width.read(cx).value() == "1600")
            );
            assert_eq!(view.onboarding_progress, Progress::Draft);
            assert!(view.snapshot.instances.is_empty());
            assert_eq!(view.shell.onboarding.step, Step::Game);
        });
    });
}

#[gpui::test]
fn configuration_error_focuses_and_reveals_the_real_field_without_losing_the_draft(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(360.), px(540.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.begin_onboarding(cx);
            view.open_onboarding_section(WizardPage::Location, window, cx);
            if let Some(profile) = &view.shell.wizard.profile {
                profile
                    .width
                    .update(cx, |field, cx| field.set_value("bad", cx));
            }
            view.finish_onboarding_section(false, window, cx);
            assert!(
                view.shell
                    .wizard
                    .profile
                    .as_ref()
                    .is_some_and(|profile| profile
                        .width
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window))
            );
            assert!(matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::OnboardingSection(WizardPage::Location))
            ));
            assert!(!view.busy);
        });
    });
    cx.run_until_parked();
    let field = crate::test_support::control_bounds(cx, "wizard-field-Width");
    let footer = crate::test_support::control_bounds(cx, "onboarding-section-footer");
    assert!(
        field
            .zip(footer)
            .is_some_and(|(field, footer)| field.top() >= px(0.)
                && field.bottom() <= footer.top()
                && footer.bottom() <= px(540.)),
        "field {field:?}; footer {footer:?}"
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(profile) = &view.shell.wizard.profile {
                profile
                    .width
                    .update(cx, |field, cx| field.set_value("1600", cx));
            }
            view.finish_onboarding_section(false, window, cx);
            assert!(view.shell.surfaces.top().is_none());
            assert!(
                view.shell
                    .wizard
                    .profile
                    .as_ref()
                    .is_some_and(|profile| profile.width.read(cx).value() == "1600")
            );
        });
    });
}

#[gpui::test]
fn account_selection_is_staged_until_setup_preference_save_and_failure_can_retry(
    cx: &mut TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let mut other = test_account()?;
        other.reference.player_id = "00000000-0000-4000-8000-000000000002".parse()?;
        other.player_name = "Other saved account".into();
        let first = test_account()?;
        let first_id = crate::account_id(&first);
        let other_id = crate::account_id(&other);
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.accounts = vec![first, other];
                view.selected_account = Some(first_id.clone());
                view.begin_onboarding(cx);
                view.change_onboarding_step(Step::Account, window, cx);
                view.choose_onboarding_account(other_id.clone());
                assert_eq!(view.selected_account.as_ref(), Some(&first_id));
                assert_eq!(view.shell.onboarding.account.as_ref(), Some(&other_id));
            });
        });
        std::fs::create_dir(root.path().join("preferences.json"))?;
        view.update(cx, |view, _| {
            assert!(!view.complete_onboarding());
            assert_eq!(view.selected_account.as_ref(), Some(&first_id));
        });
        std::fs::remove_dir(root.path().join("preferences.json"))?;
        view.update(cx, |view, _| {
            assert!(view.complete_onboarding());
            assert_eq!(view.selected_account.as_ref(), Some(&other_id));
            assert_eq!(view.tab, Tab::Home);
            assert_eq!(view.accounts.len(), 2);
        });
        assert!(crate::Preferences::load(root.path())?.onboarding_complete);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn review_change_keeps_three_steps_and_short_footer_and_invalid_name_focus(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(360.), px(540.)));
    cx.update(|window, cx| view.update(cx, |view, cx| {
        view.preferences.reduce_motion = true;
        view.begin_onboarding(cx);
        if let Some(Form::Instance(form)) = &view.form { form.name.update(cx, |field, cx| field.set_value("", cx)); }
        view.advance_onboarding(window, cx);
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.name.read(cx).focus_handle(cx).is_focused(window)));
        assert_eq!(view.shell.onboarding.step, Step::Game);
        assert!(!view.busy);
        if let Some(Form::Instance(form)) = &view.form { form.name.update(cx, |field, cx| field.set_value("Kept draft", cx)); }
        view.advance_onboarding(window, cx);
        assert_eq!(view.shell.onboarding.step, Step::Account);
        view.advance_onboarding(window, cx);
        assert_eq!(view.shell.onboarding.step, Step::Review);
    }));
    cx.run_until_parked();
    assert!(
        crate::test_support::control_bounds(cx, "onboarding-footer").is_some_and(|bounds| bounds
            .size
            .height
            == px(56.)
            && bounds.bottom() <= px(540.))
    );
    click(cx, "onboarding-review-change-0");
    assert!(view.read_with(cx, |view, cx| view.shell.onboarding.step == Step::Game && matches!(&view.form, Some(Form::Instance(form)) if form.name.read(cx).value() == "Kept draft")));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::OnboardingChange(Step::Review), window, cx);
            view.handle(UiAction::OnboardingSection(WizardPage::Version), window, cx);
            view.handle(UiAction::Drawer(None), window, cx);
            assert!(view.shell.surfaces.top().is_none());
            assert!(view.shell.wizard.page.is_none());
            assert_eq!(view.shell.onboarding.step, Step::Review);
        });
    });
}

#[gpui::test]
fn create_uses_the_reviewed_configuration_and_setup_save_retry_does_not_create_twice(
    cx: &mut TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let catalog = block_launcher::catalog::Catalog::open(root.path())?;
        let (engine, mut receiver) = crate::engine::Engine::with_root(root.path().to_owned())?;
        // An owned directory deliberately blocks preference reads and writes.
        // Engine still creates the real catalog, then fails before network setup.
        std::fs::create_dir(root.path().join("preferences.json"))?;
        let java = std::env::current_exe()?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.begin_onboarding(cx);
                if let Some(Form::Instance(form)) = &view.form {
                    form.name
                        .update(cx, |field, cx| field.set_value("Reviewed game", cx));
                }
                if let Some(profile) = &mut view.shell.wizard.profile {
                    profile.managed = false;
                    profile.java.update(cx, |field, cx| {
                        field.set_value(&java.display().to_string(), cx);
                    });
                    profile
                        .memory
                        .update(cx, |field, cx| field.set_value("4096", cx));
                    profile
                        .jvm
                        .update(cx, |field, cx| field.set_value("-Dkept=true", cx));
                    profile
                        .width
                        .update(cx, |field, cx| field.set_value("1600", cx));
                    profile
                        .height
                        .update(cx, |field, cx| field.set_value("900", cx));
                    profile.fullscreen = true;
                }
                view.change_onboarding_step(Step::Review, window, cx);
                view.handle(UiAction::OnboardingNext, window, cx);
                assert!(view.busy);
                view.handle(UiAction::OnboardingNext, window, cx);
            });
        });
        let runtime = tokio::runtime::Runtime::new()?;
        let messages = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let mut messages = Vec::new();
                loop {
                    let message = receiver.recv().await.ok_or_else(|| anyhow::anyhow!("setup engine closed before completion"))?;
                    let inner = match &message { crate::Message::ForOperation { message, .. } => message.as_ref(), message => message };
                    let finished = matches!(inner, crate::Message::Finished(_));
                    if matches!(inner, crate::Message::Created { .. }) {
                        let snapshot = catalog.snapshot()?;
                        assert_eq!(snapshot.instances.len(), 1);
                        assert_eq!(snapshot.profiles.len(), 1);
                        let saved = &snapshot.profiles[0];
                        assert_eq!(saved.name, "Reviewed game");
                        assert_eq!(saved.memory_mib, 4096);
                        assert!(matches!(&saved.runtime, block_launcher::recipe::RuntimeSelection::Local { java: saved } if saved == &java));
                        assert_eq!(saved.jvm_args, ["-Dkept=true"]);
                        assert_eq!(saved.game_args, ["--width", "1600", "--height", "900", "--fullscreen"]);
                        assert!(saved.inheritance.runtime && saved.inheritance.window);
                    }
                    messages.push(message);
                    if finished { return Ok::<_, anyhow::Error>(messages); }
                }
            }).await?
        })?;
        let (sender, saved_receiver) = tokio::sync::mpsc::channel(messages.len().max(1));
        for message in messages {
            sender.try_send(message)?;
        }
        view.update(cx, |view, cx| {
            view.messages = Some(saved_receiver);
            view.poll(cx);
            assert_eq!(view.tab, Tab::Onboarding);
            assert_eq!(view.onboarding_progress, Progress::Created);
            assert!(view.form.is_some());
            assert!(!view.busy);
        });
        std::fs::remove_dir(root.path().join("preferences.json"))?;
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::OnboardingNext, window, cx);
            });
        });
        assert_eq!(view.read_with(cx, |view, _| view.tab), Tab::Home);
        assert_eq!(catalog.snapshot()?.instances.len(), 1);
        assert!(crate::Preferences::load(root.path())?.onboarding_complete);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn revisiting_from_the_real_menu_starts_a_fresh_draft_and_keeps_saved_games(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(432.), px(768.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.preferences.onboarding_complete = true;
            view.saved_preferences = view.preferences.clone();
            view.snapshot = crate::test_support::ready_snapshot();
            view.tab = Tab::Home;
            view.handle(
                UiAction::Drawer(Some(crate::shell::Drawer::Menu)),
                window,
                cx,
            );
        });
    });
    click(cx, "menu-welcome");
    assert!(view.read_with(cx, |view, _| view.tab == Tab::Onboarding
        && view.shell.onboarding.step == Step::Game
        && view.snapshot.instances.len() == 1
        && view.saved_preferences.onboarding_complete));
    cx.update(|window, cx| view.update(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &view.form { form.name.update(cx, |field, cx| field.set_value("Discard on revisit", cx)); }
        view.handle(UiAction::Tab(Tab::Home), window, cx);
        view.handle(UiAction::Tab(Tab::Onboarding), window, cx);
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.name.read(cx).value() == "My Minecraft"));
        assert_eq!(view.snapshot.profiles.len(), 1);
        assert!(view.saved_preferences.onboarding_complete);
    }));
}

#[gpui::test]
fn unfinished_chinese_composition_cannot_commit_a_component_or_the_setup(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.begin_onboarding(cx);
            view.open_onboarding_section(WizardPage::Location, window, cx);
            if let Some(profile) = &view.shell.wizard.profile {
                profile.jvm.update(cx, |field, cx| {
                    field.replace_and_mark_text_in_range(None, "中", None, window, cx);
                });
            }
            view.handle(UiAction::OnboardingSectionDone, window, cx);
            assert!(view.shell.wizard.page.is_some());
            assert!(view.shell.wizard.error.is_none());
            view.finish_onboarding_section(true, window, cx);
            if let Some(Form::Instance(form)) = &view.form {
                form.name.update(cx, |field, cx| {
                    field.replace_and_mark_text_in_range(None, "文", None, window, cx);
                });
            }
            view.handle(UiAction::OnboardingNext, window, cx);
            assert_eq!(view.shell.onboarding.step, Step::Game);
            assert!(!view.busy);
        });
    });
}

#[gpui::test]
fn real_game_component_tiles_open_the_context_dialog_and_escape_rolls_back(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(960.), px(600.)));
    view.update(cx, |view, cx| {
        view.preferences.reduce_motion = true;
        view.begin_onboarding(cx);
        cx.notify();
    });
    click(cx, "wizard-component-location");
    assert!(view.read_with(cx, |view, _| matches!(
        view.shell.surfaces.top().map(|layer| &layer.kind),
        Some(Surface::OnboardingSection(WizardPage::Location))
    )));
    view.update(cx, |view, cx| {
        if let Some(profile) = &view.shell.wizard.profile {
            profile
                .width
                .update(cx, |field, cx| field.set_value("1600", cx));
        }
    });
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, cx| {
        view.shell.surfaces.top().is_none()
            && view.shell.wizard.page.is_none()
            && view
                .shell
                .wizard
                .profile
                .as_ref()
                .is_some_and(|profile| profile.width.read(cx).value() == "1280")
    }));
    click(cx, "wizard-component-version");
    assert!(view.read_with(cx, |view, _| matches!(
        view.shell.surfaces.top().map(|layer| &layer.kind),
        Some(Surface::OnboardingSection(WizardPage::Version))
    )));
}

#[gpui::test]
fn late_created_from_another_draft_updates_the_library_without_completing_a_new_guide(
    cx: &mut TestAppContext,
) {
    for previous_guide in [false, true] {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            if previous_guide { view.begin_onboarding(cx); } else { view.begin_game_wizard(cx); }
            view.mark_creation();
            view.busy = true;
            // Re-entering Guide generates a distinct form identity, even on the same tab.
            view.begin_onboarding(cx);
            if let Some(Form::Instance(form)) = &view.form { form.name.update(cx, |field, cx| field.set_value("Newly edited guide", cx)); }
            view.messages = Some(receiver);
            let operation = view.operation_revision;
            assert!(sender.try_send(crate::Message::ForOperation { operation, message: Box::new(crate::Message::Created { instance: "saved-instance".into(), profile: "saved-profile".into() }) }).is_ok());
            assert!(sender.try_send(crate::Message::ForOperation { operation, message: Box::new(crate::Message::Finished(Ok("Prior creation saved".into()))) }).is_ok());
            view.poll(cx);
            assert_eq!(view.onboarding_progress, Progress::Draft);
            assert_eq!(view.tab, Tab::Onboarding);
            assert!(matches!(&view.form, Some(Form::Instance(form)) if form.name.read(cx).value() == "Newly edited guide"));
            assert!(!view.preferences.onboarding_complete);
            assert_eq!(view.selected_profile.as_deref(), Some("saved-profile"));
            assert_eq!(view.shell.games.inspected.as_deref(), Some("saved-profile"));
            assert_eq!(view.shell.profiles.inspected.as_deref(), Some("saved-profile"));
        });
    }
}

#[gpui::test]
fn late_session_then_catalog_keeps_guide_game_and_auth_drafts_while_restoring_supervision(
    cx: &mut TestAppContext,
) {
    for context in 0..3 {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                if context == 1 { view.begin_game_wizard(cx); } else { view.begin_onboarding(cx); }
                if context == 2 { view.open_auth_wizard(window, cx); }
                let tab = view.tab;
                let draft = match &view.form { Some(Form::Instance(form)) => Some(form.name.entity_id()), _ => None };
                // This fixture is only an observer event. It owns no real PID,
                // credentials, session directory or running game process.
                view.session_updated(block_launcher::session_host::RunRecord {
                    schema_version: 1,
                    context: block_launcher::crash::CrashContext { run_id: "synthetic-observed-run".into(), profile_id: "profile-a".into(), profile_name: "Observed profile".into(), minecraft_version: "1.20.6".into(), loader: None },
                    instance_id: "instance-a".into(),
                    game_directory: std::env::temp_dir().join("block-ui-observer-fixture-only"),
                    supervisor_pid: None, java_pid: None,
                    state: block_launcher::session_host::RunState::Running, outcome: None,
                    status: "Running · independent game session".into(), revision: 1,
                    recent_output: vec!["Observed session output".into()], capture_warnings: Vec::new(),
                    started_at_unix_ms: 1, finished_at_unix_ms: None,
                });
                assert_eq!(view.tab, tab);
                assert!(view.busy && view.active_run.is_some());
                assert!(view.pending_profile.is_some());
                let (sender, receiver) = tokio::sync::mpsc::channel(1);
                assert!(sender.try_send(crate::Message::Catalog(Ok(crate::test_support::ready_snapshot()))).is_ok());
                view.messages = Some(receiver);
                view.poll(cx);
                assert_eq!(view.tab, tab);
                assert!(matches!(&view.form, Some(Form::Instance(form)) if Some(form.name.entity_id()) == draft));
                assert_eq!(view.shell.auth.is_some(), context == 2);
                assert!(view.pending_profile.is_none());
                assert_eq!(view.selected_profile.as_deref(), Some("profile-a"));
                assert!(view.busy && view.active_run.is_some());
                assert!(view.logs.iter().any(|line| line == "Observed session output"));
            });
        });
    }
}

/// Drives the real offline-account preparation window from first-run setup and
/// stops on its final confirmation screen, which keeps the identity in memory.
fn prepare_local_identity(
    view: &mut crate::Block,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<'_, crate::Block>,
    name: &str,
) {
    view.open_auth_wizard(window, cx);
    view.choose_auth_provider(block_auth::AccountProvider::Offline, cx);
    view.auth_next(cx);
    let username = view
        .shell
        .auth
        .as_ref()
        .and_then(|state| state.username.clone());
    if let Some(username) = &username {
        username.update(cx, |field, cx| field.set_value(name, cx));
    }
    view.start_auth(window, cx);
    view.finish_auth(window, cx);
}

fn first_run_engine(
    root: &std::path::Path,
) -> anyhow::Result<(
    crate::engine::Engine,
    tokio::sync::mpsc::Receiver<crate::Message>,
)> {
    crate::engine::Engine::with_root(root.to_owned())
}

#[track_caller]
fn submit_first_run(view: &gpui::Entity<crate::Block>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.change_onboarding_step(Step::Review, window, cx);
            view.handle(UiAction::OnboardingNext, window, cx);
            assert!(view.busy);
        });
    });
}

/// Reads the real engine channel until the requested message; the returned flag
/// distinguishes the catalog commit from the final result.
fn drain_until(
    runtime: &tokio::runtime::Runtime,
    receiver: &mut tokio::sync::mpsc::Receiver<crate::Message>,
    want_created: bool,
) -> anyhow::Result<Vec<crate::Message>> {
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut seen = Vec::new();
            loop {
                let message = receiver.recv().await.ok_or_else(|| {
                    anyhow::anyhow!("setup engine closed before the expected result")
                })?;
                let inner = match &message {
                    crate::Message::ForOperation { message, .. } => message.as_ref(),
                    message => message,
                };
                let created = matches!(inner, crate::Message::Created { .. });
                let finished = matches!(inner, crate::Message::Finished(_));
                seen.push(message);
                if (want_created && created) || (!want_created && finished) {
                    return Ok::<_, anyhow::Error>(seen);
                }
            }
        })
        .await?
    })
}

#[gpui::test]
fn first_run_saves_the_verified_identity_and_the_game_together(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, mut receiver) = first_run_engine(root.path())?;
        let catalog = block_launcher::catalog::Catalog::open(root.path())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.begin_onboarding(cx);
                if let Some(Form::Instance(form)) = &view.form {
                    form.name
                        .update(cx, |field, cx| field.set_value("First world", cx));
                }
                prepare_local_identity(view, window, cx, "Local_Player");
                assert_eq!(
                    view.onboarding_identity()
                        .map(|account| account.player_name.clone()),
                    Some("Local_Player".to_owned())
                );
                // Verification alone persists nothing.
                assert!(view.accounts.is_empty());
                assert_eq!(view.tab, Tab::Onboarding);
                let _ = window;
            });
        });
        assert!(!root.path().join("accounts.json").exists());
        assert!(!root.path().join("catalog.json").exists());

        submit_first_run(&view, cx);
        let runtime = tokio::runtime::Runtime::new()?;
        let messages = drain_until(&runtime, &mut receiver, true)?;
        // The catalog commit and the identity are durable in the same step.
        let snapshot = catalog.snapshot()?;
        assert_eq!(snapshot.instances.len(), 1);
        assert_eq!(snapshot.profiles.len(), 1);
        let accounts: Vec<block_auth::Account> =
            serde_json::from_slice(&std::fs::read(root.path().join("accounts.json"))?)?;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].player_name, "Local_Player");
        // Release the installation so it never reaches the network.
        drop(receiver);
        drop(messages);

        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        sender.try_send(crate::Message::Accounts(Ok(accounts.clone())))?;
        sender.try_send(crate::Message::Created {
            instance: snapshot.instances[0].id.clone(),
            profile: snapshot.profiles[0].id.clone(),
        })?;
        view.update(cx, |view, cx| {
            view.messages = Some(receiver);
            view.poll(cx);
            assert_eq!(view.tab, Tab::Home);
            assert!(view.shell.onboarding.pending_identity.is_none());
            assert_eq!(
                view.selected_account,
                accounts.first().map(crate::account_id)
            );
        });
        assert!(crate::Preferences::load(root.path())?.onboarding_complete);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn first_run_rolls_the_new_identity_back_when_the_game_cannot_be_saved(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, mut receiver) = first_run_engine(root.path())?;
        // A directory at the catalog path rejects the game commit after the
        // identity was already saved, which must roll the identity back.
        std::fs::create_dir(root.path().join("catalog.json"))?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.begin_onboarding(cx);
                prepare_local_identity(view, window, cx, "Rollback_Player");
                assert!(view.shell.onboarding.pending_identity.is_some());
                let _ = window;
            });
        });
        submit_first_run(&view, cx);
        let runtime = tokio::runtime::Runtime::new()?;
        let messages = drain_until(&runtime, &mut receiver, false)?;
        let Some(finished) = messages.last() else {
            anyhow::bail!("creation reported no result");
        };
        let inner = match finished {
            crate::Message::ForOperation { message, .. } => message.as_ref(),
            message => message,
        };
        assert!(matches!(inner, crate::Message::Finished(Err(_))));
        // Neither half of the commit remains: the rolled-back metadata keeps
        // no account, and the catalog directory holds no saved game.
        let accounts: Vec<block_auth::Account> =
            serde_json::from_slice(&std::fs::read(root.path().join("accounts.json"))?)?;
        assert!(accounts.is_empty(), "the new identity must be rolled back");
        assert!(catalog_free(&root));
        view.update(cx, |view, cx| {
            let (sender, receiver) = tokio::sync::mpsc::channel(2);
            assert!(
                sender
                    .try_send(crate::Message::Accounts(Ok(Vec::new())))
                    .is_ok()
            );
            assert!(
                sender
                    .try_send(crate::Message::Finished(Err(
                        "The game could not be saved.".into()
                    )))
                    .is_ok()
            );
            view.messages = Some(receiver);
            view.poll(cx);
            assert_eq!(view.tab, Tab::Onboarding);
            assert!(view.shell.onboarding.pending_identity.is_none());
            assert!(view.shell.onboarding.account.is_none());
        });
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

/// The catalog file is a directory in the rollback fixture, so its absence is
/// checked through a reading attempt instead of `exists`.
fn catalog_free(root: &tempfile::TempDir) -> bool {
    std::fs::read(root.path().join("catalog.json")).is_err()
}

#[gpui::test]
fn leaving_first_run_before_creation_saves_no_identity_or_game(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _receiver) = first_run_engine(root.path())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.begin_onboarding(cx);
                if let Some(Form::Instance(form)) = &view.form {
                    form.name
                        .update(cx, |field, cx| field.set_value("Discarded world", cx));
                }
                prepare_local_identity(view, window, cx, "Local_Player");
                // Going back keeps the verified identity in memory only.
                view.change_onboarding_step(Step::Game, window, cx);
                view.change_onboarding_step(Step::Account, window, cx);
                assert!(view.shell.onboarding.pending_identity.is_some());
                assert!(view.accounts.is_empty());
                view.handle(UiAction::OnboardingSkip, window, cx);
                assert_eq!(view.tab, Tab::Home);
                assert!(view.shell.onboarding.pending_identity.is_none());
                assert!(view.accounts.is_empty());
            });
        });
        assert!(!root.path().join("accounts.json").exists());
        assert!(!root.path().join("catalog.json").exists());
        assert!(crate::Preferences::load(root.path())?.onboarding_complete);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn cancelling_first_run_account_setup_leaves_no_identity_behind(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _receiver) = first_run_engine(root.path())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.begin_onboarding(cx);
                view.open_auth_wizard(window, cx);
                view.choose_auth_provider(block_auth::AccountProvider::Offline, cx);
                view.auth_next(cx);
                let username = view
                    .shell
                    .auth
                    .as_ref()
                    .and_then(|state| state.username.clone());
                if let Some(username) = &username {
                    username.update(cx, |field, cx| field.set_value("Cancelled_Player", cx));
                }
                view.start_auth(window, cx);
                // Cancel on the confirmation step: the identity was never saved.
                view.cancel_auth(window, cx);
                assert!(view.shell.auth.is_none());
                assert!(view.shell.onboarding.pending_identity.is_none());
                // A late result cannot adopt the cancelled attempt.
                if let Ok(account) = crate::engine::offline_account("Cancelled_Player") {
                    view.auth_prepared(account);
                }
                assert!(view.shell.auth.is_none());
                assert!(view.shell.onboarding.pending_identity.is_none());
                assert!(view.accounts.is_empty());
            });
        });
        assert!(!root.path().join("accounts.json").exists());
        assert!(!root.path().join("catalog.json").exists());
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}
