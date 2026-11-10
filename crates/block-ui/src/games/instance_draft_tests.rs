//! Draft checkpoints and final persistence use real Catalog and Engine boundaries.

use std::time::{Duration, Instant};

use block_launcher::catalog::{Catalog, InstanceColor};
use gpui::{Focusable as _, TestAppContext, VisualTestContext, px, size};

use crate::{
    Block, Form, Tab, UiAction, instance_editor::EditorPage, test_support::ready_snapshot,
};

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
        for _ in 0..2 {
            cx.update(|window, cx| window.simulate_next_frame(cx));
            cx.run_until_parked();
        }
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
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                view.router.tab(),
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.snapshot = ready_snapshot();
            view.handle(UiAction::EditInstance, window, cx);
            assert!(!view.instance_editor_dirty(cx));
        });
    });
    click(cx, "edit-instance-appearance");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::InstanceColor(InstanceColor::Purple), window, cx);
            view.handle(UiAction::InstancePageCancel, window, cx);
            assert!(matches!(&view.form, Some(Form::EditInstance(form)) if form.original.appearance.color == InstanceColor::Gray));
            assert!(!view.instance_editor_dirty(cx));
            view.handle(UiAction::InstancePage(EditorPage::Appearance), window, cx);
            view.handle(UiAction::InstanceColor(InstanceColor::Green), window, cx);
            view.handle(UiAction::InstancePageDone, window, cx);
            assert!(matches!(&view.form, Some(Form::EditInstance(form)) if form.original.appearance.color == InstanceColor::Green));
            assert!(view.instance_editor_dirty(cx));
            assert_eq!(view.snapshot.instances[0].appearance.color, InstanceColor::Gray);
            view.handle(UiAction::InstancePage(EditorPage::Launch), window, cx);
            if let Some(Form::EditInstance(form)) = &view.form {
                form.memory.update(cx, |field, cx| field.set_value("8", cx));
            }
            view.handle(UiAction::InstancePageCancel, window, cx);
            assert!(matches!(&view.form, Some(Form::EditInstance(form)) if form.memory.read(cx).value() == "2"));
            view.handle(UiAction::CancelForm, window, cx);
            assert!(view.form.is_none());
            assert!(matches!(view.router.screen(), crate::navigation::Screen::Record(crate::navigation::RecordRef::Instance(id)) if id == "instance-a"));
            assert_eq!(view.snapshot.instances[0].memory_mib, 2048);
            assert_eq!(view.snapshot.instances[0].appearance.color, InstanceColor::Gray);
        });
    });
}

