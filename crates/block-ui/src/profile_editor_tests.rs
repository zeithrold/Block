//! Draft checkpoints and final persistence use real Catalog and Engine boundaries.

use std::time::{Duration, Instant};

use block_launcher::catalog::{Catalog, ProfileColor};
use gpui::{Focusable as _, TestAppContext, VisualTestContext, px, size};

use crate::{Block, Form, Tab, UiAction, profile_editor::EditorPage, test_support::ready_snapshot};

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

#[gpui::test]
fn profile_subpage_cancel_restores_its_checkpoint_and_done_only_changes_the_draft(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.tab = Tab::Instances;
            view.snapshot = ready_snapshot();
            view.handle(UiAction::EditProfile, window, cx);
        });
    });
    click(cx, "edit-profile-appearance");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::ProfileColor(ProfileColor::Purple), window, cx);
            view.handle(UiAction::ProfilePageCancel, window, cx);
            assert!(matches!(&view.form, Some(Form::Profile(form)) if form.original.appearance.color == ProfileColor::Gray));
            view.handle(UiAction::ProfilePage(EditorPage::Appearance), window, cx);
            view.handle(UiAction::ProfileColor(ProfileColor::Green), window, cx);
            view.handle(UiAction::ProfilePageDone, window, cx);
            assert!(matches!(&view.form, Some(Form::Profile(form)) if form.original.appearance.color == ProfileColor::Green));
            assert_eq!(view.snapshot.profiles[0].appearance.color, ProfileColor::Gray);
            view.handle(UiAction::ProfilePage(EditorPage::Launch), window, cx);
            if let Some(Form::Profile(form)) = &view.form {
                form.memory.update(cx, |field, cx| field.set_value("8192", cx));
            }
            view.handle(UiAction::ProfilePageCancel, window, cx);
            assert!(matches!(&view.form, Some(Form::Profile(form)) if form.memory.read(cx).value() == "2048"));
            view.handle(UiAction::CancelForm, window, cx);
            assert!(view.form.is_none());
            assert_eq!(view.snapshot.profiles[0].memory_mib, 2048);
            assert_eq!(view.snapshot.profiles[0].appearance.color, ProfileColor::Gray);
        });
    });
}

#[gpui::test]
fn profile_validation_reveals_the_actual_invalid_field_and_recovers_without_losing_values(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(360.), px(540.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.tab = Tab::Instances;
            view.snapshot = ready_snapshot();
            view.handle(UiAction::EditProfile, window, cx);
            if let Some(Form::Profile(form)) = &view.form {
                form.memory
                    .update(cx, |field, cx| field.set_value("bad memory", cx));
                form.jvm
                    .update(cx, |field, cx| field.set_value("-Dkept=true", cx));
            }
            view.handle(UiAction::SaveProfile, window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            assert!(matches!(&view.form, Some(Form::Profile(form)) if form.memory.read(cx).focus_handle(cx).is_focused(window)
                && form.jvm.read(cx).value() == "-Dkept=true"));
            assert_eq!(view.shell.editor.page, Some(EditorPage::Launch));
            assert!(!view.busy);
        });
    });
    assert!(
        cx.debug_bounds("profile-field-Memory")
            .is_some_and(|bounds| bounds.top() >= px(40.) && bounds.bottom() <= px(484.))
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(Form::Profile(form)) = &view.form {
                form.memory.update(cx, |field, cx| field.set_value("4096", cx));
                form.jvm.update(cx, |field, cx| field.set_value("\"unfinished", cx));
            }
            view.handle(UiAction::SaveProfile, window, cx);
            assert!(matches!(&view.form, Some(Form::Profile(form)) if form.jvm.read(cx).focus_handle(cx).is_focused(window)
                && form.memory.read(cx).value() == "4096"));
            assert_eq!(view.snapshot.profiles[0].memory_mib, 2048);
            assert!(!view.busy);
        });
    });
}

