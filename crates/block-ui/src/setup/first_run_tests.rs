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
            crate::test_support::control_bounds(cx, "setup-import").is_some(),
            "the count carries the one explicit import entry"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
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
                view.handle_first_run(FirstRunAction::Rescan, window, cx);
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
            view.selected_profile = Some("profile-a".into());
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
            account.bottom() <= game.top() && account.size == game.size
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
                view.selected_profile = Some("profile-a".into());
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
        view.selected_profile = Some("profile-a".into());
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
fn the_game_card_counts_instances_not_profiles(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    view.update(cx, |view, _| {
        let mut snapshot = crate::test_support::ready_snapshot();
        let mut second_profile = snapshot.profiles[0].clone();
        second_profile.id = "profile-b".into();
        second_profile.name = "Second profile".into();
        snapshot.profiles.push(second_profile.clone());
        second_profile.id = "profile-c".into();
        second_profile.name = "Third profile".into();
        snapshot.profiles.push(second_profile);
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
        "a game is one Instance, however many Profiles launch it"
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

        let close = bounds(cx, "task-close")?;
        cx.simulate_click(close.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            view.snapshot = crate::test_support::ready_snapshot();
            view.selected_instance = Some("instance-a".into());
            view.selected_profile = Some("profile-a".into());
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
        let select = bounds(cx, "setup-language")?;
        cx.simulate_click(select.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.surfaces.top().is_some_and(|layer| matches!(
                &layer.kind,
                super::shell::Surface::Select(state)
                    if state.choices.iter().map(|choice| choice.label.as_str()).collect::<Vec<_>>()
                        == ["English", "简体中文", "日本語"]
            ))
        }));
        for (id, expected) in [
            ("select-choice-0", "English"),
            ("select-choice-1", "简体中文"),
            ("select-choice-2", "日本語"),
        ] {
            let label = cx.update(|window, _| {
                gpui_base::test_support::find(window, &[], &id.into())
                    .and_then(|node| node.label().map(str::to_owned))
            });
            assert_eq!(label.as_deref(), Some(expected));
        }
        crate::test_support::press(cx, "escape");
        assert!(
            view.read_with(cx, |view, _| {
                view.shell.surfaces.top().is_some_and(|layer| {
                    matches!(
                        layer.kind,
                        super::shell::Surface::Drawer(super::shell::Drawer::Language)
                    )
                })
            }),
            "surface after select escape: {:?}",
            view.read_with(cx, |view, _| view.shell.surfaces.top().cloned())
        );
        let select = bounds(cx, "setup-language")?;
        cx.simulate_click(select.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let japanese = bounds(cx, "select-choice-2")?;
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
        assert!(view.read_with(cx, |view, _| {
            view.shell.surfaces.top().is_some_and(|layer| {
                matches!(
                    layer.kind,
                    super::shell::Surface::Drawer(super::shell::Drawer::Language)
                )
            })
        }));
        let close = bounds(cx, "drawer-close")?;
        cx.simulate_click(close.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()));
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
        let select = bounds(cx, "setup-language")?;
        cx.simulate_click(select.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let chinese = bounds(cx, "select-choice-1")?;
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
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn language_drawer_select_keeps_tab_focus_inside_the_sheet(cx: &mut gpui::TestAppContext) {
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
        let select = bounds(cx, "setup-language")?;
        cx.simulate_click(select.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .surfaces
                .top()
                .is_some_and(|layer| matches!(&layer.kind, super::shell::Surface::Select(_)))
        }));
        crate::test_support::press(cx, "escape");
        cx.run_until_parked();
        let trigger = cx
            .update(|window, cx| window.focused(cx))
            .ok_or_else(|| anyhow::anyhow!("the language trigger did not regain focus"))?;
        for key in ["tab", "shift-tab"] {
            cx.update(|window, cx| trigger.focus(window, cx));
            crate::test_support::press(cx, key);
            cx.run_until_parked();
            let expected = cx.update(|window, cx| window.focused(cx));
            let select = bounds(cx, "setup-language")?;
            cx.simulate_click(select.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            crate::test_support::press(cx, key);
            for _ in 0..2 {
                cx.update(|window, cx| window.simulate_next_frame(cx));
                cx.run_until_parked();
            }
            assert_eq!(
                cx.update(|window, cx| window.focused(cx)),
                expected,
                "{key}"
            );
            assert!(
                view.read_with(cx, |view, _| {
                    matches!(
                        view.shell.surfaces.top().map(|layer| &layer.kind),
                        Some(super::shell::Surface::Drawer(
                            super::shell::Drawer::Language
                        ))
                    )
                }),
                "{key} closes only the Select"
            );
            assert!(cx.update(|window, cx| sheet_focus.contains_focused(window, cx)));
        }
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
        focus_language_drawer_close(cx);
        let select = bounds(cx, "setup-language")?;
        cx.simulate_click(select.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(
                |layer| { matches!(layer.kind, super::shell::Surface::Select(_)) }
            )),
            "the reopened Select must be on top before Escape"
        );
        crate::test_support::press(cx, "escape");
        assert!(
            view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(
                |layer| {
                    matches!(
                        layer.kind,
                        super::shell::Surface::Drawer(super::shell::Drawer::Language)
                    )
                }
            )),
            "Escape after tabbing must close only the Select: {:?}",
            view.read_with(cx, |view, _| view.shell.surfaces.top().cloned())
        );
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
        let select = bounds(cx, "setup-language")?;
        cx.simulate_click(select.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        focus_language_drawer_close(cx);
        crate::test_support::press(cx, "enter");
        assert!(
            view.read_with(cx, |view, _| !view.shell.surfaces.is_open()),
            "the close control must not leave a selector behind"
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
            // A click over the selector can change the language. Locate the
            // translated controls again before testing the next modal boundary.
            let entry = bounds(cx, "setup-language-entry")?.center();
            let target = bounds(cx, selector)?.center();
            cx.simulate_click(entry, gpui::Modifiers::none());
            cx.run_until_parked();
            assert!(
                view.read_with(cx, |view, _| view.shell.surfaces.is_open()),
                "{selector}: the language drawer must open"
            );
            let select = bounds(cx, "setup-language")?;
            cx.simulate_click(select.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                view.shell
                    .surfaces
                    .top()
                    .is_some_and(|layer| matches!(layer.kind, super::shell::Surface::Select(_)))
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
fn language_drawer_dismissal_respects_its_nested_select(cx: &mut gpui::TestAppContext) {
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
        let select = bounds(cx, "setup-language")?;
        cx.simulate_click(select.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        crate::test_support::press(cx, "escape");
        assert!(view.read_with(cx, |view, _| {
            view.shell.surfaces.top().is_some_and(|layer| {
                matches!(
                    layer.kind,
                    super::shell::Surface::Drawer(super::shell::Drawer::Language)
                )
            })
        }));
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
