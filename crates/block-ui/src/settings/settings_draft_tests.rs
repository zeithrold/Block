//! Settings regressions exercise persistence, draft ownership and visible focus.

use super::{SettingsField, format_rate, parse_rate};
use crate::{Appearance, Language, Preferences, Tab, UiAction};
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

#[gpui::test]
fn storage_page_shows_the_effective_paths_without_editing_or_actions(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let parent = directory.path().canonicalize()?;
        let root = parent.join("data");
        let library = parent.join("games");
        let cache = parent.join("cache");
        let (engine, messages) = crate::engine::Engine::with_root(root.clone())?;
        block_launcher::storage::Locations {
            library: Some(library.clone()),
            cache: Some(cache.clone()),
            ..Default::default()
        }
        .save(&root)?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.simulate_resize(size(px(960.), px(600.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.router.navigate(Tab::Settings);
                view.open_settings(cx);
                view.select_settings_page(4, window, cx);
            });
        });
        settings_frame(cx);
        assert!(view.read_with(cx, |view, cx| {
            view.shell
                .storage
                .paths
                .as_ref()
                .is_ok_and(|paths| paths.library == library && paths.cache == cache)
                && view.settings_form.as_ref().is_some_and(|draft| {
                    draft.page == 4
                        && draft.fields().is_empty()
                        && !draft.is_dirty(&view.saved_preferences, cx)
                })
        }));
        let library_row = cx.debug_bounds("settings-storage-library");
        let cache_row = cx.debug_bounds("settings-storage-cache");
        assert!(
            library_row.is_some(),
            "the game storage path must be visible"
        );
        assert!(
            cache_row.is_some(),
            "the download cache path must be visible"
        );
        for control in [
            "reset-preferences",
            "storage-move-games",
            "storage-review-cleanup",
            "storage-keep-copy",
            "storage-cancel",
            "settings-storage-library-input",
            "settings-storage-cache-input",
        ] {
            assert!(
                cx.debug_bounds(control).is_none(),
                "the read-only storage page must not expose {control}"
            );
        }
        assert!(cx.debug_bounds("save-preferences").is_some());
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn corrupt_storage_configuration_is_reported_on_the_read_only_page(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        Preferences::default().save(root.path())?;
        let original_storage = b"not valid location settings";
        std::fs::write(root.path().join("block.toml"), original_storage)?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.simulate_resize(size(px(960.), px(600.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.router.navigate(Tab::Settings);
                view.open_settings(cx);
                view.select_settings_page(4, window, cx);
                assert!(view.shell.storage.paths.is_err());
                assert!(
                    view.settings_form
                        .as_ref()
                        .is_some_and(|draft| { draft.page == 4 && draft.fields().is_empty() })
                );
                view.save_settings(window, cx);
                assert!(view.shell.storage.paths.is_err());
                assert!(
                    view.settings_form
                        .as_ref()
                        .is_some_and(|draft| { draft.page == 4 && draft.fields().is_empty() })
                );
            });
        });
        settings_frame(cx);
        assert!(cx.debug_bounds("settings-storage-error").is_some());
        assert!(cx.debug_bounds("settings-storage-library").is_none());
        assert!(cx.debug_bounds("save-preferences").is_some());
        assert_eq!(
            std::fs::read(root.path().join("block.toml"))?,
            original_storage
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
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
            view.snapshot = crate::test_support::ready_snapshot();
            let saved_instances = serde_json::to_value(&view.snapshot.instances);
            assert!(saved_instances.is_ok(), "{saved_instances:?}");
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
            view.reset_settings(window, cx);
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
                serde_json::to_value(&view.snapshot.instances).ok(),
                saved_instances.ok()
            );
            view.handle(UiAction::Tab(Tab::Home), window, cx);
            view.handle(UiAction::SettingsExitDiscard, window, cx);
            assert_eq!(view.preferences, view.saved_preferences);
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.select_settings_page(1, window, cx);
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
    cx.simulate_resize(size(px(360.), px(720.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.router.navigate(Tab::Settings);
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
        field.is_some_and(|bounds| bounds.top() >= px(0.) && bounds.bottom() <= px(648.)),
        "{field:?}; scroll={scroll:?}"
    );
    assert!(
        save.is_some_and(|bounds| bounds.bottom() <= px(720.) && bounds.left() >= px(0.)),
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
                view.router.navigate(Tab::Settings);
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

#[gpui::test]
fn general_language_previews_saves_and_cancel_or_restore_use_the_shared_paths(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.handle(UiAction::Tab(Tab::Settings), window, cx);
                view.handle(UiAction::SetLanguage(Language::Japanese), window, cx);
                assert_eq!(view.preferences.language, Some(Language::Japanese));
                assert_eq!(view.saved_preferences.language, None);
                assert!(
                    view.settings_form
                        .as_ref()
                        .is_some_and(|draft| view.settings_dirty(draft, cx)),
                    "a language-only preview must offer Save"
                );

                view.save_settings(window, cx);
                assert_eq!(view.saved_preferences.language, Some(Language::Japanese));
                assert_eq!(
                    Preferences::load(root.path())
                        .ok()
                        .and_then(|value| value.language),
                    Some(Language::Japanese)
                );
                assert!(
                    view.settings_form
                        .as_ref()
                        .is_some_and(|draft| { !view.settings_dirty(draft, cx) })
                );

                // Explicit discard restores the saved preview.
                view.handle(
                    UiAction::SetLanguage(Language::SimplifiedChinese),
                    window,
                    cx,
                );
                view.handle(UiAction::Tab(Tab::Home), window, cx);
                view.handle(UiAction::SettingsExitDiscard, window, cx);
                assert_eq!(view.preferences, view.saved_preferences);
                assert_eq!(view.preferences.language, Some(Language::Japanese));

                // Restore on General selects a concrete English choice instead
                // of rerunning operating-system detection.
                view.handle(UiAction::Tab(Tab::Settings), window, cx);
                view.select_settings_page(0, window, cx);
                view.reset_settings(window, cx);
                assert_eq!(view.preferences.language, Some(Language::English));
                assert_eq!(view.saved_preferences.language, Some(Language::Japanese));
                view.save_settings(window, cx);
                assert_eq!(
                    Preferences::load(root.path())
                        .ok()
                        .and_then(|value| value.language),
                    Some(Language::English)
                );
            });
        });
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn language_preview_and_save_keep_the_live_input_and_composition(cx: &mut gpui::TestAppContext) {
    use gpui::EntityInputHandler as _;

    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.simulate_resize(size(px(360.), px(720.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.handle(UiAction::Tab(Tab::Settings), window, cx);
                view.select_settings_page(1, window, cx);
            });
        });
        settings_frame(cx);
        let input = view
            .read_with(cx, |view, _| {
                view.settings_form.as_ref().map(|draft| draft.jvm.clone())
            })
            .ok_or_else(|| anyhow::anyhow!("settings JVM input is missing"))?;
        let identity = input.entity_id();
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.focus_handle(cx).focus(window, cx);
                input.replace_and_mark_text_in_range(None, "入力", Some(0..1), window, cx);
            });
        });
        let focus = cx.update(|window, cx| window.focused(cx));
        assert!(focus.is_some());
        crate::test_support::press(cx, "alt-left");
        assert!(view.read_with(cx, |view, _| view.shell.settings_page == Some(1)));
        assert!(input.read_with(cx, |input, _| input.is_composing()));
        for (language, expected_label) in [
            (Language::SimplifiedChinese, "保存全部更改"),
            (Language::Japanese, "すべての変更を保存"),
        ] {
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle(UiAction::SetLanguage(language), window, cx);
                });
            });
            cx.run_until_parked();
            assert_eq!(cx.update(|window, cx| window.focused(cx)), focus);
            let selection = cx.update(|window, cx| {
                input.update(cx, |input, cx| {
                    input
                        .selected_text_range(false, window, cx)
                        .map(|selection| selection.range)
                })
            });
            assert_eq!(selection, Some(0..1));
            assert!(input.read_with(cx, |input, _| input.is_composing()));
            assert_eq!(input.read_with(cx, |input, _| input.value()), "入力");
            assert_eq!(
                view.read_with(cx, |view, _| {
                    view.settings_form
                        .as_ref()
                        .map(|draft| draft.jvm.entity_id())
                }),
                Some(identity)
            );
            let label = cx.update(|window, _| {
                gpui_base::test_support::find(window, &[], &"save-preferences".into())
                    .and_then(|node| node.label().map(str::to_owned))
            });
            assert_eq!(label.as_deref(), Some(expected_label));
        }
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::SavePreferences, window, cx);
                assert_eq!(view.saved_preferences.language, None);
                assert!(input.read(cx).is_composing());
                input.update(cx, |input, cx| input.unmark_text(window, cx));
                view.handle(UiAction::SavePreferences, window, cx);
                assert_eq!(
                    view.settings_form
                        .as_ref()
                        .map(|draft| draft.jvm.entity_id()),
                    Some(identity)
                );
                assert_eq!(input.read(cx).value(), "入力");
            });
        });
        assert_eq!(
            Preferences::load(root.path())?.language,
            Some(Language::Japanese)
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn retained_settings_feedback_resolves_in_the_presentation_language() {
    let error =
        super::SettingsError::at(SettingsField::Concurrent, "settings.error.concurrent_range");
    assert_eq!(
        error.message(Language::English),
        "Choose between 1 and 16 parallel downloads."
    );
    assert_ne!(
        error.message(Language::SimplifiedChinese),
        error.message(Language::English)
    );
    assert_ne!(
        error.message(Language::Japanese),
        error.message(Language::English)
    );
    assert_eq!(error.field, Some(SettingsField::Concurrent));

    let save = super::SettingsError::save("the disk is full".into());
    assert_eq!(
        save.message(Language::English),
        "Could not save settings: the disk is full"
    );
    for language in [Language::SimplifiedChinese, Language::Japanese] {
        let message = save.message(language);
        assert!(message.contains("the disk is full"), "{message}");
        assert_ne!(message, save.message(Language::English));
    }
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
        settings_click(cx, "settings-back");
        settings_click(cx, "settings-back");
        settings_click(cx, "settings-exit-discard");
        assert_eq!(view.read_with(cx, |view, _| view.router.tab()), Tab::Home);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Tab(Tab::Settings), window, cx);
                view.select_settings_page(1, window, cx);
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
    cx.simulate_resize(size(px(1152.), px(720.)));
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
                    .map_err(|error| anyhow::anyhow!("{error:?}"))?;
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
                let original = std::fs::read(root.path().join(crate::global_config::FILE))?;
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
                    std::fs::read(root.path().join(crate::global_config::FILE))?,
                    original
                );
                view.handle(UiAction::Tab(Tab::Home), window, cx);
                view.handle(UiAction::SettingsExitDiscard, window, cx);
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

