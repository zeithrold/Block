use std::{collections::BTreeSet, fs};

use super::{
    Tab, UiAction,
    engine::Engine,
    first_run::{Action as FirstRunAction, Stage},
    import_games::{Action as ImportAction, DeferredDiscovery},
};
use block_launcher::game_import::{
    DiscoveryReport, GameCandidate, ImportSourceReport, LoaderKind, SourceKind,
};

fn bounds(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
) -> anyhow::Result<gpui::Bounds<gpui::Pixels>> {
    crate::test_support::control_bounds(cx, selector)
        .ok_or_else(|| anyhow::anyhow!("missing rendered control: {selector}"))
}

fn candidate(id: &str, source: SourceKind, importable: bool) -> GameCandidate {
    GameCandidate {
        id: id.into(),
        source,
        source_root: std::path::PathBuf::from(format!("/synthetic/{source:?}")),
        name: format!("Game {id}"),
        game_directory: std::path::PathBuf::from(format!("/synthetic/{id}")),
        configured_version: Some("release".into()),
        minecraft_version: Some("1.20.6".into()),
        loader: Some(block_launcher::game_import::DetectedLoader {
            kind: LoaderKind::Vanilla,
            version: None,
        }),
        latest_release: None,
        latest_snapshot: None,
        importable,
        unsupported_reason: (!importable).then(|| "This launcher profile is unsupported.".into()),
        metadata_warning: None,
    }
}

fn discovery_report() -> DiscoveryReport {
    DiscoveryReport {
        sources: vec![
            ImportSourceReport {
                kind: SourceKind::AdjacentMinecraft,
                root: "/synthetic/.minecraft".into(),
                candidates_found: 1,
                warning: None,
            },
            ImportSourceReport {
                kind: SourceKind::OfficialLauncher,
                root: "/synthetic/launcher".into(),
                candidates_found: 2,
                warning: None,
            },
        ],
        candidates: vec![
            candidate("adjacent-one", SourceKind::AdjacentMinecraft, true),
            candidate("launcher-one", SourceKind::OfficialLauncher, true),
            candidate("launcher-two", SourceKind::OfficialLauncher, true),
            candidate("launcher-unsupported", SourceKind::OfficialLauncher, false),
        ],
    }
}

fn version_entry(
    id: &str,
    kind: block_launcher::manifest::model::VersionType,
) -> block_launcher::manifest::model::VersionEntry {
    block_launcher::manifest::model::VersionEntry {
        id: id.into(),
        kind,
        url: "https://example.invalid/manifest".into(),
        time: "2024-01-01T00:00:00+00:00".into(),
        release_time: "2024-01-01T00:00:00+00:00".into(),
        sha1: "0".repeat(40),
        compliance_level: 1,
    }
}

