use super::*;
use crate::{
    game_wizard::{CreationKind, LoaderCatalog, WizardStep},
    modpack_import::{Phase, State, TargetDraft},
};
use block_launcher::install::fabric::LoaderVersion;
use gpui::{Focusable as _, Modifiers, ScrollDelta, ScrollWheelEvent, size};

fn settle(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn settle_tab(cx: &mut gpui::VisualTestContext) {
    for _ in 0..2 {
        cx.update(|window, cx| window.simulate_next_frame(cx));
        cx.run_until_parked();
    }
    settle(cx);
}

#[gpui::test]
fn menu_entrance_forwards_the_first_typeahead_once_without_waiting_for_animation(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view
    });
    crate::test_support::press(cx, "tab enter a");
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.choices[state.active].label == "Accounts"))), "{:?}", view.read_with(cx, |view, _| view.shell.surfaces.top().map(|layer| format!("{:?}", layer.kind))));
    crate::test_support::press(cx, "enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.router.tab()),
        Tab::Accounts
    );
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.top().is_none()));
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) -> Option<Bounds<Pixels>> {
    settle(cx);
    let bounds = crate::test_support::control_bounds(cx, selector);
    assert!(
        bounds.is_some(),
        "missing actual product control {selector}"
    );
    if let Some(bounds) = bounds {
        let window_count = cx.windows().len();
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
        if cx.windows().len() > window_count
            && let Some(window) = cx.windows().last()
        {
            *cx = gpui::VisualTestContext::from_window(*window, &cx.cx);
        }
        crate::test_support::ensure_live_window(cx);
        cx.run_until_parked();
        crate::test_support::ensure_live_window(cx);
    }
    settle(cx);
    bounds
}

fn assert_popup_within_window(cx: &mut gpui::VisualTestContext) {
    let popup = cx.debug_bounds("drawer-sheet");
    assert!(
        popup.is_some_and(|bounds| bounds.left() >= px(8.)
            && bounds.right() <= px(1144.)
            && bounds.top() >= px(8.)
            && bounds.bottom() <= px(712.)),
        "popup={popup:?}"
    );
}

#[gpui::test]
fn settings_select_anchors_to_trigger_and_keeps_inspection_separate_from_value(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(1152.), px(720.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.handle(UiAction::SettingsPage(0), window, cx);
        });
    });
    settle(cx);

    let bounds = click(cx, "settings-appearance");
    let popup = cx.debug_bounds("drawer-sheet");
    // GPUI snaps the final popup layout onto the physical pixel grid.
    assert!(
        bounds
            .zip(popup)
            .is_some_and(|(trigger, popup)| trigger.size.width == popup.size.width
                && (trigger.left() - popup.left()).abs() <= px(1.)),
        "trigger={bounds:?}; popup={popup:?}"
    );
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(|layer| {
        matches!(&layer.kind, Surface::Select(state) if state.active == 0 && state.choices[0].selected && bounds.is_some_and(|bounds| state.anchor == point(bounds.left(), bounds.bottom() + px(6.))))
    })), "trigger={bounds:?}; surface={:?}", view.read_with(cx, |view, _| view.shell.surfaces.top().cloned()));
    crate::test_support::press(cx, "end");
    assert!(view.read_with(cx, |view, _| {
        view.preferences.appearance == crate::Appearance::System && view.shell.surfaces.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.active == 2 && state.choices[0].selected && !state.choices[2].selected))
    }));

    settle(cx);
    assert_popup_within_window(cx);
    crate::test_support::press(cx, "escape");
    settle(cx);
    assert!(view.read_with(cx, |view, _| view.preferences.appearance
        == crate::Appearance::System
        && !view.shell.surfaces.is_open()));
    let trigger_focus = cx.update(|window, cx| window.focused(cx));
    click(cx, "settings-appearance");

    assert_popup_within_window(cx);
    crate::test_support::press(cx, "escape");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger_focus);
    click(cx, "settings-appearance");
    crate::test_support::press(cx, "end enter");

    assert!(view.read_with(cx, |view, _| view.preferences.appearance
        == crate::Appearance::Dark
        && view.saved_preferences.appearance == crate::Appearance::System
        && !view.shell.surfaces.is_open()));
}

#[gpui::test]
fn settings_select_tab_closes_and_traverses_once_in_both_directions(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(1152.), px(720.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.handle(UiAction::SettingsPage(0), window, cx);
        });
    });
    settle(cx);
    click(cx, "settings-appearance");
    crate::test_support::press(cx, "escape");
    settle(cx);
    let trigger = cx.update(|window, cx| window.focused(cx));
    assert!(trigger.is_some(), "Escape restores the actual trigger");
    let Some(trigger) = trigger else {
        return;
    };
    for keys in ["tab", "shift-tab"] {
        cx.update(|window, cx| trigger.focus(window, cx));
        crate::test_support::press(cx, keys);
        settle(cx);
        let expected = cx.update(|window, cx| window.focused(cx));
        assert_ne!(expected, Some(trigger.clone()), "{keys} moves focus");
        click(cx, "settings-appearance");
        crate::test_support::press(cx, keys);
        settle_tab(cx);
        assert!(!select_open(&view, cx), "{keys} closes the popup");
        assert_eq!(
            cx.update(|window, cx| window.focused(cx)),
            expected,
            "{keys}"
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.preferences.appearance),
            crate::Appearance::System
        );
    }
}

