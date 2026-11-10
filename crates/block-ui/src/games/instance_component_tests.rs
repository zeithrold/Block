use super::{EditorIssue, SaveFailure};
use crate::{Form, InstanceEditForm};
fn draft(view: &Block) -> Option<&InstanceEditForm> {
    match &view.form {
        Some(Form::EditInstance(form)) => Some(form),
        _ => None,
    }
}
fn draft_mut(view: &mut Block) -> Option<&mut InstanceEditForm> {
    match &mut view.form {
        Some(Form::EditInstance(form)) => Some(form),
        _ => None,
    }
}
use crate::{Block, Language, Tab, UiAction, test_support::ready_snapshot};
use block_launcher::catalog::{Catalog, LoaderKind};
use gpui::{Focusable as _, TestAppContext, VisualTestContext, px, size};
use std::{
    fs,
    time::{Duration, Instant},
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

fn wait_for_save(view: &gpui::Entity<Block>, cx: &mut VisualTestContext) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while view.read_with(cx, |view, _| view.busy) && Instant::now() < deadline {
        view.update(cx, Block::poll);
        std::thread::sleep(Duration::from_millis(5));
    }
    cx.run_until_parked();
    for _ in 0..2 {
        cx.update(|window, cx| window.simulate_next_frame(cx));
        cx.run_until_parked();
    }
    assert!(!view.read_with(cx, |view, _| view.busy));
}

#[gpui::test]
fn optifine_chooser_failure_preserves_the_instance_draft_and_ignores_stale_requests(
    cx: &mut TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_path_buf())?;
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                Tab::Instances,
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.snapshot = ready_snapshot();
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        cx.simulate_resize(size(px(1152.), px(900.)));
        click(cx, "component-optifine");
        view.update(cx, |view, cx| {
            if let Some(editor) = draft(view) {
                editor
                    .name
                    .update(cx, |field, cx| field.set_value("Unsaved name", cx));
            }
        });
        click(cx, "instance-optifine-select");
        assert!(cx.did_prompt_for_paths());
        let state = view.read_with(cx, |view, _| {
            draft(view).and_then(|editor| {
                editor.optifine_request.map(|request| {
                    (
                        request,
                        editor.name.entity_id(),
                        editor.original.id.as_str().to_owned(),
                        editor.original.target_id.clone(),
                    )
                })
            })
        });
        let Some((request, field, identity, target)) = state else {
            anyhow::bail!("missing pending chooser");
        };
        view.update(cx, |view, cx| {
            assert!(
                view.finish_instance_optifine_browse(
                    request,
                    field,
                    &identity,
                    &target,
                    Err(()),
                    cx,
                )
                .is_none()
            );
        });
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert!(crate::test_support::control_bounds(cx, "instance-optifine-error").is_some());
        assert!(view.read_with(cx, |view, cx| {
            draft(view).is_some_and(|editor| {
                view.shell
                    .editor
                    .optifine_error
                    .as_ref()
                    .is_some_and(|error| error.message(Language::English).contains("file chooser"))
                    && editor.optifine_request.is_none()
                    && editor.name.read(cx).value() == "Unsaved name"
            })
        }));

        click(cx, "instance-optifine-select");
        let old = view.read_with(cx, |view, _| {
            draft(view).and_then(|editor| editor.optifine_request)
        });
        assert!(old.is_some());
        click(cx, "cancel-instance");
        assert!(view.read_with(cx, |view, _| draft(view).is_some()));
        click(cx, "editor-exit-discard");
        click(cx, "component-optifine");
        click(cx, "instance-optifine-select");
        let current = view.read_with(cx, |view, _| {
            draft(view).and_then(|editor| editor.optifine_request)
        });
        assert!(current.is_some() && current != old);
        view.update(cx, |view, cx| {
            if let Some(old) = old {
                assert!(
                    view.finish_instance_optifine_browse(
                        old,
                        field,
                        &identity,
                        &target,
                        Err(()),
                        cx,
                    )
                    .is_none()
                );
            }
            assert!(draft(view).is_some_and(|editor| {
                editor.optifine_request == current && view.shell.editor.optifine_error.is_none()
            }));
        });
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            draft(view).is_some_and(|editor| editor.optifine_request == current)
        }));
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            draft(view).is_some_and(|editor| {
                editor.optifine_request.is_none() && view.shell.editor.optifine_error.is_none()
            })
        }));
        click(cx, "instance-optifine-select");
        let state = view.read_with(cx, |view, _| {
            draft(view).and_then(|editor| {
                editor.optifine_request.map(|request| {
                    (
                        request,
                        editor.name.entity_id(),
                        editor.original.id.as_str().to_owned(),
                        editor.original.target_id.clone(),
                    )
                })
            })
        });
        let Some((request, field, identity, target)) = state else {
            anyhow::bail!("missing pending chooser");
        };
        let path = root.path().join("selected.jar");
        view.update(cx, |view, cx| {
            assert_eq!(
                view.finish_instance_optifine_browse(
                    request,
                    field,
                    &identity,
                    &target,
                    Ok(Some(path.clone())),
                    cx
                ),
                Some(path)
            );
            view.finish_instance_optifine_import(
                request,
                &identity,
                &target,
                Err(anyhow::anyhow!("invalid JAR")),
                cx,
            );
        });
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            draft(view).is_some_and(|editor| {
                editor.optifine_request.is_none()
                    && view
                        .shell
                        .editor
                        .optifine_error
                        .as_ref()
                        .is_some_and(|error| {
                            error
                                .message(Language::English)
                                .contains("Could not import OptiFine")
                        })
                    && editor.optifine.is_none()
            })
        }));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn cancelling_an_instance_draft_keeps_the_snapshot_and_restores_focus(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                Tab::Instances,
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.snapshot = ready_snapshot();
            view.focus.focus(window, cx);
            view.handle(UiAction::EditInstance, window, cx);
            if let Some(editor) = draft_mut(view) {
                editor
                    .name
                    .update(cx, |field, cx| field.set_value("Uncommitted 世界", cx));
                editor.original.fullscreen = true;
            }
        });
    });
    click(cx, "cancel-instance");
    assert!(view.read_with(cx, |view, _| draft(view).is_some()));
    click(cx, "editor-exit-discard");
    cx.update(|window, cx| {
        view.read_with(cx, |view, _| {
            assert!(draft(view).is_none() && view.focus.is_focused(window));
            assert_eq!(view.snapshot.instances[0].name, "Test game");
            assert!(
                !view.snapshot.instances[0].fullscreen && view.snapshot.instances[0].width == 1280
            );
        });
    });
}

