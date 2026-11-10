use anyhow::Context as _;
use std::{collections::BTreeSet, fs};

use super::{
    Tab, UiAction,
    engine::Engine,
    first_run::{Action as FirstRunAction, Scan, Stage},
    import_games::Action as ImportAction,
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

fn label(cx: &mut gpui::VisualTestContext, selector: &'static str) -> Option<String> {
    cx.update(|window, _| {
        gpui_base::test_support::find(window, &[], &selector.into())
            .and_then(|node| node.label().map(str::to_owned))
    })
}

fn candidate(id: &str, source: SourceKind, importable: bool) -> GameCandidate {
    GameCandidate {
        id: id.into(),
        source,
        source_root: std::path::PathBuf::from(format!("/synthetic/{source:?}")),
        name: format!("Game {id}"),
        resource_directory: std::path::PathBuf::from(format!("/synthetic/{id}")),
        version_id: Some("1.20.6".into()),
        data: block_launcher::game_import::GameDataEvidence::Unknown,
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
fn library_picker_cancel_and_stale_revision_do_not_change_the_default(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let data = root.path().join("launcher");
        let old = root.path().join("old-library");
        let (engine, _messages) = Engine::with_root(data.clone())?;
        let old = crate::setup_storage::save_game_parent(&data, &old)?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.begin_first_run(cx);
            view
        });
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::FirstRun(FirstRunAction::Custom), window, cx);
            view.shell.wizard.step = crate::game_wizard::WizardStep::Setup;
            let scope = view.setup_game_library_scope().context("new game scope")?;
            for stale in [false, true] {
                let id = uuid::Uuid::new_v4();
                view.shell.first_run.library_picker = Some(crate::library_location::Picker {
                    id,
                    scope: scope.clone(),
                    wizard_revision: view.shell.wizard.revision,
                });
                let selection = if stale {
                    view.shell.wizard.revision += 1;
                    Ok(Some(root.path().join("ignored-library")))
                } else {
                    Ok(None)
                };
                view.finish_setup_library_browse(id, selection, cx);
                assert!(view.shell.first_run.library_draft.is_none());
            }
            Ok::<(), anyhow::Error>(())
        })?;
        assert_eq!(
            block_launcher::storage::Locations::load(&data)?.library(&data),
            old
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn library_choice_stays_pending_until_valid_confirmation_and_keeps_generated_identity(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let data = root.path().join("launcher");
        let old = root.path().join("old-library");
        let parent = root.path().join("chosen-library");
        let final_parent = root.path().join("confirmed-library");
        fs::create_dir_all(old.join("worlds"))?;
        fs::write(old.join("worlds/keep.txt"), "existing world")?;
        let (engine, _messages) = Engine::with_root(data.clone())?;
        let old = crate::setup_storage::save_game_parent(&data, &old)?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.begin_first_run(cx);
            view
        });
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::FirstRun(FirstRunAction::Custom), window, cx);
            view.shell.wizard.step = crate::game_wizard::WizardStep::Setup;
            let scope = view.setup_game_library_scope().context("new game scope")?;
            let directory = match &view.form {
                Some(crate::Form::Instance(form)) => form.directory.read(cx).value().to_string(),
                _ => anyhow::bail!("creation form"),
            };
            let leaf = std::path::Path::new(&directory)
                .file_name()
                .context("generated game directory")?
                .to_owned();
            let id = uuid::Uuid::new_v4();
            view.shell.first_run.library_picker = Some(crate::library_location::Picker {
                id,
                scope: scope.clone(),
                wizard_revision: view.shell.wizard.revision,
            });
            view.finish_setup_library_browse(id, Ok(Some(parent.clone())), cx);
            assert_eq!(view.first_run_game_parent(), old);
            assert_eq!(view.setup_library_parent(&scope), parent);
            if let Some(crate::Form::Instance(form)) = &view.form {
                assert_eq!(
                    std::path::PathBuf::from(form.directory.read(cx).value()),
                    parent.join(&leaf)
                );
                form.name.update(cx, |field, cx| field.set_value("", cx));
            }
            let custom_directory = parent.join(uuid::Uuid::new_v4().to_string());
            if let Some(crate::Form::Instance(form)) = &view.form {
                form.directory.update(cx, |field, cx| {
                    field.set_value(&custom_directory.display().to_string(), cx);
                });
            }
            let id = uuid::Uuid::new_v4();
            view.shell.first_run.library_picker = Some(crate::library_location::Picker {
                id,
                scope: scope.clone(),
                wizard_revision: view.shell.wizard.revision,
            });
            view.finish_setup_library_browse(id, Ok(Some(final_parent.clone())), cx);
            if let Some(crate::Form::Instance(form)) = &view.form {
                assert_eq!(
                    std::path::PathBuf::from(form.directory.read(cx).value()),
                    custom_directory,
                    "a chosen instance directory must not be inferred from its UUID-shaped name"
                );
            }
            view.submit_game_wizard(cx);
            assert!(!view.busy, "invalid creation must not start");
            assert_eq!(
                block_launcher::storage::Locations::load(&data)?.library(&data),
                old,
                "invalid creation must not accept the pending default"
            );
            if let Some(crate::Form::Instance(form)) = &view.form {
                form.name
                    .update(cx, |field, cx| field.set_value("New game", cx));
            }
            assert!(view.commit_setup_library(&scope, cx));
            assert!(view.shell.first_run.library_draft.is_none());
            assert_eq!(
                view.first_run_game_parent(),
                crate::setup_storage::comparable_path(&final_parent)
            );
            Ok::<(), anyhow::Error>(())
        })?;
        assert_eq!(
            block_launcher::storage::Locations::load(&data)?.library(&data),
            crate::setup_storage::comparable_path(&final_parent)
        );
        assert_eq!(
            fs::read_to_string(old.join("worlds/keep.txt"))?,
            "existing world"
        );
        assert!(!parent.join("worlds/keep.txt").exists());
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn import_policy_choice_survives_switching_sources(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let source = root.path().canonicalize()?;
        let mut report = discovery_report();
        for candidate in &mut report.candidates {
            candidate.resource_directory = root.path().to_path_buf();
        }
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        view.update_in(cx, |view, window, cx| {
            view.shell.import_games.apply_discovery(report);
            view.shell.import_games.page = crate::import_games::Page::List;
            view.handle_import_games(
                ImportAction::OpenDataPolicy("adjacent-one".into()),
                window,
                cx,
            );
            view.handle_import_games(
                ImportAction::SelectDataPolicy(
                    "adjacent-one".into(),
                    block_launcher::catalog::DataPolicy::Shared,
                ),
                window,
                cx,
            );
            view.handle_import_games(ImportAction::Back, window, cx);
            view.handle_import_games(ImportAction::Sources, window, cx);
            view.handle_import_games(
                ImportAction::Source(SourceKind::OfficialLauncher),
                window,
                cx,
            );
            view.handle_import_games(
                ImportAction::Source(SourceKind::AdjacentMinecraft),
                window,
                cx,
            );
            let chosen = &view.shell.import_games.selections["adjacent-one"];
            assert_eq!(
                chosen.data_policy,
                block_launcher::catalog::DataPolicy::Shared
            );
            assert_eq!(chosen.game_directory, source);
            assert!(view.shell.import_games.selected.contains("adjacent-one"));
        });
        assert!(!root.path().join("versions").exists());
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn library_chooser_failure_preserves_unrelated_version_validation(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.begin_first_run(cx);
            view
        });
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::FirstRun(FirstRunAction::Custom), window, cx);
            view.shell.wizard.step = crate::game_wizard::WizardStep::Setup;
            if let Some(crate::Form::Instance(form)) = &view.form {
                form.version.update(cx, |field, cx| field.set_value("", cx));
            }
            view.set_wizard_error(crate::i18n::UiText::message("runtime.version_required"), cx);
            let scope = view.setup_game_library_scope().context("game scope")?;
            let id = uuid::Uuid::new_v4();
            view.shell.first_run.library_picker = Some(crate::library_location::Picker {
                id,
                scope,
                wizard_revision: view.shell.wizard.revision,
            });
            view.finish_setup_library_browse(id, Err(()), cx);
            assert_eq!(
                view.shell
                    .wizard
                    .error
                    .as_ref()
                    .and_then(crate::i18n::UiText::key),
                Some("runtime.version_required"),
                "a library chooser failure must not replace a version-field error"
            );
            Ok::<(), anyhow::Error>(())
        })?;
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn direct_registration_keeps_the_new_game_library_default(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let data = root.path().join("block");
        let source = root.path().join("minecraft");
        let version = source.join("versions/1.20.6");
        fs::create_dir_all(&version)?;
        fs::write(
            version.join("1.20.6.json"),
            br#"{"id":"1.20.6","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
        )?;
        let old = crate::setup_storage::save_game_parent(&data, &root.path().join("new-games"))?;
        let (engine, messages) = Engine::with_root(data.clone())?;
        let mut detected = candidate("local", SourceKind::AdjacentMinecraft, true);
        detected.resource_directory = source.canonicalize()?;
        detected.version_id = Some("1.20.6".into());
        detected.configured_version = Some("1.20.6".into());
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.shell.first_run.enabled = true;
            view.router.navigate(Tab::Onboarding);
            view.shell.import_games.apply_discovery(DiscoveryReport {
                sources: Vec::new(),
                candidates: vec![detected],
            });
            view
        });
        view.update_in(cx, |view, window, cx| {
            view.handle_import_games(ImportAction::Import, window, cx)
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while view.read_with(cx, |view, _| view.shell.import_games.running)
            && std::time::Instant::now() < deadline
        {
            view.update(cx, crate::Block::poll);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(view.read_with(cx, |view, _| matches!(
            view.shell.import_games.outcomes.get("local"),
            Some(crate::import_games::Outcome::Saved)
        )));
        let saved = block_launcher::catalog::Catalog::open(&data)?.snapshot()?;
        assert_eq!(saved.instances.len(), 1);
        assert_eq!(
            saved.instances[0].resource_directory,
            source.canonicalize()?
        );
        assert_eq!(
            block_launcher::storage::Locations::load(&data)?.library(&data),
            old
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn begin_first_run_opens_overview_at_home_size_and_preserves_a_legacy_marker(
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
                && view.shell.first_run.stage == Stage::Overview
                && !view.wizard_task_active()
                && !view.preferences.onboarding_complete
                && view.shell.import_games.discovery.is_some()
        }));
        assert!(!root.path().join("setup-started").exists());
        fs::write(root.path().join("setup-started"), b"legacy marker")?;
        view.update(cx, |view, cx| view.begin_first_run(cx));
        assert!(view.read_with(cx, |view, _| {
            view.router.tab() == Tab::Onboarding && view.shell.first_run.stage == Stage::Overview
        }));
        assert_eq!(
            fs::read(root.path().join("setup-started"))?,
            b"legacy marker"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn initialization_opens_the_setup_center_and_a_report_only_reports_a_count(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.preferences.reduce_motion = true;
            view
        });
        let request = cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.begin_first_run(cx);
                assert_eq!(
                    view.shell.first_run.stage,
                    Stage::Overview,
                    "initialization opens the setup center without an import detour"
                );
                assert!(
                    !root.path().join("setup-started").exists(),
                    "initialization must not create a legacy setup marker"
                );
                assert!(
                    !view.shell.surfaces.is_open(),
                    "a background scan must never open the drawer on its own"
                );
                assert_eq!(
                    view.first_run_scan(crate::Language::English),
                    Some(Scan::Searching)
                );
                // Take over the pending request so the answer is the test's own.
                let request = uuid::Uuid::new_v4();
                view.shell.import_games.discovery = Some(request);
                request
            })
        });
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            view.receive_game_discovery(request, discovery_report());
            cx.notify();
        });
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |view, _| {
                view.shell.first_run.stage == Stage::Overview
                    && !view.shell.surfaces.is_open()
                    && view.first_run_scan(crate::Language::English) == Some(Scan::Found(3))
            }),
            "three of the four discovered games are importable"
        );
        assert!(
            crate::test_support::control_bounds(cx, "setup-import-badge").is_some(),
            "the game task must show the discovered importable count"
        );
        let import_badge = bounds(cx, "setup-import-badge")?;
        let status_badge = bounds(cx, "setup-game-status")?;
        assert_eq!(status_badge.size.height, import_badge.size.height);
        assert_eq!(status_badge.center().y, import_badge.center().y);
        assert!(import_badge.right() < status_badge.left());
        assert!(crate::test_support::control_bounds(cx, "setup-import-game").is_none());
        assert!(crate::test_support::control_bounds(cx, "setup-rescan").is_none());
        let game = bounds(cx, "setup-game")?;
        cx.simulate_click(game.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |view, _| view.shell.first_run.stage == Stage::Overview
                && !view.wizard_task_active())
        );
        assert!(
            view.read_with(cx, |view, _| matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(crate::shell::Surface::Drawer(
                    crate::shell::Drawer::ImportGames
                ))
            )),
            "clicking Game must open its import prompt"
        );
        crate::test_support::press(cx, "escape");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()
            && view.shell.first_run.stage == Stage::Overview));
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::FirstRun(FirstRunAction::Game), window, cx);
            view.handle_import_games(ImportAction::New, window, cx);
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()
            && view.shell.first_run.stage == Stage::GameChoice));
        assert!(crate::test_support::control_bounds(cx, "setup-import-game").is_some());
        assert!(
            bounds(cx, "setup-recommended")?.origin.y < bounds(cx, "setup-custom")?.origin.y
                && bounds(cx, "setup-custom")?.origin.y < bounds(cx, "setup-import-pack")?.origin.y
                && bounds(cx, "setup-import-pack")?.origin.y
                    < bounds(cx, "setup-import-game")?.origin.y,
            "three creation cards stay ordered and existing imports remain secondary"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn late_discovery_keeps_choice_order_and_saved_imports_stop_reminding(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let request = uuid::Uuid::new_v4();
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.begin_first_run(cx);
            view.shell.import_games = crate::import_games::State::default();
            view.shell.import_games.discovery = Some(request);
            view
        });
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::FirstRun(FirstRunAction::Game), window, cx);
            assert_eq!(view.shell.import_games.discovery, Some(request));
            assert!(!view.shell.surfaces.is_open());
            assert_eq!(view.shell.first_run.stage, Stage::GameChoice);
        });
        cx.run_until_parked();
        assert!(bounds(cx, "setup-recommended")?.origin.y < bounds(cx, "setup-custom")?.origin.y);
        assert!(bounds(cx, "setup-custom")?.origin.y < bounds(cx, "setup-import-game")?.origin.y);
        view.update(cx, |view, cx| {
            view.receive_game_discovery(request, discovery_report());
            assert!(
                !view.shell.surfaces.is_open(),
                "a late scan cannot reopen a dismissed prompt"
            );
            cx.notify();
        });
        cx.run_until_parked();
        assert!(
            bounds(cx, "setup-custom")?.origin.y < bounds(cx, "setup-import-game")?.origin.y,
            "a late report must not move Import ahead of the current click targets"
        );
        view.update(cx, |view, cx| {
            let batch = uuid::Uuid::new_v4();
            view.shell.import_games.batch = Some(batch);
            view.receive_game_import(batch, "adjacent-one".into(), Ok("instance-adjacent".into()));
            assert_eq!(
                view.first_run_scan(crate::Language::English),
                Some(Scan::Found(2))
            );
            for id in ["launcher-one", "launcher-two"] {
                view.receive_game_import(batch, id.into(), Ok(format!("instance-{id}")));
            }
            assert_eq!(
                view.first_run_scan(crate::Language::English),
                Some(Scan::Empty)
            );
            cx.notify();
        });
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::FirstRun(FirstRunAction::Back), window, cx);
        });
        cx.run_until_parked();
        assert!(crate::test_support::control_bounds(cx, "setup-import-badge").is_none());
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::FirstRun(FirstRunAction::Game), window, cx);
            assert!(!view.shell.surfaces.is_open());
            assert_eq!(view.shell.first_run.stage, Stage::GameChoice);
        });
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn import_library_is_readonly_on_sources_and_candidates(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.begin_first_run(cx);
        view.shell.import_games = crate::import_games::State::default();
        view.shell.import_games.report = Some(discovery_report());
        view
    });
    cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(720.)));
    for (action, control) in [
        (UiAction::FirstRun(FirstRunAction::Game), "import-new"),
        (
            UiAction::ImportGames(ImportAction::View(SourceKind::OfficialLauncher)),
            "import-select-all",
        ),
    ] {
        view.update_in(cx, |view, window, cx| view.handle(action, window, cx));
        cx.run_until_parked();
        assert!(crate::test_support::control_bounds(cx, control).is_some());
        assert!(
            crate::test_support::control_bounds(cx, "setup-library-change").is_none(),
            "the import destination is displayed without a library picker"
        );
    }
}