#[gpui::test]
fn drawer_select_tab_closes_only_the_popup_and_stays_in_the_sheet(cx: &mut gpui::TestAppContext) {
    use crate::shell::Drawer;

    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view
    });
    cx.simulate_resize(size(px(360.), px(720.)));
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Drawer(Some(Drawer::Language)), window, cx);
    });
    settle(cx);
    let before = view.read_with(cx, |view, _| view.preferences.language);
    // The drawer no longer offers a nested selector. Exercise the existing
    // layered-host contract through the shared settings-language action.
    view.update_in(cx, |view, window, cx| {
        view.handle(
            UiAction::OpenProductSelect(ProductChoice::Language),
            window,
            cx,
        );
    });
    settle(cx);
    assert!(
        select_open(&view, cx),
        "the shared selector must really open"
    );
    crate::test_support::press(cx, "escape");
    settle(cx);
    assert!(view.read_with(cx, |view, _| matches!(
        view.shell.surfaces.top().map(|layer| &layer.kind),
        Some(Surface::Drawer(Drawer::Language))
    )));
    let trigger = cx.update(|window, cx| window.focused(cx));
    assert!(
        trigger.is_some(),
        "Escape returns focus to the sheet trigger"
    );
    let Some(trigger) = trigger else {
        return;
    };
    for keys in ["tab", "shift-tab"] {
        cx.update(|window, cx| trigger.focus(window, cx));
        crate::test_support::press(cx, keys);
        settle(cx);
        let expected = cx.update(|window, cx| window.focused(cx));
        cx.update(|window, cx| trigger.focus(window, cx));
        view.update_in(cx, |view, window, cx| {
            view.handle(
                UiAction::OpenProductSelect(ProductChoice::Language),
                window,
                cx,
            );
        });
        settle(cx);
        assert!(
            select_open(&view, cx),
            "the shared selector must really reopen"
        );
        crate::test_support::press(cx, keys);
        settle_tab(cx);
        assert!(!select_open(&view, cx), "{keys} closes only the selector");
        assert_eq!(
            cx.update(|window, cx| window.focused(cx)),
            expected,
            "{keys}"
        );
        assert!(view.read_with(cx, |view, _| matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(Surface::Drawer(Drawer::Language))
        )));
        assert_eq!(
            view.read_with(cx, |view, _| view.preferences.language),
            before,
            "{keys} chooses no language"
        );
        assert!(
            cx.update(|window, cx| {
                gpui_base::active_focus_trap(window, cx)
                    .is_some_and(|trap| trap.contains_focused(window, cx))
            }),
            "{keys} keeps focus inside the sheet"
        );
    }
    crate::test_support::press(cx, "escape");
    settle(cx);
    assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()));
}

#[gpui::test]
fn tab_in_a_dialog_never_closes_the_select_beneath_it(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(1152.), px(720.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.handle(UiAction::SettingsPage(0), window, cx);
        });
    });
    click(cx, "settings-appearance");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.handle(UiAction::GamesHelp, window, cx));
    });
    settle(cx);
    for keys in ["tab", "shift-tab"] {
        crate::test_support::press(cx, keys);
        settle_tab(cx);
        assert!(view.read_with(cx, |view, _| {
            matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::GamesHelp)
            ) && view
                .shell
                .surfaces
                .layers()
                .any(|layer| matches!(layer.kind, Surface::Select(_)))
        }));
    }
    crate::test_support::press(cx, "escape");
    settle(cx);
    assert!(select_open(&view, cx), "Escape closes the top dialog only");
    crate::test_support::press(cx, "escape");
    settle(cx);
    assert!(
        !select_open(&view, cx),
        "the second Escape closes the selector"
    );
}

