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
            "save-preferences",
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
        assert!(cx.debug_bounds("save-preferences").is_none());
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
    cx.simulate_resize(size(px(1152.), px(720.)));
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
        save.is_some_and(|bounds| bounds.bottom() <= px(720.) && bounds.left() >= px(360.)),
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
        assert_eq!(view.read_with(cx, |view, _| view.router.tab()), Tab::Home);
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