#[gpui::test]
fn begin_first_run_starts_welcome_at_home_size_and_resumes_after_setup_started(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        let home = cx.update(|window, _| crate::window_geometry::home_size(window));
        cx.simulate_resize(home);
        cx.run_until_parked();
        cx.update(|window, cx| {
            view.update(cx, |view, cx| view.begin_first_run(cx));
            assert_eq!(window.viewport_size(), home);
        });
        assert!(view.read_with(cx, |view, _| {
            view.router.tab() == Tab::Onboarding
                && view.shell.first_run.enabled
                && view.shell.first_run.stage == Stage::Welcome
        }));

        fs::write(root.path().join("setup-started"), b"1")?;
        view.update(cx, |view, cx| view.begin_first_run(cx));
        assert!(view.read_with(cx, |view, _| {
            view.router.tab() == Tab::Onboarding && view.shell.first_run.stage == Stage::Overview
        }));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn welcome_start_defers_the_drawer_and_empty_discovery_continues_to_the_overview(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        let request = cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.begin_first_run(cx);
                view.handle_first_run(FirstRunAction::Start, window, cx);
                assert!(
                    view.shell.import_games.deferred_open,
                    "a Welcome start must scan before showing the drawer"
                );
                assert!(
                    !view.shell.surfaces.is_open(),
                    "the drawer must not flash before the scan resolves"
                );
                let request = view.shell.import_games.discovery;
                assert!(
                    request.is_some(),
                    "Welcome start begins a discovery request"
                );
                request.unwrap_or_else(uuid::Uuid::nil)
            })
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let decision = view.resolve_game_discovery(request, DiscoveryReport::default());
                assert_eq!(decision, DeferredDiscovery::ContinueToOverview);
                assert!(!view.shell.surfaces.is_open());
                view.apply_deferred_discovery(decision, window, cx);
            });
        });
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == Stage::Overview && !view.shell.surfaces.is_open()
        }));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn welcome_start_opens_the_drawer_only_when_games_are_found(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        let request = cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.begin_first_run(cx);
                view.handle_first_run(FirstRunAction::Start, window, cx);
                let request = view.shell.import_games.discovery;
                assert!(
                    request.is_some(),
                    "Welcome start begins a discovery request"
                );
                request.unwrap_or_else(uuid::Uuid::nil)
            })
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let decision = view.resolve_game_discovery(request, discovery_report());
                assert_eq!(decision, DeferredDiscovery::OpenDrawer);
                view.apply_deferred_discovery(decision, window, cx);
            });
        });
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == Stage::Welcome
                && view.shell.import_games.report.is_some()
                && view.shell.surfaces.top().is_some_and(|layer| {
                    matches!(
                        &layer.kind,
                        crate::shell::Surface::Drawer(crate::shell::Drawer::ImportGames)
                    )
                })
        }));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn explicit_import_from_the_overview_opens_the_drawer_immediately(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        fs::write(root.path().join("setup-started"), b"1")?;
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.begin_first_run(cx);
                assert_eq!(view.shell.first_run.stage, Stage::Overview);
                view.handle_first_run(FirstRunAction::Import, window, cx);
                assert!(
                    !view.shell.import_games.deferred_open,
                    "an explicit import must show its drawer without scanning first"
                );
                assert!(view.shell.surfaces.is_open());
            });
        });
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn setup_navigation_stays_inside_first_run_even_while_auth_uses_accounts_tab(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.begin_first_run(cx);
                view.open_auth_wizard(window, cx);
                assert_eq!(view.router.tab(), Tab::Accounts);
                view.handle(UiAction::Tab(Tab::Instances), window, cx);
                for drawer in [
                    crate::shell::Drawer::Menu,
                    crate::shell::Drawer::Accounts,
                    crate::shell::Drawer::Profiles,
                    crate::shell::Drawer::Downloads,
                ] {
                    view.handle(UiAction::Drawer(Some(drawer)), window, cx);
                    assert!(
                        !view.shell.surfaces.is_open(),
                        "setup must own the allowed navigation routes"
                    );
                }
            });
        });
        assert!(view.read_with(cx, |view, _| {
            view.router.tab() == Tab::Accounts
                && view.shell.auth.is_some()
                && view.shell.first_run.enabled
        }));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn back_hides_custom_game_draft_without_resetting_it_or_leaking_into_account_flow(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.begin_first_run(cx);
            view.handle_first_run(FirstRunAction::Game, window, cx);
            view.handle_first_run(FirstRunAction::Custom, window, cx);
            if let Some(super::Form::Instance(form)) = &mut view.form {
                form.name
                    .update(cx, |field, cx| field.set_value("Preserved game", cx));
                form.version
                    .update(cx, |field, cx| field.set_value("1.20.6", cx));
                form.loader = block_launcher::catalog::LoaderKind::Fabric;
                form.loader_version = Some("0.15.11".into());
            }
            view.handle_first_run(FirstRunAction::Back, window, cx);
        });
    });
    assert!(view.read_with(cx, |view, cx| {
        view.shell.first_run.stage == Stage::Overview
            && !view.wizard_task_active()
            && matches!(&view.form, Some(super::Form::Instance(form))
                if form.name.read(cx).value() == "Preserved game"
                    && form.version.read(cx).value() == "1.20.6"
                    && form.loader == block_launcher::catalog::LoaderKind::Fabric
                    && form.loader_version.as_deref() == Some("0.15.11"))
    }));

    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle_first_run(FirstRunAction::Account, window, cx);
        });
    });
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| {
        view.shell.auth.is_some()
            && view.shell.first_run.stage == Stage::Overview
            && view.wizard_task_active()
    }));
    assert!(crate::test_support::control_bounds(cx, "auth-microsoft").is_some());
    assert!(crate::test_support::control_bounds(cx, "wizard-footer").is_none());

    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.cancel_auth(window, cx);
            view.handle_first_run(FirstRunAction::Game, window, cx);
            view.handle_first_run(FirstRunAction::Custom, window, cx);
        });
    });
    assert!(view.read_with(cx, |view, cx| {
        view.shell.first_run.stage == Stage::GameWizard
            && view.wizard_task_active()
            && matches!(&view.form, Some(super::Form::Instance(form))
                if form.name.read(cx).value() == "Preserved game"
                    && form.version.read(cx).value() == "1.20.6"
                    && form.loader == block_launcher::catalog::LoaderKind::Fabric
                    && form.loader_version.as_deref() == Some("0.15.11"))
    }));
}