#[gpui::test]
fn rescan_keeps_escape_in_the_drawer_until_its_late_report(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.engine = Some(engine);
            view.begin_first_run(cx);
            view.shell.import_games = crate::import_games::State::default();
            view.shell.import_games.report = Some(discovery_report());
            view
        });
        cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(720.)));
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::FirstRun(FirstRunAction::Game), window, cx);
        });
        cx.run_until_parked();
        let retry = bounds(cx, "import-rescan")?;
        cx.simulate_click(retry.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let request = view
            .read_with(cx, |view, _| view.shell.import_games.discovery)
            .context("the retry is still waiting")?;
        crate::test_support::press(cx, "escape");
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |view, _| !view.shell.surfaces.is_open()
                && view.shell.first_run.stage == Stage::Overview),
            "Escape must still close the prompt after its focused retry becomes busy"
        );
        view.update(cx, |view, cx| {
            view.receive_game_discovery(request, discovery_report());
            assert!(!view.shell.surfaces.is_open());
            assert_eq!(
                view.first_run_scan(crate::Language::English),
                Some(Scan::Found(3))
            );
            cx.notify();
        });
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn nonpositive_discovery_states_do_not_show_a_green_import_badge(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.begin_first_run(cx);
        view
    });
    let cases = [
        (Some(uuid::Uuid::new_v4()), None, Some(discovery_report())),
        (
            None,
            Some(crate::i18n::UiText::message("import.error.dataDirectory")),
            Some(discovery_report()),
        ),
        (
            None,
            None,
            Some(DiscoveryReport {
                sources: Vec::new(),
                candidates: Vec::new(),
            }),
        ),
        (None, None, None),
        (
            None,
            None,
            Some(DiscoveryReport {
                sources: Vec::new(),
                candidates: vec![candidate(
                    "unsupported",
                    SourceKind::OfficialLauncher,
                    false,
                )],
            }),
        ),
    ];
    for (discovery, error, report) in cases {
        view.update(cx, |view, cx| {
            view.shell.import_games.discovery = discovery;
            view.shell.import_games.error = error;
            view.shell.import_games.report = report;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(crate::test_support::control_bounds(cx, "setup-game").is_some());
        assert!(
            crate::test_support::control_bounds(cx, "setup-import-badge").is_none(),
            "loading, failure, empty and unscanned states must not advertise a positive count"
        );
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::FirstRun(FirstRunAction::Game), window, cx);
            assert!(!view.shell.surfaces.is_open());
            assert_eq!(view.shell.first_run.stage, Stage::GameChoice);
            view.handle(UiAction::FirstRun(FirstRunAction::Back), window, cx);
        });
    }
}