#[gpui::test]
fn category_return_preserves_focus_and_drafts_and_storage_can_save_all(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.router.navigate(Tab::Home);
            view.preferences.reduce_motion = true;
            view.saved_preferences.reduce_motion = true;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        cx.simulate_resize(size(px(360.), px(720.)));
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
        });
        settings_click(cx, "settings-page-game");
        view.update(cx, |view, cx| {
            if let Some(draft) = &view.settings_form {
                draft
                    .memory
                    .update(cx, |input, cx| input.set_value("4", cx));
            }
        });
        settings_click(cx, "settings-back");
        assert!(view.read_with(cx, |view, _| view.shell.settings_page.is_none()));
        assert!(cx.update(|window, cx| {
            view.read(cx)
                .settings_form
                .as_ref()
                .is_some_and(|draft| draft.navigation_focus[1].is_focused(window))
        }));
        let game_row = cx.update(|window, _| {
            gpui_base::test_support::find(window, &[], &"settings-page-game".into())
                .and_then(|node| node.label().map(str::to_owned))
        });
        assert!(game_row.is_some_and(|label| label.contains("Modified")));
        settings_click(cx, "settings-page-downloads");
        view.update(cx, |view, cx| {
            if let Some(draft) = &view.settings_form {
                draft
                    .concurrent
                    .update(cx, |input, cx| input.set_value("3", cx));
            }
        });
        settings_click(cx, "settings-back");
        settings_click(cx, "settings-page-storage");
        assert!(
            view.read_with(cx, |view, cx| view.settings_form.as_ref().is_some_and(
                |draft| { draft.fields().is_empty() && view.settings_dirty(draft, cx) }
            ))
        );
        settings_click(cx, "save-preferences");
        let saved = Preferences::load(root.path())?;
        assert_eq!(saved.game.memory_mib, 4096);
        assert_eq!(saved.downloads.max_concurrent, 3);
        assert!(
            view.read_with(cx, |view, cx| view.router.tab() == Tab::Settings
                && view.shell.settings_page == Some(4)
                && view.settings_form.as_ref().is_some_and(|draft| {
                    !view.settings_dirty(draft, cx)
                        && draft.notice == Some(super::SettingsNotice::Saved)
                }))
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn save_and_leave_retains_invalid_and_unwritable_drafts_until_retry_succeeds(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("blocked");
        let (engine, messages) = crate::engine::Engine::with_root(root.clone())?;
        std::fs::remove_dir_all(&root)?;
        std::fs::write(&root, b"not a directory")?;
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.router.navigate(Tab::Home);
            view.preferences.reduce_motion = true;
            view.saved_preferences.reduce_motion = true;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        cx.simulate_resize(size(px(360.), px(720.)));
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
        });
        settings_click(cx, "settings-page-game");
        view.update(cx, |view, cx| {
            if let Some(draft) = &view.settings_form {
                draft
                    .memory
                    .update(cx, |input, cx| input.set_value("invalid", cx));
                draft
                    .concurrent
                    .update(cx, |input, cx| input.set_value("3", cx));
            }
        });
        settings_click(cx, "settings-back");
        settings_click(cx, "settings-back");
        settings_click(cx, "settings-exit-save");
        assert!(cx.update(
            |window, cx| view.read(cx).settings_form.as_ref().is_some_and(|draft| {
                draft.page == 1
                    && draft.memory.read(cx).value() == "invalid"
                    && draft.concurrent.read(cx).value() == "3"
                    && draft.memory.read(cx).focus_handle(cx).is_focused(window)
                    && draft
                        .error
                        .as_ref()
                        .is_some_and(|error| error.field == Some(SettingsField::Memory))
            })
        ));
        view.update(cx, |view, cx| {
            if let Some(draft) = &view.settings_form {
                draft
                    .memory
                    .update(cx, |input, cx| input.set_value("4", cx));
            }
        });
        settings_click(cx, "settings-back");
        settings_click(cx, "settings-back");
        settings_click(cx, "settings-exit-save");
        assert!(
            view.read_with(cx, |view, cx| view.router.tab() == Tab::Settings
                && view.shell.settings_page.is_none()
                && view.settings_form.as_ref().is_some_and(|draft| {
                    draft
                        .error
                        .as_ref()
                        .is_some_and(|error| error.field.is_none())
                        && draft.memory.read(cx).value() == "4"
                        && draft.concurrent.read(cx).value() == "3"
                }))
        );
        assert_eq!(std::fs::read(&root)?, b"not a directory");
        std::fs::remove_file(&root)?;
        std::fs::create_dir(&root)?;
        settings_click(cx, "settings-back");
        settings_click(cx, "settings-exit-save");
        assert_eq!(view.read_with(cx, |view, _| view.router.tab()), Tab::Home);
        let saved = Preferences::load(&root)?;
        assert_eq!(saved.game.memory_mib, 4096);
        assert_eq!(saved.downloads.max_concurrent, 3);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn native_close_and_quit_use_distinct_exit_prompts_without_discarding(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view.preferences.reduce_motion = true;
        view.saved_preferences.reduce_motion = true;
        view
    });
    cx.simulate_resize(size(px(360.), px(720.)));
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Settings), window, cx);
        view.preview_appearance(Appearance::Dark);
        assert!(!view.allow_native_window_close(window, cx));
        assert!(view.shell.surfaces.top().is_some_and(|entry| {
            matches!(&entry.kind, crate::shell::Surface::SettingsExit(exit)
                if matches!(exit.as_ref(), UiAction::CloseWindow))
                && view.surface_dialog_title(&entry.kind) == "Close Block?"
        }));
    });
    settings_click(cx, "settings-exit-stay");
    view.update_in(cx, |view, window, cx| {
        assert_eq!(view.preferences.appearance, Appearance::Dark);
        assert!(view.settings_form.is_some());
        assert!(!view.allow_application_quit(window, cx));
        assert!(view.shell.surfaces.top().is_some_and(|entry| {
            matches!(&entry.kind, crate::shell::Surface::SettingsExit(exit)
                if matches!(exit.as_ref(), UiAction::QuitApplication))
                && view.surface_dialog_title(&entry.kind) == "Quit Block?"
        }));
    });
    settings_click(cx, "settings-exit-stay");
    assert!(
        view.read_with(cx, |view, _| view.router.tab() == Tab::Settings
            && view.preferences.appearance == Appearance::Dark
            && view.settings_form.is_some())
    );
}

