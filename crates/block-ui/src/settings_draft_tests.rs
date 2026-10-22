//! Settings regressions exercise persistence, draft ownership and visible focus.

use super::{SettingsField, format_rate, parse_rate};
use crate::{Appearance, Preferences, Tab, UiAction};
use gpui::{Focusable as _, px, size};

#[test]
fn decimal_megabytes_preserve_backend_bytes_without_float_rounding() -> anyhow::Result<()> {
    for bytes in [1, 999_999, 1_000_000, 1_048_576, 10_500_000, u64::MAX] {
        assert_eq!(parse_rate(&format_rate(Some(bytes)))?, Some(bytes));
    }
    assert_eq!(parse_rate("")?, None);
    assert_eq!(parse_rate("0")?, None);
    assert_eq!(parse_rate("0.000001")?, Some(1));
    for invalid in ["-1", "NaN", "1e6", "0.0000001", "1.1.1", "18446744073710"] {
        assert!(parse_rate(invalid).is_err(), "{invalid}");
    }
    Ok(())
}

/// Wait for the storage worker's real end state.
///
/// `pending` clears inside the `StorageFinished` handler, but the widget state a
/// caller asserts on is settled by the same wake-up. Requiring the finished
/// state to hold across two polls keeps a test from racing a state that only
/// settles one poll later, and a timeout names the condition that never held.
fn wait_storage(
    view: &gpui::Entity<crate::Block>,
    cx: &mut gpui::VisualTestContext,
) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut settled = 0;
    loop {
        view.update(cx, crate::Block::poll);
        let (pending, busy) = view.read_with(cx, |view, _| {
            (view.shell.storage.pending.is_some(), view.busy)
        });
        if pending || busy {
            settled = 0;
        } else {
            settled += 1;
            if settled >= 2 {
                return Ok(());
            }
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "Storage worker did not finish: pending={pending} busy={busy}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[gpui::test]
fn unreadable_or_unrelated_storage_results_cannot_claim_success(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    view.update(cx, |view, cx| {
        let request = super::StorageRequest {
            id: uuid::Uuid::new_v4(),
            operation: view.operation_revision,
            draft: 0,
            library: String::new(),
            cache: String::new(),
            preferences: view.saved_preferences.clone(),
        };
        view.shell.storage.pending = Some(request.clone());
        view.busy = true;
        view.status = "Still relocating".into();
        let mut unrelated = request.clone();
        unrelated.id = uuid::Uuid::new_v4();
        view.storage_finished(
            super::StorageFinished {
                request: unrelated,
                result: Ok("Wrong task".into()),
                locations: Ok(Default::default()),
                cleanup: Ok(None),
                preferences: Ok(view.saved_preferences.clone()),
            },
            cx,
        );
        assert!(view.busy && view.shell.storage.pending.is_some());
        assert_eq!(view.status, "Still relocating");
        view.storage_finished(
            super::StorageFinished {
                request,
                result: Ok("Worker ended".into()),
                locations: Err("Unreadable saved locations".into()),
                cleanup: Ok(None),
                preferences: Ok(view.saved_preferences.clone()),
            },
            cx,
        );
        assert!(!view.busy && view.shell.storage.locations.is_err());
        assert!(
            view.shell
                .storage
                .error
                .as_ref()
                .is_some_and(|error| error.contains("could not be read"))
        );
        assert!(
            view.shell
                .toast
                .as_ref()
                .is_some_and(|toast| matches!(toast.kind, crate::feedback::ToastKind::Error))
        );
    });
}

#[gpui::test]
fn corrupt_storage_configuration_stays_visible_and_is_never_reset_by_settings_save(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        Preferences::default().save(root.path())?;
        let original_preferences = std::fs::read(root.path().join("preferences.json"))?;
        std::fs::write(
            root.path().join("storage.json"),
            b"not valid location settings",
        )?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.tab = Tab::Settings;
                view.open_settings(cx);
                view.select_settings_page(4, window, cx);
                view.save_settings(window, cx);
                assert!(view.shell.storage.locations.is_err());
                assert!(view.shell.storage.pending.is_none());
                assert!(
                    view.settings_form
                        .as_ref()
                        .is_some_and(|draft| draft.error.is_some())
                );
            })
        });
        assert_eq!(
            std::fs::read(root.path().join("storage.json"))?,
            b"not valid location settings"
        );
        assert_eq!(
            std::fs::read(root.path().join("preferences.json"))?,
            original_preferences
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn storage_worker_saves_real_locations_preserves_edited_drafts_and_keeps_old_copies(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let parent = directory.path().canonicalize()?;
        let root = parent.join("control");
        let catalog = block_launcher::catalog::Catalog::open(&root)?;
        let (old, _) = catalog.create_instance("Existing".into(), "1.20.6".into(), None, None)?;
        std::fs::create_dir(root.join("objects"))?;
        std::fs::write(root.join("objects/synthetic"), b"reusable cache fixture")?;
        let cache = parent.join("cache");
        let library = parent.join("games");
        let later = parent.join("later-draft");
        let (engine, messages) = crate::engine::Engine::with_root(root.clone())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.snapshot = catalog.snapshot().unwrap_or_default();
                view.tab = Tab::Settings;
                view.open_settings(cx);
                view.select_settings_page(4, window, cx);
                if let Some(draft) = &view.settings_form {
                    draft.library.update(cx, |field, cx| {
                        field.set_value(&library.display().to_string(), cx)
                    });
                    draft.cache.update(cx, |field, cx| {
                        field.set_value(&cache.display().to_string(), cx)
                    });
                }
                assert!(view.storage_dirty(cx));
                view.save_settings(window, cx);
                assert!(view.busy && view.shell.storage.pending.is_some());
                if let Some(draft) = &view.settings_form {
                    draft.library.update(cx, |field, cx| {
                        field.set_value(&later.display().to_string(), cx)
                    });
                }
            })
        });
        wait_storage(&view, cx)?;
        assert!(
            !view.read_with(cx, |view, _| view.busy),
            "the storage worker must clear its busy state before the test asserts"
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.storage.error.clone()),
            None,
            "a successful relocation must not record an error"
        );
        assert!(
            view.read_with(cx, |view, _| view.shell.storage.cleanup.is_some()),
            "a relocation that retained the previous copy must expose its cleanup"
        );
        assert!(
            view.read_with(cx, |view, cx| view.storage_dirty(cx)),
            "the draft edited after saving must remain dirty"
        );
        assert_eq!(
            view.read_with(cx, |view, cx| view
                .settings_form
                .as_ref()
                .map(|draft| draft.library.read(cx).value())),
            Some(later.display().to_string()),
            "the current draft must keep the path edited after saving"
        );
        let saved = block_launcher::storage::Locations::load(&root)?;
        assert_eq!(saved.library, Some(library.clone()));
        assert_eq!(saved.cache, Some(cache.clone()));
        assert_eq!(
            std::fs::read(cache.join("objects/synthetic"))?,
            b"reusable cache fixture"
        );
        assert!(root.join("objects/synthetic").exists());
        assert_eq!(catalog.snapshot()?.instances[0].directory, old.directory);
        let (new, _) = catalog.create_instance("New".into(), "1.20.6".into(), None, None)?;
        assert!(new.directory.starts_with(&library));
        let (reopened, reopened_messages) = crate::engine::Engine::with_root(root.clone())?;
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.engine = Some(reopened);
                view.messages = Some(reopened_messages);
                view.handle(UiAction::Tab(Tab::Home), window, cx);
                view.handle(UiAction::Tab(Tab::Settings), window, cx);
                assert!(!view.storage_dirty(cx));
                if let Some(cleanup) = &view.shell.storage.cleanup {
                    view.clean_storage(cleanup.id, false, window, cx);
                }
            })
        });
        wait_storage(&view, cx)?;
        assert!(
            view.read_with(cx, |view, _| view.shell.storage.cleanup.is_none()
                && !view.busy)
        );
        assert!(root.join("objects/synthetic").exists());
        assert!(block_launcher::storage::cleanup_status(&root)?.is_none());
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn storage_errors_cancel_and_late_completion_preserve_current_drafts_and_real_state(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.tab = Tab::Settings;
            view.open_settings(cx);
            view.select_settings_page(4, window, cx);
            if let Some(draft) = &view.settings_form {
                draft
                    .cache
                    .update(cx, |field, cx| field.set_value("relative/cache", cx));
            }
            view.apply_storage(false, window, cx);
            assert!(view.shell.storage.pending.is_none());
            assert!(
                view.settings_form
                    .as_ref()
                    .is_some_and(|draft| draft.storage_paths(cx).is_err())
            );
            let request = super::StorageRequest {
                id: uuid::Uuid::new_v4(),
                operation: view.operation_revision,
                draft: view
                    .settings_form
                    .as_ref()
                    .map_or(0, |draft| draft.library.entity_id().as_u64()),
                library: String::new(),
                cache: "relative/cache".into(),
                preferences: view.saved_preferences.clone(),
            };
            view.shell.storage.pending = Some(request.clone());
            view.busy = true;
            view.operation_cancellable = true;
            view.handle(UiAction::Stop, window, cx);
            assert!(view.cancel.is_cancelled());
            view.handle(UiAction::Tab(Tab::Home), window, cx);
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            if let Some(draft) = &view.settings_form {
                draft
                    .library
                    .update(cx, |field, cx| field.set_value("/new-draft", cx));
            }
            let identity = view
                .settings_form
                .as_ref()
                .map(|draft| draft.library.entity_id());
            view.storage_finished(
                super::StorageFinished {
                    request,
                    result: Err("Storage relocation cancelled".into()),
                    locations: Ok(Default::default()),
                    cleanup: Ok(None),
                    preferences: Ok(view.saved_preferences.clone()),
                },
                cx,
            );
            assert!(!view.busy);
            assert!(
                view.shell
                    .storage
                    .error
                    .as_ref()
                    .is_some_and(|error| error.contains("cancelled"))
            );
            assert!(
                view.settings_form
                    .as_ref()
                    .is_some_and(|draft| Some(draft.library.entity_id()) == identity
                        && draft.library.read(cx).value() == "/new-draft")
            );
            assert_eq!(view.tab, Tab::Settings);
        })
    });
}

