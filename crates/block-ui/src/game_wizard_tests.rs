//! Product outcomes for drafts, catalog choices, and wizard boundaries.

use block_launcher::catalog::{ProfileColor, ProfileIcon, ProfileMode};
use gpui::{EntityInputHandler as _, Focusable as _, Modifiers, TestAppContext, point, px, size};

use super::{CreationKind, WizardPage, WizardState, WizardStep};
use crate::{Form, test_support::ready_snapshot};

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = crate::test_support::control_bounds(cx, selector);
    assert!(bounds.is_some(), "required control is missing: {selector}");
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

fn entry(
    id: &str,
    kind: minecraft_manifest::model::VersionType,
) -> minecraft_manifest::model::VersionEntry {
    minecraft_manifest::model::VersionEntry {
        id: id.into(),
        kind,
        url: "https://example.invalid/manifest".into(),
        time: String::new(),
        release_time: String::new(),
        sha1: "0".repeat(40),
        compliance_level: 1,
    }
}

#[test]
fn bundled_and_separated_steps_keep_their_own_back_stack() {
    let mut bundled = WizardState::default();
    bundled.back();
    assert_eq!(bundled.step, WizardStep::Setup);
    bundled.next();
    assert_eq!(bundled.step, WizardStep::Review);
    bundled.back();
    assert_eq!(bundled.step, WizardStep::Setup);

    let mut separated = WizardState {
        separated: true,
        step: WizardStep::GetStarted,
        ..WizardState::default()
    };
    for expected in [WizardStep::Setup, WizardStep::Profile, WizardStep::Review] {
        separated.next();
        assert_eq!(separated.step, expected);
    }
    for expected in [
        WizardStep::Profile,
        WizardStep::Setup,
        WizardStep::GetStarted,
    ] {
        separated.back();
        assert_eq!(separated.step, expected);
    }
    separated.back();
    assert_eq!(separated.step, WizardStep::GetStarted);
}

#[gpui::test]
fn wizard_component_rows_fit_both_sides_of_the_column_breakpoint(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.begin_game_wizard(cx);
        view
    });
    for width in [960., 759., 760., 360., 960.] {
        cx.simulate_resize(size(px(width), px(600.)));
        cx.run_until_parked();
        let grid = crate::test_support::control_bounds(cx, "wizard-components");
        let tile = crate::test_support::control_bounds(cx, "wizard-component-version");
        assert!(
            grid.is_some() && tile.is_some(),
            "live wizard components must be rendered"
        );
        if let (Some(grid), Some(tile)) = (grid, tile) {
            let rows = if width < 760. { 2. } else { 1. };
            let expected = tile.size.height * rows + px(12. * (rows - 1.));
            assert!(
                (tile.size.width - tile.size.height).abs() <= px(1.),
                "wizard components must stay square"
            );
            assert!(
                (grid.size.height - expected).abs() <= px(1.),
                "wizard grid must only reserve its real rows: grid={grid:?}, tile={tile:?}"
            );
        }
        assert!(view.read_with(cx, |view, _| view.shell.wizard.page.is_none()));
    }
}

#[gpui::test]
fn retained_wizard_snapshot_freezes_editors_and_scroll_without_sharing_mutation(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.snapshot.profile_mode = ProfileMode::Separated;
        view.begin_game_wizard(cx);
        view
    });
    view.update(cx, |view, cx| {
        view.shell
            .wizard
            .body_scroll
            .set_offset(point(px(0.), px(-112.)));
        view.shell
            .wizard
            .version_scroll
            .set_offset(point(px(0.), px(-64.)));
        if let Some(profile) = &view.shell.wizard.profile {
            profile
                .name
                .update(cx, |input, cx| input.set_value("中文 Profile", cx));
        }
        let frozen = view.shell.wizard.snapshot(cx);
        view.shell
            .wizard
            .body_scroll
            .set_offset(point(px(0.), px(-8.)));
        view.shell
            .wizard
            .version_scroll
            .set_offset(point(px(0.), px(-2.)));
        if let Some(profile) = &view.shell.wizard.profile {
            profile
                .name
                .update(cx, |input, cx| input.set_value("New live name", cx));
        }
        assert_eq!(frozen.body_scroll.offset(), point(px(0.), px(-112.)));
        assert_eq!(frozen.version_scroll.offset(), point(px(0.), px(-64.)));
        assert!(
            frozen.reveal.get().is_none(),
            "a retained exit must not reveal or focus fields"
        );
        let profile = frozen.profile.as_ref();
        assert!(profile.is_some());
        if let Some(profile) = profile {
            assert_eq!(profile.name.read(cx).value(), "中文 Profile");
        }
    });
}