#[gpui::test]
fn overview_back_to_welcome_restores_the_home_frame_and_keeps_saved_progress(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.preferences.reduce_motion = true;
            view
        });
        fs::write(root.path().join("setup-started"), b"1")?;
        let management = cx.update(|window, _| crate::window_geometry::management_size(window));
        cx.simulate_resize(management);
        cx.run_until_parked();
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.begin_first_run(cx);
                assert_eq!(view.shell.first_run.stage, Stage::Overview);
                assert!(!view.wizard_task_active());
            });
        });
        cx.run_until_parked();
        assert!(
            crate::test_support::control_bounds(cx, "setup-back-welcome").is_some(),
            "the Overview chrome must expose Back to welcome"
        );
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle_first_run(FirstRunAction::BackToWelcome, window, cx);
            });
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.router.tab() == Tab::Onboarding
                && view.shell.first_run.enabled
                && view.shell.first_run.stage == Stage::Welcome
                && view.shell.first_run.entered.is_none()
                && view.is_home_frame()
        }));
        // `request_frame_resize` asks the platform for the Home-sized outer
        // width; deliver that resize like the real platform callback would.
        let home = cx.update(|window, _| crate::window_geometry::home_size(window));
        cx.update(|window, cx| window.bounds_changed(cx));
        cx.run_until_parked();
        assert_eq!(
            cx.update(|window, _| window.viewport_size().width),
            home.width,
            "Back to welcome must restore the Home-sized frame"
        );
        assert!(
            root.path().join("setup-started").is_file(),
            "returning to Welcome must keep saved setup progress"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn returning_to_welcome_keeps_the_hidden_game_draft_and_restores_the_home_frame(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.begin_first_run(cx);
            view.handle_first_run(FirstRunAction::Custom, window, cx);
            if let Some(super::Form::Instance(form)) = &view.form {
                form.name
                    .update(cx, |field, cx| field.set_value("Retained game", cx));
            }
            view.handle_first_run(FirstRunAction::Back, window, cx);
            view.handle_first_run(FirstRunAction::BackToWelcome, window, cx);
            assert_eq!(view.shell.first_run.stage, Stage::Welcome);
            assert!(view.is_home_frame());
            assert_eq!(
                window.bounds().size.width,
                crate::window_geometry::home_size(window).width
            );
            assert!(matches!(&view.form, Some(super::Form::Instance(form))
                if form.name.read(cx).value() == "Retained game"));
        });
    });
}