#[gpui::test]
fn finished_setup_rejects_a_discovery_that_is_still_in_flight(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.preferences.reduce_motion = true;
            view
        });
        let request = cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.begin_first_run(cx);
                let request = view
                    .shell
                    .import_games
                    .discovery
                    .unwrap_or_else(uuid::Uuid::nil);
                // Skipping right after initialization is the shortest possible race.
                view.handle_first_run(FirstRunAction::Skip, window, cx);
                assert!(
                    view.shell.import_games.discovery.is_none(),
                    "finishing setup drops the scan it no longer waits for"
                );
                request
            })
        });
        view.update(cx, |view, _| {
            view.receive_game_discovery(request, discovery_report())
        });
        assert!(
            view.read_with(cx, |view, _| {
                view.preferences.onboarding_complete
                    && view.router.tab() == Tab::Home
                    && view.shell.import_games.report.is_none()
                    && view.first_run_scan(crate::Language::English).is_none()
            }),
            "a finished setup must not take a late discovery result"
        );
        assert!(crate::Preferences::load(root.path())?.onboarding_complete);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn rescan_retries_only_the_scan_and_keeps_the_answered_requests_apart(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.preferences.reduce_motion = true;
            view
        });
        let (first, second) = cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.begin_first_run(cx);
                let first = uuid::Uuid::new_v4();
                view.shell.import_games.discovery = Some(first);
                view.receive_game_discovery(first, discovery_report());
                assert_eq!(
                    view.first_run_scan(crate::Language::English),
                    Some(Scan::Found(3))
                );
                view.handle_import_games(crate::import_games::Action::Rescan, window, cx);
                assert_eq!(view.shell.first_run.stage, Stage::Overview);
                assert!(
                    !view.shell.surfaces.is_open(),
                    "a retry is a scan, not a drawer"
                );
                let second = view
                    .shell
                    .import_games
                    .discovery
                    .unwrap_or_else(uuid::Uuid::nil);
                assert_ne!(first, second, "the retry owns a newer request");
                assert_eq!(
                    view.first_run_scan(crate::Language::English),
                    Some(Scan::Searching)
                );
                (first, second)
            })
        });
        view.update(cx, |view, _| {
            view.receive_game_discovery(first, discovery_report());
        });
        assert!(
            view.read_with(cx, |view, _| {
                view.first_run_scan(crate::Language::English) == Some(Scan::Searching)
            }),
            "an answered request cannot overwrite the retry"
        );
        view.update(cx, |view, _| {
            view.receive_game_discovery(second, DiscoveryReport::default());
        });
        assert!(
            view.read_with(cx, |view, _| {
                view.first_run_scan(crate::Language::English) == Some(Scan::Empty)
            }),
            "an empty answer is its own state, not a loading or failed one"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn the_center_exposes_the_language_entry(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.preferences.language = Some(crate::Language::English);
            view.router.navigate(Tab::Onboarding);
            view.shell.first_run.enabled = true;
            view.shell.first_run.stage = Stage::Overview;
            view
        });
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
        cx.run_until_parked();
        let entry = bounds(cx, "setup-language-entry")?;
        let title = bounds(cx, "setup-overview-title")?;
        assert!(
            entry.left() >= title.right()
                && entry.top() >= gpui::px(56.)
                && (entry.center().y - title.center().y).abs() <= gpui::px(1.),
            "language must align beside the body title, below window chrome"
        );
        cx.simulate_click(entry.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == Stage::Overview
                && matches!(
                    view.shell.surfaces.top().map(|layer| &layer.kind),
                    Some(super::shell::Surface::Drawer(
                        super::shell::Drawer::Language
                    ))
                )
        }));
        assert_eq!(
            view.read_with(cx, |view, _| {
                view.surface_sheet_chrome(&super::shell::Surface::Drawer(
                    super::shell::Drawer::Language,
                ))
                .1
            }),
            "Choose the language for Block.",
            "the language drawer must show its translated description"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn empty_setup_can_leave_the_center_for_home(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.preferences.reduce_motion = true;
            view.begin_first_run(cx);
            view
        });
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == Stage::Overview
                && !view.first_run_account_ready()
                && !view.first_run_game_ready()
        }));
        let home = bounds(cx, "setup-continue")?;
        cx.simulate_click(home.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |view, _| {
                view.preferences.onboarding_complete
                    && view.router.tab() == Tab::Home
                    && view.accounts.is_empty()
                    && view.snapshot.instances.is_empty()
            }),
            "an empty setup leaves explicitly and fabricates nothing"
        );
        assert!(crate::Preferences::load(root.path())?.onboarding_complete);
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
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.begin_first_run(cx);
                assert_eq!(view.shell.first_run.stage, Stage::Overview);
                view.handle_first_run(FirstRunAction::Import, window, cx);
                assert!(
                    view.shell.import_games.discovery.is_some(),
                    "an explicit import starts the scan its sheet shows"
                );
                assert!(view.shell.surfaces.is_open());
                view.handle_import_games(crate::import_games::Action::New, window, cx);
                assert!(
                    !view.shell.surfaces.is_open(),
                    "Create a new game must leave the prompt closed"
                );
                assert_eq!(view.shell.first_run.stage, Stage::GameChoice);
            });
        });
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn compact_setup_keeps_accounts_first_with_equal_cards_and_both_exit_actions_visible(
    cx: &mut gpui::TestAppContext,
) {
    for language in crate::Language::ALL {
        let (_view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.preferences.language = Some(language);
            view.snapshot = crate::test_support::ready_snapshot();
            view.snapshot.instances[0].pack = Some(block_launcher::catalog::PackAttachment {
                target_id: "vanilla".into(),
                status: block_launcher::catalog::PackStatus::Pending,
            });
            view.selected_instance = Some("instance-a".into());
            view.selected_instance = Some("instance-a".into());
            view.router.navigate(Tab::Onboarding);
            view.shell.first_run.enabled = true;
            view.shell.first_run.stage = Stage::Overview;
            view
        });
        cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(540.)));
        cx.run_until_parked();
        let body = crate::test_support::control_bounds(cx, "setup-scroll");
        let footer = crate::test_support::control_bounds(cx, "setup-footer");
        let game = crate::test_support::control_bounds(cx, "setup-game");
        let account = crate::test_support::control_bounds(cx, "setup-account");
        assert!(body.zip(footer).is_some_and(|(body, footer)| {
            body.size.height >= gpui::px(120.) && body.bottom() <= footer.top()
        }));
        assert!(account.zip(game).is_some_and(|(account, game)| {
            game.top() - account.bottom() == gpui::px(12.) && account.size == game.size
        }));
        for selector in ["setup-later", "setup-continue"] {
            let action = crate::test_support::control_bounds(cx, selector);
            assert!(
                action.zip(footer).is_some_and(|(action, footer)| {
                    action.left() >= gpui::px(0.)
                        && action.right() <= gpui::px(360.)
                        && action.top() >= footer.top()
                        && action.bottom() <= gpui::px(540.)
                }),
                "{language:?}: {selector} must remain visible, action={action:?}, footer={footer:?}"
            );
        }
    }
}