#[gpui::test]
fn component_cancel_restores_values_and_done_only_commits_the_local_draft(cx: &mut TestAppContext) {
    let directory = std::env::temp_dir().join("block-new-game");
    let directory = directory.to_string_lossy().into_owned();
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.21.1".into(), "1.20.6".into()];
        view.snapshot = ready_snapshot();
        view.begin_game_wizard(cx);
        view
    });
    view.update(cx, |view, cx| {
        view.open_game_wizard_page(Some(WizardPage::Loader), cx);
        if let Some(Form::Instance(form)) = &mut view.form {
            form.version
                .update(cx, |field, cx| field.set_value("1.20.6", cx));
            form.directory
                .update(cx, |field, cx| field.set_value(&directory, cx));
            form.loader = block_launcher::catalog::LoaderKind::NeoForge;
            form.loader_version = Some("20.6.144".into());
        }
        view.back_game_wizard(cx);
        if let Some(Form::Instance(form)) = &view.form {
            assert_eq!(form.version.read(cx).value(), "1.21.1");
            assert_eq!(form.directory.read(cx).value(), "");
            assert!(!form.is_neoforge());
        }
        assert_eq!(view.snapshot.instances.len(), 1);
        view.open_game_wizard_page(Some(WizardPage::Location), cx);
        if let Some(Form::Instance(form)) = &view.form {
            form.directory
                .update(cx, |field, cx| field.set_value("relative-path", cx));
        }
        view.advance_game_wizard(cx);
        assert_eq!(view.shell.wizard.page, Some(WizardPage::Location));
        assert!(view.shell.wizard.error.is_some());
        if let Some(Form::Instance(form)) = &view.form {
            form.directory
                .update(cx, |field, cx| field.set_value(&directory, cx));
        }
        view.advance_game_wizard(cx);
        assert_eq!(view.shell.wizard.page, None);
        assert!(view.shell.wizard.error.is_none());
        if let Some(Form::Instance(form)) = &view.form {
            assert_eq!(form.directory.read(cx).value(), directory);
        }
        assert_eq!(view.snapshot.instances.len(), 1);
        assert_eq!(view.snapshot.profiles.len(), 1);
    });
}

#[gpui::test]
fn loader_picker_search_preserves_the_selected_real_build_and_cancel_rolls_back(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.21.1".into(), "1.20.6".into()];
        view.begin_game_wizard(cx);
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));

    click(cx, "wizard-component-loader");
    assert!(crate::test_support::control_bounds(cx, "wizard-loader-picker").is_some());
    click(cx, "wizard-loader-neoforge");
    view.update(cx, |view, cx| {
        let request = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        if let Some(request) = request { view.receive_loader_versions(request, Ok(vec![loader_build("21.1.219", true)]), cx); }
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.is_neoforge() && form.version.read(cx).value() == "1.21.1"));
        cx.notify();
    });
    view.update(cx, |view, cx| {
        if let Some(search) = &view.shell.wizard.loader_search {
            search.update(cx, |field, cx| field.set_value("does-not-exist", cx));
        }
        cx.notify();
    });
    cx.run_until_parked();
    assert!(crate::test_support::control_bounds(cx, "wizard-loader-build-21.1.219").is_none());
    view.update(cx, |view, cx| {
        if let Some(search) = &view.shell.wizard.loader_search {
            search.update(cx, |field, cx| field.set_value("21.1", cx));
        }
        cx.notify();
    });
    cx.run_until_parked();
    assert!(crate::test_support::control_bounds(cx, "wizard-loader-build-21.1.219").is_some());
    click(cx, "wizard-loader-fabric");
    assert!(view.read_with(
        cx,
        |view, _| matches!(&view.form, Some(Form::Instance(form)) if form.loader == block_launcher::catalog::LoaderKind::Fabric && form.loader_version.is_none())
    ));
    click(cx, "wizard-back");
    view.read_with(cx, |view, cx| {
        assert!(matches!(&view.form, Some(Form::Instance(form)) if !form.is_neoforge() && form.version.read(cx).value() == "1.21.1"));
    });
}

