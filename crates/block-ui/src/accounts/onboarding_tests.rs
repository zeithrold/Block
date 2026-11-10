//! First-run completion and account state stay shared with the setup page.

use crate::{Tab, first_run::Stage as FirstRunStage, test_support::test_account};
use gpui::{TestAppContext, VisualTestContext};

fn first_run_engine(
    root: &std::path::Path,
) -> anyhow::Result<(
    crate::engine::Engine,
    tokio::sync::mpsc::Receiver<crate::Message>,
)> {
    crate::engine::Engine::with_root(root.to_owned())
}

fn saved_accounts(root: &std::path::Path) -> anyhow::Result<Vec<block_auth::Account>> {
    Ok(crate::global_config::read(root)?
        .map(|config| config.accounts)
        .unwrap_or_default())
}

fn prepare_local_identity(
    view: &mut crate::Block,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<'_, crate::Block>,
    name: &str,
) {
    view.open_auth_wizard(window, cx);
    view.choose_auth_provider(block_auth::AccountProvider::Offline, cx);
    let username = view
        .shell
        .auth
        .as_ref()
        .and_then(|state| state.username.clone());
    if let Some(username) = &username {
        username.update(cx, |field, cx| field.set_value(name, cx));
    }
    view.start_auth(window, cx);
}

fn apply_engine_messages(
    view: &gpui::Entity<crate::Block>,
    cx: &mut VisualTestContext,
    messages: Vec<crate::Message>,
) -> anyhow::Result<()> {
    let (sender, receiver) = tokio::sync::mpsc::channel(messages.len().saturating_add(1).max(1));
    for message in messages {
        sender.try_send(message)?;
    }
    view.update(cx, |view, cx| {
        view.messages = Some(receiver);
        view.poll(cx);
    });
    cx.run_until_parked();
    Ok(())
}

fn finish_local_account_save(
    view: &gpui::Entity<crate::Block>,
    cx: &mut VisualTestContext,
    receiver: &mut tokio::sync::mpsc::Receiver<crate::Message>,
    player_name: &str,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let messages = runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut messages = Vec::new();
            loop {
                let message = receiver.recv().await.ok_or_else(|| {
                    anyhow::anyhow!("account-save engine closed before completion")
                })?;
                let inner = match &message {
                    crate::Message::ForOperation { message, .. } => message.as_ref(),
                    message => message,
                };
                let finished = matches!(inner, crate::Message::Finished(_));
                messages.push(message);
                if finished {
                    return Ok::<_, anyhow::Error>(messages);
                }
            }
        })
        .await?
    })?;
    apply_engine_messages(view, cx, messages)?;
    for _ in 0..12 {
        view.update(cx, crate::Block::poll);
        cx.run_until_parked();
        if view.read_with(cx, |view, _| {
            view.shell.auth.is_none()
                && !view.busy
                && view
                    .accounts
                    .iter()
                    .any(|account| account.player_name == player_name)
                && view.shell.onboarding.account_decided
        }) {
            return Ok(());
        }
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
    }
    anyhow::bail!("saved account did not return to first-run setup")
}

#[gpui::test]
fn late_session_then_catalog_keeps_first_run_and_game_wizard_drafts(cx: &mut TestAppContext) {
    for context in 0..3 {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                if context == 1 {
                    view.begin_game_wizard(cx);
                } else {
                    view.begin_first_run(cx);
                }
                if context == 2 {
                    view.open_auth_wizard(window, cx);
                }
                let tab = view.router.tab();
                let draft = match &view.form {
                    Some(crate::Form::Instance(form)) => Some(form.name.entity_id()),
                    _ => None,
                };
                view.session_updated(
                    block_launcher::session_host::RunRecord {
                        schema_version: 2,
                        context: block_launcher::crash::CrashContext {
                            run_id: "synthetic-observed-run".into(),
                            instance_id: "instance-a".into(),
                            instance_name: "Observed instance".into(),
                            minecraft_version: "1.20.6".into(),
                            loader: None,
                        },
                        game_directory: std::env::temp_dir().join("block-ui-observer-fixture-only"),
                        supervisor_pid: None,
                        java_pid: None,
                        state: block_launcher::session_host::RunState::Running,
                        outcome: None,
                        status: "Running · independent game session".into(),
                        revision: 1,
                        recent_output: vec!["Observed session output".into()],
                        capture_warnings: Vec::new(),
                        started_at_unix_ms: 1,
                        finished_at_unix_ms: None,
                    },
                    cx,
                );
                assert_eq!(view.router.tab(), tab);
                assert!(view.busy && view.active_run.is_some());
                assert!(view.pending_instance.is_some());
                let (sender, receiver) = tokio::sync::mpsc::channel(1);
                assert!(
                    sender
                        .try_send(crate::Message::Catalog(Ok(
                            crate::test_support::ready_snapshot(),
                        )))
                        .is_ok()
                );
                view.messages = Some(receiver);
                view.poll(cx);
                assert_eq!(view.router.tab(), tab);
                let current_draft = match &view.form {
                    Some(crate::Form::Instance(form)) => Some(form.name.entity_id()),
                    _ => None,
                };
                assert_eq!(current_draft, draft);
                assert_eq!(view.shell.auth.is_some(), context == 2);
                assert!(view.pending_instance.is_none());
                assert_eq!(view.selected_instance.as_deref(), Some("instance-a"));
                assert!(view.busy && view.active_run.is_some());
                assert!(
                    view.logs
                        .iter()
                        .any(|line| line == "Observed session output")
                );
            });
        });
    }
}