#[gpui::test]
fn invalid_instance_dimensions_reveal_and_focus_the_field_in_a_short_window(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(1152.), px(540.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                Tab::Instances,
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.snapshot = ready_snapshot();
            view.handle(UiAction::EditInstance, window, cx);
            if let Some(editor) = draft(view) {
                editor
                    .width
                    .update(cx, |field, cx| field.set_value("bad", cx));
                editor
                    .name
                    .update(cx, |field, cx| field.set_value("Preserved draft", cx));
            }
            view.handle(UiAction::SaveInstance, window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            assert!(draft(view).is_some_and(|editor| {
                editor.width.read(cx).focus_handle(cx).is_focused(window)
                    && editor.name.read(cx).value() == "Preserved draft"
            }));
            assert!(!view.busy);
        });
    });
    let field = cx.debug_bounds("instance-field-Width");
    let body = cx.debug_bounds("instance-editor-body");
    let footer = cx.debug_bounds("instance-editor-footer");
    assert!(
        field.zip(body).is_some_and(
            |(field, body)| field.top() >= body.top() && field.bottom() <= body.bottom()
        )
    );
    assert!(
        footer.is_some_and(|footer| footer.size.height == px(97.) && footer.bottom() <= px(540.))
    );
}

#[gpui::test]
fn a_real_save_failure_keeps_the_draft_and_retry_commits_complete_settings(
    cx: &mut TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let catalog = Catalog::open(root.path())?;
        let instance = catalog.create_instance("Before".into(), "1.20.6".into(), None, None)?;
        let profile = instance.clone();
        let state = catalog.snapshot()?;
        let path = root.path().join("block.toml");
        let bytes = fs::read(&path)?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.router.navigate(Tab::Instances);
                view.router.open_record(
                    Tab::Instances,
                    crate::navigation::RecordRef::Instance("instance-a".into()),
                );
                view.snapshot = state;
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.selected_instance = Some(instance.id.clone());
                view.selected_instance = Some(profile.id.clone());
                view.router.open_record(
                    Tab::Instances,
                    crate::navigation::RecordRef::Instance(profile.id.clone()),
                );
                view.handle(UiAction::EditInstance, window, cx);
                if let Some(editor) = draft_mut(view) {
                    editor
                        .name
                        .update(cx, |field, cx| field.set_value("Saved instance", cx));
                    editor
                        .width
                        .update(cx, |field, cx| field.set_value("1536", cx));
                    editor.original.fullscreen = true;
                }
            });
        });
        assert_eq!(fs::read(&path)?, bytes);
        fs::remove_file(&path)?;
        fs::create_dir(&path)?;
        click(cx, "save-instance");
        wait_for_save(&view, cx);
        assert!(view.read_with(cx, |view, cx| {
            draft(view).is_some_and(|editor| {
                view.shell.editor.save_error.is_some()
                    && editor.name.read(cx).value() == "Saved instance"
                    && editor.width.read(cx).value() == "1536"
            })
        }));
        fs::remove_dir(&path)?;
        fs::write(&path, &bytes)?;
        click(cx, "save-instance");
        wait_for_save(&view, cx);
        assert!(view.read_with(cx, |view, _| draft(view).is_none()));
        let state = catalog.snapshot()?;
        let (saved, _) = state.selection(&profile.id)?;
        assert_eq!(saved.name, "Saved instance");
        assert_eq!(
            (saved.width, saved.height, saved.fullscreen),
            (1536, 720, true)
        );
        assert_eq!(saved.resource_directory, instance.resource_directory);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