#[gpui::test]
fn separated_profile_review_preserves_home_selection_until_submission(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.snapshot.profile_mode = ProfileMode::Separated;
        view.selected_instance = Some("instance-a".into());
        view.selected_profile = Some("profile-a".into());
        view.begin_game_wizard(cx);
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    click(cx, "wizard-kind-profile");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.kind),
        CreationKind::ExistingInstance
    );
    click(cx, "wizard-next");
    click(cx, "wizard-instance-instance-a");
    click(cx, "wizard-next");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.step),
        WizardStep::Profile
    );
    view.update(cx, |view, cx| {
        let profile = view.shell.wizard.profile.as_ref();
        assert!(profile.is_some());
        if let Some(profile) = profile {
            profile
                .name
                .update(cx, |field, cx| field.set_value("Creative 中文", cx));
            profile
                .memory
                .update(cx, |field, cx| field.set_value("4096", cx));
        }
        view.wizard_profile_icon(ProfileIcon::Leaf);
        view.wizard_profile_color(ProfileColor::Green);
        view.advance_game_wizard(cx);
        assert_eq!(view.shell.wizard.step, WizardStep::Review);
        if let Some(profile) = &view.shell.wizard.profile {
            let values = profile.values(cx);
            assert!(values.is_ok());
            if let Ok(values) = values {
                assert_eq!(values.name, "Creative 中文");
                assert_eq!(values.defaults.memory_mib, 4096);
                assert_eq!(values.appearance.icon, ProfileIcon::Leaf);
                assert_eq!(values.target_id.as_deref(), Some("vanilla"));
            }
        }
        assert_eq!(view.selected_profile.as_deref(), Some("profile-a"));
        assert_eq!(view.snapshot.profiles.len(), 1);
        cx.notify();
    });
    click(cx, "cancel-instance");
    assert!(view.read_with(cx, |view, _| view.form.is_none()));
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_profile.clone()),
        Some("profile-a".into())
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.snapshot.profiles.len()),
        1
    );
}

#[gpui::test]
fn version_search_filters_real_metadata_and_does_not_steal_ime_navigation(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.20.6".into(), "1.21.1".into()];
        view.version_catalog = vec![
            entry("1.20.6", minecraft_manifest::model::VersionType::Release),
            entry("1.21.1", minecraft_manifest::model::VersionType::Release),
            entry("24w44a", minecraft_manifest::model::VersionType::Snapshot),
        ];
        view.begin_game_wizard(cx);
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        view
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(search) = &view.shell.wizard.version_search {
                search.read(cx).focus_handle(cx).focus(window, cx);
            }
        });
    });
    crate::test_support::press(cx, "down");
    view.read_with(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &view.form {
            assert_eq!(form.version.read(cx).value(), "1.21.1");
            assert_eq!(view.matching_wizard_versions(form, cx).len(), 2);
        }
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(search) = &view.shell.wizard.version_search {
                search.update(cx, |search, cx| {
                    search.replace_and_mark_text_in_range(None, "中", None, window, cx);
                });
            }
        });
    });
    crate::test_support::press(cx, "up");
    view.read_with(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &view.form {
            assert_eq!(form.version.read(cx).value(), "1.21.1");
        }
    });
    view.update(cx, |view, cx| {
        if let Some(search) = &view.shell.wizard.version_search {
            search.update(cx, |search, cx| search.set_value("24w", cx));
        }
        view.shell.wizard.include_snapshots = true;
        if let Some(Form::Instance(form)) = &view.form {
            assert_eq!(
                view.matching_wizard_versions(form, cx)
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>(),
                ["24w44a"]
            );
        }
        cx.notify();
    });
    crate::test_support::press(cx, "down");
    view.read_with(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &view.form {
            assert_eq!(form.version.read(cx).value(), "24w44a");
        }
    });
}

#[gpui::test]
fn long_version_list_end_reveals_the_last_row_and_has_one_roving_tab_stop(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = (0..40).map(|index| format!("1.20.{index}")).collect();
        view.begin_game_wizard(cx);
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        if let Some(search) = &view.shell.wizard.version_search {
            search.read(cx).focus_handle(cx).focus(window, cx);
        }
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    crate::test_support::press(cx, "tab end");
    cx.run_until_parked();
    view.read_with(cx, |view, cx| { assert!(matches!(&view.form, Some(Form::Instance(form)) if form.version.read(cx).value() == "1.20.39")); });
    let last = crate::test_support::control_bounds(cx, "wizard-version-39");
    let viewport = crate::test_support::control_bounds(cx, "wizard-version-list");
    assert!(
        last.zip(viewport)
            .is_some_and(|(last, viewport)| last.top() >= viewport.top() - px(1.)
                && last.bottom() <= viewport.bottom() + px(1.)),
        "last={last:?}, viewport={viewport:?}"
    );

    crate::test_support::press(cx, "tab");
    cx.update(|window, cx| { assert!(matches!(&view.read(cx).form, Some(Form::Instance(form)) if form.version.read(cx).focus_handle(cx).is_focused(window))); });
}

