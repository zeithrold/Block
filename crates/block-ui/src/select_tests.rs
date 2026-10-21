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
    assert!(view.read_with(cx, |view, _| view.shell.overlays.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.choices[state.active].label == "Accounts"))), "{:?}", view.read_with(cx, |view, _| view.shell.overlays.top().map(|layer| format!("{:?}", layer.kind))));
    crate::test_support::press(cx, "enter");
    assert_eq!(view.read_with(cx, |view, _| view.tab), Tab::Accounts);
    assert!(view.read_with(cx, |view, _| view.shell.overlays.top().is_none()));
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) -> Option<Bounds<Pixels>> {
    settle(cx);
    let bounds = cx.debug_bounds(selector);
    assert!(
        bounds.is_some(),
        "missing actual product control {selector}"
    );
    if let Some(bounds) = bounds {
        cx.simulate_click(bounds.center(), Modifiers::none());
    }
    settle(cx);
    bounds
}

fn appearance_trigger(cx: &mut gpui::VisualTestContext) -> Option<gpui::accesskit::NodeId> {
    cx.update(|window, _| {
        let tree = window.accessibility_tree();
        let node = tree.nodes.iter().find(|(_, node)| {
            node.label() == Some("Appearance") && node.value() == Some("Follow system")
        });
        assert!(node.is_some());
        node.map(|(id, node)| {
            assert_eq!(node.role(), gpui::accesskit::Role::ComboBox);
            *id
        })
    })
}

fn native_click(window: &mut Window, cx: &mut App, target: gpui::accesskit::NodeId) {
    window.simulate_accessibility_action(
        gpui::accesskit::ActionRequest {
            action: gpui::accesskit::Action::Click,
            target_node: target,
            target_tree: gpui::accesskit::TreeId::ROOT,
            data: None,
        },
        cx,
    );
}

fn enabled_action(
    cx: &mut gpui::VisualTestContext,
    label: &str,
    role: gpui::Role,
) -> Option<gpui::accesskit::NodeId> {
    cx.update(|window, _| {
        let tree = window.accessibility_tree();
        let node = tree
            .nodes
            .iter()
            .find(|(_, node)| node.role() == role && node.label() == Some(label));
        assert!(node.is_some(), "missing native {role:?} named {label}");
        node.map(|(id, node)| {
            assert!(!node.is_disabled());
            assert!(node.supports_action(gpui::accesskit::Action::Click));
            *id
        })
    })
}

#[gpui::test]
fn repeated_native_option_commit_keeps_the_guide_parent_and_its_focus(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(960.), px(700.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.begin_onboarding(cx);
            view.open_onboarding_section(crate::game_wizard::WizardPage::Location, window, cx);
        });
    });
    click(cx, "wizard-java-runtime");
    let option = enabled_action(cx, "Custom executable", gpui::Role::ListBoxOption);
    assert!(option.is_some());
    if let Some(option) = option {
        cx.update(|window, cx| {
            native_click(window, cx, option);
            let parent = view.read_with(cx, |view, _| {
                assert_eq!(view.shell.overlays.layers().count(), 1);
                assert!(
                    view.shell
                        .wizard
                        .profile
                        .as_ref()
                        .is_some_and(|draft| !draft.managed)
                );
                view.shell.overlays.top().map(|layer| layer.revision)
            });
            let focused = window.focused(cx);
            assert!(focused.is_some());
            // The old semantic frame still contains the selected option. It
            // cannot commit against the parent that now occupies the stack.
            native_click(window, cx, option);
            view.read_with(cx, |view, _| {
                assert_eq!(view.shell.overlays.layers().count(), 1);
                assert!(view.shell.overlays.top().is_some_and(|layer| {
                    Some(layer.revision) == parent
                        && matches!(
                            layer.kind,
                            Surface::OnboardingSection(crate::game_wizard::WizardPage::Location)
                        )
                }));
            });
            assert_eq!(window.focused(cx), focused);
        });
    }
}

