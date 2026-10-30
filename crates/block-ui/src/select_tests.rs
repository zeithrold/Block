use super::*;
use gpui::{Focusable as _, Modifiers, size};

fn settle(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[gpui::test]
fn menu_entrance_forwards_the_first_typeahead_once_without_waiting_for_animation(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.tab = Tab::Home;
        view
    });
    crate::test_support::press(cx, "tab enter a");
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.choices[state.active].label == "Accounts"))), "{:?}", view.read_with(cx, |view, _| view.shell.surfaces.top().map(|layer| format!("{:?}", layer.kind))));
    crate::test_support::press(cx, "enter");
    assert_eq!(view.read_with(cx, |view, _| view.tab), Tab::Accounts);
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
fn settings_runtime_window_and_proxy_use_real_choices_without_saving_draft_early(
    cx: &mut gpui::TestAppContext,
) {
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
    assert!(view.read_with(cx, |view, _| view.settings_form.is_none()
        && view.preferences == view.saved_preferences));
}

#[gpui::test]
fn busy_settings_choice_commits_only_its_ui_draft_and_keeps_the_task_busy(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
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
            view.tab = Tab::Instances;
            view.snapshot = crate::test_support::ready_snapshot();
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
            |layer| matches!(&layer.kind, Surface::Select(state) if state.active == 5)
        ))
    );
    assert_eq!(view.read_with(cx, |view, _| view.tab), Tab::Home);
    cx.executor().advance_clock(Duration::from_millis(701));
    crate::test_support::press(cx, "s e t");
    assert!(
        view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(
            |layer| matches!(&layer.kind, Surface::Select(state) if state.active == 3)
        ))
    );

    crate::test_support::press(cx, "enter");
    assert!(view.read_with(cx, |view, _| view.tab == Tab::Settings
        && view.settings_form.is_some()
        && !view.shell.surfaces.is_open()));
    click(cx, "app-menu");
    click(cx, "menu-games");
    assert!(view.read_with(cx, |view, _| view.tab == Tab::Instances
        && !view.shell.surfaces.is_open()));
}

#[gpui::test]
fn guide_menu_filters_real_actions_and_settings_cancel_preserves_the_guide_draft(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.saved_preferences.reduce_motion = true;
            view.handle(UiAction::Tab(Tab::Onboarding), window, cx);
            if let Some(Form::Instance(form)) = &view.form {
                form.name
                    .update(cx, |input, cx| input.set_value("Kept guide 世界", cx));
            }
        });
    });
    click(cx, "app-menu");
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.choices.iter().map(|choice| choice.label.as_str()).eq(["Settings", "About"])))));
    assert!(
        cx.debug_bounds("menu-home").is_none()
            && cx.debug_bounds("menu-settings").is_some()
            && cx.debug_bounds("menu-about").is_some()
    );
    click(cx, "menu-settings");

    assert!(view.read_with(cx, |view, _| view.tab == Tab::Settings
        && view.shell.guide_preview.is_some()
        && view.shell.surfaces.layers().next().is_none()));
    click(cx, "settings-appearance");
    crate::test_support::press(cx, "end enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.preferences.appearance),
        crate::Appearance::Dark
    );
    click(cx, "cancel-preferences");

    assert!(view.read_with(cx, |view, cx| view.tab == Tab::Onboarding && !view.shell.surfaces.is_open()
        && view.preferences == view.saved_preferences && view.settings_form.is_none()
        && matches!(&view.form, Some(Form::Instance(form)) if form.name.read(cx).value() == "Kept guide 世界")));
}