#[gpui::test]
fn version_loading_failure_retry_and_recovery_preserve_the_draft(cx: &mut TestAppContext) {
    let (sender, receiver) = tokio::sync::mpsc::channel(4);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.20.6".into()];
        view.version_load = crate::browser::CollectionLoad::Loading;
        view.messages = Some(receiver);
        view.begin_game_wizard(cx);
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        if let Some(Form::Instance(form)) = &view.form {
            form.name
                .update(cx, |field, cx| field.set_value("保留 draft", cx));
        }
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.run_until_parked();

    assert!(crate::test_support::control_bounds(cx, "wizard-retry-versions").is_none());
    assert!(
        sender
            .try_send(crate::Message::Versions(Err(
                "Fixture network unavailable".into()
            )))
            .is_ok()
    );
    view.update(cx, crate::Block::poll);
    cx.run_until_parked();

    click(cx, "wizard-retry-versions");
    // No test Engine: retry must report the actual missing engine, never Ready.
    assert!(view.read_with(cx, |view, _| matches!(&view.version_load, crate::browser::CollectionLoad::Failed(message) if message.contains("Engine unavailable"))));
    assert!(
        sender
            .try_send(crate::Message::Versions(Ok(vec![entry(
                "1.20.6",
                minecraft_manifest::model::VersionType::Release
            )])))
            .is_ok()
    );
    view.update(cx, crate::Block::poll);
    cx.run_until_parked();

    view.read_with(cx, |view, cx| {
        assert!(matches!(view.version_load, crate::browser::CollectionLoad::Ready));
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.name.read(cx).value() == "保留 draft" && form.version.read(cx).value() == "1.20.6"));
        assert_eq!(view.version_catalog.len(), 1);
    });
}

#[gpui::test]
fn exact_version_metadata_and_filtered_result_count_never_guess_a_release_type(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.20.6".into()];
        view.version_catalog = vec![
            entry("1.20.6", minecraft_manifest::model::VersionType::Release),
            entry("24w44a", minecraft_manifest::model::VersionType::Snapshot),
            entry("b1.7.3", minecraft_manifest::model::VersionType::OldBeta),
        ];
        view.begin_game_wizard(cx);
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        view.shell.wizard.include_snapshots = true;
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.run_until_parked();

    for (id, _source) in [
        ("24w44a", "Official catalog · Snapshot"),
        ("b1.7.3", "Official catalog · Old beta"),
        ("1.20.6", "Official catalog · Stable release"),
        (
            "snapshot-looking-unknown",
            "Exact version ID · not found in the loaded official catalog; installation will verify it",
        ),
    ] {
        view.update(cx, |view, cx| {
            if let Some(Form::Instance(form)) = &view.form {
                form.version.update(cx, |field, cx| field.set_value(id, cx));
            }
        });
        cx.run_until_parked();
    }
    view.update(cx, |view, cx| {
        if let Some(search) = &view.shell.wizard.version_search {
            search.update(cx, |field, cx| field.set_value("24w", cx));
        }
    });
    cx.run_until_parked();

    view.update(cx, |view, cx| {
        if let Some(search) = &view.shell.wizard.version_search {
            search.update(cx, |field, cx| field.set_value("no-match", cx));
        }
    });
    cx.run_until_parked();
}

#[gpui::test]
fn minecraft_change_clears_the_build_and_cancel_restores_the_loader(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.versions = vec!["1.20.6".into(), "1.21.1".into()];
        view.begin_game_wizard(cx);
        if let Some(Form::Instance(form)) = &mut view.form {
            form.loader = block_launcher::catalog::LoaderKind::NeoForge;
            form.loader_version = Some("20.6.144".into());
        }
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        view.set_wizard_version(" 1.21.1 ".into(), cx);
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.run_until_parked();

    view.read_with(cx, |view, cx| { assert!(matches!(&view.form, Some(Form::Instance(form)) if form.is_neoforge() && form.loader_version.is_none() && form.version.read(cx).value() == "1.21.1")); });
    click(cx, "wizard-back");
    view.update(cx, |view, cx| {
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.is_neoforge() && form.version.read(cx).value() == "1.20.6"));
        assert!(view.shell.wizard.version_notice.is_none());
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        view.set_wizard_version("1.20.6".into(), cx);
        assert!(view.shell.wizard.version_notice.is_none(), "the unchanged compatible version cannot announce a removal");
        view.set_wizard_version("1.21.1".into(), cx);
        let request = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        if let Some(request) = request { view.receive_loader_versions(request, Ok(vec![loader_build("21.1.219", true)]), cx); }
        view.advance_game_wizard(cx);
        assert!(view.shell.wizard.version_notice.is_some());
        assert!(view.shell.wizard.page.is_none());
        assert_eq!(view.snapshot.instances.len(), 1);
        assert_eq!(view.snapshot.profiles.len(), 1);
    });
}