#[gpui::test]
fn welcome_play_hover_resets_after_start_and_overview_back(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.preferences.reduce_motion = true;
            view.begin_first_run(cx);
            view
        });
        let home = cx.update(|window, _| crate::window_geometry::home_size(window));
        let management = cx.update(|window, _| crate::window_geometry::management_size(window));
        cx.simulate_resize(home);
        cx.run_until_parked();

        for _ in 0..2 {
            let play = bounds(cx, "welcome-play")?;
            cx.simulate_mouse_move(play.center(), None, gpui::Modifiers::none());
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                view.shell.first_run.welcome_button.is_hovered()
            }));
            cx.simulate_click(play.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            view.update_in(cx, |view, window, cx| {
                let request = view.shell.import_games.discovery;
                assert!(request.is_some(), "Play must start game discovery");
                let decision = view.resolve_game_discovery(
                    request.unwrap_or_else(uuid::Uuid::nil),
                    DiscoveryReport::default(),
                );
                assert_eq!(decision, DeferredDiscovery::ContinueToOverview);
                view.apply_deferred_discovery(decision, window, cx);
            });
            cx.simulate_resize(management);
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                view.shell.first_run.stage == Stage::Overview
            }));

            let back = bounds(cx, "setup-back-welcome")?;
            cx.simulate_mouse_move(back.center(), None, gpui::Modifiers::none());
            cx.simulate_click(back.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            cx.simulate_resize(home);
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                view.shell.first_run.stage == Stage::Welcome
            }));
            assert!(
                !view.read_with(cx, |view, _| view
                    .shell
                    .first_run
                    .welcome_button
                    .is_hovered()),
                "Back to Welcome must not retain the Play button's previous hover"
            );
            assert!(
                root.path().join("setup-started").is_file(),
                "resetting hover must preserve saved setup progress"
            );

            let play = bounds(cx, "welcome-play")?;
            cx.simulate_mouse_move(play.center(), None, gpui::Modifiers::none());
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                view.shell.first_run.welcome_button.is_hovered()
            }));
            cx.simulate_mouse_move(
                gpui::point(play.left() - gpui::px(12.), play.center().y),
                None,
                gpui::Modifiers::none(),
            );
            cx.run_until_parked();
            assert!(!view.read_with(cx, |view, _| {
                view.shell.first_run.welcome_button.is_hovered()
            }));
        }
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn first_run_keeps_an_existing_saved_library(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let saved = root.path().join("saved-games");
        fs::create_dir_all(&saved)?;
        block_launcher::storage::Locations {
            library: Some(saved.clone()),
            cache: None,
            generation: None,
        }
        .save(root.path())?;
        fs::write(root.path().join("setup-started"), b"1")?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        view.update(cx, |view, cx| view.begin_first_run(cx));
        assert_eq!(
            view.read_with(cx, |view, _| view.first_run_game_parent()),
            saved,
            "automatic selection must not override an existing library"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn overview_footer_keeps_a_single_continue_row(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Onboarding);
        view.shell.first_run.enabled = true;
        view.shell.first_run.stage = Stage::Overview;
        view
    });
    cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
    cx.run_until_parked();
    let finish = crate::test_support::control_bounds(cx, "setup-continue");
    assert!(finish.is_some(), "Continue must stay in the footer");
    assert!(
        crate::test_support::control_bounds(cx, "setup-more-options").is_none(),
        "storage is automatic, so the footer must not offer More options"
    );
    assert!(
        crate::test_support::control_bounds(cx, "setup-overview-skip").is_none(),
        "the Overview footer must not offer Skip setup"
    );
}