#[gpui::test]
fn overview_single_action_footer_has_symmetric_vertical_space(cx: &mut gpui::TestAppContext) {
    let (_, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot.instances.clear();
        view.snapshot.instances.clear();
        view.selected_instance = None;
        view.selected_instance = None;
        view.router.navigate(Tab::Onboarding);
        view.shell.first_run.enabled = true;
        view.shell.first_run.stage = Stage::Overview;
        view
    });
    cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(720.)));
    cx.run_until_parked();
    let footer = crate::test_support::control_bounds(cx, "setup-footer");
    let action = crate::test_support::control_bounds(cx, "setup-continue");
    assert!(
        footer.zip(action).is_some_and(|(footer, action)| {
            let above = action.top() - footer.top();
            let below = footer.bottom() - action.bottom();
            (above - below).abs() <= gpui::px(1.)
        }),
        "the single primary action must be vertically centered: footer={footer:?}, action={action:?}"
    );
}

#[gpui::test]
fn overview_keyboard_order_reaches_account_before_game(cx: &mut gpui::TestAppContext) {
    let (_, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.router.navigate(Tab::Onboarding);
        view.shell.first_run.enabled = true;
        view.shell.first_run.stage = Stage::Overview;
        view
    });
    cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(720.)));
    cx.run_until_parked();
    let mut reached_account = false;
    for _ in 0..12 {
        let focused = cx.update(|window, _| {
            ["setup-account", "setup-game"].map(|id| {
                gpui_base::test_support::find(window, &[], &id.into())
                    .and_then(|node| node.focused())
                    == Some(true)
            })
        });
        assert!(
            !focused[1],
            "Game must not precede Account in keyboard order"
        );
        if focused[0] {
            reached_account = true;
            break;
        }
        crate::test_support::press(cx, "tab");
        cx.run_until_parked();
    }
    assert!(
        reached_account,
        "the Account task must be keyboard reachable"
    );
    crate::test_support::press(cx, "tab");
    cx.run_until_parked();
    assert!(cx.update(|window, _| {
        gpui_base::test_support::find(window, &[], &"setup-game".into())
            .and_then(|node| node.focused())
            == Some(true)
    }));
}

#[gpui::test]
fn pending_pack_offers_resume_or_home_without_opening_the_picker(cx: &mut gpui::TestAppContext) {
    for resume in [true, false] {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let (engine, _messages) = Engine::with_root(root.path().to_owned())?;
            let (view, cx) = crate::test_support::window(cx, move |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.snapshot = crate::test_support::ready_snapshot();
                view.snapshot.instances[0].pack = Some(block_launcher::catalog::PackAttachment {
                    target_id: "vanilla".into(),
                    status: block_launcher::catalog::PackStatus::Pending,
                });
                view.selected_instance = Some("instance-a".into());
                view.selected_instance = Some("instance-a".into());
                view.begin_first_run(cx);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(900.)));
            cx.run_until_parked();
            assert_eq!(
                label(cx, "setup-continue").as_deref(),
                Some("Continue import")
            );
            let action = bounds(
                cx,
                if resume {
                    "setup-continue"
                } else {
                    "setup-later"
                },
            )?;
            cx.simulate_click(action.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            assert!(
                !cx.did_prompt_for_paths(),
                "entry must not open the archive picker"
            );
            if resume {
                assert!(view.read_with(cx, |view, _| {
                    view.pack_wizard_active()
                        && view.shell.pack_import.instance.as_deref() == Some("instance-a")
                        && view.router.tab() == Tab::Onboarding
                }));
            } else {
                assert!(view.read_with(cx, |view, _| {
                    view.router.tab() == Tab::Home && view.preferences.onboarding_complete
                }));
            }
            Ok(())
        })();
        assert!(result.is_ok(), "resume={resume}: {result:?}");
    }
}