#[gpui::test]
fn narrow_wizard_keeps_step_rail_and_actions_within_the_client(cx: &mut TestAppContext) {
    for width in [960., 640., 360.] {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.begin_game_wizard(cx);
            view
        });
        cx.simulate_resize(size(px(width), px(540.)));
        cx.run_until_parked();
        for selector in ["wizard-rail", "wizard-footer", "wizard-next"] {
            let bounds = crate::test_support::control_bounds(cx, selector);
            assert!(
                bounds.is_some_and(|bounds| bounds.left() >= px(0.)
                    && bounds.right() <= px(width)
                    && bounds.bottom() <= px(540.)),
                "{selector} outside {width}: {bounds:?}"
            );
        }
        let footer = crate::test_support::control_bounds(cx, "wizard-footer");
        assert_eq!(
            footer.map(|bounds| bounds.size.height),
            Some(px(if width < 640. { 56. } else { 72. }))
        );
        let rail = crate::test_support::control_bounds(cx, "wizard-rail");
        if width < 640. {
            assert_eq!(rail.map(|bounds| bounds.size.width), Some(px(48.)));
        }
        // A disabled capability tile consumes the click and preserves the draft.
        if let Some(bounds) = crate::test_support::control_bounds(cx, "wizard-unavailable-optifine")
        {
            cx.simulate_click(
                point(bounds.left() + px(4.), bounds.top() + px(4.)),
                Modifiers::none(),
            );
        }
        assert!(view.read_with(cx, |view, _| view.shell.wizard.page.is_none()));
    }
}