#[gpui::test]
fn final_save_persists_the_complete_profile_and_subpage_actions_do_not_write(
    cx: &mut TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let catalog = Catalog::open(root.path())?;
        let (instance, profile) =
            catalog.create_instance("Editor".into(), "1.20.6".into(), None, None)?;
        catalog.separate_profiles()?;
        let snapshot = catalog.snapshot()?;
        let before = std::fs::read(root.path().join("catalog.json"))?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.tab = Tab::Profiles;
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.snapshot = snapshot;
                view.selected_instance = Some(instance.id);
                view.selected_profile = Some(profile.id.clone());
                view.handle(UiAction::EditProfile, window, cx);
                view.handle(UiAction::ProfilePage(EditorPage::Launch), window, cx);
                if let Some(Form::Profile(form)) = &view.form {
                    form.memory
                        .update(cx, |field, cx| field.set_value("6144", cx));
                    form.jvm.update(cx, |field, cx| {
                        field.set_value("-Dtitle=\"Hello world\"", cx);
                    });
                }
                view.handle(UiAction::ProfilePageDone, window, cx);
                view.handle(UiAction::ProfilePage(EditorPage::Appearance), window, cx);
                view.handle(UiAction::ProfileColor(ProfileColor::Blue), window, cx);
                view.handle(UiAction::ProfilePageDone, window, cx);
                if let Some(Form::Profile(form)) = &view.form {
                    form.name
                        .update(cx, |field, cx| field.set_value("Saved 世界", cx));
                    form.server.update(cx, |field, cx| {
                        field.set_value("play.example.com:25566", cx);
                    });
                }
                view.handle(UiAction::ProfileConnect, window, cx);
            });
        });
        assert_eq!(std::fs::read(root.path().join("catalog.json"))?, before);
        click(cx, "save-profile");
        let deadline = Instant::now() + Duration::from_secs(3);
        while view.read_with(cx, |view, _| view.busy) && Instant::now() < deadline {
            view.update(cx, Block::poll);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(view.read_with(cx, |view, _| !view.busy && view.form.is_none()));
        let saved = catalog.snapshot()?;
        let saved = saved
            .profiles
            .iter()
            .find(|saved| saved.id == profile.id)
            .ok_or_else(|| anyhow::anyhow!("saved profile missing"))?;
        assert_eq!(saved.name, "Saved 世界");
        assert_eq!(saved.memory_mib, 6144);
        assert_eq!(saved.jvm_args, ["-Dtitle=Hello world"]);
        assert_eq!(saved.appearance.color, ProfileColor::Blue);
        assert_eq!(saved.server.address, "play.example.com:25566");
        assert!(saved.server.connect_on_launch);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn invalid_server_recovers_on_the_main_page_and_clearing_it_disables_automatic_connection(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(360.), px(540.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.tab = Tab::Instances;
            view.snapshot = ready_snapshot();
            view.handle(UiAction::EditProfile, window, cx);
            if let Some(Form::Profile(form)) = &view.form {
                form.server.update(cx, |field, cx| field.set_value("https://invalid.example", cx));
            }
            view.handle(UiAction::ProfilePage(EditorPage::Launch), window, cx);
            view.handle(UiAction::SaveProfile, window, cx);
            assert!(view.shell.editor.page.is_none());
            assert!(matches!(&view.form, Some(Form::Profile(form)) if form.server.read(cx).focus_handle(cx).is_focused(window)));
            assert!(!view.busy);
            assert_eq!(view.snapshot.profiles[0].server.address, "");
            if let Some(Form::Profile(form)) = &mut view.form {
                form.original.server.connect_on_launch = true;
                form.server.update(cx, |field, cx| field.set_value("", cx));
                let parsed = super::profile_editor::parse_profile(form, cx);
                assert!(parsed.is_ok_and(|profile| profile.server.address.is_empty() && !profile.server.connect_on_launch));
            }
        });
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("profile-field-Server")
            .is_some_and(|bounds| bounds.top() >= px(40.) && bounds.bottom() <= px(484.))
    );
}

fn finish_operation(view: &gpui::Entity<Block>, cx: &mut VisualTestContext) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        view.update(cx, Block::poll);
        cx.run_until_parked();
        if !view.read_with(cx, |view, _| view.busy) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "profile metadata task did not finish"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[gpui::test]