#[gpui::test]
fn repeated_native_menu_or_appearance_open_keeps_one_popup_revision(cx: &mut gpui::TestAppContext) {
    for appearance in [false, true] {
        let (view, cx) = open_editor(
            cx,
            if appearance {
                UiAction::EditProfile
            } else {
                UiAction::Tab(Tab::Home)
            },
        );
        let (label, role) = if appearance {
            click(cx, "edit-profile-appearance");
            (
                view.read_with(cx, |view, _| match &view.form {
                    Some(Form::Profile(form)) => form.original.appearance.icon.label().to_owned(),
                    _ => String::new(),
                }),
                gpui::Role::ComboBox,
            )
        } else {
            ("Open menu".to_owned(), gpui::Role::Button)
        };
        let trigger = enabled_action(cx, &label, role);
        if let Some(trigger) = trigger {
            cx.update(|window, cx| {
                native_click(window, cx, trigger);
                let first = view.read_with(cx, |view, _| {
                    view.shell.overlays.top().map(|layer| layer.revision)
                });
                assert!(first.is_some());
                let focused = window.focused(cx);
                native_click(window, cx, trigger);
                view.read_with(cx, |view, _| {
                    assert_eq!(view.shell.overlays.layers().count(), 1);
                    assert_eq!(view.shell.overlays.top().map(|layer| layer.revision), first);
                });
                assert_eq!(window.focused(cx), focused);
            });
        }
        settle(cx);
        crate::test_support::press(cx, "escape");
        assert!(view.read_with(cx, |view, _| !view.shell.overlays.is_open()));
    }
}

#[gpui::test]
fn same_frame_busy_change_rejects_old_appearance_and_profile_actions_callbacks(
    cx: &mut gpui::TestAppContext,
) {
    for appearance in [false, true] {
        let (view, cx) = open_editor(
            cx,
            if appearance {
                UiAction::EditProfile
            } else {
                UiAction::Tab(Tab::Instances)
            },
        );
        let (label, role) = if appearance {
            click(cx, "edit-profile-appearance");
            (
                view.read_with(cx, |view, _| match &view.form {
                    Some(Form::Profile(form)) => form.original.appearance.icon.label().to_owned(),
                    _ => String::new(),
                }),
                gpui::Role::ComboBox,
            )
        } else {
            ("Profile actions".to_owned(), gpui::Role::Button)
        };
        let trigger = enabled_action(cx, &label, role);
        if let Some(trigger) = trigger {
            cx.update(|window, cx| {
                let focused = window.focused(cx);
                view.update(cx, |view, cx| {
                    view.busy = true;
                    cx.notify();
                });
                native_click(window, cx, trigger);
                assert!(view.read_with(cx, |view, _| view.busy && !view.shell.overlays.is_open()));
                assert_eq!(window.focused(cx), focused);
            });
        }
    }
}

fn assert_narrow_popup(cx: &mut gpui::VisualTestContext) {
    let popup = cx.debug_bounds("drawer-sheet");
    assert!(
        popup.is_some_and(|bounds| bounds.left() >= px(8.)
            && bounds.right() <= px(352.)
            && bounds.top() >= px(8.)
            && bounds.bottom() <= px(532.)),
        "narrow popup={popup:?}"
    );
}

