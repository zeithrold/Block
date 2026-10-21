//! User outcomes across focus, cancellation and asynchronous page lifetimes.
use super::{Block, Drawer, Tab, UiAction, engine::Message, shell::Surface};
use gpui::{
    Focusable as _, Modifiers,
    accesskit::{Action, ActionRequest, NodeId, Role, TreeId, TreeUpdate},
    px, size,
};

/// Read the completed official tree after Base Root and its deferred modal have
/// painted. No synthetic focus is installed while settling the first frame.
fn completed_tree(cx: &mut gpui::VisualTestContext) -> TreeUpdate {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|window, _| window.accessibility_tree())
}

fn actionable_nodes(tree: &TreeUpdate) -> Vec<NodeId> {
    let nodes = tree
        .nodes
        .iter()
        .filter(|(_, node)| {
            node.supports_action(Action::Click) || node.supports_action(Action::Focus)
        })
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    assert!(
        !nodes.is_empty(),
        "the real background must contain controls"
    );
    nodes
}

fn descendants(tree: &TreeUpdate, root: NodeId) -> Vec<NodeId> {
    let mut pending = vec![root];
    let mut found = Vec::new();
    while let Some(id) = pending.pop() {
        if found.contains(&id) {
            continue;
        }
        found.push(id);
        if let Some((_, node)) = tree.nodes.iter().find(|(candidate, _)| *candidate == id) {
            pending.extend_from_slice(node.children());
        }
    }
    found
}

fn assert_modal_tree(tree: &TreeUpdate, role: Role, title: &str, background: &[NodeId]) {
    let outline = tree
        .nodes
        .iter()
        .map(|(id, node)| (*id, node.role(), node.label(), node.children()))
        .collect::<Vec<_>>();
    let dialogs = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::Dialog)
        .collect::<Vec<_>>();
    assert!(
        dialogs
            .iter()
            .all(|(_, node)| node.label().is_some_and(|label| !label.trim().is_empty())),
        "a Base host must not add an unnamed Dialog: {outline:?}"
    );
    if role == Role::Dialog {
        assert_eq!(
            dialogs.len(),
            1,
            "only the top named Dialog belongs in the final tree: {outline:?}"
        );
    }
    let surfaces = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == role && node.label() == Some(title))
        .collect::<Vec<_>>();
    assert_eq!(
        surfaces.len(),
        1,
        "expected one {role:?} named {title:?}: {outline:?}"
    );
    if let Some((id, _)) = surfaces.first() {
        assert!(
            descendants(tree, *id).contains(&tree.focus),
            "initial final-tree focus {:?} must belong to {role:?} {title:?}, never fall back to Window: {outline:?}",
            tree.focus,
        );
    }
    assert!(
        tree.nodes.iter().all(|(id, _)| !background.contains(id)),
        "inert background controls must be absent from the final modal tree: {outline:?}"
    );
}

#[gpui::test]
fn games_help_final_tree_has_one_named_dialog_and_initial_modal_focus(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.saved_preferences.reduce_motion = true;
        view.snapshot = crate::test_support::ready_snapshot();
        view.initialize_browsers(cx);
        view
    });
    let background = actionable_nodes(&completed_tree(cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::GamesHelp, window, cx);
        });
    });
    assert_modal_tree(
        &completed_tree(cx),
        Role::Dialog,
        "Instances and Profiles",
        &background,
    );
}