#[gpui::test]
fn settings_language_select_previews_cancels_and_saves(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = crate::Engine::with_root(root.path().to_path_buf())?;
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        cx.simulate_resize(size(px(1152.), px(720.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Tab(Tab::Settings), window, cx)
            });
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::SettingsPage(0), window, cx)
            });
        });
        click(cx, "settings-language");
        assert!(view.read_with(cx, |view, _| {
            view.shell.surfaces.top().is_some_and(|layer| matches!(
                &layer.kind,
                Surface::Select(state)
                    if state.product == Some(ProductChoice::Language)
                        && state.choices.iter().map(|choice| choice.label.as_str()).collect::<Vec<_>>()
                            == ["English", "简体中文", "日本語"]
                        && state.choices[0].selected
            ))
        }));
        click(cx, "select-choice-2");
        assert!(view.read_with(cx, |view, _| {
            view.preferences.language == Some(Language::Japanese)
                && view.saved_preferences.language.is_none()
        }));
        assert_eq!(crate::Preferences::load(root.path())?.language, None);

        click(cx, "settings-language");
        click(cx, "select-choice-1");
        assert_eq!(
            view.read_with(cx, |view, _| view.preferences.language),
            Some(Language::SimplifiedChinese)
        );
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Tab(Tab::Home), window, cx)
            });
        });
        click(cx, "settings-exit-discard");
        assert_eq!(view.read_with(cx, |view, _| view.router.tab()), Tab::Home);
        assert_eq!(
            view.read_with(cx, |view, _| view.preferences.language),
            None
        );

        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Tab(Tab::Settings), window, cx);
                view.handle(UiAction::SettingsPage(0), window, cx);
            });
        });
        click(cx, "settings-language");
        click(cx, "select-choice-1");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::SavePreferences, window, cx)
            });
        });
        assert_eq!(
            crate::Preferences::load(root.path())?.language,
            Some(Language::SimplifiedChinese)
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn settings_runtime_window_and_proxy_use_real_choices_without_saving_draft_early(
    cx: &mut gpui::TestAppContext,
) {
    // This tests draft semantics; the dialog entrance uses a wall clock that
    // settle's virtual clock cannot advance before the physical discard click.
    let _system_motion = crate::platform::test_system_motion(true);
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.handle(UiAction::SettingsPage(1), window, cx);
        });
    });
    click(cx, "settings-default-java");
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.choices.len() == 2 && state.choices[0].label == "Automatic Java" && state.choices[1].label == "Custom executable"))));
    crate::test_support::press(cx, "end enter");
    assert!(
        view.read_with(cx, |view, cx| view.settings_form.as_ref().is_some_and(
            |draft| !draft.java_automatic && draft.is_dirty(&view.saved_preferences, cx)
        ))
    );
    cx.update(|window, cx| view.update(cx, |view, cx| view.save_settings(window, cx)));
    assert!(
        view.read_with(cx, |view, _| view.settings_form.as_ref().is_some_and(
            |draft| draft
                .error
                .as_ref()
                .is_some_and(|error| error.field == Some(crate::settings::SettingsField::Java))
        ))
    );
    view.update(cx, |view, cx| {
        if let Some(draft) = &view.settings_form {
            draft
                .java
                .update(cx, |input, cx| input.set_value("/usr/bin/java", cx));
        }
    });
    click(cx, "settings-default-java");
    crate::test_support::press(cx, "home enter");
    assert!(
        view.read_with(cx, |view, cx| view.settings_form.as_ref().is_some_and(
            |draft| draft.java_automatic
                && draft.java.read(cx).value() == "/usr/bin/java"
                && draft
                    .parse(&view.saved_preferences, cx)
                    .is_ok_and(|preferences| preferences.game.java.is_empty())
        ))
    );
    click(cx, "settings-default-window");
    crate::test_support::press(cx, "home enter");
    assert!(
        view.read_with(cx, |view, cx| view.settings_form.as_ref().is_some_and(
            |draft| draft.width.read(cx).value() == "854"
                && draft.height.read(cx).value() == "480"
                && !draft.window_custom
        ))
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::SettingsPage(3), window, cx);
        });
    });
    click(cx, "settings-proxy");
    crate::test_support::press(cx, "down enter");
    assert!(view.read_with(cx, |view, _| {
        view.settings_form
            .as_ref()
            .is_some_and(|draft| draft.proxy_mode == 1)
            && view.saved_preferences == crate::Preferences::default()
    }));
    click(cx, "settings-proxy");
    crate::test_support::press(cx, "end enter");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(draft) = &view.settings_form {
                draft.proxy_url.update(cx, |input, cx| {
                    input.set_value("ftp://127.0.0.1:7890", cx);
                });
            }
            view.save_settings(window, cx);
        });
    });
    assert!(
        view.read_with(cx, |view, cx| view.settings_form.as_ref().is_some_and(
            |draft| draft.proxy_mode == 2
                && draft.proxy_url.read(cx).value() == "ftp://127.0.0.1:7890"
                && draft.error.as_ref().is_some_and(
                    |error| error.field == Some(crate::settings::SettingsField::ProxyUrl)
                )
        ) && view.saved_preferences
            == crate::Preferences::default())
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Tab(Tab::Home), window, cx);
        });
    });
    let discard = click(cx, "settings-exit-discard");
    let state = view.read_with(cx, |view, _| {
        (
            view.router.tab(),
            view.settings_form.is_none(),
            view.preferences == view.saved_preferences,
            view.shell.surfaces.is_open(),
        )
    });
    let viewport = cx.update(|window, _| window.viewport_size());
    assert_eq!(
        state,
        (Tab::Home, true, true, false),
        "discard={discard:?}, viewport={viewport:?}"
    );
}