#[gpui::test]
fn first_run_completion_saves_the_account_draft_and_retries_after_failure(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _) = first_run_engine(root.path())?;
        let first = test_account()?;
        let mut other = test_account()?;
        other.reference.player_id = "00000000-0000-4000-8000-000000000002".parse()?;
        other.player_name = "Other saved account".into();
        let first_id = crate::account_id(&first);
        let other_id = crate::account_id(&other);
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.accounts = vec![first, other];
                view.selected_account = Some(first_id.clone());
                view.begin_first_run(cx);
                view.shell.onboarding.account = Some(other_id.clone());
                view.shell.onboarding.account_decided = true;
            });
        });

        std::fs::create_dir(root.path().join(crate::global_config::FILE))?;
        view.update(cx, |view, _| {
            assert!(!view.complete_onboarding());
            assert_eq!(view.selected_account.as_ref(), Some(&first_id));
        });
        std::fs::remove_dir(root.path().join(crate::global_config::FILE))?;
        view.update(cx, |view, _| {
            assert!(view.complete_onboarding());
            assert_eq!(view.selected_account.as_ref(), Some(&other_id));
            assert_eq!(view.router.tab(), Tab::Home);
            assert_eq!(view.accounts.len(), 2);
        });
        assert!(crate::Preferences::load(root.path())?.onboarding_complete);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn saved_account_returns_to_center_and_game_task_remains_available(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, mut receiver) = first_run_engine(root.path())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.begin_first_run(cx);
                prepare_local_identity(view, window, cx, "Footer_Player");
            });
        });
        finish_local_account_save(&view, cx, &mut receiver, "Footer_Player")?;
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == FirstRunStage::Overview
                && view.first_run_account_ready()
                && view.shell.onboarding.account_decided
        }));
        let accounts = saved_accounts(root.path())?;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].player_name, "Footer_Player");

        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(1000.)));
        let game = crate::test_support::control_bounds(cx, "setup-game")
            .ok_or_else(|| anyhow::anyhow!("game task is not rendered"))?;
        cx.simulate_click(game.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == FirstRunStage::GameChoice
        }));
        assert_eq!(saved_accounts(root.path())?, accounts);
        assert_eq!(
            block_launcher::storage::Locations::load(root.path())?.library(root.path()),
            view.read_with(cx, |view, _| view.first_run_game_parent())
        );
        assert!(!crate::Preferences::load(root.path())?.onboarding_complete);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn a_saved_identity_returns_from_first_run_to_its_center(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, mut receiver) = first_run_engine(root.path())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let dimensions = cx.update(|window, _| crate::window_geometry::home_size(window));
        cx.simulate_resize(dimensions);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.begin_first_run(cx);
                // The center's own Add account action opens the page.
                view.handle(
                    crate::UiAction::FirstRun(crate::first_run::Action::Account),
                    window,
                    cx,
                );
                view.choose_auth_provider(block_auth::AccountProvider::Offline, cx);
                let username = view
                    .shell
                    .auth
                    .as_ref()
                    .and_then(|state| state.username.clone());
                if let Some(username) = &username {
                    username.update(cx, |field, cx| field.set_value("Center_Player", cx));
                }
                view.start_auth(window, cx);
            });
        });
        finish_local_account_save(&view, cx, &mut receiver, "Center_Player")?;
        cx.run_until_parked();
        let account = crate::engine::offline_account("Center_Player")?;
        let id = crate::account_id(&account);
        assert!(view.read_with(cx, |view, _| {
            view.router.tab() == Tab::Onboarding
                && view.page_spec().page == crate::navigation::Page::FirstRun
                && !view.page_spec().task
                && view.shell.first_run.stage == FirstRunStage::Overview
                && view.shell.onboarding.account.as_deref() == Some(id.as_str())
                && view.selected_account.as_deref() == Some(id.as_str())
                && view.first_run_account_ready()
                && !view.busy
                && !view.pending_play
                && view.active_run.is_none()
        }));
        cx.update(|window, cx| {
            view.read_with(cx, |view, _| assert!(view.focus.is_focused(window)));
            assert_eq!(
                crate::window_geometry::content_size(window),
                gpui::size(gpui::px(360.), gpui::px(720.))
            );
        });
        assert_eq!(saved_accounts(root.path())?.len(), 1);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn cancelling_first_run_account_setup_leaves_no_account_behind(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _receiver) = first_run_engine(root.path())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.begin_first_run(cx);
                view.open_auth_wizard(window, cx);
                view.choose_auth_provider(block_auth::AccountProvider::Offline, cx);
                let username = view
                    .shell
                    .auth
                    .as_ref()
                    .and_then(|state| state.username.clone());
                if let Some(username) = &username {
                    username.update(cx, |field, cx| field.set_value("Cancelled_Player", cx));
                }
                view.cancel_auth(window, cx);
                assert!(view.shell.auth.is_none());
                assert!(!view.first_run_account_ready());
                if let Ok(account) = crate::engine::offline_account("Cancelled_Player") {
                    view.auth_prepared(account);
                }
                assert!(view.shell.auth.is_none());
                assert!(!view.first_run_account_ready());
                assert!(view.accounts.is_empty());
                assert!(!view.shell.onboarding.account_decided);
            });
        });
        assert!(saved_accounts(root.path())?.is_empty());
        assert_eq!(
            block_launcher::storage::Locations::load(root.path())?.library(root.path()),
            view.read_with(cx, |view, _| view.first_run_game_parent())
        );
        assert!(!crate::Preferences::load(root.path())?.onboarding_complete);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}
