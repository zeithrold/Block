//! Browser outcomes use synthetic records, never credentials or player files.

use block_launcher::catalog::{InstallStatus, LaunchTarget, LoaderKind, ProfileMode};
use gpui::{EntityInputHandler as _, Focusable as _, Modifiers, TestAppContext, px, size};

use super::{CollectionLoad, RowIcon};
use crate::{
    Block, Tab, UiAction,
    test_support::{ready_snapshot, test_account},
};

fn fixture(
    window: &mut gpui::Window,
    cx: &mut gpui::Context<'_, Block>,
    mode: ProfileMode,
) -> Block {
    let mut view = crate::tests::test_view(window, cx);
    view.preferences.reduce_motion = true;
    view.saved_preferences.reduce_motion = true;
    view.snapshot = ready_snapshot();
    view.snapshot.profile_mode = mode;
    let mut instance = view.snapshot.instances[0].clone();
    instance.id = "instance-b".into();
    instance.name = "Modded Instance".into();
    instance.minecraft_version = "1.21.1".into();
    instance.targets.push(LaunchTarget {
        id: "neoforge-b".into(),
        kind: LoaderKind::NeoForge,
        loader_version: Some("21.1.90".into()),
        manifest: None,
        status: InstallStatus::Ready,
        optifine: None,
        optifine_mod: None,
    });
    let mut profile = view.snapshot.profiles[0].clone();
    profile.id = "profile-b".into();
    profile.name = "Adventure 中文".into();
    profile.instance_id.clone_from(&instance.id);
    profile.target_id = "neoforge-b".into();
    view.snapshot.instances.push(instance);
    view.snapshot.profiles.push(profile);
    view.selected_instance = Some("instance-a".into());
    view.selected_profile = Some("profile-a".into());
    view.router.navigate(Tab::Instances);
    view.initialize_browsers(cx);
    view
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = crate::test_support::control_bounds(cx, selector);
    assert!(
        bounds.is_some(),
        "required control did not paint: {selector}"
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
}

#[gpui::test]
fn separated_categories_roving_tabs_move_focus_and_expose_only_the_current_panel(
    cx: &mut TestAppContext,
) {
    let (view, cx) =
        crate::test_support::window(cx, |window, cx| fixture(window, cx, ProfileMode::Separated));
    cx.simulate_resize(size(px(960.), px(600.)));
    click(cx, "games-tab-instances");
    for (key, expected, _label) in [
        ("right", Tab::Profiles, "Profile"),
        ("right", Tab::Instances, "Instance"),
        ("end", Tab::Profiles, "Profile"),
        ("home", Tab::Instances, "Instance"),
    ] {
        crate::test_support::press(cx, key);
        assert_eq!(view.read_with(cx, |view, _| view.router.tab()), expected);
    }
    crate::test_support::press(cx, "tab");

    // Category changes never alter Home's chosen Profile.
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_profile.clone())
            .as_deref(),
        Some("profile-a")
    );
}

#[gpui::test]
fn bundled_games_are_profiles_joined_to_their_real_instance_and_target(cx: &mut TestAppContext) {
    let (view, cx) =
        crate::test_support::window(cx, |window, cx| fixture(window, cx, ProfileMode::Bundled));
    view.update(cx, |view, cx| {
        let records = view.browser_records(cx);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].title, "Test profile");
        assert!(matches!(records[1].icon, RowIcon::Profile(_)));
        assert!(
            records[1]
                .badges
                .iter()
                .any(|(value, _)| value == "NeoForge 21.1.90")
        );
        if let Some(query) = &view.shell.games.query {
            query.update(cx, |field, cx| field.set_value("1.21.1 neoforge", cx));
        }
        // The actual search is a substring; unlike demo data, source values come
        // from the joined Profile, Instance and selected real launch target.
        if let Some(query) = &view.shell.games.query {
            query.update(cx, |field, cx| field.set_value("neoforge", cx));
        }
        let records = view.browser_records(cx);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "profile-b");
        assert_eq!(view.selected_profile.as_deref(), Some("profile-a"));
    });
}

#[gpui::test]
fn inspection_and_current_disabled_action_do_not_change_home_until_use(cx: &mut TestAppContext) {
    let (view, cx) =
        crate::test_support::window(cx, |window, cx| fixture(window, cx, ProfileMode::Bundled));
    cx.simulate_resize(size(px(960.), px(600.)));
    click(cx, "instance-instance-b");
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.inspected_profile().map(|profile| profile.id.as_str()),
            Some("profile-b")
        );
        assert_eq!(view.selected_profile.as_deref(), Some("profile-a"));
    });
    click(cx, "use-profile");
    view.read_with(cx, |view, _| {
        assert_eq!(view.selected_profile.as_deref(), Some("profile-b"));
        assert_eq!(view.selected_instance.as_deref(), Some("instance-b"));
        assert!(view.shell.toast.is_some());
        assert_eq!(view.router.tab(), Tab::Instances);
    });
    view.update(cx, |view, cx| {
        view.shell.toast = None;
        cx.notify();
    });
    click(cx, "use-profile");
    assert!(
        view.read_with(cx, |view, _| view.shell.toast.is_none()),
        "Current game must not activate again"
    );
}

