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
fn repair_uses_the_inspected_ready_profile_without_changing_home(cx: &mut TestAppContext) {
    use std::{
        io::{Read as _, Write as _},
        net::TcpListener,
        sync::Arc,
        time::{Duration, Instant},
    };

    let result = (|| -> anyhow::Result<()> {
        for mode in [ProfileMode::Bundled, ProfileMode::Separated] {
            let proxy = TcpListener::bind("127.0.0.1:0")?;
            proxy.set_nonblocking(true)?;
            let proxy_url = format!("http://{}", proxy.local_addr()?);
            let server = std::thread::spawn(move || -> anyhow::Result<()> {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    match proxy.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false)?;
                            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                            let mut bytes = [0; 4096];
                            let _ = stream.read(&mut bytes)?;
                            stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
                            return Ok(());
                        }
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
            });
            let root = tempfile::tempdir()?;
            let mut preferences = crate::appearance::Preferences::default();
            preferences.downloads.proxy = block_launcher::ProxyMode::Http { url: proxy_url };
            preferences.downloads.max_retries = 0;
            preferences.save(root.path())?;
            let library = block_launcher::library::Library::new(
                root.path().to_owned(),
                Arc::new(|_| {}),
                Arc::new(|_| {}),
            )?;
            let catalog = library.catalog();
            let (first, chosen) =
                catalog.create_instance("Home game".into(), "1.20.6".into(), None, None)?;
            let (damaged, inspected) =
                catalog.create_instance("Damaged game".into(), "1.20.6".into(), None, None)?;
            for instance in [&first, &damaged] {
                let mut target = instance.targets[0].clone();
                target.status = InstallStatus::Ready;
                catalog.set_target(&instance.id, &target)?;
            }
            let world = damaged.directory.join("saves/Test world/level.dat");
            let options = damaged.directory.join("options.txt");
            std::fs::create_dir_all(
                world
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("world path"))?,
            )?;
            std::fs::write(&world, b"world marker")?;
            std::fs::write(&options, b"configuration marker")?;

            // This Ready target has no installed metadata. The real launch
            // boundary refuses it and asks for Repair before authentication.
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let refusal = runtime.block_on(library.prepare_launch(
                &inspected.id,
                &block_launcher::session::OfflineSession::Offline {
                    username: "Player".into(),
                },
                &block_launcher::CancellationToken::new(),
                None,
                None,
            ));
            assert!(
                matches!(refusal, Err(ref error) if error.to_string().contains("choose Repair"))
            );

            let mut snapshot = catalog.snapshot()?;
            snapshot.profile_mode = mode;
            let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
            let chosen_id = chosen.id.clone();
            let inspected_id = inspected.id.clone();
            let first_id = first.id.clone();
            let damaged_id = damaged.id.clone();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.snapshot = snapshot;
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.selected_instance = Some(first_id.clone());
                view.selected_profile = Some(chosen_id.clone());
                view.shell.games.inspected = Some(inspected_id.clone());
                view.shell.profiles.inspected = Some(inspected_id.clone());
                view.router.navigate(if mode == ProfileMode::Bundled {
                    Tab::Instances
                } else {
                    Tab::Profiles
                });
                view.initialize_browsers(cx);
                view
            });
            cx.simulate_resize(size(px(960.), px(600.)));
            cx.run_until_parked();
            let repair = crate::test_support::control_bounds(cx, "repair-profile");
            let edit = crate::test_support::control_bounds(cx, "edit-profile");
            let choose = crate::test_support::control_bounds(cx, "use-profile");
            assert!(
                repair
                    .zip(edit)
                    .zip(choose)
                    .is_some_and(|((repair, edit), choose)| {
                        repair.left() >= edit.right()
                            && repair.right() <= choose.left()
                            && repair.bottom() <= px(600.)
                            && choose.right() <= px(960.)
                    }),
                "the repair and existing actions must fit in the compact detail footer: {repair:?} {edit:?} {choose:?}"
            );
            click(cx, "repair-profile");
            assert!(view.read_with(cx, |view, _| view.busy));
            let deadline = Instant::now() + Duration::from_secs(5);
            while view.read_with(cx, |view, _| view.busy) && Instant::now() < deadline {
                view.update(cx, Block::poll);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(!view.read_with(cx, |view, _| view.busy));
            server
                .join()
                .map_err(|_| anyhow::anyhow!("proxy worker failed"))??;
            let state = catalog.snapshot()?;
            assert!(matches!(
                state.selection(&inspected.id)?.2.status,
                InstallStatus::Failed { .. }
            ));
            assert_eq!(state.selection(&chosen.id)?.2.status, InstallStatus::Ready);
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_profile.clone()),
                Some(chosen_id)
            );
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_instance.clone()),
                Some(first_id)
            );
            assert_eq!(std::fs::read(world)?, b"world marker");
            assert_eq!(std::fs::read(options)?, b"configuration marker");
            assert_eq!(state.selection(&inspected.id)?.0.id, damaged_id);
        }
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn crash_return_reconciles_selection_and_inspection_in_both_modes(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        for mode in [ProfileMode::Bundled, ProfileMode::Separated] {
            let root = tempfile::tempdir()?;
            let guard = std::fs::File::create(root.path().join("presentation.lock"))?;
            let report = block_launcher::crash::CrashReport::new(
                block_launcher::crash::CrashContext {
                    run_id: "run-a".into(),
                    profile_id: "profile-a".into(),
                    profile_name: "Test profile".into(),
                    minecraft_version: "1.20.6".into(),
                    loader: None,
                },
                block_launcher::crash::ExitOutcome::start_failed(),
                "Synthetic start failure",
                Vec::new(),
            );
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = fixture(window, cx, mode);
                view.selected_profile = Some("profile-b".into());
                view.selected_instance = Some("instance-b".into());
                view.shell.games.inspected = Some(
                    if mode == ProfileMode::Bundled {
                        "profile-b"
                    } else {
                        "instance-b"
                    }
                    .into(),
                );
                view.shell.profiles.inspected = Some("profile-b".into());
                view.pending_crashes.push_back(crate::PendingCrash {
                    report,
                    directory: root.path().to_path_buf(),
                    guard,
                });
                view
            });
            cx.simulate_resize(size(px(1152.), px(900.)));
            click(cx, "crash-return");
            view.read_with(cx, |view, _| {
                assert_eq!(
                    view.selected_profile.as_deref(),
                    Some("profile-a"),
                    "{mode:?}"
                );
                assert_eq!(
                    view.selected_instance.as_deref(),
                    Some("instance-a"),
                    "{mode:?}"
                );
                assert_eq!(
                    view.inspected_instance()
                        .map(|instance| instance.id.as_str()),
                    Some("instance-a"),
                    "{mode:?}"
                );
                assert_eq!(
                    view.inspected_profile().map(|profile| profile.id.as_str()),
                    Some("profile-a"),
                    "{mode:?}"
                );
            });
            if mode == ProfileMode::Separated {
                click(cx, "linked-profile-profile-a");
            }
            click(cx, "edit-profile");
            assert!(
                view.read_with(cx, |view, _| matches!(
                    &view.form,
                    Some(crate::Form::Profile(form)) if form.original.id == "profile-a"
                )),
                "Edit must target the requested profile in {mode:?}"
            );
        }
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
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