#[gpui::test]
fn first_run_defaults_to_the_install_shaped_storage_parent(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join("setup-started"), b"1")?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        view.update(cx, |view, cx| view.begin_first_run(cx));
        let expected = std::env::current_exe()
            .ok()
            .and_then(|executable| crate::setup_storage::automatic_game_parent(&executable));
        let Some(expected) = expected else {
            anyhow::bail!("the test host must classify its own install");
        };
        assert_eq!(
            view.read_with(cx, |view, _| view.first_run_game_parent()),
            expected
        );
        assert!(
            !root.path().join("block.toml").exists(),
            "opening the Overview must not write a storage choice"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn welcome_pointer_events_keep_the_window_alive_and_hit_area_fixed(cx: &mut gpui::TestAppContext) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.begin_first_run(cx);
        view
    });
    cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(720.)));
    cx.run_until_parked();
    let button = crate::test_support::control_bounds(cx, "welcome-play");
    assert!(button.is_some(), "Welcome must expose the start control");
    if let Some(button) = button {
        let outside = gpui::point(button.left() - gpui::px(12.), button.center().y);
        for (position, hovered) in [
            (
                gpui::point(button.left() + gpui::px(4.), button.center().y),
                true,
            ),
            (outside, false),
            (
                gpui::point(button.right() - gpui::px(4.), button.center().y),
                true,
            ),
            (outside, false),
        ] {
            cx.simulate_mouse_move(position, None, gpui::Modifiers::none());
            cx.run_until_parked();
            assert_eq!(
                view.read_with(cx, |view, _| view
                    .shell
                    .first_run
                    .welcome_button
                    .is_hovered()),
                hovered
            );
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(40));
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
            assert_eq!(
                crate::test_support::control_bounds(cx, "welcome-play"),
                Some(button)
            );
        }
    }
    assert!(view.read_with(cx, |view, _| {
        view.shell.first_run.stage == Stage::Welcome && view.shell.first_run.error.is_none()
    }));
}

#[gpui::test]
fn welcome_pointer_activation_stays_inside_the_visible_circle(cx: &mut gpui::TestAppContext) {
    let _system = crate::platform::test_system_motion(false);
    let result = (|| -> anyhow::Result<()> {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.begin_first_run(cx);
            view
        });
        cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(540.)));
        cx.run_until_parked();
        let button = bounds(cx, "welcome-play")?;
        // Four pixels inside the square frame's corner is far outside the circle
        // inscribed in that frame, which is the control's visible shape.
        let corner = gpui::point(
            button.origin.x + gpui::px(4.),
            button.origin.y + gpui::px(4.),
        );
        let started = |cx: &mut gpui::VisualTestContext, view: &gpui::Entity<super::Block>| {
            view.read_with(cx, |view, _| view.shell.first_run.error.is_some())
        };

        // The engine-free fixture reports the setup-save failure only when Start
        // actually runs, so this observes activation without persisting a setup.
        cx.simulate_click(corner, gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(
            !started(cx, &view),
            "a press inside the square frame but outside the circle must not start setup"
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.first_run.stage),
            Stage::Welcome
        );

        cx.simulate_click(button.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(
            started(cx, &view),
            "the visible circle must still start setup when pressed"
        );

        // A press is accepted only when it both starts and ends on the circle.
        view.update(cx, |view, _| view.shell.first_run.error = None);
        cx.simulate_mouse_down(corner, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_up(
            button.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();
        assert!(
            !started(cx, &view),
            "a press that starts outside the circle must not start setup"
        );
        cx.simulate_mouse_down(
            button.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.simulate_mouse_up(corner, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(
            !started(cx, &view),
            "a press that ends outside the circle must not start setup"
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.first_run.stage),
            Stage::Welcome
        );

        // Keyboard activation does not depend on where the pointer rests.
        view.update(cx, |view, _| view.shell.first_run.error = None);
        cx.simulate_mouse_move(corner, None, gpui::Modifiers::none());
        cx.run_until_parked();
        crate::test_support::press(cx, "enter");
        cx.run_until_parked();
        assert!(
            started(cx, &view),
            "Enter must start setup while the pointer is outside the circle"
        );
        view.update(cx, |view, _| view.shell.first_run.error = None);
        crate::test_support::press(cx, "space");
        cx.run_until_parked();
        assert!(started(cx, &view), "Space must start setup");
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn cancelled_game_wizard_releases_its_click_barrier_before_continue(cx: &mut gpui::TestAppContext) {
    let _system = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.begin_first_run(cx);
        view.shell.first_run.stage = Stage::Overview;
        view
    });
    cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
    cx.run_until_parked();
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::FirstRun(FirstRunAction::Game), window, cx);
        view.handle(UiAction::FirstRun(FirstRunAction::Custom), window, cx);
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(400));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| !view.motion.moving));

    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::FirstRun(FirstRunAction::Back), window, cx);
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(400));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| {
        view.shell.first_run.stage == Stage::Overview
            && view.motion.page.outgoing().is_none()
            && !view.motion.moving
    }));
    let button = crate::test_support::control_bounds(cx, "setup-continue");
    assert!(
        button.is_some(),
        "Continue must be available after cancellation"
    );
    if let Some(button) = button {
        cx.simulate_click(button.center(), gpui::Modifiers::none());
    }
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.shell.auth.is_some()));
}