#[gpui::test]
fn native_close_dismisses_an_open_selector_before_routing_save_errors(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view.preferences.reduce_motion = true;
        view.saved_preferences.reduce_motion = true;
        view
    });
    cx.simulate_resize(size(px(360.), px(720.)));
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Settings), window, cx);
        view.handle(UiAction::SettingsPage(0), window, cx);
        if let Some(draft) = &view.settings_form {
            draft
                .memory
                .update(cx, |input, cx| input.set_value("invalid", cx));
        }
    });
    settings_click(cx, "settings-appearance");
    view.update_in(cx, |view, window, cx| {
        assert!(!view.allow_native_window_close(window, cx));
        assert_eq!(view.shell.surfaces.layers().count(), 1);
    });
    settings_click(cx, "settings-exit-save");
    assert!(cx.update(|window, cx| {
        let view = view.read(cx);
        !view.shell.surfaces.is_open()
            && view.shell.settings_page == Some(1)
            && view.settings_form.as_ref().is_some_and(|draft| {
                draft.memory.read(cx).value() == "invalid"
                    && draft.memory.read(cx).focus_handle(cx).is_focused(window)
            })
    }));
}

#[gpui::test]
fn returning_restores_index_and_category_scroll_positions(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view.preferences.reduce_motion = true;
        view.saved_preferences.reduce_motion = true;
        view
    });
    cx.simulate_resize(size(px(360.), px(400.)));
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Settings), window, cx);
    });
    settings_frame(cx);
    view.update(cx, |view, cx| {
        if let Some(draft) = &view.settings_form {
            draft.navigation_scroll.scroll_to_bottom();
        }
        cx.notify();
    });
    settings_frame(cx);
    let index_offset = view.read_with(cx, |view, _| {
        view.settings_form
            .as_ref()
            .map(|draft| draft.navigation_scroll.offset())
    });
    assert!(index_offset.is_some_and(|offset| offset.y < px(0.)));
    settings_click(cx, "settings-page-storage");
    settings_click(cx, "settings-back");
    assert_eq!(
        view.read_with(cx, |view, _| view
            .settings_form
            .as_ref()
            .map(|draft| draft.navigation_scroll.offset())),
        index_offset
    );
    assert!(cx.update(|window, cx| {
        view.read(cx)
            .settings_form
            .as_ref()
            .is_some_and(|draft| draft.navigation_focus[4].is_focused(window))
    }));
    crate::test_support::press(cx, "shift-tab");
    cx.update(|window, cx| window.simulate_next_frame(cx));
    settings_frame(cx);
    let downloads = cx.debug_bounds("settings-page-downloads");
    assert!(cx.update(|window, cx| {
        view.read(cx).settings_form.as_ref().is_some_and(|draft| {
            draft.navigation_focus[3].is_focused(window)
                && downloads.is_some_and(|row| {
                    let viewport = draft.navigation_scroll.bounds();
                    row.top() >= viewport.top() && row.bottom() <= viewport.bottom()
                })
        })
    }));
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::SettingsPage(1), window, cx)
    });
    settings_frame(cx);
    view.update(cx, |view, cx| {
        if let Some(draft) = &view.settings_form {
            draft.scroll.scroll_to_bottom();
        }
        cx.notify();
    });
    settings_frame(cx);
    let game_offset = view.read_with(cx, |view, _| {
        view.settings_form
            .as_ref()
            .map(|draft| draft.scroll.offset())
    });
    assert!(game_offset.is_some_and(|offset| offset.y < px(0.)));
    settings_click(cx, "settings-back");
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::SettingsPage(1), window, cx)
    });
    settings_frame(cx);
    assert_eq!(
        view.read_with(cx, |view, _| view
            .settings_form
            .as_ref()
            .map(|draft| draft.scroll.offset())),
        game_offset
    );
    assert!(cx.update(|window, cx| {
        view.read(cx)
            .settings_form
            .as_ref()
            .is_some_and(|draft| !draft.heading.is_focused(window))
    }));
    crate::test_support::press(cx, "tab");
    cx.update(|window, cx| window.simulate_next_frame(cx));
    settings_frame(cx);
    let field = cx.debug_bounds("settings-field-Memory");
    let focus_state = cx.update(|window, cx| {
        view.read(cx).settings_form.as_ref().map(|draft| {
            (
                window.focused(cx),
                draft.memory.read(cx).focus_handle(cx),
                draft.scroll.bounds(),
                draft.scroll.offset(),
            )
        })
    });
    assert!(
        cx.update(|window, cx| {
            view.read(cx).settings_form.as_ref().is_some_and(|draft| {
                draft.memory.read(cx).focus_handle(cx).is_focused(window)
                    && field.is_some_and(|field| {
                        let viewport = draft.scroll.bounds();
                        field.top() >= viewport.top() && field.bottom() <= viewport.bottom()
                    })
            })
        }),
        "focus={focus_state:?}, field={field:?}"
    );
    for (index, key) in [
        "tab",
        "tab",
        "tab",
        "tab",
        "tab",
        "tab",
        "shift-tab",
        "shift-tab",
    ]
    .into_iter()
    .enumerate()
    {
        crate::test_support::press(cx, key);
        cx.update(|window, cx| window.simulate_next_frame(cx));
        settings_frame(cx);
        let focused = cx.update(|window, cx| {
            view.read(cx).settings_form.as_ref().and_then(|draft| {
                let viewport = draft.scroll.bounds();
                draft
                    .focus_regions
                    .regions
                    .borrow()
                    .iter()
                    .find_map(|region| {
                        region.focus.contains_focused(window, cx).then_some((
                            region.id.clone(),
                            region.bounds,
                            viewport,
                        ))
                    })
            })
        });
        assert!(
            focused.as_ref().is_some_and(|(_, bounds, viewport)| {
                bounds.is_some_and(|bounds| {
                    bounds.top() >= viewport.top() && bounds.bottom() <= viewport.bottom()
                })
            }),
            "{index} {key}: {focused:?}, focused={:?}",
            cx.update(|window, cx| window.focused(cx))
        );
    }
}