#[gpui::test]
#[expect(
    clippy::too_many_lines,
    reason = "One save test verifies event order, disk atomicity, and duplicate submission together."
)]
fn submitting_an_existing_instance_saves_one_fully_configured_profile(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let catalog = block_launcher::catalog::Catalog::open(root.path())?;
        let (instance, original) = catalog.create_instance(
            "Shared".into(),
            "1.20.6".into(),
            None,
            Some("20.6.144".into()),
        )?;
        catalog.separate_profiles()?;
        let snapshot = catalog.snapshot()?;
        let (engine, mut receiver) = crate::Engine::with_root(root.path().into())?;
        let instance_id = instance.id;
        let original_id = original.id.clone();
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.snapshot = snapshot;
            view.engine = Some(engine);
            view.selected_instance = Some(instance_id);
            view.selected_profile = Some(original_id);
            view.begin_game_wizard(cx);
            view
        });
        view.update(cx, |view, cx| {
            view.wizard_kind(false, cx);
            view.wizard_profile_target("neoforge-20.6.144".into());
            if let Some(profile) = &view.shell.wizard.profile {
                profile
                    .name
                    .update(cx, |field, cx| field.set_value("Creative 中文", cx));
                profile
                    .memory
                    .update(cx, |field, cx| field.set_value("4096", cx));
                profile.server.update(cx, |field, cx| {
                    field.set_value("play.example.test:25565", cx);
                });
                profile
                    .width
                    .update(cx, |field, cx| field.set_value("1600", cx));
                profile
                    .height
                    .update(cx, |field, cx| field.set_value("900", cx));
            }
            view.wizard_profile_color(ProfileColor::Green);
            view.wizard_profile_icon(ProfileIcon::Leaf);
            view.shell.wizard.step = WizardStep::Review;
            view.submit_game_wizard(cx);
            assert!(view.busy);
            view.submit_game_wizard(cx);
        });
        let runtime = tokio::runtime::Runtime::new()?;
        let messages = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let mut messages = Vec::new();
                loop {
                    let message = receiver.recv().await.ok_or_else(|| {
                        anyhow::anyhow!("engine channel closed before save completed")
                    })?;
                    let inner = match &message {
                        crate::Message::ForOperation { message, .. } => message.as_ref(),
                        message => message,
                    };
                    let finished = matches!(inner, crate::Message::Finished(_));
                    if matches!(inner, crate::Message::Created { .. }) {
                        let snapshot = catalog.snapshot()?;
                        assert_eq!(snapshot.profiles.len(), 2);
                        let created = snapshot
                            .profiles
                            .iter()
                            .find(|profile| profile.id != original.id);
                        assert!(created.is_some_and(|profile| {
                            profile.name == "Creative 中文"
                                && profile.memory_mib == 4096
                                && profile.target_id == "neoforge-20.6.144"
                                && profile.appearance.icon == ProfileIcon::Leaf
                                && profile.server.address == "play.example.test:25565"
                                && !profile.server.connect_on_launch
                                && profile
                                    .game_args
                                    .windows(2)
                                    .any(|pair| pair[0] == "--width" && pair[1] == "1600")
                                && profile
                                    .game_args
                                    .windows(2)
                                    .any(|pair| pair[0] == "--height" && pair[1] == "900")
                        }));
                    }
                    messages.push(message);
                    if finished {
                        return Ok::<_, anyhow::Error>(messages);
                    }
                }
            })
            .await?
        })?;
        let (sender, saved_receiver) = tokio::sync::mpsc::channel(messages.len().max(1));
        for message in messages {
            sender.try_send(message)?;
        }
        view.update(cx, |view, cx| {
            view.messages = Some(saved_receiver);
            view.poll(cx);
        });
        let snapshot = catalog.snapshot()?;
        assert_eq!(snapshot.instances.len(), 1);
        assert_eq!(snapshot.profiles.len(), 2);
        assert!(
            snapshot
                .profiles
                .iter()
                .any(|profile| profile.id == original.id && profile.name == original.name)
        );
        assert!(!view.read_with(cx, |view, _| view.busy));
        assert!(view.read_with(cx, |view, _| view.form.is_none()));
        assert_ne!(
            view.read_with(cx, |view, _| view.selected_profile.clone()),
            Some(original.id)
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn profile_error_expands_advanced_settings_and_reveals_the_invalid_input(cx: &mut TestAppContext) {
    for width in [960., 360.] {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.snapshot = ready_snapshot();
            view.snapshot.profile_mode = ProfileMode::Separated;
            view.begin_game_wizard(cx);
            view.shell.wizard.step = WizardStep::Profile;
            if let Some(profile) = &view.shell.wizard.profile {
                profile
                    .height
                    .update(cx, |field, cx| field.set_value("not-a-number", cx));
            }
            view
        });
        cx.simulate_resize(size(px(width), px(540.)));
        click(cx, "wizard-next");
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.wizard.step),
            WizardStep::Profile
        );
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .wizard
                .profile
                .as_ref()
                .is_some_and(|profile| profile.options)
        }));
        let input = crate::test_support::control_bounds(cx, "wizard-field-Height");
        let body = crate::test_support::control_bounds(cx, "wizard-body");
        assert!(
            input.zip(body).is_some_and(
                |(input, body)| input.top() >= body.top() && input.bottom() <= body.bottom()
            ),
            "invalid input must be visible at {width}: {input:?}, {body:?}"
        );
        cx.update(|window, cx| {
            let profile = view.read(cx).shell.wizard.profile.as_ref();
            assert!(profile.is_some_and(|profile| {
                profile.height.read(cx).focus_handle(cx).is_focused(window)
                    && profile.height.read(cx).value() == "not-a-number"
            }));
        });
        assert_eq!(
            view.read_with(cx, |view, _| view.snapshot.profiles.len()),
            1
        );
    }
}

