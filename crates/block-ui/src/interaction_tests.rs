//! Product drafts and late asynchronous results remain independent of window presentation.
use super::{Block, Tab, UiAction, engine::Message, shell::Surface};
use gpui::{Focusable as _, px, size};

#[gpui::test]
fn browsing_and_cancelling_an_editor_preserve_the_home_selection(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, super::tests::test_view);
    view.update(cx, |view, cx| {
        view.snapshot = super::test_support::ready_snapshot();
        let mut second = view.snapshot.instances[0].clone();
        second.id = "second-instance".into();
        second.name = "Second".into();
        let mut profile = view.snapshot.profiles[0].clone();
        profile.id = "second-profile".into();
        profile.instance_id.clone_from(&second.id);
        view.snapshot.instances.push(second);
        view.snapshot.profiles.push(profile);
        view.selected_instance = Some("instance-a".into());
        view.selected_profile = Some("profile-a".into());
        view.initialize_browsers(cx);
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Inspect("second-instance".into()), window, cx);
            view.handle(UiAction::EditProfile, window, cx);
            if let Some(super::Form::Profile(form)) = &view.form {
                form.name
                    .update(cx, |field, cx| field.set_value("Uncommitted 世界", cx));
            }
            view.handle(UiAction::CancelForm, window, cx);
            assert_eq!(view.selected_profile.as_deref(), Some("profile-a"));
            assert_eq!(view.selected_instance.as_deref(), Some("instance-a"));
            assert_eq!(
                view.shell.games.inspected.as_deref(),
                Some("second-instance")
            );
            assert_eq!(
                view.snapshot.profiles[1].name,
                view.snapshot.profiles[0].name
            );
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
        && view.status == "Current task"));
}

#[gpui::test]
fn ime_escape_keeps_the_current_form_open_until_composition_finishes(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::EntityInputHandler as _;
    let (view, cx) = crate::test_support::window(cx, super::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::CreateInstance, window, cx);
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
        && view.tab == Tab::Instances));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(super::Form::Instance(form)) = &view.form {
                form.name
                    .update(cx, |field, cx| field.unmark_text(window, cx));
            }
        });
    });
    crate::test_support::press(cx, "escape");
    assert!(view.read_with(cx, |view, _| view.form.is_none()));
}
#[gpui::test]
fn guide_information_preserves_setup_draft_and_cancels_settings_preview(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.saved_preferences.reduce_motion = true;
            view.begin_onboarding(cx);
            if let Some(crate::Form::Instance(form)) = &view.form {
                form.name.update(cx, |input, cx| input.set_value("Retain 世界", cx));
            }
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.handle(UiAction::SetAppearance(super::Appearance::Dark), window, cx);
            assert_eq!(view.preferences.appearance, super::Appearance::Dark);
            assert_eq!(view.tab, Tab::Onboarding);
            assert!(view.shell.surfaces.top().is_some_and(|layer| matches!(layer.kind, Surface::GuideInformation(Tab::Settings))));
            view.handle(UiAction::Drawer(None), window, cx);
            assert_eq!(view.preferences.appearance, super::Appearance::System);
            assert!(view.settings_form.is_none());
            assert!(matches!(&view.form, Some(crate::Form::Instance(form)) if form.name.read(cx).value() == "Retain 世界"));
            view.handle(UiAction::Tab(Tab::About), window, cx);
        });
    });
    cx.run_until_parked();
    assert!(crate::test_support::control_bounds(cx, "about-body").is_some());
    crate::test_support::press(cx, "escape");
    assert!(view.read_with(cx, |view, cx| view.tab == Tab::Onboarding && !view.shell.surfaces.is_open()
        && matches!(&view.form, Some(crate::Form::Instance(form)) if form.name.read(cx).value() == "Retain 世界")));
}