#[gpui::test]
fn restore_only_current_section_and_cancel_restore_the_saved_preview(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.saved_preferences.onboarding_complete = true;
            view.saved_preferences.game.memory_mib = 4096;
            view.preferences.clone_from(&view.saved_preferences);
            view.snapshot.profile_mode = block_launcher::catalog::ProfileMode::Separated;
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.preview_appearance(Appearance::Dark);
            if let Some(draft) = &view.settings_form {
                draft
                    .memory
                    .update(cx, |input, cx| input.set_value("8", cx));
                draft
                    .concurrent
                    .update(cx, |input, cx| input.set_value("3", cx));
            }
            view.select_settings_page(1, window, cx);
            view.reset_settings(cx);
            assert!(view.settings_form.as_ref().is_some_and(|draft| {
                draft.page == 1
                    && draft.memory.read(cx).value() == "2"
                    && draft.concurrent.read(cx).value() == "3"
                    && draft.appearance == Appearance::Dark
                    && draft.is_dirty(&view.saved_preferences, cx)
            }));
            assert_eq!(view.saved_preferences.game.memory_mib, 4096);
            assert!(view.saved_preferences.onboarding_complete);
            assert_eq!(view.preferences.game.memory_mib, 4096);
            assert_eq!(
                view.snapshot.profile_mode,
                block_launcher::catalog::ProfileMode::Separated
            );
            view.handle(UiAction::Tab(Tab::Home), window, cx);
            assert_eq!(view.preferences, view.saved_preferences);
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            assert!(view.settings_form.as_ref().is_some_and(|draft| {
                draft.page == 1
                    && draft.memory.read(cx).value() == "4"
                    && !draft.is_dirty(&view.saved_preferences, cx)
            }));
        });
    });
}