fn main_cancel_restores_the_actual_edit_trigger_and_does_not_change_the_profile(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(960.), px(600.)));
    view.update(cx, |view, cx| {
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.tab = Tab::Instances;
        view.shell.games.inspected = Some("profile-a".into());
        view.shell.games.detail_visible = true;
        cx.notify();
    });
    let trigger = view.update(cx, |view, cx| view.profile_editor_trigger_focus(cx));
    click(cx, "edit-profile");
    view.update(cx, |view, cx| {
        if let Some(Form::Profile(form)) = &view.form {
            form.name
                .update(cx, |input, cx| input.set_value("Discarded", cx));
        }
    });
    click(cx, "cancel-profile");
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.read_with(cx, |view, _| {
            assert!(trigger.is_focused(window));
            assert!(view.form.is_none());
            assert_eq!(view.snapshot.profiles[0].name, "Test profile");
        });
    });
}

#[gpui::test]
fn real_save_failure_keeps_the_draft_and_retry_restores_trigger_after_persistence(
    cx: &mut TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let catalog = Catalog::open(root.path())?;
        let (_, profile) = catalog.create_instance("Focus".into(), "1.20.6".into(), None, None)?;
        let snapshot = catalog.snapshot()?;
        let path = root.path().join("catalog.json");
        let original = std::fs::read(&path)?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.simulate_resize(size(px(360.), px(540.)));
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.snapshot = snapshot;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.tab = Tab::Instances;
            view.shell.games.inspected = Some(profile.id.clone());
            view.shell.games.detail_visible = true;
            cx.notify();
        });
        let trigger = view.update(cx, |view, cx| view.profile_editor_trigger_focus(cx));
        click(cx, "edit-profile");
        view.update(cx, |view, cx| {
            if let Some(Form::Profile(form)) = &view.form {
                form.name
                    .update(cx, |input, cx| input.set_value("Retried profile", cx));
            }
        });
        // Only this test's own metadata document is replaced with an empty directory.
        std::fs::remove_file(&path)?;
        std::fs::create_dir(&path)?;
        click(cx, "save-profile");
        finish_operation(&view, cx);
        assert!(view.read_with(cx, |view, cx| matches!(&view.form, Some(Form::Profile(form)) if form.name.read(cx).value() == "Retried profile")));
        assert!(
            cx.debug_bounds("profile-save-error")
                .is_some_and(|bounds| bounds.top() >= px(40.) && bounds.bottom() <= px(484.))
        );
        let save = cx.debug_bounds("save-profile");
        let toast = cx.debug_bounds("toast");
        assert!(
            save.zip(toast).is_some_and(
                |(save, toast)| toast.bottom() <= save.top() || toast.top() >= save.bottom()
            ),
            "failure toast must not consume the retry action: {toast:?}, {save:?}"
        );
        std::fs::remove_dir(&path)?;
        std::fs::write(&path, original)?;
        click(cx, "save-profile");
        finish_operation(&view, cx);
        assert_eq!(catalog.snapshot()?.profiles[0].name, "Retried profile");
        cx.update(|window, cx| {
            view.read_with(cx, |view, _| {
                assert!(view.form.is_none());
                assert!(trigger.is_focused(window));
            });
        });
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn saving_after_inspection_changes_restores_the_current_detail_instead_of_an_old_trigger(
    cx: &mut TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let catalog = Catalog::open(root.path())?;
        let (instance, first) =
            catalog.create_instance("First".into(), "1.20.6".into(), None, None)?;
        catalog.separate_profiles()?;
        let other = catalog.create_profile(&instance.id, "Other".into())?;
        let snapshot = catalog.snapshot()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.snapshot = snapshot;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.tab = Tab::Profiles;
            view.shell.profiles.inspected = Some(first.id);
            view.shell.profiles.detail_visible = true;
            cx.notify();
        });
        click(cx, "edit-profile");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::SaveProfile, window, cx);
                view.shell.profiles.inspected = Some(other.id);
            });
        });
        finish_operation(&view, cx);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                assert!(view.browser_detail_focus(cx).is_focused(window));
                assert!(view.form.is_none());
                assert_eq!(view.snapshot.profiles.len(), 2);
            });
        });
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}