#[gpui::test]
fn configuration_done_validates_its_own_fields_and_cancel_restores_editable_entities(
    cx: &mut TestAppContext,
) {
    let java = std::env::temp_dir().join(format!("java{}", std::env::consts::EXE_SUFFIX));
    let java = java.to_string_lossy().into_owned();
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.begin_game_wizard(cx);
        view.open_game_wizard_page(Some(WizardPage::Location), cx);
        if let Some(Form::Instance(form)) = &view.form {
            form.name.update(cx, |field, cx| field.set_value("", cx));
        }
        if let Some(profile) = &mut view.shell.wizard.profile {
            profile.managed = false;
            profile
                .java
                .update(cx, |field, cx| field.set_value(&java, cx));
            profile
                .width
                .update(cx, |field, cx| field.set_value("invalid", cx));
        }
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    click(cx, "wizard-next");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.page),
        Some(WizardPage::Location)
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.error_field),
        Some(super::WizardField::Width)
    );
    view.update(cx, |view, cx| {
        if let Some(profile) = &mut view.shell.wizard.profile {
            profile
                .width
                .update(cx, |field, cx| field.set_value("1600", cx));
            profile
                .height
                .update(cx, |field, cx| field.set_value("900", cx));
            profile.fullscreen = true;
        }
    });
    click(cx, "wizard-next");
    view.update(cx, |view, cx| {
        assert_eq!(
            view.shell.wizard.page, None,
            "section Done must not validate the external blank game name"
        );
        assert_eq!(
            view.wizard_configuration_summary(cx),
            ("Custom Java".to_owned(), "Fullscreen".to_owned())
        );
        let original = view.shell.wizard.profile.as_ref().map(|profile| {
            (
                profile.width.entity_id(),
                profile.width.read(cx).value(),
                profile.java.read(cx).value(),
            )
        });
        view.open_game_wizard_page(Some(WizardPage::Location), cx);
        if let Some(profile) = &mut view.shell.wizard.profile {
            profile
                .width
                .update(cx, |field, cx| field.set_value("1920", cx));
            profile
                .java
                .update(cx, |field, cx| field.set_value("/different/java", cx));
            profile.fullscreen = false;
        }
        view.cancel_component_draft(cx);
        assert_eq!(
            view.shell.wizard.profile.as_ref().map(|profile| (
                profile.width.entity_id(),
                profile.width.read(cx).value(),
                profile.java.read(cx).value()
            )),
            original
        );
        assert!(
            view.shell
                .wizard
                .profile
                .as_ref()
                .is_some_and(|profile| profile.fullscreen)
        );
        assert_eq!(view.snapshot.instances.len(), 1);
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(profile) = &view.shell.wizard.profile {
                profile.width.update(cx, |field, cx| {
                    field.replace_text_in_range(Some(0..4), "1280", window, cx);
                });
                assert_eq!(
                    profile.width.read(cx).value(),
                    "1280",
                    "cancel must restore the editable original, never a frozen outgoing entity"
                );
            }
        });
    });
}

#[gpui::test]
fn server_address_can_be_saved_while_unknown_quick_play_cannot_be_enabled(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.snapshot.profile_mode = ProfileMode::Separated;
        view.begin_game_wizard(cx);
        view.wizard_kind(false, cx);
        view.shell.wizard.step = WizardStep::Profile;
        if let Some(profile) = &view.shell.wizard.profile {
            profile.server.update(cx, |field, cx| {
                field.set_value("play.example.test:25565", cx);
            });
        }
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    click(cx, "wizard-profile-connect");
    view.read_with(cx, |view, cx| {
        assert!(
            !view
                .shell
                .wizard
                .profile
                .as_ref()
                .is_some_and(|profile| profile.connect_on_launch)
        );
        let values = view
            .shell
            .wizard
            .profile
            .as_ref()
            .and_then(|profile| profile.values(cx).ok());
        assert!(
            values.is_some_and(|values| values.server.address == "play.example.test:25565"
                && !values.server.connect_on_launch)
        );
    });
    view.update(cx, |view, cx| {
        if let Some(profile) = &mut view.shell.wizard.profile {
            profile.connect_on_launch = true;
        }
        cx.notify();
    });
    click(cx, "wizard-next");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.step),
        WizardStep::Profile
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.error_field),
        Some(super::WizardField::Server)
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.snapshot.profiles.len()),
        1
    );
}

fn loader_build(version: &str, stable: bool) -> block_launcher::install::fabric::LoaderVersion {
    block_launcher::install::fabric::LoaderVersion {
        version: version.into(),
        stable,
    }
}

#[gpui::test]
fn loader_selection_is_a_local_draft_and_cancel_rejects_the_cancelled_catalog_result(
    cx: &mut TestAppContext,
) {
    use block_launcher::catalog::LoaderKind;
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.begin_game_wizard(cx);
        view.visual_only = true;
        view
    });
    view.update(cx, |view, cx| {
        view.open_game_wizard_page(Some(WizardPage::Loader), cx);
        view.choose_wizard_loader(LoaderKind::Fabric, cx);
        let request = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        assert!(request.is_some());
        if let Some(request) = request {
            view.receive_loader_versions(request, Ok(vec![loader_build("0.9.0", true), loader_build("0.20.0-beta.1", false), loader_build("0.19.5", true)]), cx);
        }
        assert!(view.wizard_loader_issue(cx).is_none());
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader_version.as_deref() == Some("0.19.5")));
        view.choose_wizard_loader_build("0.9.0".into(), cx);
        view.advance_game_wizard(cx);
        assert!(view.shell.wizard.page.is_none());
        assert!(view.snapshot.instances.is_empty());
        view.open_game_wizard_page(Some(WizardPage::Loader), cx);
        view.choose_wizard_loader(LoaderKind::Quilt, cx);
        let cancelled = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        let cancel = view.shell.wizard.loader_request.as_ref().map(|request| request.0.clone());
        view.cancel_component_draft(cx);
        assert!(cancel.is_some_and(|cancel| cancel.is_cancelled()));
        if let Some(request) = cancelled { view.receive_loader_versions(request, Ok(vec![loader_build("0.29.2", true)]), cx); }
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader == LoaderKind::Fabric && form.loader_version.as_deref() == Some("0.9.0")));
        assert!(view.wizard_loader_issue(cx).is_none());
        assert!(view.snapshot.instances.is_empty());
    });
}