#[gpui::test]
fn busy_settings_choice_commits_only_its_ui_draft_and_keeps_the_task_busy(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.handle(UiAction::SettingsPage(0), window, cx);
            view.busy = true;
            cx.notify();
        });
    });
    click(cx, "settings-appearance");
    crate::test_support::press(cx, "end enter");
    assert!(view.read_with(cx, |view, _| view.busy
        && view.preferences.appearance == crate::Appearance::Dark
        && view.saved_preferences.appearance == crate::Appearance::System
        && !view.shell.surfaces.is_open()));
}

fn open_editor(
    cx: &mut gpui::TestAppContext,
    action: UiAction,
) -> (gpui::Entity<Block>, &mut gpui::VisualTestContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(960.), px(700.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.router.navigate(Tab::Instances);
            view.snapshot = crate::test_support::ready_snapshot();
            view.handle(UiAction::Inspect("profile-a".into()), window, cx);
            view.handle(action, window, cx);
        });
    });
    settle(cx);
    (view, cx)
}

#[gpui::test]
fn profile_selects_commit_only_the_page_draft_and_cancel_restores_its_checkpoint(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open_editor(cx, UiAction::EditProfile);
    click(cx, "edit-profile-data");
    click(cx, "data-policy");
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.choices.len() == 2 && state.active == 0))));
    crate::test_support::press(cx, "end enter");
    assert!(view.read_with(cx, |view, _| matches!(&view.form, Some(Form::Profile(form)) if form.original.data_policy == DataPolicy::Shared) && view.snapshot.profiles[0].data_policy == DataPolicy::Isolated));
    click(cx, "cancel-profile-page");
    assert!(view.read_with(cx, |view, _| matches!(&view.form, Some(Form::Profile(form)) if form.original.data_policy == DataPolicy::Isolated)));
    click(cx, "edit-profile-launch");
    click(cx, "profile-runtime");
    crate::test_support::press(cx, "end enter");
    assert!(view.read_with(
        cx,
        |view, _| matches!(&view.form, Some(Form::Profile(form)) if !form.managed)
    ));
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            assert!(matches!(&view.form, Some(Form::Profile(form)) if form.java.read(cx).focus_handle(cx).is_focused(window)));
        });
    });
    view.update(cx, |view, cx| {
        if let Some(Form::Profile(form)) = &view.form {
            form.java
                .update(cx, |input, cx| input.set_value("/usr/bin/java", cx));
        }
    });
    click(cx, "profile-runtime");
    crate::test_support::press(cx, "home enter");
    assert!(view.read_with(cx, |view, cx| matches!(&view.form, Some(Form::Profile(form)) if form.managed && form.java.read(cx).value() == "/usr/bin/java")));
    click(cx, "cancel-profile-page");
    assert!(view.read_with(cx, |view, cx| matches!(&view.form, Some(Form::Profile(form)) if form.managed && form.java.read(cx).value().is_empty())));
}

#[gpui::test]
fn instance_selects_preserve_custom_input_validate_real_values_and_discard_on_cancel(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open_editor(cx, UiAction::EditInstance);
    click(cx, "instance-runtime");
    crate::test_support::press(cx, "end enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.instance_runtime_automatic()),
        Some(false)
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.save_instance_draft(window, cx));
    });

    // Selecting automatic uses the managed runtime; no unavailable version is advertised.
    click(cx, "instance-runtime");
    assert!(
        view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(
            |layer| matches!(&layer.kind, Surface::Select(state) if state.choices.len() == 2)
        ))
    );
    crate::test_support::press(cx, "home enter");
    click(cx, "instance-window");
    crate::test_support::press(cx, "home enter");
    assert_eq!(
        view.read_with(cx, Block::instance_window_values),
        Some(("854".into(), "480".into(), false))
    );
    click(cx, "instance-window");
    crate::test_support::press(cx, "end enter");
    assert_eq!(
        view.read_with(cx, Block::instance_window_values),
        Some(("854".into(), "480".into(), true))
    );
    settle(cx);

    click(cx, "cancel-instance-settings");
    assert!(view.read_with(cx, |view, _| {
        view.shell.instance_editor.is_some()
            && matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::EditorExit(_))
            )
            && view.snapshot.instances[0].settings.is_none()
    }));
    click(cx, "editor-exit-discard");
    assert!(
        view.read_with(cx, |view, _| view.shell.instance_editor.is_none()
            && view.snapshot.instances[0].settings.is_none())
    );
}

#[gpui::test]
fn selector_typeahead_moves_real_focus_without_committing_until_activation(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.handle(UiAction::SettingsPage(0), window, cx);
        });
    });
    click(cx, "settings-appearance");
    crate::test_support::press(cx, "d");
    settle(cx);
    assert!(view.read_with(cx, |view, _| view.preferences.appearance == crate::Appearance::System
        && view.shell.surfaces.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.active == 2 && state.choices[0].selected))));

    cx.executor().advance_clock(Duration::from_millis(701));
    crate::test_support::press(cx, "l enter");
    assert!(view.read_with(cx, |view, _| view.preferences.appearance
        == crate::Appearance::Light
        && view.saved_preferences.appearance == crate::Appearance::System));
}