#[gpui::test]
fn a_hidden_invalid_section_is_opened_and_its_exact_field_receives_focus(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(360.), px(540.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.tab = Tab::Settings;
            view.open_settings(cx);
            view.preview_reduce_motion();
            if let Some(draft) = &view.settings_form {
                draft.width.update(cx, |input, cx| input.set_value("0", cx));
            }
            view.select_settings_page(3, window, cx);
            view.save_settings(window, cx);
            assert!(view.settings_form.as_ref().is_some_and(|draft| {
                draft.page == 1
                    && draft
                        .error
                        .as_ref()
                        .is_some_and(|error| error.field == Some(SettingsField::Width))
                    && draft.width.read(cx).focus_handle(cx).is_focused(window)
                    && draft.width.read(cx).value() == "0"
            }));
            assert_eq!(view.saved_preferences, Preferences::default());
        });
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    let field = cx.debug_bounds("settings-field-Width");
    let save = cx.debug_bounds("save-preferences");
    let scroll = view.read_with(cx, |view, _| {
        view.settings_form
            .as_ref()
            .map(|draft| draft.scroll.clone())
    });
    assert!(
        field.is_some_and(|bounds| bounds.top() >= px(0.) && bounds.bottom() <= px(484.)),
        "{field:?}; scroll={scroll:?}"
    );
    assert!(
        save.is_some_and(|bounds| bounds.bottom() <= px(540.) && bounds.left() >= px(56.)),
        "{save:?}"
    );
}