#[gpui::test]
fn changing_minecraft_keeps_the_loader_but_cancels_old_mappings_and_rejects_late_results(
    cx: &mut TestAppContext,
) {
    use block_launcher::catalog::LoaderKind;
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.begin_game_wizard(cx);
        view.visual_only = true;
        view
    });
    view.update(cx, |view, cx| {
        view.set_wizard_version("1.20.6".into(), cx);
        view.choose_wizard_loader(LoaderKind::Fabric, cx);
        let old = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        let cancel = view.shell.wizard.loader_request.as_ref().map(|request| request.0.clone());
        view.set_wizard_version("1.21.1".into(), cx);
        assert!(cancel.is_some_and(|cancel| cancel.is_cancelled()));
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader == LoaderKind::Fabric && form.loader_version.is_none()));
        if let Some(request) = old { view.receive_loader_versions(request, Ok(vec![loader_build("0.15.0", true)]), cx); }
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader_version.is_none()));
        let current = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        assert_ne!(old, current);
        if let Some(request) = current { view.receive_loader_versions(request, Ok(vec![loader_build("0.16.14", true)]), cx); }
        assert!(view.wizard_loader_issue(cx).is_none());
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader_version.as_deref() == Some("0.16.14") && form.version.read(cx).value() == "1.21.1"));
        assert!(view.shell.wizard.version_notice.is_some());
        view.choose_wizard_loader(LoaderKind::Quilt, cx);
        assert!(view.shell.wizard.version_notice.is_none());
        assert!(view.snapshot.instances.is_empty());
    });
}

#[gpui::test]
fn loader_empty_error_retry_and_delisted_build_states_cannot_be_committed_as_vanilla(
    cx: &mut TestAppContext,
) {
    use block_launcher::catalog::LoaderKind;
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.begin_onboarding(cx);
        view.visual_only = true;
        view.open_game_wizard_page(Some(WizardPage::Loader), cx);
        view.choose_wizard_loader(LoaderKind::Quilt, cx);
        view
    });
    view.update(cx, |view, cx| {
        assert!(view.wizard_loader_issue(cx).is_some());
        let request = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        if let Some(request) = request { view.receive_loader_versions(request, Err("Fixture throttle".into()), cx); }
        assert!(!view.validate_instance_draft(cx));
        view.advance_game_wizard(cx);
        assert_eq!(view.shell.wizard.page, Some(WizardPage::Loader));
        view.load_wizard_loader_versions(true, cx);
        let next = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        if let Some(request) = request { view.receive_loader_versions(request, Ok(vec![loader_build("0.1.0", true)]), cx); }
        if let Some(request) = next { view.receive_loader_versions(request, Ok(Vec::new()), cx); }
        assert!(!view.validate_instance_draft(cx));
        view.choose_wizard_loader_build("not-listed".into(), cx);
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader_version.is_none()));
        view.load_wizard_loader_versions(true, cx);
        let loaded = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        if let Some(request) = loaded { view.receive_loader_versions(request, Ok(vec![loader_build("0.29.2", true)]), cx); }
        assert!(view.validate_instance_draft(cx));
        view.load_wizard_loader_versions(true, cx);
        let refresh = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        if let Some(request) = refresh { view.receive_loader_versions(request, Ok(vec![loader_build("0.30.0", true)]), cx); }
        assert!(!view.validate_instance_draft(cx));
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader == LoaderKind::Quilt && form.loader_version.is_none()));
        view.choose_wizard_loader_build("0.30.0".into(), cx);
        assert!(view.validate_instance_draft(cx));
        assert!(view.snapshot.instances.is_empty());
    });
}
