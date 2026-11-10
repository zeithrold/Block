//! Product drafts and late asynchronous results remain independent of window presentation.
use super::{Block, Tab, UiAction, engine::Message};
use gpui::{Focusable as _, px, size};

#[gpui::test]
fn browsing_and_cancelling_an_editor_preserve_the_home_selection(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, super::tests::test_view);
    view.update(cx, |view, cx| {
        view.snapshot = super::test_support::ready_snapshot();
        let mut second = view.snapshot.instances[0].clone();
        second.id = "second-instance".into();
        second.name = "Second".into();
        view.snapshot.instances.push(second);
        view.selected_instance = Some("instance-a".into());
        view.initialize_browsers(cx);
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Inspect("second-instance".into()), window, cx);
            view.handle(UiAction::EditInstance, window, cx);
            if let Some(super::Form::EditInstance(form)) = &view.form {
                form.name
                    .update(cx, |field, cx| field.set_value("Uncommitted 世界", cx));
            }
            view.handle(UiAction::CancelForm, window, cx);
            assert_eq!(view.selected_instance.as_deref(), Some("instance-a"));
            assert_eq!(
                view.shell.games.inspected.as_deref(),
                Some("second-instance")
            );
            assert_eq!(view.snapshot.instances[1].name, "Second");
        });
    });
}

#[gpui::test]
fn late_operation_cannot_close_a_new_draft_or_replace_the_current_status(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, super::tests::test_view);
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::CreateInstance, window, cx);
            view.operation_revision = 2;
            view.busy = true;
            view.messages = Some(receiver);
        });
    });
    for message in [
        Message::Status("old failure".into()),
        Message::Finished(Ok("old success".into())),
    ] {
        assert!(
            sender
                .try_send(Message::ForOperation {
                    operation: 1,
                    message: Box::new(message)
                })
                .is_ok()
        );
    }
    assert!(
        sender
            .try_send(Message::ForOperation {
                operation: 2,
                message: Box::new(Message::Status("Current task".into()))
            })
            .is_ok()
    );
    view.update(cx, Block::poll);
    assert!(view.read_with(cx, |view, _| view.form.is_some()
        && view.busy
        && view.status.text(crate::Language::English) == "Current task"));
}

#[gpui::test]
fn completion_and_toast_keep_their_identity_across_language_changes(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, super::tests::test_view);
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    view.update(cx, |view, _| {
        view.operation_revision = 2;
        view.busy = true;
        view.messages = Some(receiver);
    });
    assert!(
        sender
            .try_send(Message::ForOperation {
                operation: 1,
                message: Box::new(Message::Finished(Ok(msg!("completion.instance_deleted")))),
            })
            .is_ok()
    );
    assert!(
        sender
            .try_send(Message::ForOperation {
                operation: 2,
                message: Box::new(Message::Finished(Ok(msg!(
                    "completion.provider_saved",
                    name = "Example"
                )))),
            })
            .is_ok()
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(
                UiAction::SetLanguage(crate::Language::SimplifiedChinese),
                window,
                cx,
            );
            view.poll(cx);
            assert_eq!(
                view.status.text(view.language()),
                "认证提供者 Example 已保存"
            );
            assert_eq!(view.status.key(), Some("completion.provider_saved"));
            let toast = view.shell.toast.as_ref().map(|toast| toast.message.clone());
            view.handle(UiAction::SetLanguage(crate::Language::Japanese), window, cx);
            assert_eq!(
                view.status.text(view.language()),
                "プロバイダー Example を保存しました"
            );
            assert_eq!(
                toast
                    .map(|message| message.text(view.language()))
                    .as_deref(),
                Some("プロバイダー Example を保存しました")
            );
        });
    });
}

#[gpui::test]
fn ime_escape_keeps_the_current_form_open_until_composition_finishes(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::EntityInputHandler as _;
    let (view, cx) = crate::test_support::window(cx, super::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(UiAction::CreateInstance, window, cx);
        });
    });
    cx.run_until_parked();
    let create = crate::test_support::control_bounds(cx, "wizard-kind-instance");
    assert!(
        create.is_some(),
        "the source page exposes the new-game action"
    );
    let Some(create) = create else {
        return;
    };
    cx.simulate_click(create.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.step),
        crate::game_wizard::WizardStep::Setup
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(super::Form::Instance(form)) = &view.form {
                form.name.update(cx, |field, cx| {
                    field.focus_handle(cx).focus(window, cx);
                    field.replace_and_mark_text_in_range(None, "中文", None, window, cx);
                });
            }
        });
    });
    cx.run_until_parked();
    crate::test_support::press(cx, "escape");
    assert!(view.read_with(cx, |view, _| view.form.is_some()
        && view.router.tab() == Tab::Instances));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(super::Form::Instance(form)) = &view.form {
                form.name
                    .update(cx, |field, cx| field.unmark_text(window, cx));
            }
        });
    });
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| {
        view.form.is_some()
            && view.shell.surfaces.top().is_some_and(|layer| {
                matches!(layer.kind, crate::shell::Surface::DiscardGameDraft { .. })
            })
    }));
    let confirm = crate::test_support::control_bounds(cx, "discard-game-confirm");
    assert!(confirm.is_some(), "the changed draft requires confirmation");
    if let Some(confirm) = confirm {
        cx.simulate_click(confirm.center(), gpui::Modifiers::none());
        cx.run_until_parked();
    }
    assert!(view.read_with(cx, |view, _| view.form.is_none()));
}
#[gpui::test]
fn games_route_during_setup_keeps_the_setup_page_with_a_status(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, super::tests::test_view);
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.saved_preferences.reduce_motion = true;
            view.preferences.onboarding_complete = false;
            view.saved_preferences.onboarding_complete = false;
            view.begin_first_run(cx);
            view.handle_first_run(crate::first_run::Action::Custom, window, cx);
            if let Some(crate::Form::Instance(form)) = &view.form {
                form.name
                    .update(cx, |input, cx| input.set_value("Keep setup", cx));
            }
            // A programmatic Games route while setup is active must not blank
            // the page: setup stays available and reports why.
            view.handle(UiAction::Tab(Tab::Instances), window, cx);
            assert_eq!(view.router.tab(), Tab::Onboarding);
            assert!(view.shell.toast.as_ref().is_some_and(|toast| {
                toast
                    .message
                    .text(crate::Language::English)
                    .contains("Finish setup or choose Skip setup to continue.")
            }));
            assert!(matches!(&view.form, Some(crate::Form::Instance(form)) if form.name.read(cx).value() == "Keep setup"));
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.handle(UiAction::Tab(Tab::About), window, cx);
            assert_eq!(view.router.tab(), Tab::Onboarding);
            assert!(matches!(&view.form, Some(crate::Form::Instance(form)) if form.name.read(cx).value() == "Keep setup"));
        });
    });
    cx.run_until_parked();
    assert!(
        crate::test_support::control_bounds(cx, "game-wizard").is_some(),
        "setup must render a meaningful page after the blocked Games route"
    );
}