#[gpui::test]
fn save_failure_keeps_all_drafts_and_preview_without_changing_disk(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("blocked");
        let (engine, messages) = crate::engine::Engine::with_root(root.clone())?;
        std::fs::remove_dir_all(&root)?;
        std::fs::write(&root, b"not a directory")?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.tab = Tab::Settings;
                view.open_settings(cx);
                view.preview_appearance(Appearance::Dark);
                view.select_settings_page(3, window, cx);
                if let Some(draft) = &view.settings_form {
                    draft
                        .concurrent
                        .update(cx, |input, cx| input.set_value("3", cx));
                }
                view.save_settings(window, cx);
                assert!(view.settings_form.as_ref().is_some_and(|draft| {
                    draft.page == 3
                        && draft.concurrent.read(cx).value() == "3"
                        && draft.appearance == Appearance::Dark
                        && draft.error.is_some()
                        && draft.is_dirty(&view.saved_preferences, cx)
                }));
                assert_eq!(view.saved_preferences, Preferences::default());
                assert_eq!(view.preferences.appearance, Appearance::Dark);
            });
        });
        assert_eq!(std::fs::read(&root)?, b"not a directory");
        std::fs::remove_file(&root)?;
        std::fs::create_dir(&root)?;
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.save_settings(window, cx);
                assert_eq!(view.saved_preferences.appearance, Appearance::Dark);
                assert_eq!(view.saved_preferences.downloads.max_concurrent, 3);
                assert!(view.settings_form.as_ref().is_some_and(|draft| {
                    draft.page == 3
                        && draft.error.is_none()
                        && !draft.is_dirty(&view.saved_preferences, cx)
                }));
            });
        });
        let saved = Preferences::load(&root)?;
        assert_eq!(saved.appearance, Appearance::Dark);
        assert_eq!(saved.downloads.max_concurrent, 3);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn outgoing_snapshot_owns_independent_inputs_scroll_and_reveal_state(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    view.update(cx, |view, cx| {
        view.open_settings(cx);
        if let Some(draft) = &view.settings_form {
            draft.scroll.set_offset(gpui::point(px(0.), px(-24.)));
            draft.reveal.set(Some(SettingsField::Width));
        }
        let frozen = view.settings_form.as_ref().map(|draft| draft.snapshot(cx));
        if let Some(draft) = &view.settings_form {
            draft
                .memory
                .update(cx, |input, cx| input.set_value("8", cx));
            draft.scroll.set_offset(gpui::point(px(0.), px(-96.)));
            draft.reveal.set(None);
        }
        assert!(frozen.as_ref().is_some_and(|frozen| {
            view.settings_form.as_ref().is_some_and(|live| {
                frozen.memory.entity_id() != live.memory.entity_id()
                    && frozen.memory.read(cx).value() == "2"
                    && live.memory.read(cx).value() == "8"
                    && frozen.scroll.offset().y == px(-24.)
                    && live.scroll.offset().y == px(-96.)
                    && frozen.reveal.get() == Some(SettingsField::Width)
                    && live.reveal.get().is_none()
            })
        }));
    });
}