#[gpui::test]
fn app_menu_shares_roving_typeahead_current_page_and_real_navigation(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(UiAction::Tab(Tab::Home), window, cx);
        });
    });
    click(cx, "app-menu");
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.menu == Some(MenuKind::App) && state.active == 0 && state.width == px(192.)))));

    crate::test_support::press(cx, "a");
    assert!(
        view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(
            |layer| matches!(&layer.kind, Surface::Select(state) if state.active == 1)
        ))
    );
    crate::test_support::press(cx, "a");
    assert!(
        view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(
            |layer| matches!(&layer.kind, Surface::Select(state) if state.active == 4)
        ))
    );
    assert_eq!(view.read_with(cx, |view, _| view.router.tab()), Tab::Home);
    cx.executor().advance_clock(Duration::from_millis(701));
    crate::test_support::press(cx, "s e t");
    assert!(
        view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(
            |layer| matches!(&layer.kind, Surface::Select(state) if state.active == 3)
        ))
    );

    crate::test_support::press(cx, "enter");
    assert!(
        view.read_with(cx, |view, _| view.router.tab() == Tab::Settings
            && view.settings_form.is_some()
            && !view.shell.surfaces.is_open())
    );
    click(cx, "settings-back");
    assert_eq!(view.read_with(cx, |view, _| view.router.tab()), Tab::Home);
    click(cx, "app-menu");
    click(cx, "menu-games");
    assert!(
        view.read_with(cx, |view, _| view.router.tab() == Tab::Instances
            && !view.shell.surfaces.is_open())
    );
}

/// A real language change refreshes the open menu without replacing it.
#[gpui::test]
fn menu_labels_resolve_in_the_view_language(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.open_app_menu_at(None, window, cx));
    });
    cx.run_until_parked();
    crate::test_support::press(cx, "down");
    let focus = view.read_with(cx, |view, _| {
        view.shell
            .surfaces
            .top()
            .and_then(|layer| match &layer.kind {
                Surface::Select(state) => Some(state.focus.clone()),
                _ => None,
            })
    });
    for (language, title, first_choice) in [
        (Language::English, "Block menu", "Home"),
        (Language::SimplifiedChinese, "Block 菜单", "主页"),
        (Language::Japanese, "Block メニュー", "ホーム"),
    ] {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::SetLanguage(language), window, cx);
            });
        });
        let resolved = view.read_with(cx, |view, _| {
            let layer = view.shell.surfaces.top()?;
            let Surface::Select(state) = &layer.kind else {
                return None;
            };
            assert_eq!(state.active, 1);
            assert_eq!(Some(state.focus.clone()), focus);
            assert!(state.choices[0].selected);
            Some((state.title(language), state.choices[0].label.clone()))
        });
        assert_eq!(
            resolved,
            Some((title.to_owned(), first_choice.to_owned())),
            "{}",
            language.code()
        );
    }
}

/// Several screens of options: past the popup list's 624 px maximum.
const LONG_CATALOG: usize = 70;

fn version_label(index: usize) -> String {
    format!("1.21.{index}")
}

fn build_label(index: usize) -> String {
    format!("0.16.{index}")
}

/// The real Plain-selector target inside the wizard and its version catalog.
fn long_target(
    cx: &mut gpui::TestAppContext,
    draft: TargetDraft,
) -> Option<(gpui::Entity<Block>, &mut gpui::VisualTestContext)> {
    let inspection = (|| -> anyhow::Result<_> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("plain.zip");
        crate::test_support::write_plain_pack(&source)?;
        block_launcher::modpack::inspect(&source)
    })();
    assert!(inspection.is_ok(), "{inspection:?}");
    let Ok(inspection) = inspection else {
        return None;
    };
    let (view, cx) = crate::test_support::window(cx, move |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = (0..LONG_CATALOG).map(version_label).collect();
        view.begin_game_wizard(cx);
        view.shell.wizard.kind = CreationKind::ImportPack;
        view.shell.wizard.step = WizardStep::Setup;
        view.shell.pack_import = State::default();
        view.shell.pack_import.phase = Phase::Review;
        view.shell.pack_import.draft = Some(draft);
        view.shell.pack_import.inspection = Some(inspection);
        view
    });
    cx.simulate_resize(size(px(1152.), px(720.)));
    settle(cx);
    Some((view, cx))
}