#[gpui::test]
fn completed_instance_edit_survives_discarding_the_settings_draft(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let catalog = block_launcher::catalog::Catalog::open(root.path())?;
        let original = catalog.create_instance("Original".into(), "1.20.6".into(), None, None)?;
        let snapshot = catalog.snapshot()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.saved_preferences.reduce_motion = true;
            view.snapshot = snapshot;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.preview_appearance(Appearance::Dark);
            view.begin(
                msg!("games.instance_editor.saving"),
                crate::ui_logging::Operation::SaveInstance,
            );
            if let Some(engine) = &view.engine {
                engine.save_instance(
                    original.id.clone(),
                    crate::engine::InstanceDraftValues {
                        name: "Updated".into(),
                        defaults: crate::settings::GameDefaults::default(),
                        appearance: original.appearance,
                        target_id: Some(original.target_id.clone()),
                        server: original.server.clone(),
                        data_policy: original.data_policy,
                        game_directory: original.game_directory.clone(),
                        game_args: original.game_args.clone(),
                        default_account: original.default_account.clone(),
                    },
                    None,
                );
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while view.read_with(cx, |view, _| view.busy) && std::time::Instant::now() < deadline {
            view.update(cx, crate::Block::poll);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(view.read_with(cx, |view, _| !view.busy
            && view.snapshot.instances[0].name == "Updated"));
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::Tab(Tab::Home), window, cx);
            view.handle(UiAction::SettingsExitDiscard, window, cx);
        });
        assert!(view.read_with(cx, |view, _| view.router.tab() == Tab::Home
            && view.snapshot.instances[0].name == "Updated"
            && view.preferences.appearance == view.saved_preferences.appearance));
        assert_eq!(catalog.snapshot()?.instances[0].name, "Updated");
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn settings_controls_and_long_storage_paths_fit_the_home_frame(cx: &mut gpui::TestAppContext) {
    for language in [
        Language::English,
        Language::SimplifiedChinese,
        Language::Japanese,
    ] {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.router.navigate(Tab::Home);
                view.preferences.reduce_motion = true;
                view.preferences.language = Some(language);
                view.preferences.appearance = appearance;
                view.saved_preferences = view.preferences.clone();
                view
            });
            cx.simulate_resize(size(px(360.), px(720.)));
            view.update_in(cx, |view, window, cx| {
                view.handle(UiAction::Tab(Tab::Settings), window, cx);
            });
            settings_frame(cx);
            for selector in [
                "settings-back",
                "settings-page-general",
                "settings-page-game",
                "settings-page-downloads",
                "settings-page-storage",
                "save-preferences",
            ] {
                let bounds = crate::test_support::control_bounds(cx, selector);
                assert!(
                    bounds.is_some_and(|bounds| bounds.left() >= px(0.)
                        && bounds.right() <= px(360.)
                        && bounds.top() >= px(0.)
                        && bounds.bottom() <= px(720.)),
                    "{language:?} {appearance:?} {selector}: {bounds:?}"
                );
            }
            view.update_in(cx, |view, window, cx| {
                view.handle(UiAction::SettingsPage(1), window, cx);
                if let Some(draft) = &mut view.settings_form {
                    draft.java_automatic = false;
                    draft.window_custom = true;
                    draft.java.update(cx, |input, cx| {
                        input.set_value(
                            "/example/a-very-long-directory-name/runtime/java/bin/java",
                            cx,
                        );
                    });
                }
            });
            settings_frame(cx);
            for selector in [
                "settings-field-Memory",
                "settings-default-java",
                "settings-field-Java",
                "settings-java-full-path",
                "settings-field-Width",
                "settings-field-Height",
            ] {
                let bounds = cx
                    .debug_bounds(selector)
                    .or_else(|| crate::test_support::control_bounds(cx, selector));
                assert!(bounds.is_some_and(|bounds| bounds.left() >= px(0.)
                    && bounds.right() <= px(360.)),
                    "{language:?} {appearance:?} {selector}: {bounds:?}");
            }
            let width = cx.debug_bounds("settings-field-Width");
            let height = cx.debug_bounds("settings-field-Height");
            assert!(
                width
                    .zip(height)
                    .is_some_and(|(width, height)| width.bottom() <= height.top())
            );
            let footer = cx.debug_bounds("settings-footer");
            let save = crate::test_support::control_bounds(cx, "save-preferences");
            assert!(footer.zip(save).is_some_and(|(footer, save)| {
                footer.bottom() <= px(720.)
                    && save.top() >= footer.top()
                    && save.bottom() <= footer.bottom()
            }));
            view.update_in(cx, |view, window, cx| {
                view.shell.storage.paths = Ok(super::StoragePaths {
                    library: format!("/library/{}", "long-directory-name".repeat(20)).into(),
                    cache: format!("/cache/{}", "long-cache-name".repeat(20)).into(),
                });
                view.select_settings_page(4, window, cx);
            });
            settings_frame(cx);
            for selector in ["settings-storage-library", "settings-storage-cache"] {
                let bounds = cx.debug_bounds(selector);
                assert!(
                    bounds.is_some_and(|bounds| bounds.left() >= px(0.)
                        && bounds.right() <= px(360.)
                        && bounds.size.height > px(64.)),
                    "{language:?} {appearance:?} {selector}: {bounds:?}"
                );
            }
            cx.update(|window, _| window.remove_window());
        }
    }
}