#[gpui::test]
fn linked_profile_and_instance_navigation_preserves_selection_and_each_query(
    cx: &mut TestAppContext,
) {
    let (view, cx) =
        crate::test_support::window(cx, |window, cx| fixture(window, cx, ProfileMode::Separated));
    cx.simulate_resize(size(px(960.), px(600.)));
    view.update(cx, |view, cx| {
        if let Some(query) = &view.shell.games.query {
            query.update(cx, |field, cx| field.set_value("Test", cx));
        }
        if let Some(query) = &view.shell.profiles.query {
            query.update(cx, |field, cx| field.set_value("Adventure", cx));
        }
        view.shell.games.inspected = Some("instance-b".into());
        cx.notify();
    });
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        view.shell.games.detail_scroll.scroll_to_bottom();
        cx.notify();
    });
    click(cx, "linked-profile-profile-b");
    view.read_with(cx, |view, cx| {
        assert_eq!(view.router.tab(), Tab::Profiles);
        assert_eq!(
            view.inspected_profile().map(|profile| profile.id.as_str()),
            Some("profile-b")
        );
        assert_eq!(view.selected_profile.as_deref(), Some("profile-a"));
        assert_eq!(
            view.shell
                .profiles
                .query
                .as_ref()
                .map(|field| field.read(cx).value())
                .as_deref(),
            Some("Adventure")
        );
    });
    click(cx, "linked-instance");
    view.read_with(cx, |view, cx| {
        assert_eq!(view.router.tab(), Tab::Instances);
        assert_eq!(
            view.inspected_instance()
                .map(|instance| instance.id.as_str()),
            Some("instance-b")
        );
        assert_eq!(
            view.shell
                .games
                .query
                .as_ref()
                .map(|field| field.read(cx).value())
                .as_deref(),
            Some("Test")
        );
        assert_eq!(view.selected_profile.as_deref(), Some("profile-a"));
    });
}

#[gpui::test]
fn search_keyboard_roves_without_selecting_or_interrupting_chinese_composition(
    cx: &mut TestAppContext,
) {
    let (view, cx) =
        crate::test_support::window(cx, |window, cx| fixture(window, cx, ProfileMode::Bundled));
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(query) = &view.shell.games.query {
                query.read(cx).focus_handle(cx).focus(window, cx);
            }
        });
    });
    crate::test_support::press(cx, "down down");
    assert_eq!(
        view.read_with(cx, |view, _| view
            .inspected_profile()
            .map(|profile| profile.id.clone()))
            .as_deref(),
        Some("profile-b")
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_profile.clone())
            .as_deref(),
        Some("profile-a")
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(query) = &view.shell.games.query {
                query.read(cx).focus_handle(cx).focus(window, cx);
                query.update(cx, |field, cx| {
                    field.replace_and_mark_text_in_range(None, "中", Some(1..1), window, cx);
                });
            }
        });
    });
    crate::test_support::press(cx, "up enter");
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.inspected_profile().map(|profile| profile.id.as_str()),
            Some("profile-b")
        );
        assert_eq!(view.selected_profile.as_deref(), Some("profile-a"));
        assert!(
            view.shell
                .games
                .query
                .as_ref()
                .is_some_and(|field| field.read(cx).is_composing())
        );
    });
}

#[gpui::test]
fn details_keep_fixed_action_rail_and_selected_game(cx: &mut TestAppContext) {
    let (view, cx) =
        crate::test_support::window(cx, |window, cx| fixture(window, cx, ProfileMode::Bundled));
    cx.simulate_resize(size(px(1152.), px(720.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Inspect("profile-b".into()), window, cx);
        });
    });
    cx.run_until_parked();
    let footer = cx.debug_bounds("detail-footer");
    assert!(footer.is_some());
    if let Some(footer) = footer {
        assert_eq!(footer.size.height, px(72.));
        assert!(footer.bottom() <= px(720.));
        assert!(footer.left() >= px(0.) && footer.right() <= px(1152.));
    }
    let use_game = cx.debug_bounds("use-profile");
    assert!(
        use_game.is_some_and(|bounds| bounds.bottom() <= px(720.) && bounds.right() <= px(1152.))
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_profile.clone())
            .as_deref(),
        Some("profile-a")
    );
}