fn open_target_select(
    view: &gpui::Entity<Block>,
    kind: ProductChoice,
    cx: &mut gpui::VisualTestContext,
) {
    let id = match kind {
        ProductChoice::PackTargetVersion => "modpack-target-version",
        ProductChoice::PackTargetBuild => "modpack-target-build",
        _ => return,
    };
    let bounds = crate::test_support::control_bounds(cx, id);
    assert!(bounds.is_some(), "the target control {id} is painted");
    if let Some(bounds) = bounds {
        view.update(cx, |view, cx| {
            let scroll = &view.shell.pack_import.scroll;
            let viewport = scroll.bounds();
            let delta = if bounds.bottom() > viewport.bottom() {
                viewport.bottom() - bounds.bottom() - px(8.)
            } else if bounds.top() < viewport.top() {
                viewport.top() - bounds.top() + px(8.)
            } else {
                px(0.)
            };
            scroll.set_offset(point(scroll.offset().x, scroll.offset().y + delta));
            cx.notify();
        });
        settle(cx);
        click(cx, id);
    }
    settle(cx);
    for _ in 0..2 {
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        settle(cx);
    }
}

/// A selector as the surface stack holds it, after the last paint.
#[derive(Debug, Default)]
struct Open {
    labels: Vec<String>,
    selected: Vec<bool>,
    active: usize,
    product: Option<ProductChoice>,
    focus: Option<FocusHandle>,
    offset: Point<Pixels>,
    max: Point<Pixels>,
}

fn open_selector(view: &gpui::Entity<Block>, cx: &mut gpui::VisualTestContext) -> Open {
    let open = view.read_with(cx, |view, _| {
        let layer = view.shell.surfaces.top()?;
        let Surface::Select(state) = &layer.kind else {
            return None;
        };
        Some(Open {
            labels: state
                .choices
                .iter()
                .map(|choice| choice.label.clone())
                .collect(),
            selected: state.choices.iter().map(|choice| choice.selected).collect(),
            active: state.active,
            product: state.product,
            focus: Some(state.focus.clone()),
            offset: state.scroll.offset(),
            max: state.scroll.max_offset(),
        })
    });
    assert!(open.is_some(), "the target selector owns the top surface");
    open.unwrap_or_default()
}

fn at_bottom(open: &Open) -> bool {
    open.max.y > px(0.) && (open.offset.y + open.max.y).abs() <= px(1.)
}

fn at_top(open: &Open) -> bool {
    open.offset.y.abs() <= px(1.)
}

fn draft_version(view: &gpui::Entity<Block>, cx: &mut gpui::VisualTestContext) -> Option<String> {
    view.read_with(cx, |view, _| {
        view.shell
            .pack_import
            .draft
            .as_ref()
            .and_then(|draft| draft.version.clone())
    })
}

fn pack_page_open(view: &gpui::Entity<Block>, cx: &mut gpui::VisualTestContext) -> bool {
    view.read_with(cx, |view, _| view.pack_wizard_active())
}

fn select_open(view: &gpui::Entity<Block>, cx: &mut gpui::VisualTestContext) -> bool {
    view.read_with(cx, |view, _| {
        view.shell
            .surfaces
            .layers()
            .any(|layer| matches!(layer.kind, Surface::Select(_)))
    })
}

/// A plain archive's version list is longer than the popup: the list scrolls
/// inside a bounded popup, its first and last rows are real choices, and the
/// chosen version survives the selector closing.
#[gpui::test]
fn long_target_version_list_scrolls_inside_a_bounded_popup_and_keeps_every_choice(
    cx: &mut gpui::TestAppContext,
) {
    let Some((view, cx)) = long_target(cx, TargetDraft::default()) else {
        return;
    };
    open_target_select(&view, ProductChoice::PackTargetVersion, cx);

    let expected: Vec<String> = (0..LONG_CATALOG).map(version_label).collect();
    let open = open_selector(&view, cx);
    assert_eq!(open.product, Some(ProductChoice::PackTargetVersion));
    assert_eq!(open.labels, expected, "every offered version is one choice");
    assert_eq!(open.active, 0, "an unset target visits the first version");
    assert!(open.selected.iter().all(|selected| !selected));
    assert_eq!(open.focus, cx.update(|window, cx| window.focused(cx)));

    // The popup stays inside the window and the list inside the popup.
    assert_popup_within_window(cx);
    let popup = cx.debug_bounds("drawer-sheet");
    assert!(
        popup.is_some_and(|bounds| bounds.size.height <= px(696.)),
        "popup={popup:?}"
    );
    assert!(
        open.max.y > px(0.),
        "the catalog is taller than the popup: {open:?}"
    );

    // Home and End move the active row and the real viewport with it.
    crate::test_support::press(cx, "end");
    settle(cx);
    let last = open_selector(&view, cx);
    assert_eq!(last.active, LONG_CATALOG - 1);
    assert!(at_bottom(&last), "End exposes the last row: {last:?}");

    crate::test_support::press(cx, "home");
    settle(cx);
    let first = open_selector(&view, cx);
    assert_eq!(first.active, 0);
    assert!(at_top(&first), "Home returns to the first row: {first:?}");

    crate::test_support::press(cx, "down");
    settle(cx);
    assert_eq!(open_selector(&view, cx).active, 1);
    crate::test_support::press(cx, "up");
    settle(cx);
    assert_eq!(open_selector(&view, cx).active, 0);

    // Enter chooses the last version and closes only the selector.
    crate::test_support::press(cx, "end enter");
    settle(cx);
    assert_eq!(
        draft_version(&view, cx),
        Some(version_label(LONG_CATALOG - 1)),
        "the last version is the chosen target"
    );
    assert!(!select_open(&view, cx), "the selector closed");
    assert!(pack_page_open(&view, cx), "the wizard kept the choice");
    assert!(
        crate::test_support::control_bounds(cx, "modpack-import").is_some(),
        "the wizard branch is still painted"
    );

    // Reopening inspects the chosen version with the popup focus.
    open_target_select(&view, ProductChoice::PackTargetVersion, cx);
    let reopened = open_selector(&view, cx);
    assert_eq!(reopened.active, LONG_CATALOG - 1);
    assert_eq!(
        reopened
            .selected
            .iter()
            .filter(|selected| **selected)
            .count(),
        1,
        "the chosen version is the only selected row"
    );
    assert!(
        reopened.selected[reopened.active],
        "the active row is the chosen value"
    );
    assert_eq!(
        reopened.labels.last(),
        Some(&version_label(LONG_CATALOG - 1))
    );
    assert!(
        at_bottom(&reopened),
        "reopening scrolls the chosen version into view: {reopened:?}"
    );
    assert_eq!(reopened.focus, cx.update(|window, cx| window.focused(cx)));
}