#[gpui::test]
fn nested_account_confirmation_final_tree_filters_parent_and_restores_its_focus(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let account = crate::test_support::test_account()?;
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.saved_preferences.reduce_motion = true;
            view.accounts = vec![account];
            view
        });
        let background = actionable_nodes(&completed_tree(cx));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Drawer(Some(Drawer::Accounts)), window, cx);
            });
        });
        let parent = completed_tree(cx);
        assert_modal_tree(&parent, Role::Dialog, "Switch account", &background);
        let parent_controls = actionable_nodes(&parent);
        let remove = parent
            .nodes
            .iter()
            .find(|(_, node)| node.role() == Role::Button && node.label() == Some("Remove"))
            .map(|(id, _)| *id);
        assert!(
            remove.is_some(),
            "the saved account must expose its real Remove action"
        );
        if let Some(id) = remove {
            cx.update(|window, cx| {
                window.simulate_accessibility_action(
                    ActionRequest {
                        action: Action::Click,
                        target_node: id,
                        target_tree: TreeId::ROOT,
                        data: None,
                    },
                    cx,
                );
            });
        }
        let nested = completed_tree(cx);
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.overlays.layers().count()),
            2
        );
        assert_modal_tree(&nested, Role::Dialog, "Remove account", &parent_controls);
        assert!(nested.nodes.iter().all(|(id, _)| !background.contains(id)));
        crate::test_support::press(cx, "escape");
        let restored = completed_tree(cx);
        assert_modal_tree(&restored, Role::Dialog, "Switch account", &background);
        // The official Click fallback sends mouse down/up. Base focuses Remove
        // before its handler opens the nested layer, so this actual trigger,
        // rather than the previously focused search input, owns restoration.
        assert_eq!(
            Some(restored.focus),
            remove,
            "Escape restores the actual Remove trigger in the final official tree"
        );
        assert!(restored.nodes.iter().any(|(id, node)| *id == restored.focus
            && node.role() == Role::Button
            && node.label() == Some("Remove")));
        assert_eq!(view.read_with(cx, |view, _| view.accounts.len()), 1);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn product_listbox_final_tree_has_no_unnamed_host_and_initial_option_focus(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.saved_preferences.reduce_motion = true;
        view
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
        });
    });
    let background = actionable_nodes(&completed_tree(cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(
                UiAction::OpenProductSelect(crate::select::ProductChoice::Appearance),
                window,
                cx,
            );
        });
    });
    let tree = completed_tree(cx);
    assert_modal_tree(&tree, Role::ListBox, "Appearance", &background);
    assert!(tree.nodes.iter().any(|(id, node)| *id == tree.focus
        && node.role() == Role::ListBoxOption
        && node.label() == Some("Follow system")));
}

#[gpui::test]
fn app_menu_final_tree_has_no_unnamed_host_and_initial_current_item_focus(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.saved_preferences.reduce_motion = true;
        view.tab = Tab::Home;
        view
    });
    let background = actionable_nodes(&completed_tree(cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Drawer(Some(Drawer::Menu)), window, cx);
        });
    });
    let tree = completed_tree(cx);
    assert_modal_tree(&tree, Role::Menu, "Block menu", &background);
    assert!(tree.nodes.iter().any(|(id, node)| *id == tree.focus
        && node.role() == Role::MenuItem
        && node.label() == Some("Home")));
}

#[gpui::test]
fn nested_escape_restores_parent_focus_and_backdrop_does_not_activate_chrome(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, super::tests::test_view);
    cx.simulate_resize(size(px(432.), px(600.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(UiAction::Drawer(Some(Drawer::Accounts)), window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.shell
                .overlays
                .open(Surface::RemoveAccount("synthetic".into()), window, cx);
            cx.notify();
        });
    });
    cx.run_until_parked();
    crate::test_support::press(cx, "escape");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.drawer()),
        Some(Drawer::Accounts)
    );
    for _ in 0..10 {
        crate::test_support::press(cx, "tab");
    }
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            assert!(
                view.shell
                    .overlays
                    .top()
                    .is_some_and(|layer| layer.focus.contains_focused(window, cx))
            );
        });
    });
    cx.simulate_click(gpui::point(px(424.), px(4.)), Modifiers::none());
    assert!(!view.read_with(cx, |view, _| view.shell.overlays.is_open()));
    // The same point is over window chrome; dismissal must not close the window.
    cx.update(|window, _| assert_eq!(window.viewport_size(), size(px(432.), px(600.))));
}

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
            assert!(view.shell.overlays.top().is_some_and(|layer| matches!(layer.kind, Surface::GuideInformation(Tab::Settings))));
            view.handle(UiAction::Drawer(None), window, cx);
            assert_eq!(view.preferences.appearance, super::Appearance::System);
            assert!(view.settings_form.is_none());
            assert!(matches!(&view.form, Some(crate::Form::Instance(form)) if form.name.read(cx).value() == "Retain 世界"));
            view.handle(UiAction::Tab(Tab::About), window, cx);
        });
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("about-body").is_some());
    crate::test_support::press(cx, "escape");
    assert!(view.read_with(cx, |view, cx| view.tab == Tab::Onboarding && !view.shell.overlays.is_open()
        && matches!(&view.form, Some(crate::Form::Instance(form)) if form.name.read(cx).value() == "Retain 世界")));
}