#[gpui::test]
fn native_close_confirms_a_changed_game_draft(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, super::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(UiAction::CreateInstance, window, cx);
            view.handle(
                UiAction::WizardKind(crate::game_wizard::CreationKind::NewInstance),
                window,
                cx,
            );
            if let Some(super::Form::Instance(form)) = &view.form {
                form.name
                    .update(cx, |field, cx| field.set_value("Keep this draft", cx));
            }
        });
    });
    assert!(!cx.simulate_close());
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| {
        view.shell.surfaces.top().is_some_and(|layer| {
            matches!(layer.kind, crate::shell::Surface::DiscardGameDraft { .. })
        })
    }));
    let stay = crate::test_support::control_bounds(cx, "discard-game-stay");
    assert!(stay.is_some());
    if let Some(stay) = stay {
        cx.simulate_click(stay.center(), gpui::Modifiers::none());
        cx.run_until_parked();
    }
    assert!(view.read_with(cx, |view, cx| {
        matches!(&view.form, Some(super::Form::Instance(form))
            if form.name.read(cx).value() == "Keep this draft")
    }));
}

#[gpui::test]
fn application_quit_confirms_the_current_draft_and_ignores_a_stale_confirmation(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, super::tests::test_view);
    cx.update(|window, cx| {
        crate::initialize_application_quit(cx);
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(UiAction::CreateInstance, window, cx);
            view.handle(
                UiAction::WizardKind(crate::game_wizard::CreationKind::NewInstance),
                window,
                cx,
            );
            if let Some(super::Form::Instance(form)) = &view.form {
                form.name
                    .update(cx, |field, cx| field.set_value("First draft", cx));
            }
        });
    });
    crate::test_support::press(cx, "cmd-q");
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| {
        view.shell.surfaces.top().is_some_and(|layer| {
            matches!(
                &layer.kind,
                crate::shell::Surface::DiscardGameDraft {
                    exit,
                    ..
                } if matches!(exit.as_ref(), UiAction::QuitApplication)
            )
        })
    }));
    let stay = crate::test_support::control_bounds(cx, "discard-game-stay");
    assert!(stay.is_some());
    if let Some(stay) = stay {
        cx.simulate_click(stay.center(), gpui::Modifiers::none());
        cx.run_until_parked();
    }
    assert!(view.read_with(cx, |view, cx| {
        matches!(&view.form, Some(super::Form::Instance(form))
            if form.name.read(cx).value() == "First draft")
            && !view.shell.surfaces.is_open()
    }));

    crate::test_support::press(cx, "cmd-q");
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        view.begin_game_wizard(cx);
        if let Some(super::Form::Instance(form)) = &view.form {
            form.name
                .update(cx, |field, cx| field.set_value("Replacement draft", cx));
        }
        cx.notify();
    });
    let confirm = crate::test_support::control_bounds(cx, "discard-game-confirm");
    assert!(confirm.is_some());
    if let Some(confirm) = confirm {
        cx.simulate_click(confirm.center(), gpui::Modifiers::none());
        cx.run_until_parked();
    }
    assert!(view.read_with(cx, |view, cx| {
        matches!(&view.form, Some(super::Form::Instance(form))
            if form.name.read(cx).value() == "Replacement draft")
            && !view.shell.surfaces.is_open()
    }));
    crate::test_support::press(cx, "cmd-q");
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| {
        view.shell.surfaces.top().is_some_and(|layer| {
            matches!(layer.kind, crate::shell::Surface::DiscardGameDraft { .. })
        })
    }));
    let confirm = crate::test_support::control_bounds(cx, "discard-game-confirm");
    assert!(confirm.is_some());
    if let Some(confirm) = confirm {
        cx.simulate_click(confirm.center(), gpui::Modifiers::none());
        cx.run_until_parked();
    }
    assert!(view.read_with(cx, |view, _| {
        !view.shell.surfaces.is_open() && view.shell.wizard.discard_confirmed.is_none()
    }));
}