/// The wheel scrolls the selector's own list and nothing behind it.
#[gpui::test]
fn wheel_scrolls_the_long_selector_without_moving_the_wizard_beneath(
    cx: &mut gpui::TestAppContext,
) {
    let Some((view, cx)) = long_target(cx, TargetDraft::default()) else {
        return;
    };
    open_target_select(&view, ProductChoice::PackTargetVersion, cx);
    let popup = cx.debug_bounds("drawer-sheet");
    assert!(popup.is_some(), "the selector popup is painted");
    if let Some(popup) = popup {
        let pointer = popup.center();
        let list = open_selector(&view, cx);
        let drawer_scroll = view.read_with(cx, |view, _| view.shell.pack_import.scroll.offset());
        let tab = view.read_with(cx, |view, _| view.router.tab());
        cx.simulate_mouse_move(pointer, None, Modifiers::none());
        cx.run_until_parked();
        cx.simulate_event(ScrollWheelEvent {
            position: pointer,
            delta: ScrollDelta::Pixels(point(px(0.), px(-2400.))),
            ..ScrollWheelEvent::default()
        });
        settle(cx);

        let scrolled = open_selector(&view, cx);
        assert!(
            scrolled.offset.y < list.offset.y,
            "the wheel moved the selector's own list: {scrolled:?}"
        );
        assert!(
            at_bottom(&scrolled),
            "the wheel reached the end of the list: {scrolled:?}"
        );
        assert_eq!(scrolled.active, list.active, "scrolling chooses nothing");
        assert_eq!(scrolled.selected, list.selected);

        // The surface behind the popup keeps its own state.
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.pack_import.scroll.offset()),
            drawer_scroll,
            "the wizard keeps its own scroll"
        );
        assert_eq!(view.read_with(cx, |view, _| view.router.tab()), tab);
        assert!(pack_page_open(&view, cx) && select_open(&view, cx));

        cx.simulate_event(ScrollWheelEvent {
            position: pointer,
            delta: ScrollDelta::Pixels(point(px(0.), px(2400.))),
            ..ScrollWheelEvent::default()
        });
        settle(cx);
        let returned = open_selector(&view, cx);
        assert!(
            at_top(&returned),
            "the wheel returns to the first row: {returned:?}"
        );
    }
}