#[gpui::test]
fn repeated_accessible_open_in_one_frame_keeps_one_select_revision_and_can_reopen(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
        });
    });
    settle(cx);
    let trigger = appearance_trigger(cx);
    assert!(trigger.is_some());
    if let Some(trigger) = trigger {
        cx.update(|window, cx| {
            let activate = |window: &mut Window, cx: &mut App| {
                window.simulate_accessibility_action(
                    gpui::accesskit::ActionRequest {
                        action: gpui::accesskit::Action::Click,
                        target_node: trigger,
                        target_tree: gpui::accesskit::TreeId::ROOT,
                        data: None,
                    },
                    cx,
                );
            };
            activate(window, cx);
            let first = view.read_with(cx, |view, _| {
                view.shell
                    .overlays
                    .top()
                    .map(|layer| (layer.revision, layer.focus.clone()))
            });
            assert!(first.is_some());
            let focused = window.focused(cx);
            assert!(focused.is_some());
            // Both actions target the same completed native tree. No draw
            // between them may mask the callback's captured closed state.
            activate(window, cx);
            assert_eq!(
                window.focused(cx),
                focused,
                "a repeated accessible open must retain the popup's actual focus"
            );
            view.read_with(cx, |view, _| {
                assert_eq!(view.shell.overlays.layers().count(), 1);
                assert_eq!(
                    view.shell.overlays.top().map(|layer| layer.revision),
                    first.as_ref().map(|(revision, _)| *revision)
                );
                assert_eq!(
                    view.shell.overlays.top().map(|layer| layer.focus.clone()),
                    first.map(|(_, focus)| focus)
                );
            });
        });
    }
    settle(cx);
    let first_revision = view.read_with(cx, |view, _| {
        view.shell.overlays.top().map(|layer| layer.revision)
    });
    crate::test_support::press(cx, "escape");
    settle(cx);
    assert!(view.read_with(cx, |view, _| !view.shell.overlays.is_open()));
    let trigger = appearance_trigger(cx);
    cx.update(|window, cx| {
        assert_eq!(Some(window.accessibility_tree().focus), trigger);
        if let Some(trigger) = trigger {
            window.simulate_accessibility_action(
                gpui::accesskit::ActionRequest {
                    action: gpui::accesskit::Action::Click,
                    target_node: trigger,
                    target_tree: gpui::accesskit::TreeId::ROOT,
                    data: None,
                },
                cx,
            );
        }
    });
    assert!(view.read_with(cx, |view, _| {
        view.shell.overlays.layers().count() == 1
            && view
                .shell
                .overlays
                .top()
                .is_some_and(|layer| Some(layer.revision) != first_revision)
    }));
}