#[gpui::test]
fn load_error_preserves_old_records_and_successful_update_recovers_typed_state(
    cx: &mut TestAppContext,
) {
    let (view, cx) =
        crate::test_support::window(cx, |window, cx| fixture(window, cx, ProfileMode::Bundled));
    view.update(cx, |view, cx| {
        view.shell.games.load = CollectionLoad::Failed("Read failed in isolated fixture".into());
        assert_eq!(view.browser_records(cx).len(), 2);
        assert_eq!(view.selected_profile.as_deref(), Some("profile-a"));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("reload-library").is_some());
    view.update(cx, |view, cx| {
        view.shell.games.load = CollectionLoad::Ready;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("reload-library").is_none());
    assert_eq!(
        view.read_with(cx, |view, cx| view.browser_records(cx).len()),
        2
    );
}

#[gpui::test]
fn account_inspection_selection_and_remove_cancel_keep_real_boundaries(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = fixture(window, cx, ProfileMode::Bundled);
        view.router.navigate(Tab::Accounts);
        if let Ok(first) = test_account() {
            let mut second = first.clone();
            second.player_name = "SecondPlayer".into();
            second.reference.player_id = "00000000-0000-4000-8000-000000000002"
                .parse()
                .ok()
                .unwrap_or_default();
            view.selected_account = Some(crate::account_id(&first));
            view.accounts = vec![first, second];
        }
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    let first = view.read_with(cx, |view, _| view.selected_account.clone());
    let second = view.read_with(cx, |view, _| view.accounts.get(1).map(crate::account_id));
    assert!(second.is_some() && first != second);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(id) = second.clone() {
                view.handle(UiAction::Inspect(id), window, cx);
            }
        });
    });
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_account.clone()),
        first
    );
    click(cx, "account-logout-00000000-0000-4000-8000-000000000002");
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
    click(cx, "close-drawer");
    assert_eq!(view.read_with(cx, |view, _| view.accounts.len()), 2);
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_account.clone()),
        first
    );
}

#[gpui::test]
fn profile_actions_keep_the_captured_profile_when_inspection_refreshes(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = fixture(window, cx, ProfileMode::Separated);
        view.router.navigate(Tab::Profiles);
        view.shell.profiles.inspected = Some("profile-b".into());
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    click(cx, "profile-actions");
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
    view.update(cx, |view, cx| {
        // A catalog/inspection refresh must not retarget an already-open menu.
        view.shell.profiles.inspected = Some("profile-a".into());
        cx.notify();
    });
    click(cx, "select-choice-1");
    view.read_with(cx, |view, _| {
        assert!(view.shell.surfaces.top().is_some_and(|layer| matches!(&layer.kind, crate::shell::Surface::DeleteProfile(id) if id == "profile-b")));
        assert_eq!(view.snapshot.profiles.len(), 2);
        assert_eq!(view.selected_profile.as_deref(), Some("profile-a"));
    });
    click(cx, "close-drawer");
    assert_eq!(
        view.read_with(cx, |view, _| view.snapshot.profiles.len()),
        2
    );
}

#[gpui::test]
fn component_cards_open_and_return_to_their_own_editor_context(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = fixture(window, cx, ProfileMode::Bundled);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        cx.simulate_resize(size(px(1152.), px(720.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Inspect("profile-a".into()), window, cx);
            });
        });
        cx.run_until_parked();
        let trigger = view.read_with(cx, |view, cx| {
            view.shell.surfaces.trigger_focus("component-optifine", cx)
        });
        click(cx, "component-optifine");
        assert_eq!(
            view.read_with(cx, |view, _| view
                .shell
                .instance_editor
                .as_ref()
                .map(|editor| editor.identity().to_owned())),
            Some("instance-a".to_owned())
        );
        let body = cx.debug_bounds("instance-editor-body");
        let action = cx.debug_bounds("instance-optifine-select");
        assert!(
            action.zip(body).is_some_and(|(action, body)| {
                action.top() >= body.top() && action.bottom() <= body.bottom()
            }),
            "the OptiFine action is revealed in the editor: {action:?} {body:?}"
        );
        crate::test_support::press(cx, "enter");
        assert!(cx.did_prompt_for_paths(), "OptiFine action has focus");
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        click(cx, "cancel-instance-settings");
        cx.update(|window, _| assert!(trigger.is_focused(window)));

        click(cx, "component-instance-settings");
        let initial = cx.update(|window, cx| window.focused(cx));
        let name = cx.debug_bounds("instance-field-Name");
        assert!(name.is_some(), "generic settings show the name field");
        if let Some(name) = name {
            cx.simulate_click(
                gpui::point(name.center().x, name.bottom() - px(12.)),
                Modifiers::none(),
            );
        }
        assert_eq!(
            cx.update(|window, cx| window.focused(cx)),
            initial,
            "generic settings initially focus the instance name"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}
