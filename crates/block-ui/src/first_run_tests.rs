use std::{collections::BTreeSet, fs};

use super::{
    Tab, UiAction,
    engine::Engine,
    first_run::{Action as FirstRunAction, Stage, StorageChoice},
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

/// Let the page-transition motion finish so its mouse barrier releases before a
/// test clicks a first-run control.
fn settle_transition(cx: &mut gpui::VisualTestContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(400));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// Open Stage::Storage with a deterministic Local path, exactly like the
/// Overview's More options action does.
fn open_storage_page<'a>(
    cx: &'a mut gpui::TestAppContext,
    root: &std::path::Path,
    local: std::path::PathBuf,
) -> anyhow::Result<(gpui::Entity<super::Block>, &'a mut gpui::VisualTestContext)> {
    fs::write(root.join("setup-started"), b"1")?;
    let (engine, messages) = Engine::with_root(root.to_owned())?;
    let (view, cx) = crate::test_support::window(cx, move |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.engine = Some(engine);
        view.messages = Some(messages);
        view
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.begin_first_run(cx);
            view.shell.first_run.game_parent = Some(local);
            view.handle_first_run(FirstRunAction::MoreOptions, window, cx);
        });
    });
    cx.run_until_parked();
    settle_transition(cx);
    Ok((view, cx))
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
    kind: minecraft_manifest::model::VersionType,
) -> minecraft_manifest::model::VersionEntry {
    minecraft_manifest::model::VersionEntry {
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
        let home = cx.update(|window, cx| crate::window_geometry::home_size(window, cx));
        cx.simulate_resize(home);
        cx.run_until_parked();
        cx.update(|window, cx| {
            view.update(cx, |view, cx| view.begin_first_run(cx));
            assert_eq!(window.viewport_size(), home);
        });
        assert!(view.read_with(cx, |view, _| {
            view.tab == Tab::Onboarding
                && view.shell.first_run.enabled
                && view.shell.first_run.stage == Stage::Welcome
        }));

        fs::write(root.path().join("setup-started"), b"1")?;
        view.update(cx, |view, cx| view.begin_first_run(cx));
        assert!(view.read_with(cx, |view, _| {
            view.tab == Tab::Onboarding && view.shell.first_run.stage == Stage::Overview
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
                assert_eq!(view.tab, Tab::Accounts);
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
            view.tab == Tab::Accounts && view.shell.auth.is_some() && view.shell.first_run.enabled
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
        let management =
            cx.update(|window, cx| crate::window_geometry::management_size(window, cx));
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
            view.tab == Tab::Onboarding
                && view.shell.first_run.enabled
                && view.shell.first_run.stage == Stage::Welcome
                && view.shell.first_run.entered.is_none()
                && view.is_home_frame()
        }));
        // `request_frame_resize` asks the platform for the Home-sized outer
        // width; deliver that resize like the real platform callback would.
        let home = cx.update(|window, cx| crate::window_geometry::home_size(window, cx));
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
fn overview_footer_keeps_a_single_more_options_and_continue_row(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.tab = Tab::Onboarding;
        view.shell.first_run.enabled = true;
        view.shell.first_run.stage = Stage::Overview;
        view
    });
    cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
    cx.run_until_parked();
    let more = crate::test_support::control_bounds(cx, "setup-more-options");
    let finish = crate::test_support::control_bounds(cx, "setup-continue");
    assert!(more.is_some(), "More options must stay in the footer");
    assert!(finish.is_some(), "Continue must stay in the footer");
    assert!(
        crate::test_support::control_bounds(cx, "setup-overview-skip").is_none(),
        "the Overview footer must not offer Skip setup"
    );
    if let (Some(more), Some(finish)) = (more, finish) {
        assert_eq!(
            more.center().y,
            finish.center().y,
            "More options and Continue must share one footer row"
        );
        assert!(
            more.right() <= finish.left(),
            "More options stays left of Continue"
        );
    }
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
    let snapshot = version_entry("24w14a", minecraft_manifest::model::VersionType::Snapshot);
    let release = version_entry("1.20.6", minecraft_manifest::model::VersionType::Release);
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
                view.tab = Tab::Onboarding;
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
                && view.tab == Tab::Home
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
                && view.tab == Tab::Home
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

#[test]
fn storage_choice_prefers_global_only_when_the_saved_parent_matches() {
    let global = std::path::Path::new("/scenario/Application Support/one.ztd.block/.minecraft");
    assert_eq!(
        StorageChoice::infer(global, Some(global)),
        StorageChoice::Global
    );
    assert_eq!(StorageChoice::infer(global, None), StorageChoice::Local);
    assert_eq!(
        StorageChoice::infer(
            std::path::Path::new("/scenario/Block/.minecraft"),
            Some(global)
        ),
        StorageChoice::Local
    );
}

#[gpui::test]
fn storage_cards_are_read_only_mutually_exclusive_and_only_local_browses(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let local = root.path().join("local-games");
        fs::create_dir_all(&local)?;
        let (view, cx) = open_storage_page(cx, root.path(), local.clone())?;
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
        cx.run_until_parked();
        settle_transition(cx);
        let global = root.path().join("app-data/one.ztd.block/.minecraft");
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.shell.first_run.storage_global = Some(global.clone());
                assert_eq!(view.shell.first_run.storage_choice, StorageChoice::Local);
                assert_eq!(
                    view.shell.first_run.storage_local.as_deref(),
                    Some(local.as_path())
                );
                assert_eq!(view.shell.first_run.stage, Stage::Storage);
                cx.notify();
            });
        });
        cx.run_until_parked();

        // Both cards render a read-only path box and the Save action sits below
        // them; only the Local card has a folder picker.
        let global_card = bounds(cx, "setup-storage-global-card")?;
        let local_card = bounds(cx, "setup-storage-local-card")?;
        let global_path = bounds(cx, "setup-storage-global-path")?;
        let local_path = bounds(cx, "setup-storage-local-path")?;
        let choose = bounds(cx, "setup-storage-choose-folder")?;
        let save = bounds(cx, "setup-save-storage")?;
        assert!(global_card.contains(&global_path.center()));
        assert!(local_card.contains(&local_path.center()));
        assert!(local_card.contains(&choose.center()));
        assert!(
            !global_card.contains(&choose.center()),
            "Global storage is read-only and must not browse"
        );
        assert!(
            global_card.bottom() <= local_card.top(),
            "the storage cards are stacked"
        );
        assert!(
            save.top() >= local_card.bottom(),
            "the primary Save action sits at the bottom of the page"
        );

        // Selecting a card is exclusive: each click leaves exactly one choice.
        cx.simulate_click(global_card.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.storage_choice == StorageChoice::Global
                && view.shell.first_run.storage_local.as_deref() == Some(local.as_path())
        }));

        cx.simulate_click(local_path.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.storage_choice == StorageChoice::Local
        }));

        cx.simulate_click(global_path.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.storage_choice == StorageChoice::Global
        }));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn storage_save_persists_the_selected_card_path(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let local = root.path().join("local-games");
        fs::create_dir_all(&local)?;
        let (view, cx) = open_storage_page(cx, root.path(), local.clone())?;
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
        cx.run_until_parked();
        settle_transition(cx);

        let save = bounds(cx, "setup-save-storage")?;
        cx.simulate_click(save.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == Stage::Overview
                && view.shell.first_run.game_parent.as_deref() == Some(local.as_path())
        }));
        assert_eq!(
            block_launcher::storage::Locations::load(root.path())?
                .library
                .as_deref(),
            Some(fs::canonicalize(&local)?.as_path())
        );

        // The Global card persists the fixed app-data library instead.
        let global = root.path().join("app-data/one.ztd.block/.minecraft");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle_first_run(FirstRunAction::MoreOptions, window, cx);
                view.shell.first_run.storage_global = Some(global);
                view.handle_first_run(
                    FirstRunAction::SelectStorage(StorageChoice::Global),
                    window,
                    cx,
                );
            });
        });
        cx.run_until_parked();
        settle_transition(cx);
        let save = bounds(cx, "setup-save-storage")?;
        cx.simulate_click(save.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let expected_global =
            fs::canonicalize(root.path())?.join("app-data/one.ztd.block/.minecraft");
        assert_eq!(
            block_launcher::storage::Locations::load(root.path())?
                .library
                .as_deref(),
            Some(expected_global.as_path())
        );
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == Stage::Overview
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