#[gpui::test]
fn settings_select_anchors_to_trigger_and_keeps_inspection_separate_from_value(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
        });
    });
    settle(cx);
    let trigger_id = appearance_trigger(cx);
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
    assert!(view.read_with(cx, |view, _| view.shell.overlays.top().is_some_and(|layer| {
        matches!(&layer.kind, Surface::Select(state) if state.active == 0 && state.choices[0].selected && bounds.is_some_and(|bounds| state.anchor == point(bounds.left(), bounds.bottom() + px(6.))))
    })), "trigger={bounds:?}; surface={:?}", view.read_with(cx, |view, _| view.shell.overlays.top().cloned()));
    crate::test_support::press(cx, "end");
    assert!(view.read_with(cx, |view, _| {
        view.preferences.appearance == crate::Appearance::System && view.shell.overlays.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.active == 2 && state.choices[0].selected && !state.choices[2].selected))
    }));
    cx.update(|window, _| {
        let tree = window.accessibility_tree();
        assert!(
            tree.nodes
                .iter()
                .any(|(_, node)| node.label() == Some("Follow system")
                    && node.is_selected() == Some(true))
        );
        assert!(
            tree.nodes.iter().any(|(id, node)| *id == tree.focus
                && node.label() == Some("Dark")
                && node.is_selected() == Some(false)),
            "roving inspection moves actual focus without changing the selected value"
        );
    });
    cx.update(|window, cx| {
        let before = window.focused(cx);
        window.focus_next(cx);
        assert_eq!(
            window.focused(cx),
            before,
            "selector has one roving Tab stop"
        );
    });
    cx.simulate_resize(size(px(360.), px(540.)));
    settle(cx);
    assert_narrow_popup(cx);
    crate::test_support::press(cx, "escape");
    settle(cx);
    assert!(view.read_with(cx, |view, _| view.preferences.appearance
        == crate::Appearance::System
        && !view.shell.overlays.is_open()));
    let trigger_focus = cx.update(|window, cx| {
        let tree = window.accessibility_tree();
        assert_eq!(
            Some(tree.focus),
            trigger_id,
            "a mouse-opened selector must restore focus to its actual trigger"
        );
        window.focused(cx)
    });
    cx.simulate_resize(size(px(360.), px(540.)));
    click(cx, "settings-appearance");
    assert_narrow_popup(cx);
    crate::test_support::press(cx, "escape");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), trigger_focus);
    click(cx, "settings-appearance");
    crate::test_support::press(cx, "end enter");
    assert!(view.read_with(cx, |view, _| view.preferences.appearance
        == crate::Appearance::Dark
        && view.saved_preferences.appearance == crate::Appearance::System
        && !view.shell.overlays.is_open()));
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
    assert!(view.read_with(cx, |view, _| view.shell.overlays.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.choices.len() == 2 && state.choices[0].label == "Automatic Java" && state.choices[1].label == "Custom executable"))));
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
                    input.set_value("socks5://127.0.0.1:7890", cx);
                });
            }
            view.save_settings(window, cx);
        });
    });
    assert!(
        view.read_with(cx, |view, cx| view.settings_form.as_ref().is_some_and(
            |draft| draft.proxy_mode == 2
                && draft.proxy_url.read(cx).value() == "socks5://127.0.0.1:7890"
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
        && !view.shell.overlays.is_open()));
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
fn same_frame_busy_change_rejects_an_old_enabled_profile_or_instance_select_action(
    cx: &mut gpui::TestAppContext,
) {
    for action in [UiAction::EditProfile, UiAction::EditInstance] {
        let profile = matches!(action, UiAction::EditProfile);
        let (view, cx) = open_editor(cx, action);
        if profile {
            click(cx, "edit-profile-launch");
        }
        let trigger = cx.update(|window, _| {
            window
                .accessibility_tree()
                .nodes
                .iter()
                .find(|(_, node)| {
                    node.role() == gpui::Role::ComboBox && node.label() == Some("Java runtime")
                })
                .map(|(id, node)| {
                    assert!(!node.is_disabled());
                    assert!(node.supports_action(gpui::accesskit::Action::Click));
                    *id
                })
        });
        assert!(
            trigger.is_some(),
            "capture the real enabled runtime select before the task starts"
        );
        if let Some(trigger) = trigger {
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.busy = true;
                    cx.notify();
                });
                // Deliberately do not draw: AccessKit is still delivering an
                // action against the enabled semantic frame captured above.
                window.simulate_accessibility_action(
                    gpui::accesskit::ActionRequest {
                        action: gpui::accesskit::Action::Click,
                        target_node: trigger,
                        target_tree: gpui::accesskit::TreeId::ROOT,
                        data: None,
                    },
                    cx,
                );
                view.read_with(cx, |view, _| {
                    assert!(view.busy);
                    assert!(
                        !view.shell.overlays.is_open(),
                        "an old enabled callback must recheck the current busy rule"
                    );
                    assert!(if profile {
                        matches!(&view.form, Some(Form::Profile(_)))
                    } else {
                        view.shell.instance_editor.is_some()
                    });
                });
            });
        }
    }
}