fn settings_frame(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn settings_click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    settings_frame(cx);
    let bounds = crate::test_support::control_bounds(cx, selector);
    assert!(bounds.is_some(), "missing Settings consumer: {selector}");
    if let Some(bounds) = bounds {
        let window_count = cx.windows().len();
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        if cx.windows().len() > window_count
            && let Some(window) = cx.windows().last()
        {
            *cx = gpui::VisualTestContext::from_window(*window, &cx.cx);
        }
        crate::test_support::ensure_live_window(cx);
    }
    settings_frame(cx);
}

#[gpui::test]
fn memory_stepper_uses_one_gb_steps_preserves_cancel_and_really_saves_preferences(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.simulate_resize(size(px(960.), px(600.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.preferences.reduce_motion = true;
                view.saved_preferences.reduce_motion = true;
                view.handle(UiAction::Tab(Tab::Settings), window, cx);
                view.select_settings_page(1, window, cx);
            });
        });
        settings_click(cx, "settings-memory-increase");
        assert!(view.read_with(cx, |view, cx| {
            view.settings_form.as_ref().is_some_and(|draft| {
                draft.memory.read(cx).value() == "3" && draft.is_dirty(&view.saved_preferences, cx)
            })
        }));
        assert_eq!(
            view.read_with(cx, |view, _| view.saved_preferences.game.memory_mib),
            2048
        );
        settings_click(cx, "cancel-preferences");
        assert_eq!(view.read_with(cx, |view, _| view.tab), Tab::Home);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Tab(Tab::Settings), window, cx);
                if let Some(draft) = &view.settings_form {
                    assert_eq!(draft.memory.read(cx).value(), "2");
                    draft
                        .memory
                        .update(cx, |field, cx| field.set_value("1.5", cx));
                }
            });
        });
        settings_click(cx, "settings-memory-increase");
        assert!(view.read_with(cx, |view, cx| {
            view.settings_form
                .as_ref()
                .is_some_and(|draft| draft.memory.read(cx).value() == "2.5")
        }));
        settings_click(cx, "save-preferences");
        assert_eq!(Preferences::load(root.path())?.game.memory_mib, 2560);
        assert!(view.read_with(cx, |view, cx| {
            view.settings_form
                .as_ref()
                .is_some_and(|draft| !draft.is_dirty(&view.saved_preferences, cx))
        }));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn memory_keyboard_bounds_composition_and_window_range_hints_are_real_consumers(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::EntityInputHandler as _;
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(360.), px(540.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.select_settings_page(1, window, cx);
            if let Some(draft) = &mut view.settings_form {
                draft.window_custom = true;
                draft.memory.read(cx).focus_handle(cx).focus(window, cx);
            }
        });
    });
    settings_frame(cx);
    crate::test_support::press(cx, "up");
    assert!(view.read_with(cx, |view, cx| {
        view.settings_form
            .as_ref()
            .is_some_and(|draft| draft.memory.read(cx).value() == "3")
    }));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(draft) = &view.settings_form {
                draft.memory.update(cx, |field, cx| {
                    field.replace_and_mark_text_in_range(None, "中文", None, window, cx);
                });
            }
            let before = view
                .settings_form
                .as_ref()
                .map(|draft| draft.memory.read(cx).value());
            view.adjust_settings_memory(true, cx);
            assert_eq!(
                view.settings_form
                    .as_ref()
                    .map(|draft| draft.memory.read(cx).value()),
                before
            );
        });
    });
    crate::test_support::press(cx, "up");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(draft) = &view.settings_form {
                draft.memory.update(cx, |field, cx| {
                    field.unmark_text(window, cx);
                    field.set_value("32", cx);
                });
            }
            view.adjust_settings_memory(true, cx);
            assert_eq!(
                view.settings_form
                    .as_ref()
                    .map(|draft| draft.memory.read(cx).value()),
                Some("32".into())
            );
            if let Some(draft) = &view.settings_form {
                draft
                    .memory
                    .update(cx, |field, cx| field.set_value("1", cx));
            }
            view.adjust_settings_memory(false, cx);
            assert_eq!(
                view.settings_form
                    .as_ref()
                    .map(|draft| draft.memory.read(cx).value()),
                Some("1".into())
            );
            if let Some(draft) = &view.settings_form {
                assert_eq!(draft.width.read(cx).value(), "1280");
                draft.width.read(cx).focus_handle(cx).focus(window, cx);
                draft.reveal.set(Some(SettingsField::Width));
            }
        });
    });
    settings_frame(cx);
}