#[gpui::test]
fn alt_left_dismisses_select_before_returning_to_the_index(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view.preferences.reduce_motion = true;
        view.saved_preferences.reduce_motion = true;
        view
    });
    cx.simulate_resize(size(px(360.), px(720.)));
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Settings), window, cx);
    });
    settings_click(cx, "settings-page-general");
    settings_click(cx, "settings-appearance");
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
    crate::test_support::press(cx, "alt-left");
    settings_frame(cx);
    assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()
        && view.shell.settings_page == Some(0)));
    crate::test_support::press(cx, "alt-left");
    settings_frame(cx);
    assert!(
        view.read_with(cx, |view, _| view.shell.settings_page.is_none()
            && view.router.tab() == Tab::Settings)
    );
}

#[gpui::test]
fn alt_left_dismisses_settings_dialogs_without_leaving_their_parent(cx: &mut gpui::TestAppContext) {
    use gpui_component::WindowExt as _;

    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.snapshot = crate::test_support::ready_snapshot();
        view.router.navigate(Tab::Home);
        view.preferences.reduce_motion = true;
        view.saved_preferences.reduce_motion = true;
        view
    });
    cx.simulate_resize(size(px(360.), px(720.)));
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Settings), window, cx);
        view.handle(UiAction::SettingsPage(1), window, cx);
        view.preview_appearance(Appearance::Dark);
        view.handle(UiAction::Tab(Tab::Home), window, cx);
    });
    settings_frame(cx);
    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    let initial_focus = cx.update(|window, cx| window.focused(cx));
    crate::test_support::press(cx, "shift-tab");
    settings_frame(cx);
    assert_ne!(cx.update(|window, cx| window.focused(cx)), initial_focus);
    crate::test_support::press(cx, "alt-left");
    settings_frame(cx);
    assert!(view.read_with(cx, |view, _| {
        !view.shell.surfaces.is_open() && view.shell.settings_page == Some(1)
    }));
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Home), window, cx);
    });
    settings_frame(cx);
    let initial_focus = cx.update(|window, cx| window.focused(cx));
    crate::test_support::press(cx, "tab");
    settings_frame(cx);
    assert_ne!(cx.update(|window, cx| window.focused(cx)), initial_focus);
    crate::test_support::press(cx, "alt-left");
    settings_frame(cx);
    assert!(view.read_with(cx, |view, _| {
        !view.shell.surfaces.is_open() && view.shell.settings_page == Some(1)
    }));
    view.update_in(cx, |view, window, cx| {
        view.preview_appearance(Appearance::Dark);
        view.handle(UiAction::SettingsBack, window, cx);
        view.handle(UiAction::SettingsBack, window, cx);
    });
    settings_frame(cx);
    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    crate::test_support::press(cx, "alt-left");
    settings_frame(cx);
    assert!(view.read_with(cx, |view, _| {
        !view.shell.surfaces.is_open()
            && view.shell.settings_page.is_none()
            && view.router.tab() == Tab::Settings
            && view.preferences.appearance == Appearance::Dark
    }));
}