#[gpui::test]
fn finishing_first_run_confirms_a_changed_retained_game_draft(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, _messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.engine = Some(engine);
            view.begin_first_run(cx);
            view
        });
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(1000.)));
        for selector in ["setup-game", "setup-custom"] {
            let action = bounds(cx, selector)?;
            cx.simulate_click(action.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            assert!(!view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
        }
        view.update(cx, |view, cx| {
            if let Some(crate::Form::Instance(form)) = &view.form {
                form.name
                    .update(cx, |field, cx| field.set_value("Retained game", cx));
            }
        });
        let back = bounds(cx, "task-close")?;
        cx.simulate_click(back.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == Stage::Overview && view.form.is_some()
        }));
        let home = bounds(cx, "setup-continue")?;
        cx.simulate_click(home.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            !view.preferences.onboarding_complete
                && view.shell.surfaces.top().is_some_and(|layer| {
                    matches!(layer.kind, crate::shell::Surface::DiscardGameDraft { .. })
                })
        }));
        let stay = bounds(cx, "discard-game-stay")?;
        cx.simulate_click(stay.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, cx| {
            matches!(&view.form, Some(crate::Form::Instance(form))
                if form.name.read(cx).value() == "Retained game")
        }));
        let home = bounds(cx, "setup-continue")?;
        cx.simulate_click(home.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let discard = bounds(cx, "discard-game-confirm")?;
        cx.simulate_click(discard.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.preferences.onboarding_complete && view.form.is_none()
        }));
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
                    crate::shell::Drawer::Instances,
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
            let parent = view.router.entry_id();
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
            assert_eq!(view.router.entry_id(), parent);
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
fn game_task_returns_to_overview_with_compact_geometry_and_saved_progress(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let account = crate::test_support::test_account()?;
        let account_id = crate::account_id(&account);
        crate::global_config::update(root.path(), |config| {
            config.accounts = vec![account.clone()];
        })?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.accounts = vec![account];
            view.selected_account = Some(account_id);
            view.preferences.reduce_motion = true;
            view
        });
        let home = cx.update(|window, _| crate::window_geometry::home_size(window));
        cx.simulate_resize(home);
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
            crate::test_support::control_bounds(cx, "setup-back-welcome").is_none(),
            "the Overview chrome must not expose the retired Welcome route"
        );
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle_first_run(FirstRunAction::Game, window, cx);
                view.handle_first_run(FirstRunAction::Back, window, cx);
            });
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.router.tab() == Tab::Onboarding
                && view.shell.first_run.enabled
                && view.shell.first_run.stage == Stage::Overview
                && view.shell.first_run.entered.is_none()
                && view.page_spec().page == crate::navigation::Page::FirstRun
                && !view.page_spec().task
                && view.first_run_account_ready()
        }));
        assert_eq!(
            cx.update(|window, _| window.viewport_size()),
            home,
            "returning from a game task must keep the same compact frame"
        );
        assert_eq!(
            crate::global_config::read(root.path())?.map(|config| config.accounts.len()),
            Some(1)
        );
        assert!(!crate::Preferences::load(root.path())?.onboarding_complete);
        assert!(!root.path().join("setup-started").exists());
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn returning_to_overview_keeps_the_hidden_game_draft_and_compact_bounds(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    let dimensions = cx.update(|window, _| crate::window_geometry::home_size(window));
    cx.simulate_resize(dimensions);
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
            assert_eq!(view.shell.first_run.stage, Stage::Overview);
            assert_eq!(view.page_spec().page, crate::navigation::Page::FirstRun);
            assert!(!view.page_spec().task);
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
        assert_eq!(
            block_launcher::storage::Locations::load(root.path())?.library(root.path()),
            saved,
            "initialization preserves the persisted library"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn overview_footer_offers_one_exit_beside_the_later_choice(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Onboarding);
        view.shell.first_run.enabled = true;
        view.shell.first_run.stage = Stage::Overview;
        view
    });
    cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
    cx.run_until_parked();
    assert!(
        crate::test_support::control_bounds(cx, "setup-continue").is_some(),
        "the primary exit stays in the footer"
    );
    assert_eq!(label(cx, "setup-continue").as_deref(), Some("Go to Home"));
    assert!(crate::test_support::control_bounds(cx, "setup-later").is_none());
    assert!(
        crate::test_support::control_bounds(cx, "setup-more-options").is_none(),
        "storage is automatic, so the footer must not offer More options"
    );

    // Both steps saved: the exit finishes setup instead of asking again.
    let account = crate::test_support::test_account();
    assert!(account.is_ok(), "{account:?}");
    let Ok(account) = account else {
        return;
    };
    let account_id = crate::account_id(&account);
    view.update(cx, |view, cx| {
        view.snapshot = crate::test_support::ready_snapshot();
        view.selected_instance = Some("instance-a".into());
        view.selected_instance = Some("instance-a".into());
        view.accounts = vec![account];
        view.selected_account = Some(account_id.clone());
        view.shell.onboarding.account = Some(account_id);
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(label(cx, "setup-continue").as_deref(), Some("Go to Home"));
    assert!(
        crate::test_support::control_bounds(cx, "setup-later").is_none(),
        "a setup that can finish does not need the later exit"
    );
    assert!(
        crate::test_support::control_bounds(cx, "setup-game").is_some(),
        "a saved game is what makes the exit final"
    );
}

#[gpui::test]
fn first_run_defaults_to_the_install_shaped_storage_parent(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
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
        let expected = block_launcher::storage::Locations {
            library: Some(expected),
            ..Default::default()
        }
        .validate(root.path())?
        .library(root.path());
        assert_eq!(
            view.read_with(cx, |view, _| view.first_run_game_parent()),
            expected
        );
        assert_eq!(
            block_launcher::storage::Locations::load(root.path())?.library(root.path()),
            expected,
            "initialization persists the automatic game parent"
        );
        assert!(!root.path().join("setup-started").exists());
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
        view.handle_import_games(crate::import_games::Action::New, window, cx);
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
    let button = crate::test_support::control_bounds(cx, "setup-account");
    assert!(
        button.is_some(),
        "The account task must be available after returning from a draft"
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
    assert!(
        view.read_with(cx, |view, cx| {
            view.shell.first_run.stage == Stage::GameWizard
                && view.shell.wizard.step == super::game_wizard::WizardStep::Setup
                && matches!(&view.form, Some(super::Form::Instance(form))
                    if form.version.read(cx).value() == "1.20.6")
        }),
        "Recommended pre-fills the same Setup page"
    );
    assert!(
        view.read_with(cx, |view, _| !view.busy && view.pending_creation.is_none()),
        "Recommended must not save on the user's behalf"
    );
}

#[gpui::test]
fn recommended_setup_prefills_without_replacing_an_open_draft(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    let release = version_entry(
        "1.20.6",
        block_launcher::manifest::model::VersionType::Release,
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.begin_first_run(cx);
            view.versions = vec![release.id.clone()];
            view.version_catalog = vec![release];
            view.handle_first_run(FirstRunAction::Game, window, cx);
            view.handle_first_run(FirstRunAction::Custom, window, cx);
            let draft = match &view.form {
                Some(super::Form::Instance(form)) => Some(form.name.entity_id()),
                _ => None,
            };
            if let Some(super::Form::Instance(form)) = &mut view.form {
                form.name
                    .update(cx, |field, cx| field.set_value("Kept draft", cx));
            }
            view.handle_first_run(FirstRunAction::Back, window, cx);
            assert_eq!(view.shell.first_run.stage, Stage::Overview);
            view.handle_first_run(FirstRunAction::Recommended, window, cx);
            assert!(
                matches!(&view.form, Some(super::Form::Instance(form))
                    if form.name.read(cx).value() == "Kept draft"
                        && Some(form.name.entity_id()) == draft),
                "Recommended must resume the draft instead of overwriting it"
            );
            assert!(!view.busy && view.pending_creation.is_none());
        });
    });
}