#[gpui::test]
fn profile_selects_commit_only_the_page_draft_and_cancel_restores_its_checkpoint(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open_editor(cx, UiAction::EditProfile);
    click(cx, "edit-profile-data");
    click(cx, "data-policy");
    assert!(view.read_with(cx, |view, _| view.shell.overlays.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.choices.len() == 2 && state.active == 0))));
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
    cx.update(|window, _| {
        let tree = window.accessibility_tree();
        assert!(tree.nodes.iter().any(|(id, node)| *id == tree.focus
            && node.role() == gpui::accesskit::Role::TextInput
            && node.label() == Some("Java executable")));
    });
    // Selecting automatic uses the managed runtime; no unavailable version is advertised.
    click(cx, "instance-runtime");
    assert!(
        view.read_with(cx, |view, _| view.shell.overlays.top().is_some_and(
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
    cx.update(|window, _| {
        let tree = window.accessibility_tree();
        assert!(
            tree.nodes
                .iter()
                .any(|(id, node)| *id == tree.focus && node.label() == Some("Window width (px)"))
        );
    });
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
        && view.shell.overlays.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.active == 2 && state.choices[0].selected))));
    cx.update(|window, _| {
        let tree = window.accessibility_tree();
        assert!(
            tree.nodes
                .iter()
                .any(|(id, node)| *id == tree.focus && node.label() == Some("Dark"))
        );
    });
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
    assert!(view.read_with(cx, |view, _| view.shell.overlays.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.menu == Some(MenuKind::App) && state.active == 0 && state.width == px(192.)))));
    cx.update(|window, _| {
        let tree = window.accessibility_tree();
        assert!(
            tree.nodes
                .iter()
                .any(|(_, node)| node.role() == gpui::accesskit::Role::Menu)
        );
        assert!(
            tree.nodes
                .iter()
                .any(|(_, node)| node.role() == gpui::accesskit::Role::MenuItem
                    && node.label() == Some("Home")
                    && node.aria_current() == Some(gpui::accesskit::AriaCurrent::Page))
        );
    });
    crate::test_support::press(cx, "a");
    assert!(
        view.read_with(cx, |view, _| view.shell.overlays.top().is_some_and(
            |layer| matches!(&layer.kind, Surface::Select(state) if state.active == 1)
        ))
    );
    crate::test_support::press(cx, "a");
    assert!(
        view.read_with(cx, |view, _| view.shell.overlays.top().is_some_and(
            |layer| matches!(&layer.kind, Surface::Select(state) if state.active == 5)
        ))
    );
    assert_eq!(view.read_with(cx, |view, _| view.tab), Tab::Home);
    cx.executor().advance_clock(Duration::from_millis(701));
    crate::test_support::press(cx, "s e t");
    assert!(
        view.read_with(cx, |view, _| view.shell.overlays.top().is_some_and(
            |layer| matches!(&layer.kind, Surface::Select(state) if state.active == 3)
        ))
    );
    cx.update(|window, cx| {
        let focused = window.focused(cx);
        window.focus_next(cx);
        assert_eq!(
            window.focused(cx),
            focused,
            "section labels and inactive rows do not add Tab stops"
        );
    });
    crate::test_support::press(cx, "enter");
    assert!(view.read_with(cx, |view, _| view.tab == Tab::Settings
        && view.settings_form.is_some()
        && !view.shell.overlays.is_open()));
    click(cx, "app-menu");
    click(cx, "menu-games");
    assert!(view.read_with(cx, |view, _| view.tab == Tab::Instances
        && !view.shell.overlays.is_open()));
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
    assert!(view.read_with(cx, |view, _| view.shell.overlays.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.choices.iter().map(|choice| choice.label.as_str()).eq(["Settings", "About"])))));
    assert!(
        cx.debug_bounds("menu-home").is_none()
            && cx.debug_bounds("menu-settings").is_some()
            && cx.debug_bounds("menu-about").is_some()
    );
    click(cx, "menu-settings");
    assert!(view.read_with(cx, |view, _| view.tab == Tab::Onboarding
        && view.shell.overlays.top().is_some_and(|layer| matches!(
            layer.kind,
            Surface::GuideInformation(Tab::Settings)
        ))));
    click(cx, "settings-appearance");
    crate::test_support::press(cx, "end enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.preferences.appearance),
        crate::Appearance::Dark
    );
    click(cx, "cancel-preferences");
    assert!(view.read_with(cx, |view, cx| view.tab == Tab::Onboarding && !view.shell.overlays.is_open()
        && view.preferences == view.saved_preferences && view.settings_form.is_none()
        && matches!(&view.form, Some(Form::Instance(form)) if form.name.read(cx).value() == "Kept guide 世界")));
}