#[gpui::test]
fn root_dirty_check_preserves_invalid_and_hidden_raw_profile_values(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    let result = cx.update(|window, cx| {
        view.update(cx, |view, cx| -> anyhow::Result<()> {
            view.snapshot = ready_snapshot();
            view.router.open_record(
                Tab::Instances,
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.handle(UiAction::EditInstance, window, cx);
            assert!(!view.instance_editor_dirty(cx));
            for index in 0..6 {
                let Some(Form::EditInstance(form)) = &view.form else {
                    anyhow::bail!("the routed editor owns a Profile draft");
                };
                let field = [
                    &form.name,
                    &form.memory,
                    &form.java,
                    &form.jvm,
                    &form.game,
                    &form.server,
                ][index]
                    .clone();
                let initial = field.read(cx).value();
                field.update(cx, |field, cx| field.set_value("\"invalid raw input", cx));
                assert!(view.instance_editor_dirty(cx));
                field.update(cx, |field, cx| field.set_value(&initial, cx));
                assert!(!view.instance_editor_dirty(cx));
            }
            assert_eq!(view.snapshot.instances[0].name, "Test game");
            Ok(())
        })
    });
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn profile_validation_reveals_the_actual_invalid_field_and_recovers_without_losing_values(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(1152.), px(720.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                view.router.tab(),
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.snapshot = ready_snapshot();
            view.handle(UiAction::EditInstance, window, cx);
            if let Some(Form::EditInstance(form)) = &view.form {
                form.memory
                    .update(cx, |field, cx| field.set_value("bad memory", cx));
                form.jvm
                    .update(cx, |field, cx| field.set_value("-Dkept=true", cx));
            }
            view.handle(UiAction::SaveInstance, window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            assert!(matches!(&view.form, Some(Form::EditInstance(form)) if form.memory.read(cx).focus_handle(cx).is_focused(window)
                && form.jvm.read(cx).value() == "-Dkept=true"));
            assert_eq!(view.shell.editor.page, Some(EditorPage::Launch));
            assert!(!view.busy);
        });
    });
    assert!(
        cx.debug_bounds("instance-field-Memory")
            .is_some_and(|bounds| bounds.top() >= px(0.) && bounds.bottom() <= px(648.))
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(Form::EditInstance(form)) = &view.form {
                form.memory.update(cx, |field, cx| field.set_value("4", cx));
                form.jvm.update(cx, |field, cx| field.set_value("\"unfinished", cx));
            }
            view.handle(UiAction::SaveInstance, window, cx);
            assert!(matches!(&view.form, Some(Form::EditInstance(form)) if form.jvm.read(cx).focus_handle(cx).is_focused(window)
                && form.memory.read(cx).value() == "4"));
            assert_eq!(view.snapshot.instances[0].memory_mib, 2048);
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
        let instance = catalog.create_instance("Editor".into(), "1.20.6".into(), None, None)?;
        let profile = instance.clone();
        let snapshot = catalog.snapshot()?;
        let before = serde_json::to_value(&snapshot)?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.router.navigate(Tab::Instances);
                view.router.open_record(
                    view.router.tab(),
                    crate::navigation::RecordRef::Instance("instance-a".into()),
                );
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.snapshot = snapshot;
                view.selected_instance = Some(instance.id.clone());
                view.selected_instance = Some(profile.id.clone());
                view.router.open_record(
                    Tab::Instances,
                    crate::navigation::RecordRef::Instance(profile.id.clone()),
                );
                view.handle(UiAction::EditInstance, window, cx);
                view.handle(UiAction::InstancePage(EditorPage::Launch), window, cx);
                if let Some(Form::EditInstance(form)) = &view.form {
                    form.memory.update(cx, |field, cx| field.set_value("6", cx));
                    form.jvm.update(cx, |field, cx| {
                        field.set_value("-Dtitle=\"Hello world\"", cx);
                    });
                }
                view.handle(UiAction::InstancePageDone, window, cx);
                view.handle(UiAction::InstancePage(EditorPage::Appearance), window, cx);
                view.handle(UiAction::InstanceColor(InstanceColor::Blue), window, cx);
                view.handle(UiAction::InstancePageDone, window, cx);
                if let Some(Form::EditInstance(form)) = &view.form {
                    form.name
                        .update(cx, |field, cx| field.set_value("Saved 世界", cx));
                    form.server.update(cx, |field, cx| {
                        field.set_value("play.example.com:25566", cx);
                    });
                }
                view.handle(UiAction::InstanceConnect, window, cx);
            });
        });
        assert_eq!(serde_json::to_value(catalog.snapshot()?)?, before);
        click(cx, "save-instance");
        let deadline = Instant::now() + Duration::from_secs(3);
        while view.read_with(cx, |view, _| view.busy) && Instant::now() < deadline {
            view.update(cx, Block::poll);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(view.read_with(cx, |view, _| !view.busy && view.form.is_none()));
        let saved = catalog.snapshot()?;
        let saved = saved
            .instances
            .iter()
            .find(|saved| saved.id == profile.id)
            .ok_or_else(|| anyhow::anyhow!("saved profile missing"))?;
        assert_eq!(saved.name, "Saved 世界");
        assert_eq!(saved.memory_mib, 6144);
        assert_eq!(saved.jvm_args, ["-Dtitle=Hello world"]);
        assert_eq!(saved.appearance.color, InstanceColor::Blue);
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
    cx.simulate_resize(size(px(1152.), px(720.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.router.navigate(Tab::Instances);
            view.router.open_record(view.router.tab(), crate::navigation::RecordRef::Instance("instance-a".into()));
            view.snapshot = ready_snapshot();
            view.handle(UiAction::EditInstance, window, cx);
            if let Some(Form::EditInstance(form)) = &view.form {
                form.server.update(cx, |field, cx| field.set_value("https://invalid.example", cx));
            }
            view.handle(UiAction::InstancePage(EditorPage::Launch), window, cx);
            view.handle(UiAction::SaveInstance, window, cx);
            assert!(view.shell.editor.page.is_none());
            assert!(matches!(&view.form, Some(Form::EditInstance(form)) if form.server.read(cx).focus_handle(cx).is_focused(window)));
            assert!(!view.busy);
            assert_eq!(view.snapshot.instances[0].server.address, "");
            if let Some(Form::EditInstance(form)) = &mut view.form {
                form.original.server.connect_on_launch = true;
                form.server.update(cx, |field, cx| field.set_value("", cx));
                let parsed = crate::instance_editor::parse_instance(form, cx);
                assert!(parsed.is_ok_and(|profile| profile.server.address.is_empty() && !profile.server.connect_on_launch));
            }
        });
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("instance-field-Server")
            .is_some_and(|bounds| bounds.top() >= px(0.) && bounds.bottom() <= px(648.))
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
    for _ in 0..2 {
        cx.update(|window, cx| window.simulate_next_frame(cx));
        cx.run_until_parked();
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
        view.router.navigate(Tab::Instances);
        view.router.open_record(
            view.router.tab(),
            crate::navigation::RecordRef::Instance("instance-a".into()),
        );
        view.shell.games.inspected = Some("instance-a".into());
        cx.notify();
    });
    let trigger = view.update(cx, |view, cx| view.instance_editor_trigger_focus(cx));
    click(cx, "edit-instance");
    view.update(cx, |view, cx| {
        if let Some(Form::EditInstance(form)) = &view.form {
            form.name
                .update(cx, |input, cx| input.set_value("Discarded", cx));
        }
    });
    click(cx, "cancel-instance");
    assert!(view.read_with(cx, |view, _| view.form.is_some()));
    click(cx, "editor-exit-discard");
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.read_with(cx, |view, _| {
            assert!(trigger.is_focused(window));
            assert!(view.form.is_none());
            assert_eq!(view.snapshot.instances[0].name, "Test game");
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
        let profile = catalog.create_instance("Focus".into(), "1.20.6".into(), None, None)?;
        let snapshot = catalog.snapshot()?;
        let path = root.path().join("block.toml");
        let original = std::fs::read(&path)?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.simulate_resize(size(px(1152.), px(720.)));
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.snapshot = snapshot;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                view.router.tab(),
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.shell.games.inspected = Some(profile.id.clone());
            view.router.open_record(
                Tab::Instances,
                crate::navigation::RecordRef::Instance(profile.id.clone()),
            );
            cx.notify();
        });
        let trigger = view.update(cx, |view, cx| view.instance_editor_trigger_focus(cx));
        click(cx, "edit-instance");
        view.update(cx, |view, cx| {
            if let Some(Form::EditInstance(form)) = &view.form {
                form.name
                    .update(cx, |input, cx| input.set_value("Retried profile", cx));
            }
        });
        // Only this test's own metadata document is replaced with an empty directory.
        std::fs::remove_file(&path)?;
        std::fs::create_dir(&path)?;
        click(cx, "save-instance");
        finish_operation(&view, cx);
        assert!(view.read_with(cx, |view, cx| matches!(&view.form, Some(Form::EditInstance(form)) if form.name.read(cx).value() == "Retried profile")));
        let save = cx.debug_bounds("save-instance");
        let error = cx.debug_bounds("instance-save-error");
        assert!(
            error.zip(save).is_some_and(|(error, save)| {
                error.top() >= px(40.) && error.bottom() <= save.top()
            }),
            "save error must stay visible above the retry action: {error:?}, {save:?}"
        );
        let toast = cx.debug_bounds("toast");
        assert!(
            save.zip(toast).is_some_and(
                |(save, toast)| toast.bottom() <= save.top() || toast.top() >= save.bottom()
            ),
            "failure toast must not consume the retry action: {toast:?}, {save:?}"
        );
        std::fs::remove_dir(&path)?;
        std::fs::write(&path, original)?;
        click(cx, "save-instance");
        finish_operation(&view, cx);
        assert_eq!(catalog.snapshot()?.instances[0].name, "Retried profile");
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
        let instance = catalog.create_instance("First".into(), "1.20.6".into(), None, None)?;
        let first = instance.clone();
        let other = catalog.create_instance(
            "Other".into(),
            instance.minecraft_version.clone(),
            None,
            None,
        )?;
        let snapshot = catalog.snapshot()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.snapshot = snapshot;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                view.router.tab(),
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.router.open_record(
                Tab::Instances,
                crate::navigation::RecordRef::Instance(first.id.clone()),
            );
            view.shell.games.inspected = Some(first.id);
            cx.notify();
        });
        click(cx, "edit-instance");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::SaveInstance, window, cx);
                view.router.open_record(
                    Tab::Instances,
                    crate::navigation::RecordRef::Instance(other.id.clone()),
                );
                view.shell.games.inspected = Some(other.id);
            });
        });
        finish_operation(&view, cx);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                assert!(view.browser_detail_focus(cx).is_focused(window));
                assert!(view.form.is_none());
                assert_eq!(view.snapshot.instances.len(), 2);
            });
        });
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn profile_memory_validation_uses_gb_and_enforces_the_one_to_thirty_two_gb_bounds(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                view.router.tab(),
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.snapshot = ready_snapshot();
            view.handle(UiAction::EditInstance, window, cx);
            let Some(Form::EditInstance(form)) = &view.form else {
                return;
            };
            for (value, expected) in [
                ("0.5", None),
                ("33", None),
                ("1", Some(1024)),
                ("32", Some(32768)),
                ("1.5", Some(1536)),
            ] {
                form.memory
                    .update(cx, |field, cx| field.set_value(value, cx));
                let parsed = crate::instance_editor::parse_instance(form, cx);
                match expected {
                    Some(mib) => {
                        assert_eq!(parsed.map(|profile| profile.memory_mib), Ok(mib), "{value}")
                    }
                    None => assert!(parsed.is_err(), "{value}"),
                }
            }
        });
    });
}

#[gpui::test]
fn japanese_instance_editor_keeps_its_fields_and_actions(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(1152.), px(720.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.preferences.language = Some(crate::Language::Japanese);
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                view.router.tab(),
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.snapshot = ready_snapshot();
            view.handle(UiAction::EditInstance, window, cx);
        });
    });
    cx.run_until_parked();
    for selector in [
        "instance-field-Name",
        "instance-field-Server",
        "edit-instance-appearance",
        "edit-instance-launch",
        "edit-instance-data",
        "cancel-instance",
        "save-instance",
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "{selector} did not paint in Japanese"
        );
    }
    let save = cx.debug_bounds("save-instance");
    assert!(
        save.is_some_and(|save| save.bottom() <= px(720.) && save.right() <= px(1152.)),
        "the Japanese save action stays inside the window: {save:?}"
    );
    // The semantic pages still open behind the localized triggers.
    click(cx, "edit-instance-launch");
    for selector in [
        "instance-field-Memory",
        "instance-field-Jvm",
        "instance-field-Game",
        "instance-runtime",
        "instance-window",
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "{selector} did not paint in Japanese"
        );
    }
}