#[gpui::test]
fn the_game_card_counts_registered_instances(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    view.update(cx, |view, _| {
        let mut snapshot = crate::test_support::ready_snapshot();
        let mut second_instance = snapshot.instances[0].clone();
        second_instance.id = "instance-b".into();
        second_instance.name = "Second game".into();
        snapshot.instances.push(second_instance);
        view.snapshot = snapshot;
    });
    assert!(
        view.read_with(cx, |view, _| view.first_run_game_ready()
            && view.first_run_game_description(crate::Language::English)
                == "2 games saved"),
        "each registered Instance is counted once"
    );
    assert_eq!(
        view.read_with(cx, |view, _| view
            .first_run_game_description(crate::Language::SimplifiedChinese)),
        "已保存 2 个游戏"
    );
}

#[gpui::test]
fn center_tasks_open_independently_and_a_ready_game_leaves_for_home(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });

        view.update(cx, |view, cx| view.begin_first_run(cx));
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(1000.)));
        cx.run_until_parked();
        let account_action = bounds(cx, "setup-account")?;
        cx.simulate_click(account_action.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.auth.is_some() && !view.first_run_account_ready()
        }));

        let account = crate::test_support::test_account()?;
        let account_id = crate::account_id(&account);
        let close = bounds(cx, "task-close")?;
        cx.simulate_click(close.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.accounts = vec![account];
                view.selected_account = Some(account_id.clone());
                view.shell.onboarding.account = Some(account_id);
                view.shell.onboarding.account_decided = true;
                cx.notify();
            });
        });
        cx.run_until_parked();
        let game_action = bounds(cx, "setup-game")?;
        cx.simulate_click(game_action.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == Stage::GameChoice && view.first_run_account_ready()
        }));

        crate::test_support::press(cx, "escape");
        cx.run_until_parked();
        let close = bounds(cx, "task-close")?;
        cx.simulate_click(close.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            view.snapshot = crate::test_support::ready_snapshot();
            view.selected_instance = Some("instance-a".into());
            view.selected_instance = Some("instance-a".into());
            cx.notify();
        });
        cx.run_until_parked();
        let home = bounds(cx, "setup-continue")?;
        cx.simulate_click(home.center(), gpui::Modifiers::none());
        cx.run_until_parked();
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
                && view.snapshot.instances.is_empty()
        }));
        assert!(crate::Preferences::load(root.path())?.onboarding_complete);
        assert!(
            block_launcher::catalog::Catalog::open(root.path())?
                .snapshot()?
                .instances
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
        let saved_snapshot = persisted_snapshot.clone();
        let saved_account = account.clone();
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
        view.update(cx, |view, cx| view.begin_first_run(cx));
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == Stage::Overview
                && view.first_run_account_ready()
                && view.first_run_game_ready()
        }));
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(1000.)));
        cx.run_until_parked();
        for language in crate::Language::ALL {
            for (account_ready, game_ready) in
                [(false, false), (true, false), (false, true), (true, true)]
            {
                view.update(cx, |view, cx| {
                    view.preferences.language = Some(language);
                    view.accounts = if account_ready {
                        vec![saved_account.clone()]
                    } else {
                        Vec::new()
                    };
                    view.snapshot = if game_ready {
                        saved_snapshot.clone()
                    } else {
                        block_launcher::catalog::Snapshot::default()
                    };
                    assert_eq!(view.first_run_account_ready(), account_ready);
                    assert_eq!(view.first_run_game_ready(), game_ready);
                    cx.notify();
                });
                cx.run_until_parked();
                for (selector, configured) in [
                    ("setup-account-status", account_ready),
                    ("setup-game-status", game_ready),
                ] {
                    let (key, english) = if configured {
                        ("setup.status.configured", "Configured")
                    } else {
                        ("setup.status.notConfigured", "Not Configured")
                    };
                    let expected = tr!(language, key);
                    assert_ne!(expected, key, "status must resolve a localized label");
                    if language == crate::Language::English {
                        assert_eq!(expected, english);
                    } else {
                        assert_ne!(expected, english, "status must use the selected language");
                    }
                    assert_eq!(
                        label(cx, selector).as_deref(),
                        Some(expected.as_str()),
                        "{selector} for {language:?}, account={account_ready}, game={game_ready}"
                    );
                }
            }
        }
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
        view.receive_game_import(old, "launcher-one".into(), Ok("late".into()));
        view.finish_game_import(old);
    });
    assert!(view.read_with(cx, |view, _| {
        view.shell.import_games.batch == Some(current)
            && !view.shell.import_games.running
            && matches!(view.shell.import_games.outcomes.get("launcher-one"), Some(super::import_games::Outcome::Failed(error)) if error == "keep this result")
    }));
}

#[gpui::test]
fn install_chooser_failure_has_a_semantic_identity_and_clears_only_itself(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    view.update(cx, |view, _| {
        view.shell.first_run.enabled = true;
        // The shared report path still fills the generic string channel first.
        view.shell.first_run.error = Some("displayed failure".into());

        view.record_install_chooser_failure();
        assert!(matches!(
            view.shell.first_run.notice,
            Some(super::first_run::Notice::InstallChooser)
        ));
        assert!(
            view.shell.first_run.error.is_none(),
            "the retained failure must be the semantic identity, not displayed copy"
        );
        assert!(view.clear_install_chooser_failure());
        assert!(view.shell.first_run.notice.is_none());

        // A different retained first-run failure is not cleared by identity.
        view.shell.first_run.notice = Some(super::first_run::Notice::VersionsUnavailable);
        assert!(!view.clear_install_chooser_failure());
        assert!(view.shell.first_run.notice.is_some());
    });
}