#[gpui::test]
fn manual_proxy_drafts_preserve_socks_dns_mode_through_persistence_and_cancel(
    cx: &mut gpui::TestAppContext,
) {
    use anyhow::Context as _;
    let result = (|| -> anyhow::Result<()> {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let root = tempfile::tempdir()?;
        cx.update(|window, cx| {
            view.update(cx, |view, cx| -> anyhow::Result<()> {
                view.handle(UiAction::Tab(Tab::Settings), window, cx);
                view.handle(UiAction::ProxyMode(2), window, cx);
                let draft = view.settings_form.as_ref().context("Settings draft")?;
                draft.proxy_url.update(cx, |input, cx| {
                    input.set_value("  socks5h://127.0.0.1:1080  ", cx)
                });
                let parsed = draft
                    .parse(&view.saved_preferences, cx)
                    .map_err(|error| anyhow::anyhow!(error.message))?;
                assert_eq!(
                    parsed.downloads.proxy,
                    block_launcher::ProxyMode::Socks5 {
                        url: "socks5h://127.0.0.1:1080".into()
                    }
                );
                assert_eq!(
                    view.saved_preferences.downloads.proxy,
                    block_launcher::ProxyMode::System
                );
                parsed.save(root.path())?;
                let original = std::fs::read(root.path().join("preferences.json"))?;
                view.saved_preferences = Preferences::load(root.path())?;
                view.preferences.clone_from(&view.saved_preferences);
                view.open_settings(cx);
                let draft = view
                    .settings_form
                    .as_ref()
                    .context("Reopened settings draft")?;
                assert_eq!(draft.proxy_mode, 2);
                assert_eq!(draft.proxy_url.read(cx).value(), "socks5h://127.0.0.1:1080");
                draft.proxy_url.update(cx, |input, cx| {
                    input.set_value("socks5://user:secret@localhost:1080", cx)
                });
                assert!(draft.parse_downloads(cx).is_err());
                assert_eq!(
                    std::fs::read(root.path().join("preferences.json"))?,
                    original
                );
                view.handle(UiAction::Tab(Tab::Home), window, cx);
                assert_eq!(view.preferences, view.saved_preferences);
                assert_eq!(view.preferences.downloads.proxy, parsed.downloads.proxy);
                Ok(())
            })
        })
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn settings_memory_validation_uses_gb_and_enforces_the_one_to_thirty_two_gb_bounds(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.select_settings_page(1, window, cx);
            let Some(draft) = view.settings_form.as_ref() else {
                return;
            };
            let draft = draft.snapshot(cx);
            for (value, expected) in [
                ("0.5", None),
                ("33", None),
                ("1", Some(1024)),
                ("32", Some(32768)),
                ("1.5", Some(1536)),
            ] {
                draft
                    .memory
                    .update(cx, |field, cx| field.set_value(value, cx));
                match expected {
                    Some(mib) => assert_eq!(
                        draft.parse_game(cx).ok().map(|game| game.memory_mib),
                        Some(mib),
                        "{value}"
                    ),
                    None => assert!(draft.parse_game(cx).is_err(), "{value}"),
                }
            }
        });
    });
}