fn optifine_jar(minecraft: &str) -> block_launcher::install::optifine::OptiFineJar {
    block_launcher::install::optifine::OptiFineJar {
        minecraft: minecraft.into(),
        version: format!("{minecraft}_HD_U_I6"),
        launchwrapper: "2.3".into(),
        forge: None,
        resource: block_launcher::Resource {
            sha1: "a".repeat(40),
            size: 123,
            url: "block-import:synthetic".into(),
            sha512: None,
            alternate_urls: Vec::new(),
        },
    }
}

#[gpui::test]
fn instance_optifine_draft_validates_removes_and_cancel_restores_the_catalog(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                Tab::Instances,
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.snapshot = ready_snapshot();
            view.handle(UiAction::EditInstance, window, cx);
        });
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let Some(editor) = draft_mut(view) else {
                return;
            };
            assert_eq!(editor.original.target_id, "vanilla");
            let request = uuid::Uuid::new_v4();
            editor.optifine_request = Some(request);
            view.finish_instance_optifine_import(
                request,
                "instance-a",
                "vanilla",
                Ok(optifine_jar("1.20.6")),
                cx,
            );
            let Some(editor) = draft(view) else {
                return;
            };
            assert!(
                editor
                    .optifine
                    .as_ref()
                    .is_some_and(|selected| selected.version == "1.20.6_HD_U_I6")
            );
            assert!(view.shell.editor.optifine_error.is_none());
            assert!((editor.optifine.as_deref() != editor.original.targets[0].optifine.as_ref()));
            assert!(view.instance_editor_dirty(cx));

            let request = uuid::Uuid::new_v4();
            let Some(editor) = draft_mut(view) else {
                return;
            };
            editor.optifine_request = Some(request);
            view.finish_instance_optifine_import(
                request,
                "instance-a",
                "vanilla",
                Ok(optifine_jar("1.21.1")),
                cx,
            );
            let Some(editor) = draft(view) else {
                return;
            };
            assert!(
                view.shell
                    .editor
                    .optifine_error
                    .as_ref()
                    .is_some_and(|error| error.message(Language::English).contains("1.21.1")),
                "{:?}",
                view.shell.editor.optifine_error
            );
            assert_eq!(
                editor
                    .optifine
                    .as_ref()
                    .map(|selected| selected.version.as_str()),
                Some("1.20.6_HD_U_I6"),
                "a rejected replacement keeps the previous draft"
            );

            view.handle(UiAction::InstanceOptiFineRemove, window, cx);
            let Some(editor) = draft(view) else {
                return;
            };
            assert!(editor.optifine.is_none());
            assert!(view.shell.editor.optifine_error.is_none());
            assert!(
                !(editor.optifine.as_deref() != editor.original.targets[0].optifine.as_ref()),
                "remove restores the original draft value"
            );

            let request = uuid::Uuid::new_v4();
            let Some(editor) = draft_mut(view) else {
                return;
            };
            editor.optifine_request = Some(request);
            view.finish_instance_optifine_import(
                request,
                "instance-a",
                "vanilla",
                Ok(optifine_jar("1.20.6")),
                cx,
            );
        });
    });
    click(cx, "cancel-instance");
    assert!(view.read_with(cx, |view, _| draft(view).is_some()));
    click(cx, "editor-exit-discard");
    cx.update(|window, cx| {
        view.read_with(cx, |view, _| {
            assert!(draft(view).is_none() && view.focus.is_focused(window));
            let target = &view.snapshot.instances[0].targets[0];
            assert!(
                target.optifine.is_none(),
                "Cancel never writes OptiFine to the catalog"
            );
            assert!(target.optifine_mod.is_none());
        });
    });
}