#[gpui::test]
fn recommended_game_uses_a_release_when_snapshots_are_enabled_and_listed_first(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    let snapshot = version_entry(
        "24w14a",
        block_launcher::manifest::model::VersionType::Snapshot,
    );
    let release = version_entry(
        "1.20.6",
        block_launcher::manifest::model::VersionType::Release,
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.begin_first_run(cx);
            view.saved_preferences.game.show_snapshots = true;
            view.versions = vec![snapshot.id.clone(), release.id.clone()];
            view.version_catalog = vec![snapshot, release];
            view.handle_first_run(FirstRunAction::Game, window, cx);
            view.handle_first_run(FirstRunAction::Recommended, window, cx);
        });
    });
    assert!(view.read_with(cx, |view, cx| {
        view.shell.first_run.stage == Stage::GameWizard
            && matches!(&view.form, Some(super::Form::Instance(form))
                if form.version.read(cx).value() == "1.20.6")
    }));
}

#[gpui::test]
fn continue_routes_to_missing_account_then_game_and_finishes_when_both_are_saved(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });

        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.begin_first_run(cx);
                view.handle_first_run(FirstRunAction::Continue, window, cx);
            });
        });
        assert!(view.read_with(cx, |view, _| {
            view.shell.auth.is_some() && !view.first_run_account_ready()
        }));

        let account = crate::test_support::test_account()?;
        let account_id = crate::account_id(&account);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.shell.auth = None;
                view.router.navigate(Tab::Onboarding);
                view.accounts = vec![account];
                view.selected_account = Some(account_id.clone());
                view.shell.onboarding.account = Some(account_id);
                view.shell.onboarding.account_decided = true;
                view.handle_first_run(FirstRunAction::Continue, window, cx);
            });
        });
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == Stage::GameChoice && view.first_run_account_ready()
        }));

        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.snapshot = crate::test_support::ready_snapshot();
                view.handle_first_run(FirstRunAction::Continue, window, cx);
            });
        });
        assert!(view.read_with(cx, |view, _| {
            view.preferences.onboarding_complete
                && view.router.tab() == Tab::Home
                && view.first_run_account_ready()
                && view.first_run_game_ready()
        }));
        assert!(crate::Preferences::load(root.path())?.onboarding_complete);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn skip_persists_completion_without_fabricating_account_or_game(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.begin_first_run(cx);
                view.handle_first_run(FirstRunAction::Skip, window, cx);
            });
        });
        assert!(view.read_with(cx, |view, _| {
            view.preferences.onboarding_complete
                && view.router.tab() == Tab::Home
                && view.accounts.is_empty()
                && view.selected_account.is_none()
                && view.shell.onboarding.account.is_none()
                && view.snapshot.profiles.is_empty()
        }));
        assert!(crate::Preferences::load(root.path())?.onboarding_complete);
        assert!(
            block_launcher::catalog::Catalog::open(root.path())?
                .snapshot()?
                .profiles
                .is_empty()
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn restart_reconstructs_account_and_game_completion_from_saved_data(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let account = crate::test_support::test_account()?;
        let account_id = crate::account_id(&account);
        crate::global_config::update(root.path(), |config| {
            config.accounts = vec![account.clone()];
        })?;
        let catalog = block_launcher::catalog::Catalog::open(root.path())?;
        catalog.create_instance("Saved game".into(), "1.20.6".into(), None, None)?;
        let persisted_snapshot = catalog.snapshot()?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.accounts = vec![account];
            view.snapshot = persisted_snapshot;
            view.selected_account = Some(account_id);
            view
        });
        fs::write(root.path().join("setup-started"), b"1")?;
        view.update(cx, |view, cx| view.begin_first_run(cx));
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == Stage::Overview
                && view.first_run_account_ready()
                && view.first_run_game_ready()
        }));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn game_discovery_keeps_single_and_multi_selection_and_disables_unsupported_rows(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    let request = uuid::Uuid::new_v4();
    view.update(cx, |view, _| {
        view.shell.import_games.discovery = Some(request);
        view.receive_game_discovery(request, discovery_report());
    });
    assert!(view.read_with(cx, |view, _| {
        let state = &view.shell.import_games;
        state.report.as_ref().is_some_and(|report| {
            report.sources.len() == 2
                && report.sources[0].candidates_found == 1
                && report.sources[1].candidates_found == 2
                && report.candidates.len() == 4
        }) && state.selected
            == BTreeSet::from([
                "adjacent-one".to_owned(),
                "launcher-one".to_owned(),
                "launcher-two".to_owned(),
            ])
    }));
    assert!(view.read_with(cx, |view, _| view.import_action_disabled(
        &UiAction::ImportGames(ImportAction::Toggle("launcher-unsupported".into()))
    )));

    let refresh = uuid::Uuid::new_v4();
    view.update(cx, |view, _| {
        view.shell.import_games.selected.remove("launcher-one");
        view.shell.import_games.discovery = Some(refresh);
        view.receive_game_discovery(refresh, discovery_report());
    });
    assert!(view.read_with(cx, |view, _| {
        view.shell.import_games.selected
            == BTreeSet::from(["adjacent-one".to_owned(), "launcher-two".to_owned()])
    }));

    let late = uuid::Uuid::new_v4();
    view.update(cx, |view, _| {
        view.shell.import_games.discovery = Some(refresh);
        view.receive_game_discovery(late, DiscoveryReport::default());
    });
    assert!(view.read_with(cx, |view, _| {
        view.shell.import_games.discovery == Some(refresh)
            && view
                .shell
                .import_games
                .report
                .as_ref()
                .is_some_and(|report| report.candidates.len() == 4)
    }));
}

#[gpui::test]
fn late_import_results_cannot_replace_a_new_batch(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    let old = uuid::Uuid::new_v4();
    let current = uuid::Uuid::new_v4();
    view.update(cx, |view, _| {
        view.shell.import_games.batch = Some(current);
        view.shell.import_games.outcomes.insert(
            "launcher-one".into(),
            super::import_games::Outcome::Failed("keep this result".into()),
        );
        view.receive_game_import(
            old,
            "launcher-one".into(),
            Ok(("late".into(), "late".into())),
        );
        view.finish_game_import(old);
    });
    assert!(view.read_with(cx, |view, _| {
        view.shell.import_games.batch == Some(current)
            && !view.shell.import_games.running
            && matches!(view.shell.import_games.outcomes.get("launcher-one"), Some(super::import_games::Outcome::Failed(error)) if error == "keep this result")
    }));
}