#[gpui::test]
fn instance_deletion_confirmation_actions_fit_the_home_frame(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.snapshot = crate::test_support::ready_snapshot();
        view.router.navigate(Tab::Settings);
        view.preferences.reduce_motion = true;
        view.open_settings(cx);
        view.select_settings_page(1, window, cx);
        view.handle(UiAction::DeleteInstanceId("instance-a".into()), window, cx);
        view
    });
    cx.simulate_resize(size(px(360.), px(720.)));
    settings_frame(cx);
    let cancel = crate::test_support::control_bounds(cx, "close-drawer");
    let confirm = crate::test_support::control_bounds(cx, "confirm-delete");
    assert!(
        cancel.zip(confirm).is_some_and(|(cancel, confirm)| {
            cancel.left() >= px(0.)
                && cancel.right() <= px(360.)
                && confirm.left() >= px(0.)
                && confirm.right() <= px(360.)
                && cancel.bottom() <= confirm.top()
        }),
        "cancel={cancel:?}, confirm={confirm:?}"
    );
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Drawer(None), window, cx);
    });
    cx.simulate_resize(size(px(960.), px(720.)));
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::DeleteInstanceId("instance-a".into()), window, cx);
    });
    settings_frame(cx);
    let cancel = crate::test_support::control_bounds(cx, "close-drawer");
    let confirm = crate::test_support::control_bounds(cx, "confirm-delete");
    assert!(
        cancel.zip(confirm).is_some_and(|(cancel, confirm)| {
            cancel.left() >= px(0.)
                && confirm.right() <= px(960.)
                && cancel.bottom() <= confirm.top()
        }),
        "cancel={cancel:?}, confirm={confirm:?}"
    );
}

#[gpui::test]
fn native_settings_title_stays_stable_while_chrome_names_each_category(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Settings);
        view.preferences.language = Some(Language::English);
        view.open_settings(cx);
        view
    });
    for (page, title) in [
        (0, "General"),
        (1, "Game defaults"),
        (3, "Downloads & network"),
        (4, "Storage"),
    ] {
        view.update_in(cx, |view, window, cx| {
            view.select_settings_page(page, window, cx);
            assert_eq!(view.native_window_title(), "Block · Settings");
            assert_eq!(view.navigation_title(), title);
        });
    }
}