#[gpui::test]
fn the_generic_settings_entry_still_focuses_the_instance_name(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                Tab::Instances,
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.snapshot = ready_snapshot();
            view.handle(UiAction::EditInstance, window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            let editor = draft(view);
            assert!(editor.is_some(), "the settings entry opens the draft");
            if let Some(editor) = editor {
                assert!(editor.name.read(cx).focus_handle(cx).is_focused(window));
                assert!(
                    view.shell.editor.optifine_focus.is_none(),
                    "the generic entry names no OptiFine focus target"
                );
            }
        });
    });
}

#[gpui::test]
fn the_optifine_card_entry_reveals_and_focuses_its_action_in_a_short_window(
    cx: &mut TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.simulate_resize(size(px(1152.), px(540.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.router.navigate(Tab::Instances);
                view.router.open_record(
                    Tab::Instances,
                    crate::navigation::RecordRef::Instance("instance-a".into()),
                );
                let mut snapshot = ready_snapshot();
                snapshot.instances[0].targets[0].optifine = Some(optifine_jar("1.20.6"));
                view.snapshot = snapshot;
                view.engine = Some(engine);
                view.messages = Some(messages);
                // The card's own handle reaches `return_focus` the same way.
                view.focus.focus(window, cx);
                view.open_instance_optifine_editor(window, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            view.read_with(cx, |view, cx| {
                let editor = draft(view);
                assert!(editor.is_some(), "the OptiFine card opens the draft");
                let Some(editor) = editor else {
                    return;
                };
                assert_eq!(editor.original.target_id, "vanilla");
                assert!(
                    view.shell.editor.optifine_focus.is_some(),
                    "the OptiFine entry names a focus target"
                );
                assert_eq!(
                    window.focused(cx),
                    view.shell.editor.optifine_focus.clone(),
                    "the OptiFine card focuses its own section"
                );
                assert!(
                    !editor.name.read(cx).focus_handle(cx).is_focused(window),
                    "the OptiFine card must not fall back to the name field"
                );
                assert!(
                    !view.focus.is_focused(window),
                    "the opened draft takes the focus from the card"
                );
                assert_eq!(
                    editor.name.read(cx).value(),
                    "Test game",
                    "the draft keeps the current instance name"
                );
                assert_eq!(
                    editor
                        .optifine
                        .as_ref()
                        .map(|selected| selected.version.as_str()),
                    Some("1.20.6_HD_U_I6"),
                    "the draft keeps the installed selection"
                );
            });
        });
        let body = cx.debug_bounds("instance-editor-body");
        let section = cx.debug_bounds("instance-optifine");
        let action = cx.debug_bounds("instance-optifine-select");
        assert!(action.is_some(), "a Vanilla target offers the action");
        assert!(
            section
                .zip(body)
                .is_some_and(|(section, body)| section.top() >= body.top()),
            "the OptiFine section is revealed below the viewport top: {section:?} {body:?}"
        );
        assert!(
            action.zip(body).is_some_and(
                |(action, body)| action.top() >= body.top() && action.bottom() <= body.bottom()
            ),
            "the focused OptiFine action is inside the short viewport: {action:?} {body:?}"
        );
        crate::test_support::press(cx, "enter");
        assert!(
            cx.did_prompt_for_paths(),
            "the focused OptiFine action starts the JAR prompt"
        );
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        cx.update(|_window, cx| {
            view.update(cx, |view, cx| {
                if let Some(editor) = draft_mut(view) {
                    editor
                        .name
                        .update(cx, |field, cx| field.set_value("Uncommitted 世界", cx));
                }
            });
        });
        click(cx, "cancel-instance");
        assert!(view.read_with(cx, |view, _| draft(view).is_some()));
        click(cx, "editor-exit-discard");
        cx.update(|window, cx| {
            view.read_with(cx, |view, _| {
                assert!(draft(view).is_none());
                assert!(
                    view.focus.is_focused(window),
                    "Cancel returns focus to the triggering card"
                );
                assert_eq!(view.snapshot.instances[0].name, "Test game");
                assert_eq!(
                    view.snapshot.instances[0].targets[0]
                        .optifine
                        .as_ref()
                        .map(|selected| selected.version.as_str()),
                    Some("1.20.6_HD_U_I6"),
                    "Cancel never writes the draft"
                );
            });
        });
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn the_optifine_card_entry_reports_an_unsupported_loader_without_inventing_an_action(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(1152.), px(540.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                Tab::Instances,
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            let mut snapshot = ready_snapshot();
            snapshot.instances[0].targets[0].kind = LoaderKind::NeoForge;
            view.snapshot = snapshot;
            view.focus.focus(window, cx);
            view.open_instance_optifine_editor(window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            let editor = draft(view);
            assert!(editor.is_some(), "the OptiFine card opens the draft");
            let Some(editor) = editor else {
                return;
            };
            assert!(!matches!(
                editor.original.targets[0].kind,
                LoaderKind::Vanilla | LoaderKind::Forge
            ));
            assert!(
                view.shell.editor.optifine_focus.is_some(),
                "the OptiFine entry names a focus target"
            );
            assert_eq!(
                window.focused(cx),
                view.shell.editor.optifine_focus.clone(),
                "the loader reason itself takes the focus"
            );
        });
    });
    assert!(
        cx.debug_bounds("instance-optifine-select").is_none(),
        "a NeoForge target must not offer an OptiFine action"
    );
    assert!(cx.debug_bounds("instance-optifine-remove").is_none());
    let body = cx.debug_bounds("instance-editor-body");
    let section = cx.debug_bounds("instance-optifine");
    assert!(
        section.zip(body).is_some_and(
            |(section, body)| section.top() >= body.top() && section.bottom() <= body.bottom()
        ),
        "the reason the card entry opened for is inside the viewport: {section:?} {body:?}"
    );
}

#[gpui::test]
fn retained_instance_messages_resolve_in_the_view_language(cx: &mut TestAppContext) {
    use anyhow::Context as _;
    let result = (|| -> anyhow::Result<()> {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.preferences.language = Some(Language::Japanese);
                view.router.navigate(Tab::Instances);
                view.router.open_record(
                    Tab::Instances,
                    crate::navigation::RecordRef::Instance("instance-a".into()),
                );
                view.snapshot = ready_snapshot();
                view.handle(UiAction::EditInstance, window, cx);
                if let Some(editor) = draft(view) {
                    editor
                        .width
                        .update(cx, |field, cx| field.set_value("bad", cx));
                }
                view.handle(UiAction::SaveInstance, window, cx);
            });
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| -> anyhow::Result<()> {
            draft(view).context("draft stays open")?;
            let issue = view
                .shell
                .editor
                .error
                .clone()
                .context("the width is rejected")?;
            assert_eq!(issue, EditorIssue::Width);
            assert_eq!(
                issue.message(Language::Japanese),
                "ウィンドウ幅は 320～16384 ピクセルの範囲で指定してください"
            );
            // The draft keeps the reason, not one rendered sentence: another
            // view language resolves the same failure in its own copy.
            assert_eq!(
                issue.message(Language::English),
                "Window width must be between 320 and 16384 pixels"
            );
            Ok(())
        })?;

        // A save that never reaches the engine keeps its own notice the same way.
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                if let Some(editor) = draft(view) {
                    editor
                        .width
                        .update(cx, |field, cx| field.set_value("1280", cx));
                }
                view.handle(UiAction::SaveInstance, window, cx);
            });
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| -> anyhow::Result<()> {
                draft(view).context("draft stays open")?;
                let failure = view.shell.editor.save_error
                    .clone()
                    .context("an engine-less save reports itself")?;
                assert_eq!(failure, SaveFailure::Unavailable);
                assert_eq!(
                    failure.message(Language::Japanese),
                    "インスタンス設定を保存できませんでした。Block のデータディレクトリを確認して再起動してください。"
                );
                assert_eq!(
                    failure.message(Language::English),
                    "Instance settings could not be saved. Check Block’s data directory and restart."
                );
                Ok(())
            })?;
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn japanese_instance_editor_keeps_its_fields_and_actions(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(1152.), px(540.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.preferences.language = Some(Language::Japanese);
            view.router.navigate(Tab::Instances);
            view.router.open_record(
                Tab::Instances,
                crate::navigation::RecordRef::Instance("instance-a".into()),
            );
            view.snapshot = ready_snapshot();
            view.handle(UiAction::EditInstance, window, cx);
        });
    });
    cx.run_until_parked();
    for selector in [
        "instance-field-Name",
        "instance-optifine",
        "cancel-instance",
        "save-instance",
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "{selector} did not paint in Japanese"
        );
    }
    let footer = cx.debug_bounds("instance-editor-footer");
    assert!(
        footer.is_some_and(|footer| footer.bottom() <= px(540.) && footer.right() <= px(1152.)),
        "the Japanese footer stays inside the window: {footer:?}"
    );
}