/// The selector owns activation and Escape before the wizard handles either.
#[gpui::test]
fn wizard_selector_keyboard_and_escape_preserve_the_page(cx: &mut gpui::TestAppContext) {
    let Some((view, cx)) = long_target(cx, TargetDraft::default()) else {
        return;
    };
    open_target_select(&view, ProductChoice::PackTargetVersion, cx);
    let trigger_focus = view.read_with(cx, |view, cx| {
        view.shell
            .surfaces
            .trigger_focus("modpack-target-version", cx)
    });
    assert_eq!(
        open_selector(&view, cx).focus,
        cx.update(|window, cx| window.focused(cx)),
        "the popup owns focus"
    );

    // The popup owns activation: Enter chooses its row, never a wizard control.
    crate::test_support::press(cx, "enter");
    settle(cx);
    assert_eq!(draft_version(&view, cx), Some(version_label(0)));
    assert!(!select_open(&view, cx), "Enter activated the popup's row");
    assert!(pack_page_open(&view, cx), "Enter did not leave the wizard");
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        Some(trigger_focus.clone())
    );

    // Native traversal from the trigger is the independent expected focus.
    for keys in ["tab", "shift-tab"] {
        cx.update(|window, cx| trigger_focus.focus(window, cx));
        crate::test_support::press(cx, keys);
        settle(cx);
        let expected = cx.update(|window, cx| window.focused(cx));
        assert_ne!(expected, Some(trigger_focus.clone()), "{keys} moves focus");
        open_target_select(&view, ProductChoice::PackTargetVersion, cx);
        crate::test_support::press(cx, keys);
        settle_tab(cx);
        assert!(!select_open(&view, cx), "{keys} closed the selector");
        assert_eq!(
            cx.update(|window, cx| window.focused(cx)),
            expected,
            "{keys} traverses once"
        );
        assert!(pack_page_open(&view, cx), "{keys} kept the wizard open");
        assert_eq!(
            draft_version(&view, cx),
            Some(version_label(0)),
            "{keys} chose nothing"
        );
    }

    // The selector owns Cancel before the wizard.
    open_target_select(&view, ProductChoice::PackTargetVersion, cx);
    crate::test_support::press(cx, "escape");
    settle(cx);
    assert!(!select_open(&view, cx), "Escape closed the nested selector");
    assert!(pack_page_open(&view, cx), "Escape kept the wizard");
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        Some(trigger_focus),
        "focus returned to the wizard"
    );

    crate::test_support::press(cx, "escape");
    settle(cx);
    assert!(
        !pack_page_open(&view, cx),
        "the wizard closes on its own Escape"
    );
}

fn long_builds() -> Vec<LoaderVersion> {
    (0..LONG_CATALOG)
        .map(|index| LoaderVersion {
            version: build_label(index),
            stable: true,
        })
        .collect()
}

fn modded_target(builds: Vec<LoaderVersion>) -> TargetDraft {
    let version = version_label(0);
    TargetDraft {
        version: Some(version.clone()),
        loader: Some(LoaderKind::Fabric),
        catalog: Some(LoaderCatalog {
            request: uuid::Uuid::nil(),
            minecraft: version,
            kind: LoaderKind::Fabric,
            builds,
            load: CollectionLoad::Ready,
        }),
        ..TargetDraft::default()
    }
}

/// The loader build catalog is the second long list: it scrolls the same way
/// and the last build of the chosen loader is a real choice.
#[gpui::test]
fn long_loader_build_list_scrolls_and_chooses_its_last_build(cx: &mut gpui::TestAppContext) {
    let Some((view, cx)) = long_target(cx, modded_target(long_builds())) else {
        return;
    };
    open_target_select(&view, ProductChoice::PackTargetBuild, cx);

    let expected: Vec<String> = (0..LONG_CATALOG).map(build_label).collect();
    let open = open_selector(&view, cx);
    assert_eq!(open.product, Some(ProductChoice::PackTargetBuild));
    assert_eq!(open.labels, expected, "every offered build is one choice");
    assert_eq!(open.active, 0);
    assert!(
        open.max.y > px(0.),
        "the builds are taller than the popup: {open:?}"
    );
    assert_popup_within_window(cx);

    crate::test_support::press(cx, "end enter");
    settle(cx);
    assert_eq!(
        view.read_with(cx, |view, _| {
            view.shell
                .pack_import
                .draft
                .as_ref()
                .and_then(|draft| draft.build.clone())
        }),
        Some(build_label(LONG_CATALOG - 1)),
        "the last build is the chosen one"
    );
    assert!(!select_open(&view, cx));
    assert!(
        pack_page_open(&view, cx),
        "the wizard kept the build choice"
    );
    assert!(crate::test_support::control_bounds(cx, "modpack-import").is_some());
}

/// A saved overlay snapshot keeps the selector offset it captured; scrolling
/// the live list afterwards must not move the saved copy.
#[gpui::test]
fn saved_overlay_snapshot_keeps_the_selector_offset_it_captured(cx: &mut gpui::TestAppContext) {
    let Some((view, cx)) = long_target(cx, TargetDraft::default()) else {
        return;
    };
    open_target_select(&view, ProductChoice::PackTargetVersion, cx);
    crate::test_support::press(cx, "end");
    settle(cx);
    let captured = open_selector(&view, cx).offset;
    assert!(
        captured.y < px(0.),
        "the captured list is scrolled: {captured:?}"
    );

    let saved = view.read_with(cx, |view, _| view.shell.surfaces.snapshot());
    crate::test_support::press(cx, "home");
    settle(cx);
    let live = open_selector(&view, cx);
    assert!(
        at_top(&live),
        "the live list returned to its first row: {live:?}"
    );

    let saved_offset = saved.top().and_then(|layer| match &layer.kind {
        Surface::Select(state) => Some(state.scroll.offset()),
        _ => None,
    });
    assert_eq!(
        saved_offset,
        Some(captured),
        "the saved snapshot kept its own offset"
    );
}