#[gpui::test]
fn first_setup_language_saves_immediately_without_finishing_setup_or_resetting_drafts(
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
            view.begin_first_run(cx);
            view
        });
        let home = cx.update(|window, _| crate::window_geometry::home_size(window));
        cx.simulate_resize(home);
        cx.run_until_parked();
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                // Build a draft the language choice must not reset.
                view.handle_first_run(FirstRunAction::Custom, window, cx);
                if let Some(super::Form::Instance(form)) = &view.form {
                    form.name
                        .update(cx, |field, cx| field.set_value("Retained game", cx));
                }
                view.handle_first_run(FirstRunAction::Back, window, cx);
            });
        });
        let draft_id = view.read_with(cx, |view, _| match &view.form {
            Some(super::Form::Instance(form)) => Some(form.name.entity_id()),
            _ => None,
        });
        assert!(draft_id.is_some());
        cx.run_until_parked();
        let entry = bounds(cx, "setup-language-entry")?;
        let label = cx.update(|window, _| {
            gpui_base::test_support::find(window, &[], &"setup-language-entry".into())
                .and_then(|node| node.label().map(str::to_owned))
        });
        assert_eq!(label.as_deref(), Some(crate::Language::English.self_name()));
        cx.simulate_click(entry.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.surfaces.top().is_some_and(|layer| {
                matches!(
                    layer.kind,
                    super::shell::Surface::Drawer(super::shell::Drawer::Language)
                )
            })
        }));
        assert_eq!(
            cx.update(|window, _| {
                gpui_base::test_support::find(window, &[], &"setup-language-options".into())
                    .and_then(|node| node.role())
            }),
            Some(gpui::Role::RadioGroup)
        );
        let mut previous_bottom = None;
        for (id, expected, checked) in [
            ("setup-language-en", "English", true),
            ("setup-language-zh-Hans", "简体中文", false),
            ("setup-language-ja", "日本語", false),
        ] {
            let node =
                cx.update(|window, _| gpui_base::test_support::find(window, &[], &id.into()));
            let node = node.ok_or_else(|| anyhow::anyhow!("missing language row: {id}"))?;
            assert_eq!(node.label(), Some(expected));
            assert_eq!(node.role(), Some(gpui::Role::RadioButton));
            assert_eq!(node.checked(), Some(checked));
            let row = bounds(cx, id)?;
            assert_eq!(
                row.size.height,
                gpui::px(52.),
                "single-line language choices retain exactly16px vertical insets"
            );
            if let Some(bottom) = previous_bottom {
                assert_eq!(
                    row.top() - bottom,
                    gpui::px(8.),
                    "language choices retain a small vertical gap"
                );
            }
            previous_bottom = Some(row.bottom());
        }
        let search = bounds(cx, "setup-language-search")?;
        cx.simulate_click(search.center(), gpui::Modifiers::none());
        cx.simulate_input("Japanese");
        cx.run_until_parked();
        let japanese = bounds(cx, "setup-language-ja")?;
        cx.simulate_click(japanese.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.preferences.language),
            Some(crate::Language::Japanese),
            "surface after selecting Japanese: {:?}",
            view.read_with(cx, |view, _| view.shell.surfaces.top().cloned())
        );
        assert!(view.read_with(cx, |view, cx| {
            view.shell.first_run.stage == Stage::Overview
                && view.preferences.language == Some(crate::Language::Japanese)
                && view.saved_preferences.language == Some(crate::Language::Japanese)
                && !view.preferences.onboarding_complete
                && view.language() == crate::Language::Japanese
                && matches!(&view.form, Some(super::Form::Instance(form))
                    if form.name.read(cx).value() == "Retained game"
                        && Some(form.name.entity_id()) == draft_id)
        }));
        let stored = crate::Preferences::load(root.path())?;
        assert_eq!(stored.language, Some(crate::Language::Japanese));
        assert!(!stored.onboarding_complete);
        assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()));
        assert_eq!(
            cx.update(|window, _| {
                gpui_base::test_support::find(window, &[], &"setup-language-entry".into())
                    .and_then(|node| node.focused())
            }),
            Some(true),
            "successful selection restores the Overview language trigger"
        );
        let entry = bounds(cx, "setup-language-entry")?;
        cx.simulate_click(entry.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        for (id, checked) in [
            ("setup-language-en", false),
            ("setup-language-zh-Hans", false),
            ("setup-language-ja", true),
        ] {
            let node =
                cx.update(|window, _| gpui_base::test_support::find(window, &[], &id.into()));
            let node = node.ok_or_else(|| anyhow::anyhow!("missing language row: {id}"))?;
            assert_eq!(node.role(), Some(gpui::Role::RadioButton));
            assert_eq!(node.checked(), Some(checked));
        }
        let selected = bounds(cx, "setup-language-ja")?;
        cx.simulate_click(selected.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |view, _| !view.shell.surfaces.is_open()),
            "choosing the active language must still save and dismiss the drawer"
        );
        // A later start keeps the stored choice instead of re-detecting the OS.
        let (restarted, error) =
            crate::Preferences::initialize_language(root.path(), crate::Language::English);
        assert!(error.is_none(), "{error:?}");
        assert_eq!(restarted.language, Some(crate::Language::Japanese));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn first_setup_language_save_failure_is_visible_and_never_claimed_as_saved(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = Engine::with_root(root.path().to_owned())?;
        // A corrupt storage configuration rejects the data-root lease, so the
        // language-only write cannot start.
        fs::write(root.path().join("block.toml"), b"not = [valid")?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.preferences.reduce_motion = true;
            view.begin_first_run(cx);
            view
        });
        let home = cx.update(|window, _| crate::window_geometry::home_size(window));
        cx.simulate_resize(home);
        cx.run_until_parked();
        let entry = bounds(cx, "setup-language-entry")?;
        cx.simulate_click(entry.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let chinese = bounds(cx, "setup-language-zh-Hans")?;
        cx.simulate_click(chinese.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.preferences.language == Some(crate::Language::SimplifiedChinese)
                && view.saved_preferences.language.is_none()
                && view.shell.first_run.notice.is_some()
        }));
        assert!(!root.path().join(crate::global_config::FILE).exists());
        assert!(
            crate::test_support::control_bounds(cx, "setup-language-notice").is_some(),
            "the failed save must be visible in the open language drawer"
        );
        assert!(view.read_with(cx, |view, _| matches!(
            view.shell.surfaces.top().map(|layer| &layer.kind),
            Some(super::shell::Surface::Drawer(
                super::shell::Drawer::Language
            ))
        )));
        // Retrying the now-previewed language must write again, even though its
        // row is already checked.
        fs::remove_file(root.path().join("block.toml"))?;
        let chinese = bounds(cx, "setup-language-zh-Hans")?;
        cx.simulate_click(chinese.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(
            crate::Preferences::load(root.path())?.language,
            Some(crate::Language::SimplifiedChinese)
        );
        assert!(view.read_with(cx, |view, _| {
            !view.shell.surfaces.is_open()
                && view.saved_preferences.language == Some(crate::Language::SimplifiedChinese)
                && view.shell.first_run.notice.is_none()
                && !view.preferences.onboarding_complete
        }));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn language_drawer_choices_keep_tab_focus_inside_the_sheet(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.begin_first_run(cx);
            view
        });
        let home = cx.update(|window, _| crate::window_geometry::home_size(window));
        cx.simulate_resize(home);
        cx.run_until_parked();
        for _ in 0..8 {
            let focused = cx.update(|window, _| {
                gpui_base::test_support::find(window, &[], &"setup-language-entry".into())
                    .and_then(|node| node.focused())
                    == Some(true)
            });
            if focused {
                break;
            }
            crate::test_support::press(cx, "tab");
            cx.run_until_parked();
        }
        let entry_focused = cx.update(|window, _| {
            gpui_base::test_support::find(window, &[], &"setup-language-entry".into())
                .and_then(|node| node.focused())
                == Some(true)
        });
        assert!(
            entry_focused,
            "the Overview language entry is keyboard reachable"
        );
        let entry_focus = cx.update(|window, cx| window.focused(cx));
        crate::test_support::press(cx, "enter");
        cx.run_until_parked();
        let sheet_focus = view
            .read_with(cx, |view, _| {
                view.shell.surfaces.top().map(|layer| layer.focus.clone())
            })
            .ok_or_else(|| anyhow::anyhow!("language drawer is missing"))?;
        for id in [
            "setup-language-en",
            "setup-language-zh-Hans",
            "setup-language-ja",
        ] {
            let row = bounds(cx, id)?;
            assert!(row.left() >= gpui::px(0.) && row.right() <= home.width);
            assert!(row.top() >= gpui::px(56.) && row.bottom() <= home.height);
        }
        let mut reached = std::collections::HashSet::new();
        for key in [
            "tab",
            "tab",
            "tab",
            "tab",
            "tab",
            "tab",
            "shift-tab",
            "shift-tab",
            "shift-tab",
            "shift-tab",
            "shift-tab",
            "shift-tab",
        ] {
            crate::test_support::press(cx, key);
            cx.run_until_parked();
            for id in [
                "setup-language-en",
                "setup-language-zh-Hans",
                "setup-language-ja",
            ] {
                if cx.update(|window, _| {
                    gpui_base::test_support::find(window, &[], &id.into())
                        .and_then(|node| node.focused())
                        == Some(true)
                }) {
                    reached.insert(id);
                }
            }
            for id in [
                "setup-account",
                "setup-game",
                "setup-continue",
                "setup-language-entry",
            ] {
                let focused = cx.update(|window, _| {
                    gpui_base::test_support::find(window, &[], &id.into())
                        .and_then(|node| node.focused())
                });
                assert_eq!(focused, Some(false), "{id} received focus after {key}");
            }
            assert!(
                cx.update(|window, cx| sheet_focus.contains_focused(window, cx)),
                "focus left the language drawer after {key}"
            );
            assert!(view.read_with(cx, |view, _| {
                view.shell.first_run.stage == Stage::Overview
                    && !view.preferences.onboarding_complete
            }));
        }
        assert_eq!(
            reached.len(),
            3,
            "all three language choices are keyboard reachable"
        );
        focus_language_drawer_close(cx);
        crate::test_support::press(cx, "escape");
        assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()));

        cx.run_until_parked();
        assert_eq!(
            cx.update(|window, cx| window.focused(cx)),
            entry_focus,
            "closing the drawer restores its Overview trigger"
        );
        let entry = bounds(cx, "setup-language-entry")?;
        cx.simulate_click(entry.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        focus_language_drawer_close(cx);
        crate::test_support::press(cx, "enter");
        assert!(
            view.read_with(cx, |view, _| !view.shell.surfaces.is_open()),
            "the close control must dismiss the language choices"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

fn focus_language_drawer_close(cx: &mut gpui::VisualTestContext) {
    let mut reached = false;
    for _ in 0..8 {
        reached = cx.update(|window, _| {
            gpui_base::test_support::find(window, &[], &"drawer-close".into())
                .and_then(|node| node.focused())
                == Some(true)
        });
        if reached {
            break;
        }
        crate::test_support::press(cx, "tab");
        cx.run_until_parked();
    }
    assert!(reached, "the drawer's close control must remain reachable");
}

#[gpui::test]
fn language_drawer_blocks_background_overview_actions(cx: &mut gpui::TestAppContext) {
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
        cx.simulate_resize(home);
        cx.run_until_parked();
        let targets = [
            "setup-account",
            "setup-game",
            "setup-continue",
            "setup-language-entry",
        ];
        for selector in targets {
            // A click over a choice can change the language. Locate the
            // translated controls again before testing the next modal boundary.
            let entry = bounds(cx, "setup-language-entry")?.center();
            let target = bounds(cx, selector)?.center();
            cx.simulate_click(entry, gpui::Modifiers::none());
            cx.run_until_parked();
            assert!(
                view.read_with(cx, |view, _| view.shell.surfaces.is_open()),
                "{selector}: the language drawer must open"
            );
            assert!(view.read_with(cx, |view, _| {
                view.shell.surfaces.top().is_some_and(|layer| {
                    matches!(
                        layer.kind,
                        super::shell::Surface::Drawer(super::shell::Drawer::Language)
                    )
                })
            }));
            cx.simulate_click(target, gpui::Modifiers::none());
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                view.shell.first_run.stage == Stage::Overview
                    && !view.preferences.onboarding_complete
                    && view.router.tab() == Tab::Onboarding
                    && !view.wizard_task_active()
                    && view.shell.auth.is_none()
            }));
            assert!(!root.path().join("setup-started").exists());
            for _ in 0..2 {
                if view.read_with(cx, |view, _| view.shell.surfaces.is_open()) {
                    crate::test_support::press(cx, "escape");
                    cx.run_until_parked();
                }
            }
            assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()));
        }
        assert!(!crate::Preferences::load(root.path())?.onboarding_complete);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn language_search_matches_names_and_codes_without_selecting(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.begin_first_run(cx);
            view
        });
        let home = cx.update(|window, _| crate::window_geometry::home_size(window));
        cx.simulate_resize(home);
        cx.run_until_parked();
        let entry = bounds(cx, "setup-language-entry")?;
        cx.simulate_click(entry.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let before = view.read_with(cx, |view, _| view.preferences.language);
        for (query, expected) in [
            ("  jApAnEsE  ", Some("ja")),
            ("简体", Some("zh-Hans")),
            ("ZH-HANS", Some("zh-Hans")),
            ("日本", Some("ja")),
            (" CHINESE ", Some("zh-Hans")),
            ("en", Some("en")),
            ("no such language", None),
        ] {
            let search = bounds(cx, "setup-language-search")?;
            cx.simulate_click(search.center(), gpui::Modifiers::none());
            cx.simulate_input(query);
            cx.run_until_parked();
            for code in ["en", "zh-Hans", "ja"] {
                let id = format!("setup-language-{code}");
                let present = cx.update(|window, _| {
                    gpui_base::test_support::find(window, &[], &id.into()).is_some()
                });
                assert_eq!(present, expected == Some(code), "{query}: {code}");
            }
            assert_eq!(
                crate::test_support::control_bounds(cx, "setup-language-empty").is_some(),
                expected.is_none()
            );
            assert_eq!(
                view.read_with(cx, |view, _| view.preferences.language),
                before
            );
            assert!(view.read_with(cx, |view, _| {
                view.shell.surfaces.is_open() && !view.preferences.onboarding_complete
            }));
            let clear = bounds(cx, "setup-language-clear")?;
            cx.simulate_click(clear.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            for code in ["en", "zh-Hans", "ja"] {
                let id = format!("setup-language-{code}");
                assert!(cx.update(|window, _| {
                    gpui_base::test_support::find(window, &[], &id.into()).is_some()
                }));
            }
        }
        let search = bounds(cx, "setup-language-search")?;
        cx.simulate_click(search.center(), gpui::Modifiers::none());
        cx.simulate_input("Japanese");
        crate::test_support::press(cx, "escape");
        cx.run_until_parked();
        let entry = bounds(cx, "setup-language-entry")?;
        cx.simulate_click(entry.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(crate::test_support::control_bounds(cx, "setup-language-en").is_some());
        assert!(crate::test_support::control_bounds(cx, "setup-language-clear").is_none());
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn language_drawer_dismisses_in_one_step_without_a_nested_select(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.begin_first_run(cx);
            view
        });
        let home = cx.update(|window, _| crate::window_geometry::home_size(window));
        cx.simulate_resize(home);
        cx.run_until_parked();
        let entry = bounds(cx, "setup-language-entry")?;
        cx.simulate_click(entry.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        crate::test_support::press(cx, "escape");
        assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()));

        cx.run_until_parked();
        let entry = bounds(cx, "setup-language-entry")?;
        cx.simulate_click(entry.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let backdrop = bounds(cx, "block-drawer-backdrop")?;
        let outside = gpui::point(
            backdrop.left() + gpui::px(10.),
            backdrop.top() + gpui::px(60.),
        );
        cx.simulate_click(outside, gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()));
        assert!(view.read_with(cx, |view, _| {
            view.shell.first_run.stage == Stage::Overview && !view.preferences.onboarding_complete
        }));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}
